//! Gemma 3n via candle with Metal backend (Apple Silicon GPU).
//!
//! Phase 5 implementation.

use anyhow::Result;
use tokio::sync::mpsc;
use crate::llm::{LlmBackend, Token};

pub struct CandleLlm {
    _private: (),
}

impl CandleLlm {
    pub async fn load(_model_id: &str, _cache_dir: &std::path::Path) -> Result<Self> {
        todo!("Phase 5: download Gemma 3n weights via hf-hub, load into candle with Metal device")
    }
}

#[async_trait::async_trait]
impl LlmBackend for CandleLlm {
    async fn generate(
        &self,
        _prompt: &str,
        _max_tokens: usize,
        _tx: mpsc::Sender<Token>,
    ) -> Result<()> {
        todo!("Phase 5: autoregressive generation on Metal, stream tokens via channel")
    }
}
