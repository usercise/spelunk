use anyhow::Result;
use crate::config::Config;
use super::{AskArgs, IndexArgs, SearchArgs};

pub async fn index(_args: IndexArgs, _cfg: Config) -> Result<()> {
    todo!("Phase 2: tree-sitter parsing + Phase 3: embedding + storage")
}

pub async fn search(_args: SearchArgs, _cfg: Config) -> Result<()> {
    todo!("Phase 4: vector search")
}

pub async fn ask(_args: AskArgs, _cfg: Config) -> Result<()> {
    todo!("Phase 5: RAG pipeline with Gemma 3n")
}

pub async fn status(_cfg: Config) -> Result<()> {
    todo!("Phase 1: read DB stats and print")
}

pub fn languages() -> Result<()> {
    let langs = crate::indexer::parser::SUPPORTED_LANGUAGES;
    println!("Supported languages:");
    for lang in langs {
        println!("  {lang}");
    }
    Ok(())
}
