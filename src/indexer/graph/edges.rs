use super::EdgeKind;
use super::builtins::*;

pub(super) fn rust_edges(node: &tree_sitter::Node<'_>, src: &[u8]) -> Vec<(String, EdgeKind)> {
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
                        if let Ok(name) = func.utf8_text(src)
                            && !is_rust_builtin(name)
                        {
                            out.push((name.to_owned(), EdgeKind::Calls));
                        }
                    }
                    // Type::method(…) — index the full form, the type, and the method.
                    "scoped_identifier" => {
                        if let Ok(full) = func.utf8_text(src)
                            && !is_rust_builtin(full)
                        {
                            out.push((full.to_owned(), EdgeKind::Calls));
                        }
                        // Emit the method name: `EdgeExtractor::extract` → `extract`
                        if let Some(name_node) = func.child_by_field_name("name")
                            && let Ok(name) = name_node.utf8_text(src)
                            && !is_rust_builtin(name)
                        {
                            out.push((name.to_owned(), EdgeKind::Calls));
                        }
                        // Emit the type/path: `EdgeExtractor::extract` → `EdgeExtractor`
                        if let Some(path_node) = func.child_by_field_name("path")
                            && let Ok(path) = path_node.utf8_text(src)
                            && !is_rust_builtin(path)
                        {
                            out.push((path.to_owned(), EdgeKind::Calls));
                        }
                    }
                    // obj.method(…) — index the method name.
                    "field_expression" => {
                        if let Some(field) = func.child_by_field_name("field")
                            && let Ok(name) = field.utf8_text(src)
                            && !is_rust_builtin(name)
                        {
                            out.push((name.to_owned(), EdgeKind::Calls));
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

pub(super) fn python_edges(node: &tree_sitter::Node<'_>, src: &[u8]) -> Vec<(String, EdgeKind)> {
    let mut out = Vec::new();
    match node.kind() {
        "import_statement" => {
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i)
                    && matches!(child.kind(), "dotted_name" | "aliased_import")
                {
                    let name_node = if child.kind() == "aliased_import" {
                        child.child_by_field_name("name")
                    } else {
                        Some(child)
                    };
                    if let Some(n) = name_node
                        && let Ok(text) = n.utf8_text(src)
                    {
                        out.push((text.to_owned(), EdgeKind::Imports));
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
                        if let Ok(name) = func.utf8_text(src)
                            && !is_python_builtin(name)
                        {
                            out.push((name.to_owned(), EdgeKind::Calls));
                        }
                    }
                    // obj.method(…)
                    "attribute" => {
                        if let Some(attr) = func.child_by_field_name("attribute")
                            && let Ok(name) = attr.utf8_text(src)
                            && !is_python_builtin(name)
                        {
                            out.push((name.to_owned(), EdgeKind::Calls));
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

pub(super) fn js_edges(node: &tree_sitter::Node<'_>, src: &[u8]) -> Vec<(String, EdgeKind)> {
    let mut out = Vec::new();
    match node.kind() {
        "import_statement" => {
            if let Some(source) = node.child_by_field_name("source")
                && let Ok(text) = source.utf8_text(src)
            {
                let module = text.trim_matches('"').trim_matches('\'').to_owned();
                out.push((module, EdgeKind::Imports));
            }
        }
        "call_expression" => {
            if let Some(func) = node.child_by_field_name("function") {
                match func.kind() {
                    "identifier" => {
                        if let Ok(name) = func.utf8_text(src)
                            && !is_js_builtin(name)
                        {
                            out.push((name.to_owned(), EdgeKind::Calls));
                        }
                    }
                    // obj.method(…)
                    "member_expression" => {
                        if let Some(prop) = func.child_by_field_name("property")
                            && let Ok(name) = prop.utf8_text(src)
                            && !is_js_builtin(name)
                        {
                            out.push((name.to_owned(), EdgeKind::Calls));
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

pub(super) fn go_edges(node: &tree_sitter::Node<'_>, src: &[u8]) -> Vec<(String, EdgeKind)> {
    let mut out = Vec::new();
    match node.kind() {
        "import_spec" => {
            if let Some(path) = node.child_by_field_name("path")
                && let Ok(text) = path.utf8_text(src)
            {
                let module = text.trim_matches('"').to_owned();
                out.push((module, EdgeKind::Imports));
            }
        }
        "call_expression" => {
            if let Some(func) = node.child_by_field_name("function") {
                match func.kind() {
                    "identifier" => {
                        if let Ok(name) = func.utf8_text(src)
                            && !is_go_builtin(name)
                        {
                            out.push((name.to_owned(), EdgeKind::Calls));
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

pub(super) fn java_edges(node: &tree_sitter::Node<'_>, src: &[u8]) -> Vec<(String, EdgeKind)> {
    let mut out = Vec::new();
    match node.kind() {
        "import_declaration" => {
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i)
                    && matches!(child.kind(), "scoped_identifier" | "identifier")
                {
                    if let Ok(text) = child.utf8_text(src) {
                        out.push((text.to_owned(), EdgeKind::Imports));
                    }
                    break;
                }
            }
        }
        "class_declaration" => {
            if let Some(superclass) = node.child_by_field_name("superclass")
                && let Ok(text) = superclass.utf8_text(src)
            {
                let name = text.trim_start_matches("extends").trim().to_owned();
                if !name.is_empty() {
                    out.push((name, EdgeKind::Extends));
                }
            }
            if let Some(interfaces) = node.child_by_field_name("interfaces")
                && let Ok(text) = interfaces.utf8_text(src)
            {
                for name in text.trim_start_matches("implements").trim().split(',') {
                    let n = name.trim().to_owned();
                    if !n.is_empty() {
                        out.push((n, EdgeKind::Implements));
                    }
                }
            }
        }
        "method_invocation" => {
            if let Some(name) = node.child_by_field_name("name")
                && let Ok(text) = name.utf8_text(src)
            {
                out.push((text.to_owned(), EdgeKind::Calls));
            }
        }
        _ => {}
    }
    out
}

pub(super) fn c_edges(node: &tree_sitter::Node<'_>, src: &[u8]) -> Vec<(String, EdgeKind)> {
    let mut out = Vec::new();
    match node.kind() {
        "preproc_include" => {
            if let Some(path) = node.child_by_field_name("path")
                && let Ok(text) = path.utf8_text(src)
            {
                let module = text
                    .trim_matches('"')
                    .trim_start_matches('<')
                    .trim_end_matches('>')
                    .to_owned();
                out.push((module, EdgeKind::Imports));
            }
        }
        "call_expression" => {
            if let Some(func) = node.child_by_field_name("function")
                && func.kind() == "identifier"
                && let Ok(name) = func.utf8_text(src)
                && !is_c_builtin(name)
            {
                out.push((name.to_owned(), EdgeKind::Calls));
            }
        }
        _ => {}
    }
    out
}

pub(super) fn html_edges(node: &tree_sitter::Node<'_>, src: &[u8]) -> Vec<(String, EdgeKind)> {
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

pub(super) fn css_edges(node: &tree_sitter::Node<'_>, src: &[u8]) -> Vec<(String, EdgeKind)> {
    let mut out = Vec::new();
    // @import "file.css" or @import url("file.css")
    if node.kind() == "import_statement" {
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i)
                && matches!(child.kind(), "string_value" | "call_expression")
            {
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
    out
}
