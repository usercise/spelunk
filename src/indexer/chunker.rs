use serde::{Deserialize, Serialize};

/// The semantic kind of an extracted code chunk.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChunkKind {
    Function,
    Method,
    Struct,
    Class,
    Enum,
    Interface,
    Impl,
    Trait,
    Module,
    Constant,
    TypeAlias,
    /// Fallback: plain line range (unsupported language or oversized node)
    Verbatim,
}

impl std::fmt::Display for ChunkKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = serde_json::to_value(self)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| "unknown".to_string());
        write!(f, "{s}")
    }
}

/// A single unit of source code to be embedded and stored.
#[derive(Debug, Clone)]
pub struct Chunk {
    pub file_path: String,
    pub language: String,
    pub kind: ChunkKind,
    /// Symbol name, if the node has one (e.g. function or struct name).
    pub name: Option<String>,
    pub start_line: usize,
    pub end_line: usize,
    pub content: String,
    /// Optional docstring / leading comment.
    pub docstring: Option<String>,
    /// Enclosing scope (e.g. `impl MyStruct` for a method).
    pub parent_scope: Option<String>,
}

impl Chunk {
    /// The text that gets passed to the embedding model.
    /// Follows the EmbeddingGemma input format from the cookbook notebook.
    pub fn embedding_text(&self) -> String {
        match &self.docstring {
            Some(doc) => format!("Represent this code: {doc}\n{}", self.content),
            None => format!("Represent this code: {}", self.content),
        }
    }
}

/// Split `source` into chunks using a sliding window (fallback for
/// languages without a tree-sitter grammar or for files that failed parsing).
pub fn sliding_window(
    source: &str,
    file_path: &str,
    language: &str,
    window_lines: usize,
    overlap_lines: usize,
) -> Vec<Chunk> {
    let lines: Vec<&str> = source.lines().collect();
    let step = window_lines.saturating_sub(overlap_lines).max(1);
    let mut chunks = Vec::new();
    let mut start = 0;

    while start < lines.len() {
        let end = (start + window_lines).min(lines.len());
        chunks.push(Chunk {
            file_path: file_path.to_string(),
            language: language.to_string(),
            kind: ChunkKind::Verbatim,
            name: None,
            start_line: start + 1,
            end_line: end,
            content: lines[start..end].join("\n"),
            docstring: None,
            parent_scope: None,
        });
        if end == lines.len() {
            break;
        }
        start += step;
    }

    chunks
}
