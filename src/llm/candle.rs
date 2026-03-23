//! Text generation via candle with the Metal GPU (Apple Silicon).
//!
//! Supports `model_type = "gemma"` (Gemma 1/2) and `model_type = "gemma3"`
//! (Gemma 3).  Both variants use `candle_transformers::models::gemma` or
//! `gemma3` respectively, selected by the `model_type` field in `config.json`.
//!
//! # Gemma 3n
//! Gemma 3n (`gemma3n`) has a different architecture (mixed sparse/dense
//! attention, different layer structure) and is **not compatible** with the
//! `gemma3::Config` in candle-transformers 0.8.  Attempting to load a Gemma 3n
//! model will fail at config parsing.  Support requires a newer candle release.
//!
//! # Recommended model
//! Set `llm_model = "google/gemma-3-1b-it"` in config.toml (default).
//! All Gemma models are gated — accept the licence on HuggingFace first:
//!   huggingface-cli login
//!
//! # Generation strategy
//! All GPU compute runs synchronously inside a `Mutex` guard (no `.await`),
//! producing a `Vec<String>` of decoded deltas. The guard is released before
//! the tokens are streamed through the async channel, so the async runtime
//! is never blocked.

use std::sync::Mutex;
use anyhow::{anyhow, Context, Result};
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use hf_hub::{api::tokio::Api, Repo, RepoType};
use tokenizers::Tokenizer;
use tokio::sync::mpsc;

use crate::llm::{LlmBackend, Token};

// ---------------------------------------------------------------------------
// Internal model variant
// ---------------------------------------------------------------------------

enum GemmaModel {
    V1(candle_transformers::models::gemma::Model),
    V3(candle_transformers::models::gemma3::Model),
}

impl GemmaModel {
    // Use candle_core::Result so we can ? straight through from the inner models.
    fn forward(&mut self, ids: &Tensor, offset: usize) -> candle_core::Result<Tensor> {
        match self {
            Self::V1(m) => m.forward(ids, offset),
            Self::V3(m) => m.forward(ids, offset),
        }
    }

    fn clear_kv_cache(&mut self) {
        match self {
            Self::V1(m) => m.clear_kv_cache(),
            Self::V3(m) => m.clear_kv_cache(),
        }
    }
}

// SAFETY: candle Metal tensors / devices are Send + Sync.
unsafe impl Send for GemmaModel {}
unsafe impl Sync for GemmaModel {}

// ---------------------------------------------------------------------------
// Public struct
// ---------------------------------------------------------------------------

pub struct CandleLlm {
    inner: Mutex<GemmaModel>,
    tokenizer: Tokenizer,
    device: Device,
    stop_tokens: Vec<u32>,
}

