use anyhow::Result;
use clap::Args;
use std::path::PathBuf;

#[derive(Args, Debug)]
pub struct CheckArgs {
    /// Output format: text, json, or porcelain
    #[arg(long, default_value = "text")]
    pub format: String,

    /// Path to the SQLite database (overrides auto-detect)
    #[arg(short, long)]
    pub db: Option<PathBuf>,

    /// List the stale file paths (one per line) in addition to the summary
    #[arg(long)]
    pub files: bool,

    /// Machine-readable output (deprecated — use --format porcelain)
    #[arg(long, hide = true)]
    pub porcelain: bool,
}

use crate::{
    config::{Config, resolve_db},
    storage::{Database, open_memory_backend},
    utils::{format_age, worktree_modified_files},
};

pub async fn check(args: CheckArgs, cfg: Config) -> Result<()> {
    let db_path = resolve_db(args.db.as_deref(), &cfg.db_path);
    if !db_path.exists() {
        anyhow::bail!(
            "No index found (checked current directory and parents).\n\
             Run `spelunk index <path>` first."
        );
    }

    let db = Database::open(&db_path)?;
    let stored = db.all_file_hashes()?;

    let mut stale: Vec<String> = Vec::new();

    // Check every indexed file against its current on-disk hash.
    for (path, stored_hash) in &stored {
        match std::fs::read(path) {
            Ok(bytes) => {
                let current = format!("{}", blake3::hash(&bytes));
                if current != *stored_hash {
                    stale.push(path.clone());
                }
            }
            Err(_) => {
                // File deleted since last index.
                stale.push(path.clone());
            }
        }
    }

    let effective = if args.porcelain {
        "porcelain"
    } else {
        crate::utils::effective_format(&args.format)
    };
    let fresh = stale.is_empty();
    let last_indexed: Option<i64> = db.stats().ok().and_then(|s| s.last_indexed);

    if effective == "porcelain" {
        let last_ts = last_indexed.unwrap_or(0);
        println!(
            "stale={} total={} last_indexed={}",
            stale.len(),
            stored.len(),
            last_ts
        );
        if args.files {
            for p in &stale {
                println!("{p}");
            }
        }
    } else if effective == "json" {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "fresh": fresh,
                "indexed_files": stored.len(),
                "stale_files": stale.len(),
                "stale": stale,
                "last_indexed_at": last_indexed,
            }))?
        );
    } else if fresh {
        println!("Index is up to date. ({} files indexed)", stored.len());
    } else {
        println!("{} file(s) changed since last index:", stale.len());
        for p in &stale {
            println!("  {p}");
        }
        println!("\nRun `spelunk index .` to update.");
    }

    // Show active intent entries (text mode only; silently skip if memory unavailable).
    if effective == "text" || effective == "porcelain" {
        let mem_path = resolve_db(args.db.as_deref(), &cfg.db_path).with_file_name("memory.db");
        if let Ok(backend) = open_memory_backend(&cfg, &mem_path)
            && let Ok(intents) = backend.list(Some("intent"), 20, false, None).await
            && !intents.is_empty()
        {
            println!("Active agent sessions:");
            for n in &intents {
                let age = format_age(n.created_at);
                if n.linked_files.is_empty() {
                    println!("  · \"{}\"  ({})", n.title, age);
                } else {
                    println!(
                        "  · \"{}\"  linked: {}  ({})",
                        n.title,
                        n.linked_files.join(", "),
                        age
                    );
                }
            }

            // File overlap warning: compare intent linked_files with worktree changes.
            let modified = worktree_modified_files();
            if !modified.is_empty() {
                let intent_files: std::collections::HashSet<String> = intents
                    .iter()
                    .flat_map(|n| n.linked_files.iter().cloned())
                    .collect();

                for file in &modified {
                    if intent_files.contains(file) {
                        println!("⚠  Overlap: {file} is listed in an active intent");
                    }
                }
            }
        }
    }

    if !fresh {
        std::process::exit(1);
    }
    Ok(())
}
