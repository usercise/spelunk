use std::collections::HashMap;

/// Compute PageRank scores for a set of nodes given directed edges.
/// Returns a map from node name -> score (scores sum to ~1.0).
pub fn compute_pagerank(
    edges: &[(String, String)], // (from, to) symbol name pairs
    iterations: usize,          // typically 20
    damping: f32,               // typically 0.85
) -> HashMap<String, f32> {
    if edges.is_empty() {
        return HashMap::new();
    }

    // 1. Collect unique nodes
    let mut nodes: Vec<String> = Vec::new();
    let mut node_index: HashMap<String, usize> = HashMap::new();

    for (from, to) in edges {
        if !node_index.contains_key(from) {
            node_index.insert(from.clone(), nodes.len());
            nodes.push(from.clone());
        }
        if !node_index.contains_key(to) {
            node_index.insert(to.clone(), nodes.len());
            nodes.push(to.clone());
        }
    }

    let n = nodes.len();
    if n == 0 {
        return HashMap::new();
    }

    // 2. Build adjacency: out_edges[i] = list of target indices
    let mut out_edges: Vec<Vec<usize>> = vec![vec![]; n];
    // in_edges[i] = list of source indices
    let mut in_edges: Vec<Vec<usize>> = vec![vec![]; n];

    for (from, to) in edges {
        let fi = node_index[from];
        let ti = node_index[to];
        if fi != ti {
            out_edges[fi].push(ti);
            in_edges[ti].push(fi);
        }
    }

    // 3. Initialise scores: 1.0 / n for each node
    let init_score = 1.0_f32 / n as f32;
    let mut scores: Vec<f32> = vec![init_score; n];

    let base = (1.0 - damping) / n as f32;

    // 4. Iterate
    for _ in 0..iterations {
        // Dangling nodes: those with no out-edges contribute to all nodes evenly
        let dangling_sum: f32 = scores
            .iter()
            .enumerate()
            .filter(|(i, _)| out_edges[*i].is_empty())
            .map(|(_, s)| s)
            .sum();
        let dangling_contrib = damping * dangling_sum / n as f32;

        let mut new_scores: Vec<f32> = vec![base + dangling_contrib; n];

        for v in 0..n {
            for &u in &in_edges[v] {
                let out_deg = out_edges[u].len() as f32;
                new_scores[v] += damping * scores[u] / out_deg;
            }
        }

        scores = new_scores;
    }

    // 5. Return final scores
    nodes
        .into_iter()
        .enumerate()
        .map(|(i, name)| (name, scores[i]))
        .collect()
}
