//! Embedding backend that delegates to an LM Studio server via its
//! OpenAI-compatible `/v1/embeddings` endpoint.
//!
//! Requires LM Studio running at `lmstudio_base_url` (default: `http://127.0.0.1:1234`)
//! with an embedding model loaded (e.g. `text-embedding-embeddinggemma-300m-qat`).

use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::embeddings::EmbeddingBackend;

pub struct LmStudioEmbedder {
    client: Client,
    base_url: String,
    model: String,
}

impl LmStudioEmbedder {
    pub async fn load(cfg: &crate::config::Config) -> Result<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .context("building HTTP client for LM Studio embedder")?;
        tracing::info!(
            "LM Studio embedder: {} model={}",
            cfg.lmstudio_base_url,
            cfg.embedding_model
        );
        Ok(Self {
            client,
            base_url: cfg.lmstudio_base_url.clone(),
            model: cfg.embedding_model.clone(),
        })
    }
}

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct EmbedRequest<'a> {
    model: &'a str,
    input: &'a [&'a str],
}

#[derive(Deserialize)]
struct EmbedResponse {
    data: Vec<EmbedData>,
}

#[derive(Deserialize)]
struct EmbedData {
    embedding: Vec<f32>,
}

// ---------------------------------------------------------------------------
// EmbeddingBackend impl
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl EmbeddingBackend for LmStudioEmbedder {
    async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        // Append <eos> to each input so the GGUF tokenizer produces a proper
        // final token. The GGUF for embeddinggemma-300m-qat is missing
        // `tokenizer.ggml.add_eos_token = true`, causing llama.cpp to skip the
        // EOS token it was trained with. Appending the literal "<eos>" string
        // makes the Gemma tokenizer emit the actual EOS token ID.
        let with_eos: Vec<String> = texts.iter().map(|t| format!("{t}<eos>")).collect();
        let input_refs: Vec<&str> = with_eos.iter().map(String::as_str).collect();
        let req = EmbedRequest {
            model: &self.model,
            input: &input_refs,
        };
        let resp: EmbedResponse = self
            .client
            .post(format!("{}/v1/embeddings", self.base_url))
            .json(&req)
            .send()
            .await
            .with_context(|| {
                format!(
                    "calling LM Studio /v1/embeddings at {}. \
                     Is LM Studio running with an embedding model loaded?",
                    self.base_url
                )
            })?
            .error_for_status()
            .context("LM Studio embeddings API returned an error")?
            .json()
            .await
            .context("parsing LM Studio embeddings response")?;

        if resp.data.is_empty() {
            anyhow::bail!(
                "LM Studio returned 0 embeddings for {} input(s)",
                texts.len()
            );
        }

        Ok(resp.data.into_iter().map(|d| d.embedding).collect())
    }

    fn dimension(&self) -> usize {
        crate::embeddings::EMBEDDING_DIM
    }
}
