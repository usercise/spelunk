use anyhow::{bail, Result};
use super::chunker::{sliding_window, Chunk, ChunkKind};

/// All languages with tree-sitter grammar support.
pub const SUPPORTED_LANGUAGES: &[&str] =
    &["rust", "python", "javascript", "typescript", "go", "java", "c", "cpp"];

/// Detect language from file extension.
pub fn detect_language(path: &std::path::Path) -> Option<&'static str> {
    match path.extension()?.to_str()? {
        "rs"                      => Some("rust"),
        "py"                      => Some("python"),
        "js" | "mjs" | "cjs"     => Some("javascript"),
        "ts" | "mts"              => Some("typescript"),
        "go"                      => Some("go"),
        "java"                    => Some("java"),
        "c" | "h"                 => Some("c"),
        "cpp" | "cc" | "cxx" | "hpp" | "hxx" => Some("cpp"),
        _ => None,
    }
}

fn ts_language(name: &str) -> Result<tree_sitter::Language> {
    // tree-sitter 0.21.x crates expose a `language()` fn, not a `LANGUAGE` const.
    match name {
        "rust"       => Ok(tree_sitter_rust::language()),
        "python"     => Ok(tree_sitter_python::language()),
        "javascript" => Ok(tree_sitter_javascript::language()),
        "typescript" => Ok(tree_sitter_typescript::language_typescript()),
        "go"         => Ok(tree_sitter_go::language()),
        "java"       => Ok(tree_sitter_java::language()),
        "c"          => Ok(tree_sitter_c::language()),
        "cpp"        => Ok(tree_sitter_cpp::language()),
        other        => bail!("unsupported language: {other}"),
    }
}

// ---------------------------------------------------------------------------
// Per-language semantic node configurations
// ---------------------------------------------------------------------------

/// Describes a node type that should become a chunk.
struct NodeSpec {
    /// tree-sitter node kind string
    kind: &'static str,
    /// The chunk kind to assign
    chunk_kind: ChunkKind,
    /// Field name to use for the symbol name (e.g. "name")
    name_field: Option<&'static str>,
}

fn s(kind: &'static str, chunk_kind: ChunkKind, name_field: Option<&'static str>) -> NodeSpec {
    NodeSpec { kind, chunk_kind, name_field }
}

