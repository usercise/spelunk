use anyhow::{Context, Result};

use super::super::SearchArgs;
use super::ui::{print_results_text, spinner};
use crate::{
    config::{Config, resolve_db},
    embeddings::{EmbeddingBackend as _, vec_to_blob},
    registry::Registry,
    search::SearchResult,
    storage::Database,
};

pub async fn search(args: SearchArgs, cfg: Config) -> Result<()> {
    let (db_path, dep_dbs) = resolve_project_and_deps(args.db.as_ref(), &cfg)?;

    if !args.no_stale_check {
        maybe_warn_stale(&db_path);
    }

    let mode = args.mode.as_str();

    let mut results = if mode == "text" {
        // Text mode: FTS5 only, no embedding model required.
        let sp = spinner("Searching (text)…");
        let db = Database::open(&db_path)?;
        let res = db
            .search_text(&args.query, args.limit.min(100))
            .unwrap_or_default();
        sp.finish_and_clear();
        res
    } else {
        // semantic or hybrid: need an embedding.
        let sp = spinner("Loading model…");
        let embedder = crate::backends::ActiveEmbedder::load(&cfg)
            .await
            .with_context(|| format!("loading embedding model '{}'", cfg.embedding_model))?;

        sp.set_message("Embedding query…");
        let query_text = format!("task: code retrieval | query: {}", args.query);
        let vecs = embedder.embed(&[&query_text]).await?;
        let query_vec = vecs.first().context("no embedding returned")?.clone();
        let query_blob = vec_to_blob(&query_vec);

        sp.set_message("Searching…");
        let res = if mode == "hybrid" {
            search_all_dbs_hybrid(
                &db_path,
                &dep_dbs,
                &args.query,
                &query_vec,
                args.limit.min(100),
            )?
        } else {
            // semantic
            search_all_dbs(&db_path, &dep_dbs, &query_blob, args.limit.min(100))?
        };
        sp.finish_and_clear();
        res
    };

    if results.is_empty() {
        println!("No results found. Make sure the index has embeddings (`spelunk index <path>`).");
        return Ok(());
    }

    // ── Graph-aware enrichment (primary DB only) ──────────────────────────────
    if args.graph
        && let Ok(primary_db) = Database::open(&db_path)
    {
        let seen_ids: std::collections::HashSet<i64> = results.iter().map(|r| r.chunk_id).collect();
        let names: Vec<&str> = results.iter().filter_map(|r| r.name.as_deref()).collect();

        if !names.is_empty()
            && let Ok(neighbor_ids) = primary_db.graph_neighbor_chunks(&names)
        {
            let new_ids: Vec<i64> = neighbor_ids
                .into_iter()
                .filter(|id| !seen_ids.contains(id))
                .take(args.graph_limit)
                .collect();

            if !new_ids.is_empty()
                && let Ok(mut extra) = primary_db.chunks_by_ids(&new_ids)
            {
                for r in &mut extra {
                    r.from_graph = true;
                }
                results.extend(extra);
            }
        }
    }

    match crate::utils::effective_format(&args.format) {
        "json" => println!("{}", serde_json::to_string_pretty(&results)?),
        _ => print_results_text(&results),
    }

    Ok(())
}

/// Emit a staleness warning to stderr if the index appears out of date.
/// Silently skips if the DB doesn't exist or the probe returns an error.
pub(crate) fn maybe_warn_stale(db_path: &std::path::Path) {
    if !db_path.exists() {
        return;
    }
    if let Ok(db) = Database::open(db_path)
        && let Ok(report) = db.sample_staleness_check(20)
        && report.stale > 0
    {
        eprintln!(
            "warning: index may be stale ({}/{} sampled files changed). \
             Run `spelunk index .` to refresh.",
            report.stale, report.sampled
        );
    }
}

/// Resolve the primary DB path and any dep DB paths via the registry.
/// Falls back to `resolve_db` if the registry can't find a project.
pub(crate) fn resolve_project_and_deps(
    explicit_db: Option<&std::path::PathBuf>,
    cfg: &Config,
) -> Result<(std::path::PathBuf, Vec<std::path::PathBuf>)> {
    // Explicit --db skips registry entirely.
    if let Some(p) = explicit_db {
        if !p.exists() {
            anyhow::bail!(
                "Database not found at '{}'. Run `spelunk index <path>` first.",
                p.display()
            );
        }
        return Ok((p.clone(), vec![]));
    }

    // Try registry first.
    if let Ok(reg) = Registry::open()
        && let Ok(cwd) = std::env::current_dir()
        && let Ok(Some(project)) = reg.find_project_for_path(&cwd)
        && project.db_path.exists()
    {
        let deps = reg
            .get_deps(project.id)
            .unwrap_or_default()
            .into_iter()
            .map(|d| d.db_path)
            .filter(|p| p.exists())
            .collect();
        return Ok((project.db_path, deps));
    }

    // Fallback: filesystem walk-up.
    let db_path = resolve_db(None, &cfg.db_path);
    if !db_path.exists() {
        anyhow::bail!(
            "No index found (checked current directory and parents).\n\
             Run `spelunk index <path>` inside your project first."
        );
    }
    Ok((db_path, vec![]))
}

