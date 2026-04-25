use std::collections::HashSet;
use std::sync::OnceLock;

static MENTION_STOPWORDS: OnceLock<HashSet<&'static str>> = OnceLock::new();

fn mention_stopwords() -> &'static HashSet<&'static str> {
    MENTION_STOPWORDS.get_or_init(|| {
        [
            // Rust
            "fn",
            "let",
            "mut",
            "pub",
            "use",
            "mod",
            "struct",
            "enum",
            "impl",
            "trait",
            "type",
            "where",
            "async",
            "await",
            "move",
            "dyn",
            "ref",
            "box",
            "unsafe",
            "true",
            "false",
            "self",
            "super",
            "crate",
            "extern",
            // Common types / builtins
            "None",
            "Some",
            "Ok",
            "Err",
            "Vec",
            "String",
            "str",
            "bool",
            "usize",
            "i32",
            "i64",
            "u32",
            "u64",
            "f32",
            "f64",
            "isize",
            "u8",
            "i8",
            "u16",
            "i16",
            // Python
            "def",
            "class",
            "import",
            "from",
            "with",
            "pass",
            "raise",
            "yield",
            "lambda",
            "global",
            "nonlocal",
            "assert",
            "del",
            // JavaScript / TypeScript
            "var",
            "const",
            "function",
            "typeof",
            "instanceof",
            "new",
            "delete",
            "export",
            "default",
            "extends",
            "static",
            // Go
            "func",
            "package",
            "interface",
            "chan",
            "select",
            "defer",
            "goto",
            "fallthrough",
            // Java / C
            "void",
            "null",
            "class",
            "int",
            "long",
            "double",
            "float",
            "char",
            "byte",
            "short",
            "final",
            "this",
            "super",
            "throws",
            "throw",
            "catch",
            "finally",
            // Control flow (shared)
            "if",
            "else",
            "for",
            "while",
            "do",
            "switch",
            "case",
            "break",
            "continue",
            "return",
            "match",
            "in",
            "not",
            "and",
            "or",
            "is",
            // Very common but not meaningful
            "get",
            "set",
            "add",
            "new",
            "into",
            "from",
            "with",
            "data",
            "val",
        ]
        .iter()
        .copied()
        .collect()
    })
}

/// Extract identifier-like tokens from chunk content for use as mention edges.
/// Returns up to 40 unique tokens that look like symbol names.
pub(super) fn extract_mention_tokens(content: &str, _language: &str) -> Vec<String> {
    let stop = mention_stopwords();
    let mut seen = HashSet::new();
    let mut out = Vec::new();

    let mut start: Option<usize> = None;
    let chars: Vec<char> = content.chars().collect();
    let n = chars.len();

    for i in 0..=n {
        let ch = if i < n { chars[i] } else { ' ' };
        let is_ident = ch.is_ascii_alphanumeric() || ch == '_';

        if is_ident {
            if start.is_none() {
                start = Some(i);
            }
        } else if let Some(s) = start.take() {
            let tok: String = chars[s..i].iter().collect();
            // Keep tokens that look like symbols: 3-50 chars, not all digits, not a stopword
            if tok.len() >= 3
                && tok.len() <= 50
                && !tok.chars().all(|c| c.is_ascii_digit())
                && !stop.contains(tok.as_str())
                && seen.insert(tok.clone())
            {
                out.push(tok);
                if out.len() >= 40 {
                    break;
                }
            }
        }
    }

    out
}