fn node_specs(language: &str) -> Vec<NodeSpec> {
    use ChunkKind::*;
    match language {
        "rust" => vec![
            s("function_item",  Function,  Some("name")),
            s("impl_item",      Impl,      None),
            s("struct_item",    Struct,    Some("name")),
            s("enum_item",      Enum,      Some("name")),
            s("trait_item",     Trait,     Some("name")),
            s("mod_item",       Module,    Some("name")),
            s("const_item",     Constant,  Some("name")),
            s("type_item",      TypeAlias, Some("name")),
        ],
        "python" => vec![
            s("function_definition", Function, Some("name")),
            s("class_definition",    Class,    Some("name")),
        ],
        "javascript" => vec![
            s("function_declaration",           Function, Some("name")),
            s("method_definition",              Method,   Some("name")),
            s("class_declaration",              Class,    Some("name")),
            s("generator_function_declaration", Function, Some("name")),
        ],
        "typescript" => vec![
            s("function_declaration",           Function,  Some("name")),
            s("method_definition",              Method,    Some("name")),
            s("class_declaration",              Class,     Some("name")),
            s("interface_declaration",          Interface, Some("name")),
            s("type_alias_declaration",         TypeAlias, Some("name")),
            s("generator_function_declaration", Function,  Some("name")),
        ],
        "go" => vec![
            s("function_declaration", Function, Some("name")),
            s("method_declaration",   Method,   Some("name")),
            s("type_spec",            Struct,   Some("name")),
        ],
        "java" => vec![
            s("class_declaration",       Class,     Some("name")),
            s("interface_declaration",   Interface, Some("name")),
            s("method_declaration",      Method,    Some("name")),
            s("constructor_declaration", Method,    Some("name")),
            s("enum_declaration",        Enum,      Some("name")),
        ],
        "c" => vec![
            s("function_definition", Function, None),
            s("struct_specifier",    Struct,   Some("name")),
            s("enum_specifier",      Enum,     Some("name")),
        ],
        "cpp" => vec![
            s("function_definition",  Function, None),
            s("class_specifier",      Class,    Some("name")),
            s("struct_specifier",     Struct,   Some("name")),
            s("function_declarator",  Function, Some("declarator")),
        ],
        _ => vec![],
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

pub struct SourceParser;

impl SourceParser {
    /// Parse `source` and return semantic chunks.
    /// Falls back to sliding-window if parsing fails or yields nothing.
    pub fn parse(source: &str, file_path: &str, language: &str) -> Result<Vec<Chunk>> {
        let ts_lang = ts_language(language)?;
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&ts_lang)?;

        let tree = parser
            .parse(source, None)
            .ok_or_else(|| anyhow::anyhow!("tree-sitter produced no parse tree for {file_path}"))?;

        let bytes = source.as_bytes();
        let specs = node_specs(language);
        let mut chunks = Vec::new();

        walk_node(tree.root_node(), bytes, file_path, language, &specs, None, &mut chunks);

        if chunks.is_empty() {
            tracing::debug!("{file_path}: no semantic nodes found, using sliding window");
            return Ok(sliding_window(source, file_path, language, 60, 10));
        }

        Ok(chunks)
    }
}

// ---------------------------------------------------------------------------
// Tree walker
// ---------------------------------------------------------------------------

fn walk_node(
    node: tree_sitter::Node<'_>,
    src: &[u8],
    file_path: &str,
    language: &str,
    specs: &[NodeSpec],
    parent_scope: Option<&str>,
    out: &mut Vec<Chunk>,
) {
    if let Some(spec) = specs.iter().find(|s| s.kind == node.kind()) {
        let name = spec
            .name_field
            .and_then(|field| node.child_by_field_name(field))
            .and_then(|n| n.utf8_text(src).ok())
            .map(str::to_owned)
            .or_else(|| c_function_name(&node, src));

        let content = node
            .utf8_text(src)
            .unwrap_or("")
            .to_owned();

        // Look for a doc comment immediately before this node
        let docstring = preceding_comment(&node, src);

        // Build scope label for impl/class containers
        let scope_label: Option<String> = match spec.chunk_kind {
            ChunkKind::Impl | ChunkKind::Class => name.clone().map(|n| format!("{} {}", spec.kind, n)),
            _ => parent_scope.map(str::to_owned),
        };

        out.push(Chunk {
            file_path: file_path.to_owned(),
            language: language.to_owned(),
            kind: spec.chunk_kind.clone(),
            name,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            content,
            docstring,
            parent_scope: parent_scope.map(str::to_owned),
        });

        // Recurse into children with the updated scope
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                walk_node(child, src, file_path, language, specs, scope_label.as_deref(), out);
            }
        }
    } else {
        // Not a target node — recurse with same parent scope
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                walk_node(child, src, file_path, language, specs, parent_scope, out);
            }
        }
    }
}

/// Extract the function name from a C/C++ `function_definition` node, which
/// nests the name inside a declarator rather than exposing a direct `name` field.
fn c_function_name<'a>(node: &tree_sitter::Node<'a>, src: &'a [u8]) -> Option<String> {
    // function_definition → declarator → … → identifier
    let decl = node.child_by_field_name("declarator")?;
    find_identifier(decl, src)
}

fn find_identifier(node: tree_sitter::Node<'_>, src: &[u8]) -> Option<String> {
    if node.kind() == "identifier" || node.kind() == "field_identifier" {
        return node.utf8_text(src).ok().map(str::to_owned);
    }
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if let Some(name) = find_identifier(child, src) {
                return Some(name);
            }
        }
    }
    None
}

/// Return the text of the comment node that immediately precedes `node`
/// (skipping whitespace), if any.
fn preceding_comment(node: &tree_sitter::Node<'_>, src: &[u8]) -> Option<String> {
    let mut prev = node.prev_sibling()?;
    // skip over whitespace / newline tokens
    while prev.kind() == "\n" || prev.kind() == "newline" || prev.is_extra() && prev.kind() != "comment" && prev.kind() != "line_comment" && prev.kind() != "block_comment" {
        prev = prev.prev_sibling()?;
    }
    if matches!(prev.kind(), "comment" | "line_comment" | "block_comment" | "doc_comment") {
        Some(prev.utf8_text(src).unwrap_or("").to_owned())
    } else {
        None
    }
}
