use anyhow::{Context, Result};

use super::super::StatusArgs;
use super::search::resolve_project_and_deps;
use crate::{
    config::{Config, resolve_db},
    registry::Registry,
    storage::{Database, open_memory_backend},
};

pub async fn status(args: StatusArgs, cfg: Config) -> Result<()> {
    let fmt = crate::utils::effective_format(&args.format);

    // JSON mode: current project stats only
    if fmt == "json" {
        let (db_path, _) = resolve_project_and_deps(None, &cfg)?;
        let db = Database::open(&db_path)?;
        let stats = db.stats()?;
        let drift = db.drift_candidates(30, 10).unwrap_or_default();
        let usage = db.usage_last_7_days().unwrap_or_default();
        let mem_path = resolve_db(None, &cfg.db_path).with_file_name("memory.db");
        let memory_count = match open_memory_backend(&cfg, &mem_path).ok() {
            Some(b) => b.count().await.unwrap_or(0),
            None => 0,
        };
        let usage_map: std::collections::HashMap<&str, i64> =
            usage.iter().map(|(c, n)| (c.as_str(), *n)).collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "file_count": stats.file_count,
                "chunk_count": stats.chunk_count,
                "embedding_count": stats.embedding_count,
                "last_indexed_unix": stats.last_indexed,
                "snapshot_count": stats.snapshot_count,
                "memory_entry_count": memory_count,
                "drift_candidates": drift,
                "usage_7d": {
                    "search": usage_map.get("search").copied().unwrap_or(0),
                    "explore": usage_map.get("explore").copied().unwrap_or(0),
                    "memory_search": usage_map.get("memory search").copied().unwrap_or(0),
                }
            }))?
        );
        return Ok(());
    }

    // --list implies --all
    let show_all = args.all || args.list;

    if show_all {
        let reg = Registry::open().context("opening registry")?;
        let projects = reg.all_projects()?;

        if projects.is_empty() {
            println!("No projects registered. Run `spelunk index <path>` to get started.");
            return Ok(());
        }

        if args.list {
            // Brief table: one line per project
            println!(
                "{:<6}  {:<8}  {:<10}  Root",
                "Files", "Chunks", "Embeddings"
            );
            println!("{}", "─".repeat(70));
            for p in &projects {
                let stats = Database::open(&p.db_path).and_then(|db| db.stats()).ok();
                let (files, chunks, embeddings) = stats
                    .map(|s| (s.file_count, s.chunk_count, s.embedding_count))
                    .unwrap_or((0, 0, 0));
                let exists = if p.root_path.exists() {
                    ""
                } else {
                    " [missing]"
                };
                println!(
                    "{:<6}  {:<8}  {:<10}  {}{}",
                    files,
                    chunks,
                    embeddings,
                    p.root_path.display(),
                    exists
                );
            }
        } else {
            // Detailed view per project
            for p in &projects {
                println!("\x1b[1m{}\x1b[0m", p.root_path.display());
                if !p.root_path.exists() {
                    println!("  \x1b[31m[root path missing from disk]\x1b[0m");
                }
                println!("  DB: {}", p.db_path.display());
                match Database::open(&p.db_path).and_then(|db| db.stats()) {
                    Ok(s) => {
                        println!(
                            "  Files: {}  Chunks: {}  Embeddings: {}",
                            s.file_count, s.chunk_count, s.embedding_count
                        );
                        if let Some(ts) = s.last_indexed {
                            println!("  Last indexed: {}", format_age(ts));
                        }
                    }
                    Err(_) => println!("  \x1b[2m(no index yet)\x1b[0m"),
                }
                let deps = reg.get_deps(p.id)?;
                if !deps.is_empty() {
                    println!("  Depends on:");
                    for dep in &deps {
                        println!("    → {}", dep.root_path.display());
                    }
                }
                println!();
            }
        }
        return Ok(());
    }

    // Current project only
    let reg = Registry::open().ok();
    let project = reg.as_ref().and_then(|r| {
        std::env::current_dir()
            .ok()
            .and_then(|cwd| r.find_project_for_path(&cwd).ok().flatten())
    });

    let db_path = match &project {
        Some(p) => p.db_path.clone(),
        None => resolve_db(None, &cfg.db_path),
    };

    if !db_path.exists() {
        println!("No index found for the current directory (checked parents too).");
        println!("Run `spelunk index <path>` to create one.");
        return Ok(());
    }

    let db = Database::open(&db_path)?;
    let s = db.stats()?;

    if let Some(p) = &project {
        println!("Project: \x1b[1m{}\x1b[0m", p.root_path.display());
    }
    println!("Index:      {}", db_path.display());
    println!("Files:      {}", s.file_count);
    println!("Chunks:     {}", s.chunk_count);
    println!("Embeddings: {}", s.embedding_count);
    if s.snapshot_count > 0 {
        println!("Snapshots:  {}", s.snapshot_count);
    }
    if let Some(ts) = s.last_indexed {
        println!("Last index: {}", format_age(ts));
    }

    // Show dependencies
    if let (Some(reg), Some(p)) = (&reg, &project) {
        let deps = reg.get_deps(p.id)?;
        if !deps.is_empty() {
            println!("\nDependencies:");
            for dep in &deps {
                let dep_stats = Database::open(&dep.db_path).and_then(|db| db.stats()).ok();
                let summary = dep_stats
                    .map(|s| format!("{} files, {} chunks", s.file_count, s.chunk_count))
                    .unwrap_or_else(|| "not indexed".to_string());
                println!("  → {}  ({})", dep.root_path.display(), summary);
            }
        }
    }

    // Drift signals: files that haven't changed while the project has evolved
    let drift = db.drift_candidates(30, 5).unwrap_or_default();
    if !drift.is_empty() {
        println!("\n\x1b[33mDrift signals\x1b[0m  (unchanged while project evolved):");
        println!("  {:<6}  {:<8}  File", "Days", "Callers");
        println!("  {}", "─".repeat(60));
        for d in &drift {
            let callers = if d.caller_count > 0 {
                format!("{}", d.caller_count)
            } else {
                "—".to_string()
            };
            println!("  {:<6}  {:<8}  {}", d.days_behind, callers, d.path);
        }
        println!(
            "  \x1b[2mRun `spelunk search \"<topic>\"` to check if these are still relevant.\x1b[0m"
        );
    }

    // Usage summary (last 7 days)
    let usage = db.usage_last_7_days().unwrap_or_default();
    let total: i64 = usage.iter().map(|(_, n)| n).sum();
    if total > 0 {
        const COMMANDS: &[&str] = &["search", "explore", "memory search"];
        println!("\nUsage (last 7 days)");
        for cmd in COMMANDS {
            let count = usage
                .iter()
                .find(|(c, _)| c == cmd)
                .map(|(_, n)| *n)
                .unwrap_or(0);
            if count > 0 {
                println!("  {:<16}  {} calls", cmd, count);
            }
        }
    }

    Ok(())
}

pub(crate) fn format_age(unix_ts: i64) -> String {
    use std::time::{Duration, UNIX_EPOCH};
    if let Ok(t) = UNIX_EPOCH
        .checked_add(Duration::from_secs(unix_ts as u64))
        .ok_or(())
        && let Ok(elapsed) = std::time::SystemTime::now().duration_since(t)
    {
        let secs = elapsed.as_secs();
        return if secs < 60 {
            format!("{secs}s ago")
        } else if secs < 3600 {
            format!("{}m ago", secs / 60)
        } else if secs < 86400 {
            format!("{}h ago", secs / 3600)
        } else {
            format!("{}d ago", secs / 86400)
        };
    }
    "unknown".to_string()
}
