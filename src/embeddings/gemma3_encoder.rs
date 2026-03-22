//! Bidirectional Gemma 3 encoder for `google/embeddinggemma-300m` and similar models.
//!
//! EmbeddingGemma repurposes the Gemma 3 transformer body as an encoder by removing
//! the causal attention mask (all tokens attend to all others — bidirectional).  The
//! output is a mean-pooled, L2-normalised embedding vector.
//!
//! Weight layout is identical to `candle_transformers::models::gemma3::Model`, so the
//! same `config.json` / `model.safetensors` files work here.  Only the forward pass
//! differs: no KV cache, no causal mask, no lm_head.
//!
//! Reuses `candle_transformers::models::gemma3::Config` for deserialisation.

use std::sync::Arc;

use anyhow::{Context, Result};
use candle_core::{DType, Device, Module, Tensor, D};
use candle_nn::{linear_b as linear, Activation, Linear, VarBuilder};

pub use candle_transformers::models::gemma3::Config;

// ---------------------------------------------------------------------------
// RmsNorm
// Gemma 3 variant: normalises then multiplies by (1 + weight) rather than weight.
// Copied from candle-transformers — the upstream struct is not public.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct RmsNorm {
    weight: Tensor,
    eps: f64,
}

impl RmsNorm {
    fn new(dim: usize, eps: f64, vb: VarBuilder) -> candle_core::Result<Self> {
        let weight = vb.get(dim, "weight")?;
        Ok(Self { weight, eps })
    }
}

impl Module for RmsNorm {
    fn forward(&self, x: &Tensor) -> candle_core::Result<Tensor> {
        let x_dtype = x.dtype();
        // Promote to F32 for numerical stability when inputs are half-precision.
        let internal = match x_dtype {
            DType::F16 | DType::BF16 => DType::F32,
            d => d,
        };
        let hidden_size = x.dim(D::Minus1)?;
        let x = x.to_dtype(internal)?;
        let norm_x = (x.sqr()?.sum_keepdim(D::Minus1)? / hidden_size as f64)?;
        let x_normed = x.broadcast_div(&(norm_x + self.eps)?.sqrt()?)?;
        x_normed
            .to_dtype(x_dtype)?
            .broadcast_mul(&(&self.weight + 1.0)?)
    }
}

// ---------------------------------------------------------------------------
// Rotary position embeddings
// Different base frequencies for local (sliding-window) vs global layers.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct RotaryEmbedding {
    sin: Tensor,
    cos: Tensor,
}

impl RotaryEmbedding {
    fn new(dtype: DType, cfg: &Config, dev: &Device, local: bool) -> candle_core::Result<Self> {
        let dim = cfg.head_dim;
        let max_seq = cfg.max_position_embeddings;
        let base = if local { cfg.rope_local_base_freq } else { cfg.rope_theta };
        let inv_freq: Vec<f32> = (0..dim)
            .step_by(2)
            .map(|i| 1f32 / base.powf(i as f64 / dim as f64) as f32)
            .collect();
        let inv_len = inv_freq.len();
        let inv = Tensor::from_vec(inv_freq, (1, inv_len), dev)?.to_dtype(dtype)?;
        let t = Tensor::arange(0u32, max_seq as u32, dev)?
            .to_dtype(dtype)?
            .reshape((max_seq, 1))?;
        let freqs = t.matmul(&inv)?;
        Ok(Self { sin: freqs.sin()?, cos: freqs.cos()? })
    }

    fn apply(&self, q: &Tensor, k: &Tensor) -> candle_core::Result<(Tensor, Tensor)> {
        let (_b, _h, seq, _d) = q.dims4()?;
        let cos = self.cos.narrow(0, 0, seq)?;
        let sin = self.sin.narrow(0, 0, seq)?;
        let q_rot = candle_nn::rotary_emb::rope(&q.contiguous()?, &cos, &sin)?;
        let k_rot = candle_nn::rotary_emb::rope(&k.contiguous()?, &cos, &sin)?;
        Ok((q_rot, k_rot))
    }
}

// ---------------------------------------------------------------------------
// MLP  (SwiGLU / GeGLU — whichever activation is in the config)
// ---------------------------------------------------------------------------

struct MLP {
    gate_proj: Linear,
    up_proj: Linear,
    down_proj: Linear,
    act_fn: Activation,
}

impl MLP {
    fn new(cfg: &Config, vb: VarBuilder) -> candle_core::Result<Self> {
        let (h, i) = (cfg.hidden_size, cfg.intermediate_size);
        Ok(Self {
            gate_proj: linear(h, i, false, vb.pp("gate_proj"))?,
            up_proj: linear(h, i, false, vb.pp("up_proj"))?,
            down_proj: linear(i, h, false, vb.pp("down_proj"))?,
            act_fn: cfg.hidden_activation,
        })
    }
}

