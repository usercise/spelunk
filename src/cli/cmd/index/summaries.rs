use anyhow::{Context, Result};

use super::super::super::IndexArgs;
use crate::{config::Config, storage::Database};

/// Run the optional LLM summary generation pass.
///
/// Fetches chunks without summaries in batches, calls the LLM, and stores results.
/// If no `llm_model` is configured or `--no-summaries` is set, prints a message and returns.
pub(super) async fn generate_summaries(
    args: &IndexArgs,
    cfg: &Config,
    db: &Database,
) -> Result<()> {
    if args.no_summaries {
        return Ok(());
    }

    if cfg.llm_model.is_none() {
        eprintln!("  Skipping summaries (no llm_model configured)");
        return Ok(());
    }

    // Count total chunks needing summaries for progress reporting.
    let batch_size = args.summary_batch_size.max(1);
    let first_batch = db.chunks_without_summaries(1)?;
    if first_batch.is_empty() {
        return Ok(());
    }

    // Load the LLM backend.
    let llm = crate::backends::ActiveLlm::load(cfg)
        .await
        .with_context(|| {
            format!(
                "loading LLM model '{}'",
                cfg.llm_model.as_deref().unwrap_or("unknown")
            )
        })?;

    // Count pending chunks for progress display.
    let pending = db.chunks_without_summaries(usize::MAX)?;
    let total_chunks = pending.len();
    let total_batches = total_chunks.div_ceil(batch_size);

    eprintln!("Generating summaries ({total_chunks} chunks, batch size {batch_size})\u{2026}");

    let mut batch_num = 0usize;
    loop {
        let batch = db.chunks_without_summaries(batch_size)?;
        if batch.is_empty() {
            break;
        }
        batch_num += 1;
        eprintln!("  Summarising batch {batch_num}/{total_batches}\u{2026}");

        match crate::indexer::summariser::summarise_batch(&llm, &batch).await {
            Ok(summaries) => {
                let mut summarised_ids = std::collections::HashSet::new();
                for (chunk_id, summary) in summaries {
                    if let Err(e) = db.update_chunk_summary(chunk_id, &summary) {
                        tracing::warn!("failed to store summary for chunk {chunk_id}: {e}");
                    } else {
                        summarised_ids.insert(chunk_id);
                    }
                }
                // Mark chunks that received no summary with "" so they aren't
                // re-fetched on the next pass (chunks_without_summaries checks IS NULL).
                for (id, _, _, _) in &batch {
                    if !summarised_ids.contains(id) {
                        let _ = db.update_chunk_summary(*id, "");
                    }
                }
            }
            Err(e) => {
                tracing::warn!("summarise_batch failed: {e}");
                // Mark the batch as attempted so we don't loop forever.
                for (id, _, _, _) in &batch {
                    let _ = db.update_chunk_summary(*id, "");
                }
            }
        }
    }

    eprintln!("  Summarised {batch_num} batch(es).");
    Ok(())
}