/// Search a primary DB and any dep DBs, merge results by distance, return top `limit`.
pub(crate) fn search_all_dbs(
    primary_db_path: &std::path::Path,
    dep_db_paths: &[std::path::PathBuf],
    query_blob: &[u8],
    limit: usize,
) -> Result<Vec<SearchResult>> {
    let primary_db = Database::open(primary_db_path)?;
    // Over-fetch from each DB so we have enough candidates after merging.
    let fetch = (limit * 2).max(limit + 10);
    let mut all = primary_db.search_similar(query_blob, fetch)?;

    for dep_path in dep_db_paths {
        match Database::open(dep_path) {
            Ok(dep_db) => match dep_db.search_similar(query_blob, fetch) {
                Ok(mut dep_results) => all.append(&mut dep_results),
                Err(e) => tracing::warn!("search failed on dep {}: {e}", dep_path.display()),
            },
            Err(e) => tracing::warn!("could not open dep DB {}: {e}", dep_path.display()),
        }
    }

    // Sort by distance (ascending), deduplicate by (file_path, start_line, end_line).
    all.sort_by(|a, b| {
        a.distance
            .partial_cmp(&b.distance)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut seen = std::collections::HashSet::new();
    all.retain(|r| seen.insert((r.file_path.clone(), r.start_line, r.end_line)));
    all.truncate(limit);

    // Annotate results with governing specs from the primary DB.
    if let Ok(primary_db) = Database::open(primary_db_path) {
        let file_paths: Vec<String> = all.iter().map(|r| r.file_path.clone()).collect();
        if let Ok(all_specs) = primary_db.specs_for_files(&file_paths)
            && !all_specs.is_empty()
        {
            for result in &mut all {
                if let Ok(per) = primary_db.specs_for_files(std::slice::from_ref(&result.file_path))
                {
                    result.governing_specs = per.into_iter().map(|(p, _)| p).collect();
                }
            }
        }
    }

    Ok(all)
}

/// Hybrid search across a primary DB and any dep DBs.
/// Each DB is searched independently with `search_hybrid`; results are merged
/// by deduplicating on (file_path, start_line, end_line) and re-sorting by
/// ascending `distance` (lower = better RRF score).
pub(crate) fn search_all_dbs_hybrid(
    primary_db_path: &std::path::Path,
    dep_db_paths: &[std::path::PathBuf],
    query: &str,
    embedding: &[f32],
    limit: usize,
) -> Result<Vec<SearchResult>> {
    let primary_db = Database::open(primary_db_path)?;
    let fetch = (limit * 2).max(limit + 10);
    let mut all = primary_db
        .search_hybrid(query, embedding, fetch)
        .unwrap_or_default();

    for dep_path in dep_db_paths {
        match Database::open(dep_path) {
            Ok(dep_db) => match dep_db.search_hybrid(query, embedding, fetch) {
                Ok(mut dep_results) => all.append(&mut dep_results),
                Err(e) => tracing::warn!("hybrid search failed on dep {}: {e}", dep_path.display()),
            },
            Err(e) => tracing::warn!("could not open dep DB {}: {e}", dep_path.display()),
        }
    }

    // Sort by ascending distance (lower RRF reciprocal = better score).
    all.sort_by(|a, b| {
        a.distance
            .partial_cmp(&b.distance)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut seen = std::collections::HashSet::new();
    all.retain(|r| seen.insert((r.file_path.clone(), r.start_line, r.end_line)));
    all.truncate(limit);

    // Annotate results with governing specs from the primary DB.
    if let Ok(primary_db) = Database::open(primary_db_path) {
        let file_paths: Vec<String> = all.iter().map(|r| r.file_path.clone()).collect();
        if let Ok(all_specs) = primary_db.specs_for_files(&file_paths)
            && !all_specs.is_empty()
        {
            for result in &mut all {
                if let Ok(per) = primary_db.specs_for_files(std::slice::from_ref(&result.file_path))
                {
                    result.governing_specs = per.into_iter().map(|(p, _)| p).collect();
                }
            }
        }
    }

    Ok(all)
}
