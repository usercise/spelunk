use anyhow::{Context, Result};
use ignore::WalkBuilder;
use indicatif::MultiProgress;

use super::super::IndexArgs;
use crate::{config::Config, registry::Registry, storage::Database};

mod embed_phase;
mod parse_phase;
mod summaries;
mod worktree;

pub async fn index(args: IndexArgs, cfg: Config) -> Result<()> {
    // Compile secret-scanning regexes once before the hot loop.
    crate::indexer::secrets::init();

    // If running inside a git worktree, symlink .spelunk to the main worktree's
    // .spelunk so all worktrees share one index (SQLite WAL handles concurrent access).
    worktree::ensure_spelunk_symlink(&args.path);

    // Default DB lives inside the indexed directory, scoping the index to the project.
    let db_path = args
        .db
        .clone()
        .unwrap_or_else(|| args.path.join(".spelunk").join("index.db"));
    let db = match Database::open(&db_path) {
        Ok(db) => db,
        Err(e) => {
            if args.force && db_path.exists() {
                tracing::warn!("corrupt index detected, deleting and rebuilding: {e}");
                std::fs::remove_file(&db_path)
                    .with_context(|| format!("removing corrupt index at {}", db_path.display()))?;
                Database::open(&db_path)?
            } else {
                return Err(e).with_context(|| {
                    format!(
                        "failed to open index at {}\n\
                         The database may be corrupt. Run with --force to delete it and rebuild from scratch:\n\
                         \n  spelunk index {} --force\n",
                        db_path.display(),
                        args.path.display(),
                    )
                });
            }
        }
    };

    // Keep the global registry in sync with the current location.
    {
        let root_now = args
            .path
            .canonicalize()
            .unwrap_or_else(|_| args.path.clone());
        let db_now = db_path.canonicalize().unwrap_or_else(|_| db_path.clone());
        if let Ok(reg) = Registry::open() {
            let _ = reg.register(&root_now, &db_now);
        }
    }

    // --recount: backfill token_count for existing chunks, then exit.
    if args.recount {
        let updated = db.backfill_token_counts()?;
        println!("Backfilled token counts for {updated} chunk(s).");
        return Ok(());
    }

    // Canonicalise the root so symlinks don't create duplicate entries.
    let root_canonical = args
        .path
        .canonicalize()
        .unwrap_or_else(|_| args.path.clone());

    // ── Background-phases mode ────────────────────────────────────────────────
    // When spawned as a background process (--_background-phases), skip phases
    // 1 & 2 (walk, parse, embed) which are already done, and run only phases 3–5.
    if args.background_phases {
        run_background_phases(&args, &cfg, &db, &root_canonical, &db_path).await?;
        return Ok(());
    }

    let mp = MultiProgress::new();

    // ── Phase 1: parse + store chunks ────────────────────────────────────────
    let result = parse_phase::run_parse_phase(&root_canonical, &db, &args, &mp)?;
    if result.removed > 0 {
        eprintln!("Removed {} stale file(s) from index.", result.removed);
    }

    if result.chunk_ids_and_texts.is_empty() {
        let stats = db.stats()?;
        println!(
            "Index: {} files, {} chunks, {} embeddings (nothing new to embed)",
            stats.file_count, stats.chunk_count, stats.embedding_count
        );
        return Ok(());
    }

    // ── Phase 2: embed chunks ────────────────────────────────────────────────
    embed_phase::run_embed_phase(result.chunk_ids_and_texts, &db, &cfg, &args, &mp).await?;

    let stats = db.stats()?;
    println!(
        "\nIndex: {} files, {} chunks, {} embeddings",
        stats.file_count, stats.chunk_count, stats.embedding_count
    );

    // ── Background spawn for phases 3–5 ──────────────────────────────────────
    // When more than 100 files were newly indexed, detach phases 3-5 into a
    // background process so the user regains the prompt immediately.
    if result.indexed > 100 {
        eprintln!("Spawning background job for graph rank, spec discovery, and summaries\u{2026}");
        let mut cmd = std::process::Command::new(std::env::current_exe()?);
        cmd.arg("index");
        cmd.arg(&args.path);
        cmd.arg("--_background-phases");
        if let Some(db_arg) = &args.db {
            cmd.args(["--db", &db_arg.to_string_lossy()]);
        }
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        if cmd.spawn().is_ok() {
            return Ok(());
        }
        // Fall through and run phases 3-5 inline as fallback.
        tracing::warn!("failed to spawn background indexer; running inline");
    }

    run_phases_3_to_5(&args, &cfg, &db, &root_canonical, &db_path).await
}

