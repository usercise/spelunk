use anyhow::{bail, Result};
use super::chunker::{sliding_window, Chunk, ChunkKind};

/// All languages with tree-sitter grammar support.
pub const SUPPORTED_LANGUAGES: &[&str] =
    &["rust", "python", "javascript", "typescript", "go", "java", "c", "cpp",
      "json", "html", "css"];

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
        "json"                               => Some("json"),
        "html" | "htm"                       => Some("html"),
        "css"                                => Some("css"),
        _ => None,
    }
}

pub(crate) fn ts_language_pub(name: &str) -> Result<tree_sitter::Language> {
    ts_language(name)
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
        "json"       => Ok(tree_sitter_json::language()),
        "html"       => Ok(tree_sitter_html::language()),
        "css"        => Ok(tree_sitter_css::language()),
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
        // JSON: no semantic node types — falls back to sliding-window automatically.
        "json" => vec![],
        // HTML: capture inline script and style blocks as code chunks.
        "html" => vec![
            s("script_element", Function, None),
            s("style_element",  Module,   None),
        ],
        // CSS: each rule set and named @-rule becomes its own chunk.
        "css" => vec![
            s("rule_set",            Rule,    None),
            s("media_statement",     Module,  None),
            s("keyframes_statement", Function, None),
            s("supports_statement",  Module,  None),
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
        let name = extract_name(&node, src, language, spec);

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

/// Language-aware name extraction for a chunk node.
fn extract_name(node: &tree_sitter::Node<'_>, src: &[u8], language: &str, spec: &NodeSpec) -> Option<String> {
    // Try the declared name field first.
    let from_field = spec
        .name_field
        .and_then(|field| node.child_by_field_name(field))
        .and_then(|n| n.utf8_text(src).ok())
        .map(|text| match language {
            // JSON keys are wrapped in quotes — strip them.
            "json" => text.trim_matches('"').to_owned(),
            _      => text.to_owned(),
        });

    if from_field.is_some() {
        return from_field;
    }

    // Language-specific fallbacks when no name field is declared.
    match language {
        "c" | "cpp" => c_function_name(node, src),
        "css"       => css_chunk_name(node, src),
        "html"      => html_chunk_name(node, src),
        _           => None,
    }
}

/// Return the selector text from a CSS `rule_set` node, or the @-keyword for
/// at-rules, to use as the chunk name.
fn css_chunk_name(node: &tree_sitter::Node<'_>, src: &[u8]) -> Option<String> {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "selectors" {
                return child.utf8_text(src).ok().map(|s| s.trim().to_owned());
            }
            // @-rule keyword (e.g. "media", "keyframes")
            if matches!(child.kind(), "at_keyword" | "keyword") {
                return child.utf8_text(src).ok().map(|s| s.to_owned());
            }
        }
    }
    None
}

/// Return the `src`/`id` attribute value of an HTML chunk element as its name,
/// falling back to the tag name.  tree-sitter-html uses child kinds
/// (`attribute_name`, `attribute_value`) rather than named fields.
fn html_chunk_name(node: &tree_sitter::Node<'_>, src: &[u8]) -> Option<String> {
    for i in 0..node.child_count() {
        if let Some(start_tag) = node.child(i) {
            if start_tag.kind() != "start_tag" {
                continue;
            }
            let mut tag_name: Option<String> = None;
            for j in 0..start_tag.child_count() {
                let child = match start_tag.child(j) { Some(c) => c, None => continue };
                if child.kind() == "tag_name" {
                    tag_name = child.utf8_text(src).ok().map(str::to_owned);
                }
                if child.kind() == "attribute" {
                    let mut name = "";
                    let mut value = "";
                    for k in 0..child.child_count() {
                        if let Some(attr_child) = child.child(k) {
                            match attr_child.kind() {
                                "attribute_name"  => name  = attr_child.utf8_text(src).unwrap_or(""),
                                "attribute_value" | "quoted_attribute_value"
                                                  => value = attr_child.utf8_text(src).unwrap_or(""),
                                _ => {}
                            }
                        }
                    }
                    if matches!(name, "src" | "id") && !value.is_empty() {
                        return Some(
                            value.trim_matches('"').trim_matches('\'').to_owned()
                        );
                    }
                }
            }
            return tag_name;
        }
    }
    None
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
