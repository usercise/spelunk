//! Backend selection.
//!
//! This module re-exports the active EmbeddingBackend and LlmBackend
//! implementations based on the enabled feature flags.
//!
//! Exactly one backend feature must be active at compile time.
//! The rest of the codebase imports from here rather than from the
//! concrete backend modules, so swapping backends requires no changes
//! outside this file.

#[cfg(feature = "backend-metal")]
pub use crate::embeddings::candle::CandleEmbedder as ActiveEmbedder;

#[cfg(feature = "backend-metal")]
pub use crate::llm::candle::CandleLlm as ActiveLlm;

#[cfg(not(any(feature = "backend-metal")))]
compile_error!(
    "No inference backend feature is enabled. \
     Enable one of: backend-metal"
);
