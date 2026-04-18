use anyhow::{Context, Result};
use serde::Serialize;

use super::PlumbingParseFileArgs;
use crate::indexer::parser::{SourceParser, detect_language, detect_text_language};

#[derive(Serialize)]
struct ParsedChunk {
    kind: String,
    name: Option<String>,
    start_line: usize,
    end_line: usize,
    content: String,
    language: String,
}

pub(super) fn parse_file(args: PlumbingParseFileArgs) -> Result<()> {
    let path = &args.file;
    let language = match detect_language(path).or_else(|| detect_text_language(path)) {
        Some(l) => l,
        None => std::process::exit(1), // unsupported type — no results
    };

    let source =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;

    let path_str = path.to_string_lossy();
    let chunks = SourceParser::parse(&source, &path_str, language)
        .with_context(|| format!("parsing {}", path.display()))?;

    if chunks.is_empty() {
        std::process::exit(1);
    }

    for c in chunks {
        let out = ParsedChunk {
            kind: c.kind.to_string(),
            name: c.name,
            start_line: c.start_line,
            end_line: c.end_line,
            content: c.content,
            language: language.to_string(),
        };
        println!("{}", serde_json::to_string(&out)?);
    }
    Ok(())
}
