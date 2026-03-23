pub mod rag;

use serde::{Deserialize, Serialize};

/// A single search result returned to the caller.
#[derive(Debug, Serialize, Deserialize)]
pub struct SearchResult {
    pub chunk_id: i64,
    pub file_path: String,
    pub language: String,
    pub node_type: String,
    pub name: Option<String>,
    pub start_line: usize,
    pub end_line: usize,
    pub content: String,
    /// Cosine distance to the query vector (lower = more similar).
    /// For graph-expanded results this is 0.0 (not meaningful).
    pub distance: f32,
    /// True when this result was added via graph traversal rather than
    /// vector similarity — only set when `--graph` is used with `search`.
    #[serde(default)]
    pub from_graph: bool,
}
