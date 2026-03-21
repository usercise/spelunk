use anyhow::{bail, Result};
use super::chunker::Chunk;

/// All languages that have tree-sitter grammar support.
pub const SUPPORTED_LANGUAGES: &[&str] = &[
    "rust",
    "python",
    "javascript",
    "typescript",
    "go",
    "java",
    "c",
    "cpp",
];

/// Detect language from file extension.
pub fn detect_language(path: &std::path::Path) -> Option<&'static str> {
    match path.extension()?.to_str()? {
        "rs"         => Some("rust"),
        "py"         => Some("python"),
        "js" | "mjs" => Some("javascript"),
        "ts"         => Some("typescript"),
        "go"         => Some("go"),
        "java"       => Some("java"),
        "c" | "h"    => Some("c"),
        "cpp" | "cc" | "cxx" | "hpp" => Some("cpp"),
        _ => None,
    }
}

/// Returns the tree-sitter `Language` for a given language name.
fn ts_language(name: &str) -> Result<tree_sitter::Language> {
    match name {
        "rust"       => Ok(tree_sitter_rust::LANGUAGE.into()),
        "python"     => Ok(tree_sitter_python::LANGUAGE.into()),
        "javascript" => Ok(tree_sitter_javascript::LANGUAGE.into()),
        "typescript" => Ok(tree_sitter_typescript::language_typescript()),
        "go"         => Ok(tree_sitter_go::LANGUAGE.into()),
        "java"       => Ok(tree_sitter_java::LANGUAGE.into()),
        "c"          => Ok(tree_sitter_c::LANGUAGE.into()),
        "cpp"        => Ok(tree_sitter_cpp::LANGUAGE.into()),
        other        => bail!("unsupported language: {other}"),
    }
}

/// Wraps tree-sitter parsing and chunk extraction for a single source file.
pub struct SourceParser;

impl SourceParser {
    /// Parse `source` and extract semantic chunks.
    ///
    /// Falls back to sliding-window chunking on parse failure.
    pub fn parse(source: &str, file_path: &str, language: &str) -> Result<Vec<Chunk>> {
        let ts_lang = ts_language(language)?;
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&ts_lang)?;

        let tree = parser
            .parse(source, None)
            .ok_or_else(|| anyhow::anyhow!("tree-sitter failed to produce a parse tree"))?;

        let chunks = extract_chunks(&tree, source, file_path, language);
        if chunks.is_empty() {
            // Fallback: sliding window
            return Ok(super::chunker::sliding_window(
                source, file_path, language, 60, 10,
            ));
        }

        Ok(chunks)
    }
}

/// Walk the AST and extract named semantic nodes as chunks.
///
/// Phase 2: this is a skeleton — the per-language node-type lists and
/// child-traversal logic will be fleshed out in that phase.
fn extract_chunks(
    tree: &tree_sitter::Tree,
    source: &str,
    file_path: &str,
    language: &str,
) -> Vec<Chunk> {
    let _ = (tree, source, file_path, language);
    // TODO Phase 2: implement per-language cursor walk
    vec![]
}
