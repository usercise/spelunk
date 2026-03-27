use anyhow::Result;

use crate::{
    config::{Config, resolve_db},
    storage::Database,
};
use super::super::{ChunksArgs};
use super::ui::print_chunks_text;

pub fn languages() -> Result<()> {
    let langs = crate::indexer::parser::SUPPORTED_LANGUAGES;
    println!("Supported languages:");
    for lang in langs {
        println!("  {lang}");
    }
    Ok(())
}

pub fn chunks(args: ChunksArgs, cfg: Config) -> Result<()> {
    let db_path = resolve_db(args.db.as_deref(), &cfg.db_path);
    if !db_path.exists() {
        anyhow::bail!(
            "No index found (checked current directory and parents).\n\
             Run `spelunk index <path>` inside your project first."
        );
    }

    let db = Database::open(&db_path)?;
    let results = db.chunks_for_file(&args.path)?;

    if results.is_empty() {
        println!("No chunks found for '{}'.", args.path);
        return Ok(());
    }

    match crate::utils::effective_format(&args.format) {
        "json" => println!("{}", serde_json::to_string_pretty(&results)?),
        _ => print_chunks_text(&results),
    }

    Ok(())
}
