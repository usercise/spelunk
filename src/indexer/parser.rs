use super::chunker::{Chunk, ChunkKind, sliding_window};
use anyhow::{Result, bail};

/// All languages with tree-sitter grammar support.
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
    ts_language(name)
}

/// Detect text-format languages (markdown, plain text) from file path.
/// These are handled without tree-sitter.
pub fn detect_text_language(path: &std::path::Path) -> Option<&'static str> {
    // Check extension first
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        return match ext.to_lowercase().as_str() {
            "md" | "mdx" | "markdown" => Some("markdown"),
            "txt" | "rst" | "adoc" | "asciidoc" => Some("text"),
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

fn ts_language(name: &str) -> Result<tree_sitter::Language> {
    // Grammar crates 0.23+ expose a `LANGUAGE: LanguageFn` constant via the
    // stable `tree-sitter-language` ABI crate; `.into()` converts to Language.
    Ok(match name {
        "rust" => tree_sitter_rust::LANGUAGE.into(),
        "python" => tree_sitter_python::LANGUAGE.into(),
        "javascript" | "jsx" => tree_sitter_javascript::LANGUAGE.into(),
        "typescript" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "tsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),
        "go" => tree_sitter_go::LANGUAGE.into(),
        "java" => tree_sitter_java::LANGUAGE.into(),
        "c" => tree_sitter_c::LANGUAGE.into(),
        "cpp" => tree_sitter_cpp::LANGUAGE.into(),
        "json" => tree_sitter_json::LANGUAGE.into(),
        "html" => tree_sitter_html::LANGUAGE.into(),
        "css" => tree_sitter_css::LANGUAGE.into(),
        "hcl" => tree_sitter_hcl::LANGUAGE.into(),
        "sql" => tree_sitter_sequel::LANGUAGE.into(),
        "proto" => tree_sitter_proto::LANGUAGE.into(),
        other => bail!("unsupported language: {other}"),
    })
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
    NodeSpec {
        kind,
        chunk_kind,
        name_field,
    }
}

fn node_specs(language: &str) -> Vec<NodeSpec> {
    use ChunkKind::*;
    match language {
        "rust" => vec![
            s("function_item", Function, Some("name")),
            s("impl_item", Impl, None),
            s("struct_item", Struct, Some("name")),
            s("enum_item", Enum, Some("name")),
            s("trait_item", Trait, Some("name")),
            s("mod_item", Module, Some("name")),
            s("const_item", Constant, Some("name")),
            s("type_item", TypeAlias, Some("name")),
        ],
        "python" => vec![
            s("function_definition", Function, Some("name")),
            s("class_definition", Class, Some("name")),
        ],
        "javascript" | "jsx" => vec![
            s("function_declaration", Function, Some("name")),
            s("method_definition", Method, Some("name")),
            s("class_declaration", Class, Some("name")),
            s("generator_function_declaration", Function, Some("name")),
        ],
        "typescript" | "tsx" => vec![
            s("function_declaration", Function, Some("name")),
            s("method_definition", Method, Some("name")),
            s("class_declaration", Class, Some("name")),
            s("interface_declaration", Interface, Some("name")),
            s("type_alias_declaration", TypeAlias, Some("name")),
            s("generator_function_declaration", Function, Some("name")),
        ],
        "go" => vec![
            s("function_declaration", Function, Some("name")),
            s("method_declaration", Method, Some("name")),
            s("type_spec", Struct, Some("name")),
        ],
        "java" => vec![
            s("class_declaration", Class, Some("name")),
            s("interface_declaration", Interface, Some("name")),
            s("method_declaration", Method, Some("name")),
            s("constructor_declaration", Method, Some("name")),
            s("enum_declaration", Enum, Some("name")),
        ],
        "c" => vec![
            s("function_definition", Function, None),
            s("struct_specifier", Struct, Some("name")),
            s("enum_specifier", Enum, Some("name")),
        ],
        "cpp" => vec![
            s("function_definition", Function, None),
            s("class_specifier", Class, Some("name")),
            s("struct_specifier", Struct, Some("name")),
            s("function_declarator", Function, Some("declarator")),
        ],
        // JSON: no semantic node types — falls back to sliding-window automatically.
        "json" => vec![],
        // HTML: capture inline script and style blocks as code chunks.
        "html" => vec![
            s("script_element", Function, None),
            s("style_element", Module, None),
        ],
        // CSS: each rule set and named @-rule becomes its own chunk.
        "css" => vec![
            s("rule_set", Rule, None),
            s("media_statement", Module, None),
            s("keyframes_statement", Function, None),
            s("supports_statement", Module, None),
        ],
        // HCL/Terraform: top-level blocks (resource, data, module, locals, …).
        // Name extraction is handled by hcl_block_name (identifier + string labels).
        "hcl" => vec![s("block", Module, None)],
        // Protobuf: message, enum, service, and rpc definitions.
        // Name extraction finds the *_name child node.
        "proto" => vec![
            s("message", Struct, None),
            s("enum", Enum, None),
            s("service", Interface, None),
            s("rpc", Method, None),
        ],
        // SQL: major DDL statements.
        // Name extraction finds the object_reference child.
        "sql" => vec![
            s("create_table", Struct, None),
            s("create_view", TypeAlias, None),
            s("create_function", Function, None),
            s("create_index", Constant, None),
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
        // Text formats bypass tree-sitter entirely.
        if language == "markdown" {
            return Ok(parse_markdown(source, file_path));
        }
        if language == "text" {
            return Ok(sliding_window(source, file_path, language, 120, 15));
        }

        let ts_lang = ts_language(language)?;
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&ts_lang)?;

        let tree = parser
            .parse(source, None)
            .ok_or_else(|| anyhow::anyhow!("tree-sitter produced no parse tree for {file_path}"))?;

        let bytes = source.as_bytes();
        let specs = node_specs(language);
        let mut chunks = Vec::new();

        walk_node(
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
        // Skip keyword leaf tokens: grammars like proto reuse the node kind
        // name for both the keyword token ("message") and the structural block.
        // Structural nodes always have named children; keyword leaves do not.
        if node.named_child_count() == 0 {
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    walk_node(child, src, file_path, language, specs, parent_scope, out);
                }
            }
            return;
        }

        let name = extract_name(&node, src, language, spec);

        let content = node.utf8_text(src).unwrap_or("").to_owned();

        // Look for a doc comment immediately before this node
        let docstring = preceding_comment(&node, src);

        // Build scope label for impl/class containers
        let scope_label: Option<String> = match spec.chunk_kind {
            ChunkKind::Impl | ChunkKind::Class => {
                name.clone().map(|n| format!("{} {}", spec.kind, n))
            }
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
                walk_node(
                    child,
                    src,
                    file_path,
                    language,
                    specs,
                    scope_label.as_deref(),
                    out,
                );
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
fn extract_name(
    node: &tree_sitter::Node<'_>,
    src: &[u8],
    language: &str,
    spec: &NodeSpec,
) -> Option<String> {
    // Try the declared name field first.
    let from_field = spec
        .name_field
        .and_then(|field| node.child_by_field_name(field))
        .and_then(|n| n.utf8_text(src).ok())
        .map(|text| match language {
            // JSON keys are wrapped in quotes — strip them.
            "json" => text.trim_matches('"').to_owned(),
            _ => text.to_owned(),
        });

    if from_field.is_some() {
        return from_field;
    }

    // Language-specific fallbacks when no name field is declared.
    match language {
        "c" | "cpp" => c_function_name(node, src),
        "css" => css_chunk_name(node, src),
        "html" => html_chunk_name(node, src),
        "hcl" => hcl_block_name(node, src),
        "proto" => proto_named_child(node, src),
        "sql" => sql_object_name(node, src),
        _ => None,
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
                let child = match start_tag.child(j) {
                    Some(c) => c,
                    None => continue,
                };
                if child.kind() == "tag_name" {
                    tag_name = child.utf8_text(src).ok().map(str::to_owned);
                }
                if child.kind() == "attribute" {
                    let mut name = "";
                    let mut value = "";
                    for k in 0..child.child_count() {
                        if let Some(attr_child) = child.child(k) {
                            match attr_child.kind() {
                                "attribute_name" => name = attr_child.utf8_text(src).unwrap_or(""),
                                "attribute_value" | "quoted_attribute_value" => {
                                    value = attr_child.utf8_text(src).unwrap_or("")
                                }
                                _ => {}
                            }
                        }
                    }
                    if matches!(name, "src" | "id") && !value.is_empty() {
                        return Some(value.trim_matches('"').trim_matches('\'').to_owned());
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
        if let Some(child) = node.child(i)
            && let Some(name) = find_identifier(child, src)
        {
            return Some(name);
        }
    }
    None
}

/// Build an HCL block name from its type identifier and string labels.
/// e.g. `resource "aws_instance" "main"` → `"resource.aws_instance.main"`.
fn hcl_block_name(node: &tree_sitter::Node<'_>, src: &[u8]) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            match child.kind() {
                "identifier" => {
                    if let Ok(t) = child.utf8_text(src) {
                        parts.push(t.to_owned());
                    }
                }
                "string_lit" => {
                    if let Ok(t) = child.utf8_text(src) {
                        parts.push(t.trim_matches('"').to_owned());
                    }
                }
                _ => {}
            }
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("."))
    }
}

/// Return the text of the first `*_name` child node (used for proto grammars).
fn proto_named_child(node: &tree_sitter::Node<'_>, src: &[u8]) -> Option<String> {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i)
            && child.kind().ends_with("_name")
        {
            return child.utf8_text(src).ok().map(str::to_owned);
        }
    }
    None
}

/// Return the text of the first `object_reference` child (used for SQL DDL nodes).
fn sql_object_name(node: &tree_sitter::Node<'_>, src: &[u8]) -> Option<String> {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i)
            && child.kind() == "object_reference"
        {
            return child.utf8_text(src).ok().map(str::to_owned);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Markdown chunker (heading-based)
// ---------------------------------------------------------------------------

/// Split a Markdown document into per-section chunks.
/// Each ATX heading (`#`, `##`, …) starts a new chunk that includes the
/// heading line and all content until the next same-or-higher-level heading.
/// If the file has no headings the whole document is split by sliding window.
fn parse_markdown(source: &str, file_path: &str) -> Vec<Chunk> {
    let lines: Vec<&str> = source.lines().collect();
    let mut chunks: Vec<Chunk> = Vec::new();
    // (start_line_idx, heading_text)
    let mut section: Option<(usize, String)> = None;
    let mut preamble: Vec<usize> = Vec::new(); // line indices before first heading

    let flush = |start: usize,
                 title: Option<String>,
                 end: usize,
                 lines: &[&str],
                 chunks: &mut Vec<Chunk>| {
        let content = lines[start..end].join("\n");
        if content.trim().is_empty() {
            return;
        }
        chunks.push(Chunk {
            file_path: file_path.to_owned(),
            language: "markdown".to_owned(),
            kind: ChunkKind::Section,
            name: title,
            start_line: start + 1,
            end_line: end,
            content,
            docstring: None,
            parent_scope: None,
        });
    };

    for (i, line) in lines.iter().enumerate() {
        if let Some(heading_text) = atx_heading(line) {
            if let Some((start, title)) = section.take() {
                flush(start, Some(title), i, &lines, &mut chunks);
            } else if !preamble.is_empty() {
                // Flush preamble (content before the first heading)
                let start = *preamble.first().unwrap();
                flush(start, None, i, &lines, &mut chunks);
                preamble.clear();
            }
            section = Some((i, heading_text));
        } else if section.is_none() {
            preamble.push(i);
        }
    }

    // Flush the last section / remaining preamble
    if let Some((start, title)) = section {
        flush(start, Some(title), lines.len(), &lines, &mut chunks);
    } else if !preamble.is_empty() {
        let start = *preamble.first().unwrap();
        flush(start, None, lines.len(), &lines, &mut chunks);
    }

    if chunks.is_empty() {
        return sliding_window(source, file_path, "markdown", 120, 15);
    }
    chunks
}

/// Extract the text of an ATX heading line (`# Foo` → `"Foo"`).
/// Returns None for non-heading lines or fenced-code-block lines.
fn atx_heading(line: &str) -> Option<String> {
    let stripped = line.trim_start_matches('#');
    let hashes = line.len() - stripped.len();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    // Must be followed by a space (or end of line for empty heading)
    if !stripped.is_empty() && !stripped.starts_with(' ') {
        return None;
    }
    Some(stripped.trim().to_owned())
}

/// Return the text of the comment node that immediately precedes `node`
/// (skipping whitespace), if any.
fn preceding_comment(node: &tree_sitter::Node<'_>, src: &[u8]) -> Option<String> {
    let mut prev = node.prev_sibling()?;
    // skip over whitespace / newline tokens
    while prev.kind() == "\n"
        || prev.kind() == "newline"
        || prev.is_extra()
            && prev.kind() != "comment"
            && prev.kind() != "line_comment"
            && prev.kind() != "block_comment"
    {
        prev = prev.prev_sibling()?;
    }
    if matches!(
        prev.kind(),
        "comment" | "line_comment" | "block_comment" | "doc_comment"
    ) {
        Some(prev.utf8_text(src).unwrap_or("").to_owned())
    } else {
        None
    }
}