impl Module for MLP {
    fn forward(&self, xs: &Tensor) -> candle_core::Result<Tensor> {
        let gate = xs.apply(&self.gate_proj)?.apply(&self.act_fn)?;
        let up = xs.apply(&self.up_proj)?;
        (gate * up)?.apply(&self.down_proj)
    }
}

// ---------------------------------------------------------------------------
// Multi-head attention  (full context, no KV cache, no causal mask)
// ---------------------------------------------------------------------------

struct Attention {
    q_proj: Linear,
    k_proj: Linear,
    v_proj: Linear,
    o_proj: Linear,
    q_norm: RmsNorm,
    k_norm: RmsNorm,
    num_heads: usize,
    num_kv_heads: usize,
    num_kv_groups: usize,
    head_dim: usize,
    attn_logit_softcapping: Option<f64>,
    rotary: Arc<RotaryEmbedding>,
}

impl Attention {
    fn new(rotary: Arc<RotaryEmbedding>, cfg: &Config, vb: VarBuilder) -> candle_core::Result<Self> {
        let (h, nh, nkv, hd) = (
            cfg.hidden_size,
            cfg.num_attention_heads,
            cfg.num_key_value_heads,
            cfg.head_dim,
        );
        let bias = cfg.attention_bias;
        Ok(Self {
            q_proj: linear(h, nh * hd, bias, vb.pp("q_proj"))?,
            k_proj: linear(h, nkv * hd, bias, vb.pp("k_proj"))?,
            v_proj: linear(h, nkv * hd, bias, vb.pp("v_proj"))?,
            o_proj: linear(nh * hd, h, bias, vb.pp("o_proj"))?,
            q_norm: RmsNorm::new(hd, cfg.rms_norm_eps, vb.pp("q_norm"))?,
            k_norm: RmsNorm::new(hd, cfg.rms_norm_eps, vb.pp("k_norm"))?,
            num_heads: nh,
            num_kv_heads: nkv,
            num_kv_groups: nh / nkv,
            head_dim: hd,
            attn_logit_softcapping: cfg.attn_logit_softcapping,
            rotary,
        })
    }

    fn forward(&self, xs: &Tensor) -> candle_core::Result<Tensor> {
        let (b, q_len, _) = xs.dims3()?;

        let q = xs.apply(&self.q_proj)?
            .reshape((b, q_len, self.num_heads, self.head_dim))?
            .transpose(1, 2)?;
        let k = xs.apply(&self.k_proj)?
            .reshape((b, q_len, self.num_kv_heads, self.head_dim))?
            .transpose(1, 2)?;
        let v = xs.apply(&self.v_proj)?
            .reshape((b, q_len, self.num_kv_heads, self.head_dim))?
            .transpose(1, 2)?;

        let q = self.q_norm.forward(&q)?;
        let k = self.k_norm.forward(&k)?;
        let (q, k) = self.rotary.apply(&q, &k)?;

        let k = candle_transformers::utils::repeat_kv(k, self.num_kv_groups)?.contiguous()?;
        let v = candle_transformers::utils::repeat_kv(v, self.num_kv_groups)?.contiguous()?;

        let scale = 1f64 / (self.head_dim as f64).sqrt();
        let w = (q.matmul(&k.transpose(2, 3)?)? * scale)?;

        let w = match self.attn_logit_softcapping {
            None => w,
            Some(sc) => ((w / sc)?.tanh()? * sc)?,
        };

        // No attention mask — full bidirectional attention over all positions.
        let w = candle_nn::ops::softmax_last_dim(&w)?;
        w.matmul(&v)?
            .transpose(1, 2)?
            .reshape((b, q_len, ()))?
            .apply(&self.o_proj)
    }
}

// ---------------------------------------------------------------------------
// Decoder layer  (same residual structure as gemma3::DecoderLayer)
// ---------------------------------------------------------------------------

struct DecoderLayer {
    self_attn: Attention,
    mlp: MLP,
    input_layernorm: RmsNorm,
    post_attention_layernorm: RmsNorm,
    pre_feedforward_layernorm: RmsNorm,
    post_feedforward_layernorm: RmsNorm,
}

impl DecoderLayer {
    fn new(cfg: &Config, local: bool, vb: VarBuilder) -> candle_core::Result<Self> {
        let rotary = Arc::new(RotaryEmbedding::new(vb.dtype(), cfg, vb.device(), local)?);
        let h = cfg.hidden_size;
        let eps = cfg.rms_norm_eps;
        Ok(Self {
            self_attn: Attention::new(rotary, cfg, vb.pp("self_attn"))?,
            mlp: MLP::new(cfg, vb.pp("mlp"))?,
            input_layernorm: RmsNorm::new(h, eps, vb.pp("input_layernorm"))?,
            post_attention_layernorm: RmsNorm::new(h, eps, vb.pp("post_attention_layernorm"))?,
            pre_feedforward_layernorm: RmsNorm::new(h, eps, vb.pp("pre_feedforward_layernorm"))?,
            post_feedforward_layernorm: RmsNorm::new(h, eps, vb.pp("post_feedforward_layernorm"))?,
        })
    }

