use anyhow::Result;
use tokio::sync::mpsc;

pub mod lmstudio;

/// A streamed token from the LLM.
pub type Token = String;

/// A single chat message (role + content).
pub struct Message {
    pub role: String,
    pub content: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: "system".into(), content: content.into() }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: "user".into(), content: content.into() }
    }
}

/// Trait every LLM backend must implement.
#[async_trait::async_trait]
pub trait LlmBackend: Send + Sync {
    /// Generate a completion for a list of chat messages, streaming tokens through the sender.
    ///
    /// `json_schema`: if provided, the backend should constrain output to this JSON schema
    /// (passed as LM Studio `response_format.json_schema`). Backends that don't support
    /// structured output silently ignore it.
    async fn generate(
        &self,
        messages: &[Message],
        max_tokens: usize,
        tx: mpsc::Sender<Token>,
        json_schema: Option<serde_json::Value>,
    ) -> Result<()>;
}
