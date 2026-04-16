use anyhow::{Context, Result};
use serde::Serialize;
use std::io::{BufRead as _, IsTerminal as _};

use crate::{config::Config, embeddings::EmbeddingBackend as _};

#[derive(Serialize)]
struct EmbedLine {
    index: usize,
    embedding: Vec<f32>,
}

pub(super) async fn embed_cmd(cfg: &Config) -> Result<()> {
    if std::io::stdin().is_terminal() {
        eprintln!(
            "spelunk plumbing embed: reads lines from stdin, emits NDJSON embedding per line"
        );
        std::process::exit(2);
    }

    let embedder = crate::backends::ActiveEmbedder::load(cfg)
        .await
        .context("loading embedding model")?;

    let stdin = std::io::stdin();
    for (idx, line) in stdin.lock().lines().enumerate() {
        let text = line.context("reading stdin")?;
        if text.trim().is_empty() {
            continue;
        }
        let mut vecs = embedder
            .embed(&[text.as_str()])
            .await
            .with_context(|| format!("embedding line {idx}"))?;
        println!(
            "{}",
            serde_json::to_string(&EmbedLine {
                index: idx,
                embedding: vecs.remove(0),
            })?
        );
    }
    Ok(())
}
