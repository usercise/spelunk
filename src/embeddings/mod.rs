use anyhow::Result;

#[cfg(feature = "backend-metal")]
pub mod candle;
#[cfg(feature = "backend-metal")]
pub(crate) mod gemma3_encoder;

/// The embedding vector dimension.
/// `google/embeddinggemma-300m` and `BAAI/bge-base-en-v1.5` both output 768 dims.
#[allow(dead_code)]
pub const EMBEDDING_DIM: usize = 768;

/// Trait every embedding backend must implement.
///
/// Implementations live in submodules gated by feature flags.
/// Nothing outside `src/embeddings/` or `src/backends.rs` should
/// import concrete backend types.
#[async_trait::async_trait]
pub trait EmbeddingBackend: Send + Sync {
    /// Embed a batch of text strings. Returns one vector per input.
    async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>>;

    /// Dimensionality of the output vectors.
    #[allow(dead_code)]
    fn dimension(&self) -> usize;
}
