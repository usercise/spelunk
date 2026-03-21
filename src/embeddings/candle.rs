//! EmbeddingGemma via candle with Metal backend (Apple Silicon GPU).
//!
//! Phase 3 implementation. Stubs are in place; the model loading and
//! forward-pass wiring will be completed in that phase.

use anyhow::Result;
use crate::embeddings::{EmbeddingBackend, EMBEDDING_DIM};

pub struct CandleEmbedder {
    // Phase 3: candle model, tokenizer, device handle
    _private: (),
}

impl CandleEmbedder {
    /// Load EmbeddingGemma from the HuggingFace Hub (or local cache).
    pub async fn load(_model_id: &str, _cache_dir: &std::path::Path) -> Result<Self> {
        todo!("Phase 3: download weights via hf-hub, load into candle with Metal device")
    }
}

#[async_trait::async_trait]
impl EmbeddingBackend for CandleEmbedder {
    async fn embed(&self, _texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        todo!("Phase 3: tokenize → forward pass on Metal device → collect embeddings")
    }

    fn dimension(&self) -> usize {
        EMBEDDING_DIM
    }
}
