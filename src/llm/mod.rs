use anyhow::Result;
use tokio::sync::mpsc;

#[cfg(feature = "backend-metal")]
pub mod candle;

/// A streamed token from the LLM.
pub type Token = String;

/// Trait every LLM backend must implement.
#[async_trait::async_trait]
pub trait LlmBackend: Send + Sync {
    /// Generate a completion for `prompt`, streaming tokens through the sender.
    async fn generate(
        &self,
        prompt: &str,
        max_tokens: usize,
        tx: mpsc::Sender<Token>,
    ) -> Result<()>;
}
