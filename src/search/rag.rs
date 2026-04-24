/// LinearRAG two-stage retrieval pipeline.
///
/// Stage 1 — Entity activation: build activation scores for symbols (entities)
/// by propagating query similarity through the chunk↔symbol mention graph.
///
/// Stage 2 — Personalised PageRank: score all candidate chunks using PPR with
/// entity activations as the personalisation vector, combined with KNN distance.
///
/// Reference: LinearRAG (arxiv:2510.10114).
use std::collections::{HashMap, HashSet};

use anyhow::Result;

use crate::{
    indexer::pagerank::compute_personalised_pagerank, search::SearchResult, storage::Database,
};

/// Tuning parameters (start values from the issue spec; expose as args if the
/// evaluation suggests improvements).
const ENTITY_THRESHOLD: f32 = 0.25; // δ — minimum activation to keep a symbol
const ACTIVATION_ITERATIONS: usize = 3; // t
const PPR_ITERATIONS: usize = 20;
const PPR_DAMPING: f32 = 0.85;
const LAMBDA: f32 = 0.5; // weight of KNN similarity vs PPR score

/// Run LinearRAG retrieval and return up to `limit` results.
///
/// `query_vec` is the pre-computed query embedding. `query` is the raw string,
/// used for the initial hybrid search that seeds the entity activation.
pub fn linearrag_search(
    db: &Database,
    query_vec: &[f32],
    query: &str,
    limit: usize,
) -> Result<Vec<SearchResult>> {
    // ── Stage 0: Initial KNN / hybrid pool ───────────────────────────────────
    // Over-fetch so we have enough symbols to seed entity activation.
    let pool_size = (limit * 5).clamp(50, 300);
    let knn_results = db
        .search_hybrid(query, query_vec, pool_size)
        .unwrap_or_default();

    if knn_results.is_empty() {
        return Ok(vec![]);
    }

    // Normalise distances to [0, 1] relative to this pool's range so that the
    // similarity function is scale-independent (works for both cosine distance
    // from search_similar and RRF-inverse scores from search_hybrid).
    let min_d = knn_results
        .iter()
        .map(|r| r.distance)
        .fold(f32::INFINITY, f32::min);
    let max_d = knn_results
        .iter()
        .map(|r| r.distance)
        .fold(f32::NEG_INFINITY, f32::max);
    let dist_range = (max_d - min_d).max(1e-6);

    // Map chunk_id → normalised similarity in [0, 1] (higher = more similar).
    let knn_by_id: HashMap<i64, f32> = knn_results
        .iter()
        .map(|r| (r.chunk_id, norm_sim(r.distance, min_d, dist_range)))
        .collect();
    let knn_ids: Vec<i64> = knn_results.iter().map(|r| r.chunk_id).collect();

    // ── Stage 1: Entity activation ────────────────────────────────────────────
    // For each symbol mentioned by KNN chunks, set activation = max normalised
    // similarity of chunks containing that symbol.
    let chunk_mentions = db.mention_edges_for_chunks(&knn_ids)?;

    let mut aq: HashMap<String, f32> = HashMap::new();
    for (chunk_id, symbols) in &chunk_mentions {
        let sim = knn_by_id.get(chunk_id).copied().unwrap_or(0.0);
        if sim > ENTITY_THRESHOLD {
            for sym in symbols {
                let e = aq.entry(sym.clone()).or_insert(0.0);
                *e = e.max(sim);
            }
        }
    }

    // Propagate: for each iteration, look up KNN-resident chunks containing
    // active symbols and let their similarity update entity activations.
    for _ in 0..ACTIVATION_ITERATIONS {
        if aq.is_empty() {
            break;
        }
        let sym_refs: Vec<&str> = aq.keys().map(String::as_str).collect();
        let sym_to_chunks = db.chunks_mentioning_symbols(&sym_refs)?;

        let mut updated = false;
        for (sym, chunk_ids) in &sym_to_chunks {
            for &cid in chunk_ids {
                if let Some(&sim) = knn_by_id.get(&cid)
                    && sim > ENTITY_THRESHOLD
                {
                    let e = aq.entry(sym.clone()).or_insert(0.0);
                    let prev = *e;
                    *e = e.max(sim);
                    if *e > prev {
                        updated = true;
                    }
                }
            }
        }
        aq.retain(|_, v| *v >= ENTITY_THRESHOLD);

        if !updated {
            break;
        }
    }

    // If no entities activated, fall back to KNN.
    if aq.is_empty() {
        return Ok(knn_results.into_iter().take(limit).collect());
    }

    // ── Stage 2: Personalised PageRank ────────────────────────────────────────
    // Find ALL chunks mentioning any activated symbol (candidate expansion).
    let active_syms: Vec<&str> = aq.keys().map(String::as_str).collect();
    let sym_to_chunks = db.chunks_mentioning_symbols(&active_syms)?;

    // Build the bipartite entity↔chunk graph as undirected edge pairs.
    // Nodes are named "c:<id>" for chunks and "<symbol>" for entities.
    let mut bipartite: Vec<(String, String)> = Vec::new();
    let mut all_candidate_ids: HashSet<i64> = knn_ids.iter().copied().collect();

    // Edges from KNN chunks (we have their mention maps already).
    for (chunk_id, symbols) in &chunk_mentions {
        for sym in symbols {
            if aq.contains_key(sym) {
                let cn = chunk_node(*chunk_id);
                bipartite.push((cn.clone(), sym.clone()));
                bipartite.push((sym.clone(), cn));
            }
        }
    }

    // Edges from expanded chunks (those not in the initial KNN pool).
    for (sym, chunk_ids) in &sym_to_chunks {
        for &cid in chunk_ids {
            all_candidate_ids.insert(cid);
            let cn = chunk_node(cid);
            bipartite.push((cn.clone(), sym.clone()));
            bipartite.push((sym.clone(), cn));
        }
    }

    let ppr = compute_personalised_pagerank(&bipartite, &aq, PPR_ITERATIONS, PPR_DAMPING);

    // ── Combine scores ────────────────────────────────────────────────────────
    // Fetch full SearchResult data for expanded (non-KNN) chunks.
    let extra_ids: Vec<i64> = all_candidate_ids
        .iter()
        .filter(|id| !knn_by_id.contains_key(id))
        .copied()
        .collect();
    let extra_results = if extra_ids.is_empty() {
        vec![]
    } else {
        db.chunks_by_ids(&extra_ids).unwrap_or_default()
    };

    // Normalise PPR scores for mixing: find max chunk PPR score.
    let max_ppr: f32 = all_candidate_ids
        .iter()
        .filter_map(|id| ppr.get(&chunk_node(*id)))
        .cloned()
        .fold(f32::NEG_INFINITY, f32::max);
    let ppr_norm = if max_ppr > 0.0 { max_ppr } else { 1.0 };

    let mut scored: Vec<(f32, SearchResult)> = Vec::new();

    // Score KNN results (knn_by_id already holds normalised similarity).
    for r in knn_results {
        let knn_sim = knn_by_id.get(&r.chunk_id).copied().unwrap_or(0.0);
        let ppr_raw = ppr.get(&chunk_node(r.chunk_id)).copied().unwrap_or(0.0);
        let ppr_sim = ppr_raw / ppr_norm;
        let score = LAMBDA * knn_sim + (1.0 - LAMBDA) * ppr_sim;
        let mut result = r;
        result.distance = 1.0 - score; // re-express as distance (lower = better)
        scored.push((score, result));
    }

    // Score expanded results.
    for r in extra_results {
        let ppr_raw = ppr.get(&chunk_node(r.chunk_id)).copied().unwrap_or(0.0);
        let ppr_sim = ppr_raw / ppr_norm;
        let score = (1.0 - LAMBDA) * ppr_sim;
        let mut result = r;
        result.distance = 1.0 - score;
        scored.push((score, result));
    }

    // Sort descending by score; deduplicate by chunk_id.
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut seen_ids: HashSet<i64> = HashSet::new();
    let results = scored
        .into_iter()
        .filter_map(|(_, r)| {
            if seen_ids.insert(r.chunk_id) {
                Some(r)
            } else {
                None
            }
        })
        .take(limit)
        .collect();

    Ok(results)
}

/// Normalise a raw distance to a similarity in [0, 1] relative to the pool range.
/// Works for both cosine distances and RRF-inverse scores from hybrid search.
#[inline]
fn norm_sim(distance: f32, min_d: f32, range: f32) -> f32 {
    1.0 - (distance - min_d) / range
}

#[inline]
fn chunk_node(id: i64) -> String {
    format!("c:{id}")
}
