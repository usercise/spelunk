use anyhow::{Context, Result};

use super::MemoryPushArgs;
use crate::{
    config::Config,
    storage::{NoteInput, open_memory_backend},
};

pub(super) async fn memory_push(
    args: MemoryPushArgs,
    mem_path: &std::path::Path,
    cfg: &Config,
) -> Result<()> {
    if cfg.memory_server_url.is_none() {
        anyhow::bail!(
            "memory_server_url is not configured.\n\
             Set it in .spelunk/config.toml or via SPELUNK_SERVER_URL."
        );
    }

    let src_path = args.source.as_deref().unwrap_or(mem_path);
    let local = crate::storage::MemoryStore::open(src_path)
        .with_context(|| format!("opening local memory at {}", src_path.display()))?;

    let notes = local.list(None, 10_000, args.include_archived)?;
    if notes.is_empty() {
        println!("No local memory entries to push.");
        return Ok(());
    }

    let remote = open_memory_backend(cfg, mem_path)?;
    println!(
        "Pushing {} entries to {}…",
        notes.len(),
        cfg.memory_server_url.as_deref().unwrap_or("?")
    );
    let mut pushed = 0usize;
    let mut skipped = 0usize;

    for note in &notes {
        let blob = local.get_embedding(note.id)?;
        let result = remote
            .add(NoteInput {
                kind: note.kind.clone(),
                title: note.title.clone(),
                body: note.body.clone(),
                tags: note.tags.clone(),
                linked_files: note.linked_files.clone(),
                embedding: blob,
                source_ref: note.source_ref.clone(),
                valid_at: note.valid_at,
                supersedes: None,
            })
            .await;
        match result {
            Ok(_) => pushed += 1,
            Err(e) => {
                eprintln!("  [skip] #{}: {e}", note.id);
                skipped += 1;
            }
        }
    }
    println!("Done. Pushed: {pushed}, skipped: {skipped}.");
    Ok(())
}
