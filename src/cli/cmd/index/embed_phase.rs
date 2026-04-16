use anyhow::{Context, Result};
use futures_util::StreamExt as _;
use indicatif::{MultiProgress, ProgressBar};

use super::super::super::IndexArgs;
use super::super::ui::{is_tty, progress_style};
use crate::{
    config::Config,
    embeddings::{EmbeddingBackend as _, vec_to_blob},
    storage::Database,
};

/// Embed all pending chunks and write their vectors to the DB.
///
/// Returns the number of chunks embedded.
pub(super) async fn run_embed_phase(
    chunk_ids_and_texts: Vec<(i64, String)>,
    db: &Database,
    cfg: &Config,
    args: &IndexArgs,
    mp: &MultiProgress,
) -> Result<u64> {
    eprintln!("Embedding via: {}", cfg.embedding_model);

    let embedder = crate::backends::ActiveEmbedder::load(cfg)
        .await
        .with_context(|| format!("loading embedding model '{}'", cfg.embedding_model))?;

    let batch_size = args.batch_size.max(1);
    let total_chunks = chunk_ids_and_texts.len() as u64;

    let embed_bar = if is_tty() && !crate::utils::is_agent_mode() {
        let bar = mp.add(ProgressBar::new(total_chunks));
        bar.set_style(progress_style("Embedding"));
        bar
    } else {
        ProgressBar::hidden()
    };

    let concurrency = batch_size;

    let results: Vec<(i64, Vec<f32>)> = futures_util::stream::iter(
        chunk_ids_and_texts
            .iter()
            .map(|(chunk_id, text)| (*chunk_id, text.clone())),
    )
    .map(|(chunk_id, text)| {
        let embedder = &embedder;
        let embed_bar = &embed_bar;
        async move {
            // Simple exponential-backoff retry for transient 429 / server errors.
            let mut delay_ms = 100u64;
            let mut last_err: anyhow::Error = anyhow::anyhow!("unreachable");
            for attempt in 0..3u32 {
                match embedder.embed(&[text.as_str()]).await {
                    Ok(mut vecs) => {
                        embed_bar.inc(1);
                        return Ok::<(i64, Vec<f32>), anyhow::Error>((chunk_id, vecs.remove(0)));
                    }
                    Err(e) => {
                        last_err = e;
                        if attempt < 2 {
                            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                            delay_ms *= 2;
                        }
                    }
                }
            }
            Err(last_err.context("generating embedding (3 attempts failed)"))
        }
    })
    .buffer_unordered(concurrency)
    .collect::<Vec<_>>()
    .await
    .into_iter()
    .collect::<Result<Vec<_>>>()?;

    // Write embeddings serially — rusqlite connections are not Send.
    for (chunk_id, embedding) in results {
        let blob = vec_to_blob(&embedding);
        db.insert_embedding(chunk_id, &blob)?;
    }

    embed_bar.finish_with_message(format!("{total_chunks} chunks embedded"));
    Ok(total_chunks)
}
