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
};

pub fn check(args: CheckArgs, cfg: Config) -> Result<()> {
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
        if let Ok(backend) = open_memory_backend(&cfg, &mem_path) {
            let handle = tokio::runtime::Handle::current();
            if let Ok(intents) = handle.block_on(backend.list(Some("intent"), 20, false, None))
                && !intents.is_empty()
            {
                println!("Active agent intents: {}", intents.len());
                for n in &intents {
                    println!("  #{id}: {title}", id = n.id, title = n.title);
                }
            }
        }
    }

    if !fresh {
        std::process::exit(1);
    }
    Ok(())
}