// ── Phases 3–5 (shared between inline and background-phases mode) ─────────────

async fn run_phases_3_to_5(
    args: &IndexArgs,
    cfg: &Config,
    db: &Database,
    root_canonical: &std::path::Path,
    db_path: &std::path::Path,
) -> Result<()> {
    // Phase 3: PageRank
    eprintln!("Computing graph rank…");
    let edges = db.graph_edges_all()?;
    if !edges.is_empty() {
        let pr_scores = crate::indexer::pagerank::compute_pagerank(&edges, 20, 0.85);
        let named_chunks = db.chunks_with_names()?;
        let updates: Vec<(i64, f32)> = named_chunks
            .into_iter()
            .filter_map(|(id, name)| name.and_then(|n| pr_scores.get(&n).copied().map(|s| (id, s))))
            .collect();
        if !updates.is_empty() {
            db.update_graph_ranks(&updates)?;
        }
    }

    // Phase 4: auto-discover spec files
    run_spec_discovery(root_canonical, db, cfg)?;

    // Phase 5: LLM summaries — spawn a background thread so the caller
    // returns immediately. The thread opens its own DB connection because
    // `Database` (rusqlite::Connection) is not Send.
    let no_summaries = args.no_summaries;
    let summary_batch_size = args.summary_batch_size;
    let summary_cfg = cfg.clone();
    let summary_db_path = db_path.to_path_buf();
    eprintln!("Generating summaries in background\u{2026}");
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build();
        match rt {
            Ok(rt) => rt.block_on(async move {
                match crate::storage::Database::open(&summary_db_path) {
                    Ok(bg_db) => {
                        if let Err(e) = summaries::generate_summaries(
                            no_summaries,
                            summary_batch_size,
                            &summary_cfg,
                            &bg_db,
                        )
                        .await
                        {
                            eprintln!("summary error: {e}");
                        }
                    }
                    Err(e) => eprintln!("summary error: {e}"),
                }
            }),
            Err(e) => eprintln!("summary error: could not build runtime: {e}"),
        }
    });

    // Register / update this project in the global registry.
    if let Ok(reg) = Registry::open() {
        let db_canonical = db_path.canonicalize().unwrap_or(db_path.to_path_buf());
        if let Err(e) = reg.register(root_canonical, &db_canonical) {
            tracing::warn!("registry update failed: {e}");
        }
    }
    Ok(())
}

async fn run_background_phases(
    args: &IndexArgs,
    cfg: &Config,
    db: &Database,
    root_canonical: &std::path::Path,
    db_path: &std::path::Path,
) -> Result<()> {
    run_phases_3_to_5(args, cfg, db, root_canonical, db_path).await
}

fn run_spec_discovery(root: &std::path::Path, db: &Database, cfg: &Config) -> Result<()> {
    let files: Vec<_> = {
        let mut walk = WalkBuilder::new(root);
        walk.standard_filters(true);
        walk.build()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|ft| ft.is_file()).unwrap_or(false))
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("md"))
            .collect()
    };
    let mut specs_found = 0u32;
    for entry in &files {
        let path = entry.path();
        if super::spec::is_spec_file(path, &cfg.specs_dir) {
            let path_str = path.to_string_lossy().into_owned();
            let title = super::spec::extract_spec_title(path).unwrap_or_default();
            if let Err(e) = db.upsert_spec(&path_str, &title, true) {
                tracing::warn!("spec registration failed for {path_str}: {e}");
            } else {
                specs_found += 1;
            }
        }
    }
    if specs_found > 0 {
        eprintln!("Registered {specs_found} spec file(s).");
    }
    Ok(())
}
