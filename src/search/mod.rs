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
    pub distance: f32,
}
