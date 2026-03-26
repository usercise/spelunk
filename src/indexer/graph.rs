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
    pub fn from_str(s: &str) -> Self {
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
        let mut edges = Vec::new();
        let mut seen: HashSet<(Option<String>, String, String)> = HashSet::new();

        walk(
            tree.root_node(),
            bytes,
            file_path,
            language,
            None,
            &mut edges,
            &mut seen,
        );
        Ok(edges)
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
        "rust" => rust_edges(node, src),
        "python" => python_edges(node, src),
        "javascript" | "typescript" => js_edges(node, src),
        "go" => go_edges(node, src),
        "java" => java_edges(node, src),
        "c" | "cpp" => c_edges(node, src),
        "html" => html_edges(node, src),
        "css" => css_edges(node, src),
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

// ---------------------------------------------------------------------------
// Per-language edge extractors
// Each returns Vec<(target_name, EdgeKind)> for the given node.
// ---------------------------------------------------------------------------

fn rust_edges(node: &tree_sitter::Node<'_>, src: &[u8]) -> Vec<(String, EdgeKind)> {
    let mut out = Vec::new();
    match node.kind() {
        "use_declaration" => {
            if let Ok(text) = node.utf8_text(src) {
                let path = text
                    .trim_start_matches("use ")
                    .trim_end_matches(';')
                    .trim()
                    .to_owned();
                if !path.is_empty() {
                    out.push((path, EdgeKind::Imports));
                }
            }
        }
        "call_expression" => {
            if let Some(func) = node.child_by_field_name("function") {
                match func.kind() {
                    "identifier" => {
                        if let Ok(name) = func.utf8_text(src) {
                            if !is_rust_builtin(name) {
                                out.push((name.to_owned(), EdgeKind::Calls));
                            }
                        }
                    }
                    // Type::method(…) — index the full form, the type, and the method.
                    "scoped_identifier" => {
                        if let Ok(full) = func.utf8_text(src) {
                            if !is_rust_builtin(full) {
                                out.push((full.to_owned(), EdgeKind::Calls));
                            }
                        }
                        // Emit the method name: `EdgeExtractor::extract` → `extract`
                        if let Some(name_node) = func.child_by_field_name("name") {
                            if let Ok(name) = name_node.utf8_text(src) {
                                if !is_rust_builtin(name) {
                                    out.push((name.to_owned(), EdgeKind::Calls));
                                }
                            }
                        }
                        // Emit the type/path: `EdgeExtractor::extract` → `EdgeExtractor`
                        if let Some(path_node) = func.child_by_field_name("path") {
                            if let Ok(path) = path_node.utf8_text(src) {
                                if !is_rust_builtin(path) {
                                    out.push((path.to_owned(), EdgeKind::Calls));
                                }
                            }
                        }
                    }
                    // obj.method(…) — index the method name.
                    "field_expression" => {
                        if let Some(field) = func.child_by_field_name("field") {
                            if let Ok(name) = field.utf8_text(src) {
                                if !is_rust_builtin(name) {
                                    out.push((name.to_owned(), EdgeKind::Calls));
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
    out
}

fn python_edges(node: &tree_sitter::Node<'_>, src: &[u8]) -> Vec<(String, EdgeKind)> {
    let mut out = Vec::new();
    match node.kind() {
        "import_statement" => {
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    if matches!(child.kind(), "dotted_name" | "aliased_import") {
                        let name_node = if child.kind() == "aliased_import" {
                            child.child_by_field_name("name")
                        } else {
                            Some(child)
                        };
                        if let Some(n) = name_node {
                            if let Ok(text) = n.utf8_text(src) {
                                out.push((text.to_owned(), EdgeKind::Imports));
                            }
                        }
                    }
                }
            }
        }
        "import_from_statement" => {
            if let Some(module) = node.child_by_field_name("module_name") {
                if let Ok(text) = module.utf8_text(src) {
                    out.push((text.to_owned(), EdgeKind::Imports));
                }
            } else {
                out.push((".".to_owned(), EdgeKind::Imports));
            }
        }
        "call" => {
            if let Some(func) = node.child_by_field_name("function") {
                match func.kind() {
                    "identifier" => {
                        if let Ok(name) = func.utf8_text(src) {
                            if !is_python_builtin(name) {
                                out.push((name.to_owned(), EdgeKind::Calls));
                            }
                        }
                    }
                    // obj.method(…)
                    "attribute" => {
                        if let Some(attr) = func.child_by_field_name("attribute") {
                            if let Ok(name) = attr.utf8_text(src) {
                                if !is_python_builtin(name) {
                                    out.push((name.to_owned(), EdgeKind::Calls));
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
    out
}

fn js_edges(node: &tree_sitter::Node<'_>, src: &[u8]) -> Vec<(String, EdgeKind)> {
    let mut out = Vec::new();
    match node.kind() {
        "import_statement" => {
            if let Some(source) = node.child_by_field_name("source") {
                if let Ok(text) = source.utf8_text(src) {
                    let module = text.trim_matches('"').trim_matches('\'').to_owned();
                    out.push((module, EdgeKind::Imports));
                }
            }
        }
        "call_expression" => {
            if let Some(func) = node.child_by_field_name("function") {
                match func.kind() {
                    "identifier" => {
                        if let Ok(name) = func.utf8_text(src) {
                            if !is_js_builtin(name) {
                                out.push((name.to_owned(), EdgeKind::Calls));
                            }
                        }
                    }
                    // obj.method(…)
                    "member_expression" => {
                        if let Some(prop) = func.child_by_field_name("property") {
                            if let Ok(name) = prop.utf8_text(src) {
                                if !is_js_builtin(name) {
                                    out.push((name.to_owned(), EdgeKind::Calls));
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
    out
}

fn go_edges(node: &tree_sitter::Node<'_>, src: &[u8]) -> Vec<(String, EdgeKind)> {
    let mut out = Vec::new();
    match node.kind() {
        "import_spec" => {
            if let Some(path) = node.child_by_field_name("path") {
                if let Ok(text) = path.utf8_text(src) {
                    let module = text.trim_matches('"').to_owned();
                    out.push((module, EdgeKind::Imports));
                }
            }
        }
        "call_expression" => {
            if let Some(func) = node.child_by_field_name("function") {
                match func.kind() {
                    "identifier" => {
                        if let Ok(name) = func.utf8_text(src) {
                            if !is_go_builtin(name) {
                                out.push((name.to_owned(), EdgeKind::Calls));
                            }
                        }
                    }
                    "selector_expression" => {
                        if let Ok(text) = func.utf8_text(src) {
                            out.push((text.to_owned(), EdgeKind::Calls));
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
    out
}

fn java_edges(node: &tree_sitter::Node<'_>, src: &[u8]) -> Vec<(String, EdgeKind)> {
    let mut out = Vec::new();
    match node.kind() {
        "import_declaration" => {
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    if matches!(child.kind(), "scoped_identifier" | "identifier") {
                        if let Ok(text) = child.utf8_text(src) {
                            out.push((text.to_owned(), EdgeKind::Imports));
                        }
                        break;
                    }
                }
            }
        }
        "class_declaration" => {
            if let Some(superclass) = node.child_by_field_name("superclass") {
                if let Ok(text) = superclass.utf8_text(src) {
                    let name = text.trim_start_matches("extends").trim().to_owned();
                    if !name.is_empty() {
                        out.push((name, EdgeKind::Extends));
                    }
                }
            }
            if let Some(interfaces) = node.child_by_field_name("interfaces") {
                if let Ok(text) = interfaces.utf8_text(src) {
                    for name in text.trim_start_matches("implements").trim().split(',') {
                        let n = name.trim().to_owned();
                        if !n.is_empty() {
                            out.push((n, EdgeKind::Implements));
                        }
                    }
                }
            }
        }
        "method_invocation" => {
            if let Some(name) = node.child_by_field_name("name") {
                if let Ok(text) = name.utf8_text(src) {
                    out.push((text.to_owned(), EdgeKind::Calls));
                }
            }
        }
        _ => {}
    }
    out
}

fn c_edges(node: &tree_sitter::Node<'_>, src: &[u8]) -> Vec<(String, EdgeKind)> {
    let mut out = Vec::new();
    match node.kind() {
        "preproc_include" => {
            if let Some(path) = node.child_by_field_name("path") {
                if let Ok(text) = path.utf8_text(src) {
                    let module = text
                        .trim_matches('"')
                        .trim_start_matches('<')
                        .trim_end_matches('>')
                        .to_owned();
                    out.push((module, EdgeKind::Imports));
                }
            }
        }
        "call_expression" => {
            if let Some(func) = node.child_by_field_name("function") {
                if func.kind() == "identifier" {
                    if let Ok(name) = func.utf8_text(src) {
                        if !is_c_builtin(name) {
                            out.push((name.to_owned(), EdgeKind::Calls));
                        }
                    }
                }
            }
        }
        _ => {}
    }
    out
}

fn html_edges(node: &tree_sitter::Node<'_>, src: &[u8]) -> Vec<(String, EdgeKind)> {
    let mut out = Vec::new();
    // tree-sitter-html uses child kinds `attribute_name` / `attribute_value`,
    // not named fields.  Walk the `attribute` node's children directly.
    if node.kind() == "attribute" {
        let mut attr_name = "";
        let mut attr_value = "";

        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                match child.kind() {
                    "attribute_name" => {
                        attr_name = child.utf8_text(src).unwrap_or("");
                    }
                    "attribute_value" | "quoted_attribute_value" => {
                        attr_value = child.utf8_text(src).unwrap_or("");
                    }
                    _ => {}
                }
            }
        }

        if matches!(attr_name, "src" | "href") {
            let path = attr_value.trim_matches('"').trim_matches('\'').to_owned();
            if !path.is_empty() && !path.starts_with('#') && !path.starts_with("data:") {
                out.push((path, EdgeKind::Imports));
            }
        }
    }
    out
}

fn css_edges(node: &tree_sitter::Node<'_>, src: &[u8]) -> Vec<(String, EdgeKind)> {
    let mut out = Vec::new();
    // @import "file.css" or @import url("file.css")
    if node.kind() == "import_statement" {
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                if matches!(child.kind(), "string_value" | "call_expression") {
                    if let Ok(text) = child.utf8_text(src) {
                        let path = text
                            .trim_start_matches("url(")
                            .trim_end_matches(')')
                            .trim_matches('"')
                            .trim_matches('\'')
                            .to_owned();
                        if !path.is_empty() {
                            out.push((path, EdgeKind::Imports));
                        }
                    }
                    break;
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Builtin filters — skip very common standard-library symbols
// ---------------------------------------------------------------------------

fn is_rust_builtin(name: &str) -> bool {
    matches!(
        name,
        "Ok" | "Err"
            | "Some"
            | "None"
            | "Box"
            | "Vec"
            | "String"
            | "Default"
            | "From"
            | "Into"
            | "Clone"
            | "Drop"
    )
}

fn is_python_builtin(name: &str) -> bool {
    matches!(
        name,
        "print"
            | "len"
            | "range"
            | "enumerate"
            | "zip"
            | "map"
            | "filter"
            | "sorted"
            | "reversed"
            | "list"
            | "dict"
            | "set"
            | "tuple"
            | "str"
            | "int"
            | "float"
            | "bool"
            | "type"
            | "isinstance"
            | "hasattr"
            | "getattr"
            | "setattr"
            | "super"
            | "open"
            | "input"
            | "repr"
            | "abs"
            | "max"
            | "min"
            | "sum"
            | "any"
            | "all"
            | "iter"
            | "next"
            | "id"
            | "hash"
    )
}

fn is_js_builtin(name: &str) -> bool {
    matches!(
        name,
        "require"
            | "import"
            | "console"
            | "setTimeout"
            | "setInterval"
            | "clearTimeout"
            | "clearInterval"
            | "Promise"
            | "Array"
            | "Object"
            | "String"
            | "Number"
            | "Boolean"
            | "Error"
            | "Map"
            | "Set"
            | "JSON"
            | "Math"
            | "Date"
            | "Symbol"
            | "parseInt"
            | "parseFloat"
            | "isNaN"
            | "fetch"
    )
}

fn is_go_builtin(name: &str) -> bool {
    matches!(
        name,
        "make"
            | "new"
            | "len"
            | "cap"
            | "append"
            | "copy"
            | "delete"
            | "close"
            | "panic"
            | "recover"
            | "print"
            | "println"
    )
}

fn is_c_builtin(name: &str) -> bool {
    matches!(
        name,
        "printf"
            | "fprintf"
            | "sprintf"
            | "snprintf"
            | "scanf"
            | "fscanf"
            | "malloc"
            | "calloc"
            | "realloc"
            | "free"
            | "memcpy"
            | "memmove"
            | "memset"
            | "memcmp"
            | "strlen"
            | "strcpy"
            | "strncpy"
            | "strcmp"
            | "strncmp"
            | "fopen"
            | "fclose"
            | "fread"
            | "fwrite"
            | "fgets"
            | "fputs"
            | "assert"
            | "exit"
            | "abort"
    )
}
