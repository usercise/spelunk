use anyhow::{Context, Result};
use futures_util::StreamExt as _;
use ignore::WalkBuilder;

use super::super::{SnapshotArgs, SnapshotCommand};
use super::status::format_age;
use super::ui::{is_tty, progress_style};
use crate::{
    config::{Config, resolve_db},
    embeddings::{EmbeddingBackend as _, vec_to_blob},
    indexer::{
        parser::{SourceParser, detect_language, is_binary_file},
        secrets,
    },
    search::tokens::estimate_tokens,
    storage::Database,
};

pub async fn snapshot(args: SnapshotArgs, cfg: Config) -> Result<()> {
    let db_path = args
        .db
        .clone()
        .unwrap_or_else(|| resolve_db(None, &cfg.db_path));
    match args.command {
        SnapshotCommand::Create(a) => snapshot_create(a, &db_path, &cfg).await,
        SnapshotCommand::List(a) => snapshot_list(a, &db_path),
        SnapshotCommand::Delete(a) => snapshot_delete(a, &db_path),
    }
}

async fn snapshot_create(
    args: super::super::SnapshotCreateArgs,
    db_path: &std::path::Path,
    cfg: &Config,
) -> Result<()> {
    // Resolve the commit ref to a full SHA.
    let sha_output = std::process::Command::new("git")
        .args(["rev-parse", "--verify", &args.commit])
        .output()
        .context("running git rev-parse (is this a git repo?)")?;
    if !sha_output.status.success() {
        anyhow::bail!(
            "Could not resolve '{}' as a git ref: {}",
            args.commit,
            String::from_utf8_lossy(&sha_output.stderr).trim()
        );
    }
    let commit_sha = String::from_utf8_lossy(&sha_output.stdout)
        .trim()
        .to_string();

    let db = Database::open(db_path)?;

    // Check for duplicate.
    if db.get_snapshot_by_sha(&commit_sha)?.is_some() {
        anyhow::bail!(
            "Snapshot for {} already exists. Delete it first with `spelunk snapshot delete {}`.",
            &commit_sha[..8],
            &commit_sha[..8]
        );
    }

    let snapshot_id = db.create_snapshot(&commit_sha)?;
    println!("Snapshotting commit {} …", &commit_sha[..12]);

    // Create a temporary directory path for the git worktree.
    let worktree_path = std::env::temp_dir().join(format!("spelunk-snap-{}", &commit_sha[..12]));

    let wt_output = std::process::Command::new("git")
        .args([
            "worktree",
            "add",
            "--detach",
            &worktree_path.to_string_lossy(),
            &commit_sha,
        ])
        .output()
        .context("running git worktree add")?;
    if !wt_output.status.success() {
        let _ = db.delete_snapshot(&commit_sha);
        anyhow::bail!(
            "git worktree add failed: {}",
            String::from_utf8_lossy(&wt_output.stderr).trim()
        );
    }

    let result = do_snapshot_index(
        &db,
        snapshot_id,
        &commit_sha,
        &worktree_path,
        args.batch_size,
        cfg,
    )
    .await;

    // Always clean up the worktree, even on error.
    let _ = std::process::Command::new("git")
        .args([
            "worktree",
            "remove",
            "--force",
            &worktree_path.to_string_lossy(),
        ])
        .status();

    if let Err(e) = result {
        // Roll back the snapshot.
        let _ = db.delete_snapshot(&commit_sha);
        return Err(e);
    }

    Ok(())
}

