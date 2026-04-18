use anyhow::{Context, Result};
use clap::Args;
use std::path::PathBuf;

#[derive(Args, Debug)]
pub struct HistoryArgs {
    /// Symbol name to trace (function, struct, class, etc.)
    pub symbol: String,

    /// Output format: text or json
    #[arg(long, default_value = "text")]
    pub format: String,

    /// Path to the SQLite database (overrides auto-detect)
    #[arg(short, long)]
    pub db: Option<PathBuf>,
}

use super::status::format_age;
use crate::{
    config::{Config, resolve_db},
    storage::Database,
};

pub fn history(args: HistoryArgs, cfg: Config) -> Result<()> {
    let db_path = args
        .db
        .clone()
        .unwrap_or_else(|| resolve_db(None, &cfg.db_path));
    let db = Database::open(&db_path).context("opening database")?;

    let versions = db.symbol_history(&args.symbol)?;

    if versions.is_empty() {
        anyhow::bail!(
            "Symbol '{}' not found in the live index or any snapshot.",
            args.symbol
        );
    }

    if versions.len() == 1 && versions[0].commit_sha.is_none() {
        println!(
            "No history: '{}' appears only in the live index (no snapshots contain it).",
            args.symbol
        );
    }

    if crate::utils::effective_format(&args.format) == "json" {
        println!("{}", serde_json::to_string_pretty(&versions)?);
        return Ok(());
    }

    for v in &versions {
        let source_label = match &v.commit_sha {
            None => "\x1b[32m[live]\x1b[0m".to_string(),
            Some(sha) => {
                let short = &sha[..sha.len().min(12)];
                let age = v.snapshot_created_at.map(format_age).unwrap_or_default();
                format!("\x1b[33m[{short}]\x1b[0m  \x1b[2m{age}\x1b[0m")
            }
        };

        println!(
            "{source_label}  \x1b[1m{name}\x1b[0m  \x1b[2m{path}:{start}–{end}\x1b[0m",
            name = v.name.as_deref().unwrap_or("<anonymous>"),
            path = v.file_path,
            start = v.start_line,
            end = v.end_line,
        );
        println!("{}", v.content);
        println!("{}", "─".repeat(60));
    }

    Ok(())
}
