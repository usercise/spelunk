//! Embedding backend using candle with the Metal GPU (Apple Silicon).
//!
//! Uses a BERT-architecture model for embeddings via mean pooling over the
//! last hidden states. Compatible with any BERT-family model on HuggingFace,
//! including code-focused ones like `microsoft/codebert-base`.
//!
//! # EmbeddingGemma note
//! The intended production model is Google's EmbeddingGemma (`google/gemma-embedding`).
//! Once that model's architecture and candle-transformers support are confirmed,
//! implement `src/embeddings/gemma.rs` following the same trait and wire it up
//! in `src/backends.rs`. For now, any BERT-family model can be substituted by
//! setting `embedding_model` in config.toml.

use anyhow::{anyhow, Context, Result};
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config as BertConfig};
use hf_hub::{api::tokio::Api, Repo, RepoType};
use std::path::Path;
use tokenizers::{PaddingParams, PaddingStrategy, Tokenizer, TruncationParams, TruncationStrategy};

use crate::embeddings::EmbeddingBackend;

pub struct CandleEmbedder {
    model: BertModel,
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
        let weights_file = repo
            .get("model.safetensors")
            .await
            .with_context(|| {
                format!(
                    "downloading model.safetensors for '{model_id}'. \
                     Make sure you have accepted the model license on HuggingFace \
                     and run `huggingface-cli login` if the model is gated."
                )
            })?;

        // ── Model config ─────────────────────────────────────────────────────
        let config_str = std::fs::read_to_string(&config_file)?;

        // Warn early if the model type is not BERT-family
        let raw: serde_json::Value = serde_json::from_str(&config_str)?;
        let model_type = raw["model_type"].as_str().unwrap_or("unknown");
        if !matches!(
            model_type,
            "bert" | "roberta" | "camembert" | "xlm-roberta" | "distilbert" | "nomic_bert"
        ) {
            tracing::warn!(
                "Model type '{model_type}' may not be compatible with the BERT embedding path. \
                 If it uses Gemma architecture, implement src/embeddings/gemma.rs instead."
            );
        }

        let config: BertConfig =
            serde_json::from_str(&config_str).context("parsing model config.json")?;
        let dim = config.hidden_size;

        // ── Tokenizer ────────────────────────────────────────────────────────
        let mut tokenizer = Tokenizer::from_file(&tokenizer_file)
            .map_err(|e| anyhow!("loading tokenizer: {e}"))?;

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

        // ── Model weights ────────────────────────────────────────────────────
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[&weights_file], DType::F32, &device)
                .context("loading model weights")?
        };
        let model = BertModel::load(vb, &config).context("building BERT model")?;

        tracing::info!("Model ready (dim={dim}, device={:?})", device);
        Ok(Self { model, tokenizer, device, dim })
    }
}

#[async_trait::async_trait]
impl EmbeddingBackend for CandleEmbedder {
    async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(vec![]);
        }

        // ── Tokenize ─────────────────────────────────────────────────────────
        let encodings = self
            .tokenizer
            .encode_batch(texts.iter().copied().collect::<Vec<_>>(), true)
            .map_err(|e| anyhow!("tokenization failed: {e}"))?;

        let max_len = encodings.iter().map(|e| e.len()).max().unwrap_or(0);

        // Build padded id / mask vectors
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

        // ── Build tensors ─────────────────────────────────────────────────────
        let input_ids_t = Tensor::new(input_ids, &self.device)
            .context("building input_ids tensor")?;
        let attention_mask_t = Tensor::new(attention_masks, &self.device)
            .context("building attention_mask tensor")?;
        let token_type_ids = input_ids_t
            .zeros_like()
            .context("building token_type_ids")?;

        // ── Forward pass ──────────────────────────────────────────────────────
        // Output: [batch, seq_len, hidden_size]
        let hidden_states = self
            .model
            .forward(&input_ids_t, &token_type_ids, Some(&attention_mask_t))
            .context("model forward pass")?;

        // ── Mean pooling over non-padding tokens ─────────────────────────────
        // mask_f: [batch, seq_len, 1]  (broadcast across hidden dim)
        let mask_f = attention_mask_t
            .to_dtype(DType::F32)?
            .unsqueeze(2)?
            .broadcast_as(hidden_states.shape())?;

        let sum = (hidden_states * &mask_f)?.sum(1)?;   // [batch, hidden_size]
        let count = attention_mask_t
            .to_dtype(DType::F32)?
            .sum(1)?
            .unsqueeze(1)?;                              // [batch, 1]
        let mean = sum.broadcast_div(&count)?;           // [batch, hidden_size]

        // ── L2 normalise ──────────────────────────────────────────────────────
        let norm = mean.sqr()?.sum_keepdim(1)?.sqrt()?;
        let normalised = mean.broadcast_div(&norm)?;

        // ── Collect ───────────────────────────────────────────────────────────
        normalised
            .to_vec2::<f32>()
            .context("converting embeddings to f32")
    }

    fn dimension(&self) -> usize {
        self.dim
    }
}

/// Serialise a float vector to raw little-endian bytes for sqlite-vec storage.
pub fn vec_to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// Deserialise raw little-endian bytes back to a float vector.
pub fn blob_to_vec(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4)
        .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
        .collect()
}
