//! Embedding backend using candle with the Metal GPU (Apple Silicon).
//!
//! Supports two embedding architectures selected automatically from `config.json`:
//!
//! | `model_type`               | Architecture      | Example model                  |
//! |----------------------------|-------------------|--------------------------------|
//! | `bert`, `roberta`, …       | BERT (encoder)    | `BAAI/bge-base-en-v1.5`        |
//! | `gemma3`, `gemma3_text`    | Gemma 3 (encoder) | `google/embeddinggemma-300m`   |
//!
//! The Gemma 3 path uses the bidirectional encoder in `gemma3_encoder.rs` which
//! removes the causal attention mask so every token attends to all others — the key
//! change that turns the Gemma 3 decoder into an embedding model.

use anyhow::{anyhow, Context, Result};
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config as BertConfig};
use hf_hub::{api::tokio::Api, Repo, RepoType};
use std::path::Path;
use tokenizers::{PaddingParams, PaddingStrategy, Tokenizer, TruncationParams, TruncationStrategy};

use crate::embeddings::EmbeddingBackend;
use crate::embeddings::gemma3_encoder::Gemma3Encoder;

// ---------------------------------------------------------------------------
// Model variant
// ---------------------------------------------------------------------------

enum EmbedModel {
    Bert(BertModel),
    Gemma3(Gemma3Encoder),
}

// ---------------------------------------------------------------------------
// Public struct
// ---------------------------------------------------------------------------

pub struct CandleEmbedder {
    model: EmbedModel,
    tokenizer: Tokenizer,
    device: Device,
    dim: usize,
}

impl CandleEmbedder {
    /// Download (or use cached) model weights and load onto the Metal device.
    pub async fn load(model_id: &str, _cache_dir: &Path) -> Result<Self> {
        // ── Device ──────────────────────────────────────────────────────────
        let device = {
            #[cfg(feature = "backend-metal")]
            {
                match Device::new_metal(0) {
                    Ok(d) => {
                        tracing::info!("Embedding backend: Metal GPU");
                        d
                    }
                    Err(e) => {
                        tracing::warn!("Metal unavailable ({e}), falling back to CPU");
                        Device::Cpu
                    }
                }
            }
            #[cfg(not(feature = "backend-metal"))]
            Device::Cpu
        };

        // ── Download model files ─────────────────────────────────────────────
        tracing::info!("Loading embedding model: {model_id}");
        let api = Api::new().context("initialising HuggingFace Hub API")?;
        let repo = api.repo(Repo::new(model_id.to_string(), RepoType::Model));

        let config_file = repo
            .get("config.json")
            .await
            .with_context(|| format!("downloading config.json for '{model_id}'"))?;
        let tokenizer_file = repo
            .get("tokenizer.json")
            .await
            .with_context(|| format!("downloading tokenizer.json for '{model_id}'"))?;

        // ── Config + model type ──────────────────────────────────────────────
        let config_str = std::fs::read_to_string(&config_file)?;
        let raw: serde_json::Value = serde_json::from_str(&config_str)?;
        let model_type = raw["model_type"].as_str().unwrap_or("bert");
        tracing::info!("Embedding model type: {model_type}");

        // ── Tokenizer ────────────────────────────────────────────────────────
        let mut tokenizer = Tokenizer::from_file(&tokenizer_file)
            .map_err(|e| anyhow!("loading tokenizer: {e}"))?;

        // Determine architecture and dispatch.
        let (model, dim) = match model_type {
            "gemma3" | "gemma3_text" => {
                load_gemma3(model_id, &repo, &config_str, &raw, &mut tokenizer, &device).await?
            }
            _ => {
                if !matches!(model_type, "bert" | "roberta" | "camembert" | "xlm-roberta" | "distilbert" | "nomic_bert") {
                    tracing::warn!(
                        "Unknown model type '{model_type}'; attempting BERT architecture. \
                         For Gemma 3 embedding models use model_type = \"gemma3\" or \"gemma3_text\"."
                    );
                }
                load_bert(model_id, &repo, &config_str, &mut tokenizer, &device).await?
            }
        };

        tracing::info!("Embedding model ready (dim={dim}, device={:?})", device);
        Ok(Self { model, tokenizer, device, dim })
    }
}

// ---------------------------------------------------------------------------
// Architecture loaders
// ---------------------------------------------------------------------------

async fn load_gemma3(
    model_id: &str,
    repo: &hf_hub::api::tokio::ApiRepo,
    config_str: &str,
    raw: &serde_json::Value,
    tokenizer: &mut Tokenizer,
    device: &Device,
) -> Result<(EmbedModel, usize)> {
    let weights = get_weights(repo)
        .await
        .with_context(|| format!("downloading weights for '{model_id}'"))?;

    let cfg: crate::embeddings::gemma3_encoder::Config =
        serde_json::from_str(config_str).context("parsing Gemma3 config")?;
    let dim = cfg.hidden_size;

    let refs: Vec<&std::path::Path> = weights.iter().map(|p| p.as_path()).collect();
    let vb = unsafe {
        VarBuilder::from_mmaped_safetensors(&refs, DType::F32, device)
            .context("loading Gemma3 weights")?
    };

    let encoder = Gemma3Encoder::new(&cfg, vb)?;

    // Padding token: read from config or default to 0.
    let pad_id = raw["pad_token_id"].as_u64().unwrap_or(0) as u32;
    tokenizer.with_padding(Some(PaddingParams {
        strategy: PaddingStrategy::BatchLongest,
        pad_id,
        pad_token: "<pad>".to_string(),
        ..Default::default()
    }));
    tokenizer
        .with_truncation(Some(TruncationParams {
            max_length: 2048,
            strategy: TruncationStrategy::LongestFirst,
            ..Default::default()
        }))
        .map_err(|e| anyhow!("configuring truncation: {e}"))?;

    Ok((EmbedModel::Gemma3(encoder), dim))
}