async fn do_snapshot_index(
    db: &Database,
    snapshot_id: i64,
    commit_sha: &str,
    root: &std::path::Path,
    batch_size: usize,
    cfg: &Config,
) -> Result<()> {
    secrets::init();

    // ── Walk files ─────────────────────────────────────────────────────────
    let walker = WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .build();

    let mut files: Vec<std::path::PathBuf> = vec![];
    for entry in walker.flatten() {
        let path = entry.path().to_path_buf();
        if !path.is_file() {
            continue;
        }
        // Skip .git directory itself.
        if path.components().any(|c| c.as_os_str() == ".git") {
            continue;
        }
        files.push(path);
    }

    // ── Parse chunks ────────────────────────────────────────────────────────
    let mp = indicatif::MultiProgress::new();
    let parse_bar = if is_tty() && !crate::utils::is_agent_mode() {
        let bar = mp.add(indicatif::ProgressBar::new(files.len() as u64));
        bar.set_style(progress_style("Parsing"));
        bar
    } else {
        indicatif::ProgressBar::hidden()
    };

    let mut chunk_ids_and_texts: Vec<(i64, String)> = vec![];
    let mut file_count = 0i64;

    for path in &files {
        parse_bar.inc(1);

        let path_str = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        // Detect language; skip binaries.
        let language = match detect_language(path) {
            Some(l) => l,
            None => continue,
        };

        let source = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(_) => continue,
        };

        if is_binary_file(path) {
            continue;
        }

        let hash = format!("{}", blake3::hash(source.as_bytes()));
        let file_id = db.insert_snapshot_file(snapshot_id, &path_str, Some(language), &hash)?;
        file_count += 1;

        let chunks = match SourceParser::parse(&source, &path_str, language) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("parse error for {path_str}: {e}");
                continue;
            }
        };

        for chunk in &chunks {
            if secrets::contains_secret(&chunk.content) {
                continue;
            }
            let metadata = serde_json::json!({
                "docstring": chunk.docstring,
                "parent_scope": chunk.parent_scope,
            });
            let tc = estimate_tokens(&chunk.content);
            let chunk_id = db.insert_snapshot_chunk(
                snapshot_id,
                file_id,
                &chunk.kind.to_string(),
                chunk.name.as_deref(),
                chunk.start_line,
                chunk.end_line,
                &chunk.content,
                Some(&metadata.to_string()),
                tc,
            )?;
            chunk_ids_and_texts.push((chunk_id, chunk.embedding_text()));
        }
    }
    parse_bar.finish_and_clear();

    let chunk_count = chunk_ids_and_texts.len() as i64;

    if chunk_ids_and_texts.is_empty() {
        println!("No chunks parsed for snapshot {}.", &commit_sha[..8]);
        db.update_snapshot_stats(snapshot_id, file_count, 0)?;
        return Ok(());
    }

    // ── Embed ───────────────────────────────────────────────────────────────
    eprintln!("Embedding {} chunks…", chunk_count);

    let embedder = crate::backends::ActiveEmbedder::load(cfg)
        .await
        .with_context(|| format!("loading embedding model '{}'", cfg.embedding_model))?;

    let total = chunk_ids_and_texts.len() as u64;
    let embed_bar = if is_tty() && !crate::utils::is_agent_mode() {
        let bar = mp.add(indicatif::ProgressBar::new(total));
        bar.set_style(progress_style("Embedding"));
        bar
    } else {
        indicatif::ProgressBar::hidden()
    };

    let concurrency = batch_size.max(1);
    let results: Vec<(i64, Vec<f32>)> = futures_util::stream::iter(
        chunk_ids_and_texts
            .iter()
            .map(|(chunk_id, text)| (*chunk_id, text.clone())),
    )
    .map(|(chunk_id, text)| {
        let embedder = &embedder;
        let embed_bar = &embed_bar;
        async move {
            let mut delay_ms = 100u64;
            let mut last_err: anyhow::Error = anyhow::anyhow!("unreachable");
            for _ in 0..5 {
                match embedder.embed(&[text.as_str()]).await {
                    Ok(vecs) => {
                        embed_bar.inc(1);
                        return vecs.into_iter().next().map(|v| (chunk_id, v));
                    }
                    Err(e) => {
                        last_err = e;
                        tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
                        delay_ms = (delay_ms * 2).min(4_000);
                    }
                }
            }
            tracing::warn!("embedding failed for chunk {chunk_id}: {last_err}");
            None
        }
    })
    .buffer_unordered(concurrency)
    .filter_map(|x| async move { x })
    .collect()
    .await;

    embed_bar.finish_and_clear();

    for (chunk_id, vec) in &results {
        let blob = vec_to_blob(vec);
        db.insert_snapshot_embedding(*chunk_id, &blob)?;
    }

    db.update_snapshot_stats(snapshot_id, file_count, chunk_count)?;

    println!(
        "Snapshot {}: {} files, {} chunks, {} embeddings",
        &commit_sha[..12],
        file_count,
        chunk_count,
        results.len()
    );

    Ok(())
}

fn snapshot_list(args: super::super::SnapshotListArgs, db_path: &std::path::Path) -> Result<()> {
    let db = Database::open(db_path)?;
    let snapshots = db.list_snapshots()?;

    if snapshots.is_empty() {
        println!("No snapshots. Create one with `spelunk snapshot create [<commit>]`.");
        return Ok(());
    }

    if crate::utils::effective_format(&args.format) == "json" {
        println!("{}", serde_json::to_string_pretty(&snapshots)?);
        return Ok(());
    }

    println!(
        "{:<12}  {:<14}  {:<8}  {:<8}  ",
        "SHA", "Created", "Files", "Chunks"
    );
    println!("{}", "─".repeat(60));
    for s in &snapshots {
        println!(
            "{:<12}  {:<14}  {:<8}  {:<8}",
            &s.commit_sha[..s.commit_sha.len().min(12)],
            format_age(s.created_at),
            s.file_count,
            s.chunk_count,
        );
    }
    Ok(())
}

fn snapshot_delete(
    args: super::super::SnapshotDeleteArgs,
    db_path: &std::path::Path,
) -> Result<()> {
    let db = Database::open(db_path)?;

    // Find by prefix.
    let snap = db
        .list_snapshots()?
        .into_iter()
        .find(|s| s.commit_sha.starts_with(args.sha.as_str()))
        .ok_or_else(|| anyhow::anyhow!("No snapshot found matching '{}'.", args.sha))?;

    // vec0 doesn't honour ON DELETE CASCADE — delete embeddings explicitly first.
    db.delete_snapshot_embeddings_for_snapshot(snap.id)?;
    let deleted = db.delete_snapshot(&snap.commit_sha)?;

    if deleted {
        println!("Deleted snapshot {}.", &snap.commit_sha[..12]);
    } else {
        println!("No snapshot found for '{}'.", args.sha);
    }
    Ok(())
}
