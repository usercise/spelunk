//! Structural edge extraction from source files using tree-sitter.
//!
//! Extracts `imports`, `calls`, `extends`, and `implements` edges for every
//! supported language.  The resulting [`Edge`] values are stored in the
//! `graph_edges` SQLite table and queried by `spelunk graph`.
//!
//! # Design
//! A single recursive tree walk visits every node.  Per-language helper
//! functions decide whether a given node carries an edge; the rest of the
//! traversal logic is shared.  Call edges are deduplicated per
//! (source_name, target_name) pair to keep the graph compact.

mod builtins;
mod edges;

use anyhow::Result;
use std::collections::HashSet;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EdgeKind {
    Imports,
    Calls,
    Extends,
    Implements,
}

impl std::fmt::Display for EdgeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Imports => write!(f, "imports"),
            Self::Calls => write!(f, "calls"),
            Self::Extends => write!(f, "extends"),
            Self::Implements => write!(f, "implements"),
        }
    }
}

impl EdgeKind {
    #[allow(dead_code)]
    pub fn parse(s: &str) -> Self {
        match s {
            "calls" => Self::Calls,
            "extends" => Self::Extends,
            "implements" => Self::Implements,
            _ => Self::Imports,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Edge {
    pub source_file: String,
    /// Enclosing function/class name at the point of the edge, if known.
    pub source_name: Option<String>,
    /// Imported module path, called function, or base class name.
    pub target_name: String,
    pub kind: EdgeKind,
    /// 1-based source line where the relationship appears.
    pub line: usize,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

pub struct EdgeExtractor;

impl EdgeExtractor {
    /// Extract all structural edges from `source`.
    /// Returns an empty vec on parse failure rather than an error.
    pub fn extract(source: &str, file_path: &str, language: &str) -> Result<Vec<Edge>> {
        let ts_lang = match super::parser::ts_language_pub(language) {
            Ok(l) => l,
            Err(_) => return Ok(vec![]),
        };

        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&ts_lang)?;

        let tree = match parser.parse(source, None) {
            Some(t) => t,
            None => return Ok(vec![]),
        };

        let bytes = source.as_bytes();
        let mut out = Vec::new();
        let mut seen: HashSet<(Option<String>, String, String)> = HashSet::new();

        walk(
            tree.root_node(),
            bytes,
            file_path,
            language,
            None,
            &mut out,
            &mut seen,
        );
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// Tree walker
// ---------------------------------------------------------------------------

fn walk(
    node: tree_sitter::Node<'_>,
    src: &[u8],
    file_path: &str,
    language: &str,
    enclosing: Option<&str>,
    out: &mut Vec<Edge>,
    seen: &mut HashSet<(Option<String>, String, String)>,
) {
    // Track the enclosing function/class as we descend.
    let new_scope = enclosing_scope(&node, src, language);
    let eff = new_scope.as_deref().or(enclosing);

    collect(&node, src, file_path, language, eff, out, seen);

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            walk(child, src, file_path, language, eff, out, seen);
        }
    }
}

/// If `node` introduces a named scope (function, class, …) return its name.
fn enclosing_scope(node: &tree_sitter::Node<'_>, src: &[u8], language: &str) -> Option<String> {
    let field = match (language, node.kind()) {
        ("rust", "function_item") => "name",
        ("python", "function_definition") => "name",
        ("python", "class_definition") => "name",
        ("javascript" | "typescript", "function_declaration") => "name",
        ("javascript" | "typescript", "class_declaration") => "name",
        ("go", "function_declaration") => "name",
        ("go", "method_declaration") => "name",
        ("java", "class_declaration") => "name",
        ("java", "method_declaration") => "name",
        _ => return None,
    };
    node.child_by_field_name(field)
        .and_then(|n| n.utf8_text(src).ok())
        .map(str::to_owned)
}

/// Emit edges (if any) produced by `node`, deduplicating via `seen`.
fn collect(
    node: &tree_sitter::Node<'_>,
    src: &[u8],
    file_path: &str,
    language: &str,
    enclosing: Option<&str>,
    out: &mut Vec<Edge>,
    seen: &mut HashSet<(Option<String>, String, String)>,
) {
    let line = node.start_position().row + 1;

    let candidates: Vec<(String, EdgeKind)> = match language {
        "rust" => edges::rust_edges(node, src),
        "python" => edges::python_edges(node, src),
        "javascript" | "typescript" => edges::js_edges(node, src),
        "go" => edges::go_edges(node, src),
        "java" => edges::java_edges(node, src),
        "c" | "cpp" => edges::c_edges(node, src),
        "html" => edges::html_edges(node, src),
        "css" => edges::css_edges(node, src),
        _ => vec![],
    };

    for (target, kind) in candidates {
        let key = (
            enclosing.map(str::to_owned),
            target.clone(),
            kind.to_string(),
        );
        if seen.insert(key) {
            out.push(Edge {
                source_file: file_path.to_owned(),
                source_name: enclosing.map(str::to_owned),
                target_name: target,
                kind,
                line,
            });
        }
    }
}