async fn load_bert(
    model_id: &str,
    repo: &hf_hub::api::tokio::ApiRepo,
    config_str: &str,
    tokenizer: &mut Tokenizer,
    device: &Device,
) -> Result<(EmbedModel, usize)> {
    let weights_file = repo
        .get("model.safetensors")
        .await
        .with_context(|| {
            format!(
                "downloading model.safetensors for '{model_id}'. \
                 Make sure you have accepted the model licence on HuggingFace \
                 and run `huggingface-cli login` if the model is gated."
            )
        })?;

    let config: BertConfig =
        serde_json::from_str(config_str).context("parsing BERT config")?;
    let dim = config.hidden_size;

    tokenizer.with_padding(Some(PaddingParams {
        strategy: PaddingStrategy::BatchLongest,
        ..Default::default()
    }));
    tokenizer
        .with_truncation(Some(TruncationParams {
            max_length: 512,
            strategy: TruncationStrategy::LongestFirst,
            ..Default::default()
        }))
        .map_err(|e| anyhow!("configuring truncation: {e}"))?;

    let vb = unsafe {
        VarBuilder::from_mmaped_safetensors(&[&weights_file], DType::F32, device)
            .context("loading BERT weights")?
    };
    let model = BertModel::load(vb, &config).context("building BERT model")?;

    Ok((EmbedModel::Bert(model), dim))
}

// ---------------------------------------------------------------------------
// EmbeddingBackend impl
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl EmbeddingBackend for CandleEmbedder {
    async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(vec![]);
        }

        // ── Tokenize ─────────────────────────────────────────────────────────
        let encodings = self
            .tokenizer
            .encode_batch(texts.to_vec(), true)
            .map_err(|e| anyhow!("tokenization failed: {e}"))?;

        let max_len = encodings.iter().map(|e| e.len()).max().unwrap_or(0);

        let (input_ids, attention_masks): (Vec<Vec<u32>>, Vec<Vec<u32>>) = encodings
            .iter()
            .map(|enc| {
                let mut ids = enc.get_ids().to_vec();
                let mut mask = enc.get_attention_mask().to_vec();
                ids.resize(max_len, 0);
                mask.resize(max_len, 0);
                (ids, mask)
            })
            .unzip();

        let input_ids_t = Tensor::new(input_ids, &self.device)
            .context("building input_ids tensor")?;
        let attention_mask_t = Tensor::new(attention_masks, &self.device)
            .context("building attention_mask tensor")?;

        match &self.model {
            // ── BERT: mean pool over hidden states ───────────────────────────
            EmbedModel::Bert(model) => {
                let token_type_ids = input_ids_t
                    .zeros_like()
                    .context("building token_type_ids")?;

                let hidden = model
                    .forward(&input_ids_t, &token_type_ids, Some(&attention_mask_t))
                    .context("BERT forward pass")?;

                // Mean pool: mask_f [batch, seq, 1] broadcast over hidden dim.
                let mask_f = attention_mask_t
                    .to_dtype(DType::F32)?
                    .unsqueeze(2)?
                    .broadcast_as(hidden.shape())?;

                let sum = (&hidden * &mask_f)?.sum(1)?;
                let count = attention_mask_t
                    .to_dtype(DType::F32)?
                    .sum(1)?
                    .unsqueeze(1)?;
                let mean = sum.broadcast_div(&count)?;

                // L2 normalise.
                let norm = mean.sqr()?.sum_keepdim(1)?.sqrt()?;
                let normalised = mean.broadcast_div(&norm)?;

                normalised
                    .to_vec2::<f32>()
                    .context("converting BERT embeddings to f32")
            }

            // ── Gemma 3: bidirectional encoder with mean pooling ─────────────
            EmbedModel::Gemma3(encoder) => {
                let embeddings = encoder
                    .embed(&input_ids_t, Some(&attention_mask_t))
                    .context("Gemma3 encoder forward pass")?;

                embeddings
                    .to_vec2::<f32>()
                    .context("converting Gemma3 embeddings to f32")
            }
        }
    }

    fn dimension(&self) -> usize {
        self.dim
    }
}

// ---------------------------------------------------------------------------
// Weight download helper (single file or sharded)
// ---------------------------------------------------------------------------

async fn get_weights(repo: &hf_hub::api::tokio::ApiRepo) -> Result<Vec<std::path::PathBuf>> {
    if let Ok(p) = repo.get("model.safetensors").await {
        return Ok(vec![p]);
    }
    let index_file = repo
        .get("model.safetensors.index.json")
        .await
        .context("no model.safetensors or model.safetensors.index.json found")?;

    let index: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&index_file)?)?;
    let weight_map = index["weight_map"]
        .as_object()
        .context("malformed model.safetensors.index.json")?;

    let mut shard_names: Vec<String> = weight_map
        .values()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    shard_names.sort_unstable();
    shard_names.dedup();

    let mut paths = Vec::with_capacity(shard_names.len());
    for name in &shard_names {
        tracing::info!("Downloading shard: {name}");
        paths.push(
            repo.get(name)
                .await
                .with_context(|| format!("downloading shard {name}"))?,
        );
    }
    Ok(paths)
}

// ---------------------------------------------------------------------------
// Byte-level serialisation helpers used by storage layer
// ---------------------------------------------------------------------------

/// Serialise a float vector to raw little-endian bytes for sqlite-vec storage.
pub fn vec_to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// Deserialise raw little-endian bytes back to a float vector.
#[allow(dead_code)]
pub fn blob_to_vec(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4)
        .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
        .collect()
}