impl CandleLlm {
    pub async fn load(cfg: &crate::config::Config) -> Result<Self> {
        let model_id = &cfg.llm_model;
        let _cache_dir = &cfg.models_dir;
        // ── Device ──────────────────────────────────────────────────────────
        let device = {
            #[cfg(feature = "backend-metal")]
            {
                match Device::new_metal(0) {
                    Ok(d) => { tracing::info!("LLM backend: Metal GPU"); d }
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
        tracing::info!("Loading LLM: {model_id}");
        let api = Api::new().context("initialising HuggingFace Hub API")?;
        let repo = api.repo(Repo::new(model_id.to_string(), RepoType::Model));

        let config_file = repo.get("config.json").await
            .with_context(|| format!("downloading config.json for '{model_id}'.\n\
                If the model is gated, run: huggingface-cli login"))?;
        let tokenizer_file = repo.get("tokenizer.json").await
            .with_context(|| format!("downloading tokenizer.json for '{model_id}'"))?;
        let weights = get_weights(&repo).await
            .with_context(|| format!("downloading weights for '{model_id}'"))?;

        // ── Config + model type ───────────────────────────────────────────────
        let config_str = std::fs::read_to_string(&config_file)?;
        let raw: serde_json::Value = serde_json::from_str(&config_str)?;
        let model_type = raw["model_type"].as_str().unwrap_or("gemma");
        tracing::info!("Model type: {model_type}");

        // ── Tokenizer + stop tokens ───────────────────────────────────────────
        let tokenizer = Tokenizer::from_file(&tokenizer_file)
            .map_err(|e| anyhow!("loading tokenizer: {e}"))?;

        let mut stop_tokens: Vec<u32> = Vec::new();
        for name in &["<eos>", "</s>", "<end_of_turn>", "<|im_end|>"] {
            if let Some(id) = tokenizer.token_to_id(name) {
                stop_tokens.push(id);
            }
        }
        if let Some(id) = raw["eos_token_id"].as_u64() {
            stop_tokens.push(id as u32);
        }
        stop_tokens.sort_unstable();
        stop_tokens.dedup();
        tracing::debug!("Stop tokens: {stop_tokens:?}");

        // ── Load weights ──────────────────────────────────────────────────────
        let refs: Vec<&std::path::Path> = weights.iter().map(|p| p.as_path()).collect();
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&refs, DType::F32, &device)
                .context("loading model weights")?
        };

        // ── Build model variant ───────────────────────────────────────────────
        let model = match model_type {
            "gemma3n" => {
                anyhow::bail!(
                    "Gemma 3n is not supported by candle-transformers 0.9 — its architecture \
                     differs from standard transformers and has no candle implementation yet."
                );
            }
            // "gemma3_text" is the model_type in google/gemma-3-*-it config.json;
            // "gemma3" appears in some community re-uploads.
            "gemma3" | "gemma3_text" => {
                let cfg: candle_transformers::models::gemma3::Config =
                    serde_json::from_str(&config_str).context("parsing gemma3 config")?;
                GemmaModel::V3(
                    candle_transformers::models::gemma3::Model::new(false, &cfg, vb)
                        .context("building gemma3 model")?,
                )
            }
            _ => {
                if !matches!(model_type, "gemma" | "gemma2") {
                    tracing::warn!("Unknown model type '{model_type}', trying Gemma 1/2 architecture");
                }
                let cfg: candle_transformers::models::gemma::Config =
                    serde_json::from_str(&config_str).context("parsing gemma config")?;
                GemmaModel::V1(
                    candle_transformers::models::gemma::Model::new(false, &cfg, vb)
                        .context("building gemma model")?,
                )
            }
        };

        tracing::info!("LLM ready (device={:?})", device);
        Ok(Self { inner: Mutex::new(model), tokenizer, device, stop_tokens })
    }
}

// ---------------------------------------------------------------------------
// LlmBackend
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl LlmBackend for CandleLlm {
    async fn generate(
        &self,
        messages: &[crate::llm::Message],
        max_tokens: usize,
        tx: mpsc::Sender<Token>,
        _json_schema: Option<serde_json::Value>,
    ) -> Result<()> {
        let prompt = build_gemma_prompt(messages);

        // ── All GPU work runs synchronously inside the Mutex guard ────────────
        // The guard is released before any `.await`, so the async runtime is
        // never blocked and `MutexGuard` never crosses an await boundary.
        let output_tokens: Vec<String> = {
            let mut model = self.inner.lock()
                .map_err(|_| anyhow!("LLM mutex poisoned"))?;

            run_generation(
                &mut model,
                &self.tokenizer,
                &self.device,
                &self.stop_tokens,
                &prompt,
                max_tokens,
            )?
        }; // ← MutexGuard dropped here

        // ── Stream the collected tokens through the async channel ─────────────
        for token in output_tokens {
            if tx.send(token).await.is_err() {
                break; // Receiver dropped (e.g. user cancelled)
            }
        }
        Ok(())
    }
}

/// Build a Gemma chat-format prompt from a list of messages.
/// System content is prepended to the first user turn (Gemma has no system role).
fn build_gemma_prompt(messages: &[crate::llm::Message]) -> String {
    let mut system_content = String::new();
    let mut turns = String::new();

    for msg in messages {
        match msg.role.as_str() {
            "system" => {
                system_content = msg.content.clone();
            }
            "user" => {
                let content = if system_content.is_empty() {
                    msg.content.clone()
                } else {
                    format!("{}\n\n{}", system_content, msg.content)
                };
                turns.push_str(&format!("<start_of_turn>user\n{content}<end_of_turn>\n"));
                system_content.clear();
            }
            "assistant" | "model" => {
                turns.push_str(&format!(
                    "<start_of_turn>model\n{}<end_of_turn>\n",
                    msg.content
                ));
            }
            _ => {}
        }
    }

    format!("<bos>{turns}<start_of_turn>model\n")
}

// ---------------------------------------------------------------------------
// Core generation loop (synchronous — runs on whatever thread calls it)
// ---------------------------------------------------------------------------

fn run_generation(
    model: &mut GemmaModel,
    tokenizer: &Tokenizer,
    device: &Device,
    stop_tokens: &[u32],
    prompt: &str,
    max_tokens: usize,
) -> Result<Vec<String>> {
    model.clear_kv_cache();

    // Gemma requires exactly one BOS token at position 0.
    // The tokenizer's post-processor may add BOS (add_special_tokens=true) on
    // top of a BOS the normalizer already produces, resulting in double BOS
    // which corrupts the model's attention.  Instead, encode without the
    // post-processor and ensure exactly one BOS at the front.
    let bos_id = tokenizer.token_to_id("<bos>").unwrap_or(2);
    let encoding = tokenizer
        .encode(prompt, false)
        .map_err(|e| anyhow!("tokenising prompt: {e}"))?;
    let raw_ids = encoding.get_ids();
    let prompt_tokens: Vec<u32> = if raw_ids.first().copied() == Some(bos_id) {
        // Normalizer already added BOS — use as-is.
        raw_ids.to_vec()
    } else {
        // No BOS from normalizer — prepend one.
        let mut v = vec![bos_id];
        v.extend_from_slice(raw_ids);
        v
    };

    // Safety: Gemma 3 1B has 4 attention heads; Metal's ~4 GB single-buffer
    // limit constrains full-context (global) attention layers to ~L < 11 500
    // tokens (32 × L² bytes for two simultaneous attn buffers).  Truncate to
    // a conservative 6 144 tokens, always keeping the tail so the question at
    // the end of the Gemma chat prompt is never dropped.
    const MAX_PROMPT_TOKENS: usize = 6144;
    let prompt_tokens = if prompt_tokens.len() > MAX_PROMPT_TOKENS {
        tracing::warn!(
            "Prompt is {} tokens — truncating to {} to fit Metal attention limits. \
             Use fewer context chunks to avoid this.",
            prompt_tokens.len(),
            MAX_PROMPT_TOKENS,
        );
        let mut truncated = vec![bos_id];
        let keep = MAX_PROMPT_TOKENS - 1;
        truncated.extend_from_slice(&prompt_tokens[prompt_tokens.len() - keep..]);
        truncated
    } else {
        prompt_tokens
    };

    let prompt_len = prompt_tokens.len();

    let mut all_tokens = prompt_tokens.clone();
    let mut decoded_so_far = String::new();
    let mut output: Vec<String> = Vec::new();

    // Prefill: process entire prompt, get logits for last position.
    {
        let input = Tensor::new(prompt_tokens.as_slice(), device)?.unsqueeze(0)?;
        let logits_all = model.forward(&input, 0).map_err(anyhow::Error::from)?;
        // logits_all: [1, prompt_len, vocab] — only the last position is meaningful.
        let last = logits_all.dim(1)? - 1;
        let logits = logits_all.narrow(1, last, 1)?;  // [1, 1, vocab]
        let next = sample_token(&logits, 0.7)?;
        if stop_tokens.contains(&next) {
            return Ok(output);
        }
        all_tokens.push(next);
        let delta = incremental_decode(tokenizer, &all_tokens, prompt_len, &mut decoded_so_far)?;
        if !delta.is_empty() {
            output.push(delta);
        }
    }

    // Autoregressive decode: one token at a time.
    for step in 1..max_tokens {
        let last = *all_tokens.last().unwrap();
        let input = Tensor::new(&[last], device)?.unsqueeze(0)?;
        let offset = prompt_len + step - 1;
        let logits = model.forward(&input, offset).map_err(anyhow::Error::from)?;

        let next = sample_token(&logits, 0.7)?;
        if stop_tokens.contains(&next) {
            break;
        }

        all_tokens.push(next);
        let delta = incremental_decode(tokenizer, &all_tokens, prompt_len, &mut decoded_so_far)?;
        if !delta.is_empty() {
            output.push(delta);
        }
    }

    Ok(output)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Sample from logits `[1, 1, vocab]` with temperature.
fn sample_token(logits: &Tensor, temperature: f64) -> Result<u32> {
    // logits: [batch=1, seq=1, vocab] → squeeze to [vocab]
    let logits = logits.squeeze(0)?.squeeze(0)?;

    if temperature == 0.0 {
        return Ok(logits.argmax(candle_core::D::Minus1)?.to_scalar::<u32>()?);
    }

    let scaled = (&logits / temperature)?;
    let probs = candle_nn::ops::softmax_last_dim(&scaled)?.to_vec1::<f32>()?;

    let threshold: f32 = rand::random();
    let mut cumulative = 0.0f32;
    for (i, &p) in probs.iter().enumerate() {
        cumulative += p;
        if cumulative >= threshold {
            return Ok(i as u32);
        }
    }
    Ok((probs.len().saturating_sub(1)) as u32)
}

/// Return only the newly generated text since last call.
fn incremental_decode(
    tokenizer: &Tokenizer,
    all_tokens: &[u32],
    prompt_len: usize,
    decoded_so_far: &mut String,
) -> Result<String> {
    let full = tokenizer
        .decode(&all_tokens[prompt_len..], true)
        .map_err(|e| anyhow!("decode: {e}"))?;
    // Guard against the byte offset landing mid-character (can happen when
    // sentencepiece merges across previously decoded boundaries).
    let split = decoded_so_far.len();
    let delta = if split <= full.len() && full.is_char_boundary(split) {
        full[split..].to_string()
    } else {
        // Fallback: find the first char boundary at or after split.
        let safe = (split..=full.len()).find(|&i| full.is_char_boundary(i)).unwrap_or(full.len());
        full[safe..].to_string()
    };
    *decoded_so_far = full;
    Ok(delta)
}

/// Download model weights — single safetensors or sharded.
async fn get_weights(repo: &hf_hub::api::tokio::ApiRepo) -> Result<Vec<std::path::PathBuf>> {
    if let Ok(p) = repo.get("model.safetensors").await {
        return Ok(vec![p]);
    }
    let index_file = repo.get("model.safetensors.index.json").await
        .context("no model.safetensors or model.safetensors.index.json found")?;

    let index: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&index_file)?)?;
    let weight_map = index["weight_map"].as_object()
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
        paths.push(repo.get(name).await
            .with_context(|| format!("downloading shard {name}"))?);
    }
    Ok(paths)
}
