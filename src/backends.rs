//! Backend selection.
//!
//! This module re-exports the active EmbeddingBackend and LlmBackend
//! implementations based on the enabled feature flags.
//!
//! Feature priority: backend-lmstudio > backend-metal.
//! The rest of the codebase imports from here rather than from the
//! concrete backend modules, so swapping backends requires no changes
//! outside this file.

// LM Studio backend (default)
#[cfg(feature = "backend-lmstudio")]
pub use crate::embeddings::lmstudio::LmStudioEmbedder as ActiveEmbedder;

#[cfg(feature = "backend-lmstudio")]
pub use crate::llm::lmstudio::LmStudioLlm as ActiveLlm;

// Metal/candle backend (only when lmstudio is not active)
#[cfg(all(feature = "backend-metal", not(feature = "backend-lmstudio")))]
pub use crate::embeddings::candle::CandleEmbedder as ActiveEmbedder;

#[cfg(all(feature = "backend-metal", not(feature = "backend-lmstudio")))]
pub use crate::llm::candle::CandleLlm as ActiveLlm;

#[cfg(not(any(feature = "backend-metal", feature = "backend-lmstudio")))]
compile_error!(
    "No inference backend feature is enabled. \
     Enable one of: backend-lmstudio (default), backend-metal"
);
