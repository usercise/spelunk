pub mod explore;
pub mod tokens;
pub mod tools;

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
    /// Spec files that govern the file this result came from (via spec_links).
    /// Empty when no specs are linked to the result's file path.
    #[serde(default)]
    pub governing_specs: Vec<String>,
    /// Estimated token count for this chunk's content (chars/4 heuristic).
    #[serde(default)]
    pub token_count: usize,
    /// Name of the linked project this result came from (None = primary project).
    #[serde(default)]
    pub project_name: Option<String>,
    /// Root path of the linked project this result came from (None = primary project).
    #[serde(default)]
    pub project_path: Option<String>,
    /// LLM-generated one-sentence summary of this chunk (None if not yet generated).
    #[serde(default)]
    pub summary: Option<String>,
}
