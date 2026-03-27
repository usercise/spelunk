mod ts_walker;
mod text;

use anyhow::Result;
use super::chunker::{Chunk, sliding_window};

/// All languages recognised by the indexer (tree-sitter, text, and document formats).
pub const SUPPORTED_LANGUAGES: &[&str] = &[
    "rust",
    "python",
    "javascript",
    "jsx",
    "typescript",
    "tsx",
    "go",
    "java",
    "c",
    "cpp",
    "json",
    "html",
    "css",
    "hcl",
    "sql",
    "proto",
    // text formats (sliding-window / heading-based, no tree-sitter)
    "markdown",
    "text",
    // structured text (custom parsers, no tree-sitter)
    "notebook",
    // binary document formats (docparser, no tree-sitter)
    "docx",
    "spreadsheet",
];

/// Detect language from file extension.
pub fn detect_language(path: &std::path::Path) -> Option<&'static str> {
    match path.extension()?.to_str()? {
        "rs" => Some("rust"),
        "py" => Some("python"),
        "js" | "mjs" | "cjs" => Some("javascript"),
        "jsx" => Some("jsx"),
        "ts" | "mts" => Some("typescript"),
        "tsx" => Some("tsx"),
        "go" => Some("go"),
        "java" => Some("java"),
        "c" | "h" => Some("c"),
        "cpp" | "cc" | "cxx" | "hpp" | "hxx" => Some("cpp"),
        "json" => Some("json"),
        "html" | "htm" => Some("html"),
        "css" => Some("css"),
        "tf" | "hcl" => Some("hcl"),
        "sql" | "sequel" => Some("sql"),
        "proto" => Some("proto"),
        _ => None,
    }
}

pub(crate) fn ts_language_pub(name: &str) -> Result<tree_sitter::Language> {
    ts_walker::ts_language(name)
}

/// Detect text-format languages (markdown, plain text, notebooks) from file path.
/// These are handled without tree-sitter.
pub fn detect_text_language(path: &std::path::Path) -> Option<&'static str> {
    // Check extension first
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        return match ext.to_lowercase().as_str() {
            "md" | "mdx" | "markdown" => Some("markdown"),
            // R Markdown / Quarto documents are markdown with fenced code blocks.
            "rmd" | "qmd" => Some("markdown"),
            "txt" | "rst" | "adoc" | "asciidoc" => Some("text"),
            // Jupyter notebooks: custom JSON-based parser.
            "ipynb" => Some("notebook"),
            _ => None,
        };
    }
    // Extensionless files: README, CHANGELOG, etc.
    let name = path.file_name()?.to_str()?.to_uppercase();
    match name.as_str() {
        "README" | "CHANGELOG" | "CHANGES" | "CONTRIBUTING" | "NOTICE" | "AUTHORS" | "HISTORY" => {
            Some("text")
        }
        _ => None,
    }
}

/// Detect binary document formats (DOCX, spreadsheets) from file extension.
/// These are handled by `docparser` — they cannot be read with `read_to_string`.
pub fn detect_doc_language(path: &std::path::Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_lowercase().as_str() {
        "docx" => Some("docx"),
        "xlsx" | "xls" | "ods" => Some("spreadsheet"),
        _ => None,
    }
}

/// Return true if the file appears to be binary (contains null bytes in the
/// first 512 bytes). Used to skip compiled or binary assets.
pub fn is_binary_file(path: &std::path::Path) -> bool {
    use std::io::Read;
    if let Ok(mut f) = std::fs::File::open(path) {
        let mut buf = [0u8; 512];
        if let Ok(n) = f.read(&mut buf) {
            return buf[..n].contains(&0u8);
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

pub struct SourceParser;

impl SourceParser {
    /// Parse `source` and return semantic chunks.
    /// Falls back to sliding-window if parsing fails or yields nothing.
    pub fn parse(source: &str, file_path: &str, language: &str) -> Result<Vec<Chunk>> {
        // Text formats bypass tree-sitter entirely.
        if language == "markdown" {
            return Ok(text::parse_markdown(source, file_path));
        }
        if language == "text" {
            return Ok(sliding_window(source, file_path, language, 120, 15));
        }
        if language == "notebook" {
            return Ok(text::parse_notebook(source, file_path));
        }

        let ts_lang = ts_walker::ts_language(language)?;
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&ts_lang)?;

        let tree = parser
            .parse(source, None)
            .ok_or_else(|| anyhow::anyhow!("tree-sitter produced no parse tree for {file_path}"))?;

        let bytes = source.as_bytes();
        let specs = ts_walker::node_specs(language);
        let mut chunks = Vec::new();

        ts_walker::walk_node(
            tree.root_node(),
            bytes,
            file_path,
            language,
            &specs,
            None,
            &mut chunks,
        );

        if chunks.is_empty() {
            tracing::debug!("{file_path}: no semantic nodes found, using sliding window");
            // 120-line window fits comfortably in EmbeddingGemma's 2048-token budget.
            return Ok(sliding_window(source, file_path, language, 120, 15));
        }

        Ok(chunks)
    }
}