    fn forward(&self, xs: &Tensor) -> candle_core::Result<Tensor> {
        let residual = xs;
        let xs = xs.apply(&self.input_layernorm)?;
        let xs = self.self_attn.forward(&xs)?;
        let xs = xs.apply(&self.post_attention_layernorm)?;
        let xs = (xs + residual)?;
        let residual = &xs;
        let xs = xs.apply(&self.pre_feedforward_layernorm)?;
        let xs = xs.apply(&self.mlp)?;
        let xs = xs.apply(&self.post_feedforward_layernorm)?;
        residual + xs
    }
}

// ---------------------------------------------------------------------------
// Public encoder
// ---------------------------------------------------------------------------

pub struct Gemma3Encoder {
    embed_tokens: candle_nn::Embedding,
    layers: Vec<DecoderLayer>,
    norm: RmsNorm,
    pub hidden_size: usize,
}

impl Gemma3Encoder {
    /// Load from a VarBuilder that points at the root of the safetensors file(s).
    /// Weight paths must follow the standard Gemma 3 convention:
    ///   `model.embed_tokens`, `model.layers.N.*`, `model.norm`.
    pub fn new(cfg: &Config, vb: VarBuilder) -> Result<Self> {
        let vb_m = vb.pp("model");
        let embed_tokens =
            candle_nn::embedding(cfg.vocab_size, cfg.hidden_size, vb_m.pp("embed_tokens"))
                .context("loading embed_tokens")?;
        let vb_l = vb_m.pp("layers");
        let mut layers = Vec::with_capacity(cfg.num_hidden_layers);
        for i in 0..cfg.num_hidden_layers {
            // Mirror gemma3::Model: a layer is "local" (sliding-window) when
            // (layer_idx + 1) % sliding_window_pattern != 0.
            let local = (i + 1) % cfg.sliding_window_pattern > 0;
            layers.push(
                DecoderLayer::new(cfg, local, vb_l.pp(i))
                    .with_context(|| format!("building layer {i}"))?,
            );
        }
        let norm = RmsNorm::new(cfg.hidden_size, cfg.rms_norm_eps, vb_m.pp("norm"))
            .context("loading final norm")?;
        Ok(Self { embed_tokens, layers, norm, hidden_size: cfg.hidden_size })
    }

    /// Forward pass → L2-normalised mean-pooled embedding `[batch, hidden_size]`.
    ///
    /// `attention_mask` is a `[batch, seq]` integer tensor (1 = real token, 0 = pad).
    /// Pass `None` when all positions are real tokens.
    pub fn embed(&self, input_ids: &Tensor, attention_mask: Option<&Tensor>) -> Result<Tensor> {
        let (_b, seq) = input_ids.dims2()?;

        // Token embeddings, scaled by √hidden_size (standard Gemma pre-scaling).
        let xs = self.embed_tokens.forward(input_ids)
            .map_err(anyhow::Error::from)?;
        let mut xs = (xs * (self.hidden_size as f64).sqrt())
            .map_err(anyhow::Error::from)?;

        for layer in &self.layers {
            xs = layer.forward(&xs).map_err(anyhow::Error::from)?;
        }

        // Apply final RMSNorm to every position.
        let xs = self.norm.forward(&xs).map_err(anyhow::Error::from)?;

        // Mean pool, masking out padding positions.
        let mean = if let Some(mask) = attention_mask {
            // mask [batch, seq] → broadcast to [batch, seq, hidden]
            let mask_f = mask
                .to_dtype(xs.dtype())
                .map_err(anyhow::Error::from)?
                .unsqueeze(2)
                .map_err(anyhow::Error::from)?
                .broadcast_as(xs.shape())
                .map_err(anyhow::Error::from)?;
            let sum = (&xs * &mask_f)
                .map_err(anyhow::Error::from)?
                .sum(1)
                .map_err(anyhow::Error::from)?; // [batch, hidden]
            let count = mask
                .to_dtype(DType::F32)
                .map_err(anyhow::Error::from)?
                .sum_keepdim(1)
                .map_err(anyhow::Error::from)?
                .to_dtype(xs.dtype())
                .map_err(anyhow::Error::from)?; // [batch, 1]
            sum.broadcast_div(&count).map_err(anyhow::Error::from)?
        } else {
            // No padding: average over all seq positions directly.
            let sum = xs.sum(1).map_err(anyhow::Error::from)?;
            (sum / seq as f64).map_err(anyhow::Error::from)?
        };

        // L2 normalise: v / ‖v‖.
        let norm = mean.sqr()
            .map_err(anyhow::Error::from)?
            .sum_keepdim(1)
            .map_err(anyhow::Error::from)?
            .sqrt()
            .map_err(anyhow::Error::from)?;
        mean.broadcast_div(&norm).map_err(anyhow::Error::from)
    }
}
