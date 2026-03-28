//! Embedding backend that delegates to any OpenAI-compatible server via the
//! standard `/v1/embeddings` endpoint.
//!
//! Works with LM Studio, Ollama, vLLM, and any other server that exposes the
//! OpenAI embeddings API at `api_base_url` (default: `http://127.0.0.1:1234`).
//! An embedding model must be loaded and its API identifier passed as
//! `embedding_model` in the config (e.g. `text-embedding-embeddinggemma-300m-qat`).

use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::embeddings::EmbeddingBackend;

pub struct OpenAiCompatEmbedder {
    client: Client,
    base_url: String,
    model: String,
}

impl OpenAiCompatEmbedder {
    pub async fn load(cfg: &crate::config::Config) -> Result<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .context("building HTTP client for OpenAI-compatible embedder")?;
        tracing::info!(
            "OpenAI-compat embedder: {} model={}",
            cfg.api_base_url,
            cfg.embedding_model
        );
        Ok(Self {
            client,
            base_url: cfg.api_base_url.clone(),
            model: cfg.embedding_model.clone(),
        })
    }
}

// ---------------------------------------------------------------------------
// Request / response types (OpenAI spec)
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
impl EmbeddingBackend for OpenAiCompatEmbedder {
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
                    "calling /v1/embeddings at {}. \
                     Is your OpenAI-compatible server running with an embedding model loaded?",
                    self.base_url
                )
            })?
            .error_for_status()
            .context("embeddings API returned an error")?
            .json()
            .await
            .context("parsing embeddings response")?;

        if resp.data.is_empty() {
            anyhow::bail!("server returned 0 embeddings for {} input(s)", texts.len());
        }

        Ok(resp.data.into_iter().map(|d| d.embedding).collect())
    }

    fn dimension(&self) -> usize {
        crate::embeddings::EMBEDDING_DIM
    }
}
