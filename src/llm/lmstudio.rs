//! LLM backend that delegates to an LM Studio server via its
//! OpenAI-compatible `/v1/chat/completions` endpoint with SSE streaming.
//!
//! Requires LM Studio running at `lmstudio_base_url` (default: `http://127.0.0.1:1234`)
//! with a chat model loaded (e.g. `google/gemma-3n-e4b`).

use anyhow::{Context, Result};
use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::llm::{LlmBackend, Message, Token};

pub struct LmStudioLlm {
    client: Client,
    base_url: String,
    model: String,
}

impl LmStudioLlm {
    pub async fn load(cfg: &crate::config::Config) -> Result<Self> {
        let model = cfg.llm_model.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "llm_model is not configured.\n\
                 Add 'llm_model = \"<model-id>\"' to ~/.config/spelunk/config.toml\n\
                 to enable commands that require a chat model."
            )
        })?;
        // No warmup needed — the model is already loaded in LM Studio.
        let client = Client::builder()
            // Allow long responses without timeout during streaming.
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .context("building HTTP client for LM Studio LLM")?;
        tracing::info!("LM Studio LLM: {} model={}", cfg.lmstudio_base_url, model);
        Ok(Self {
            client,
            base_url: cfg.lmstudio_base_url.clone(),
            model: model.to_string(),
        })
    }
}

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    stream: bool,
    max_tokens: usize,
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct StreamChunk {
    choices: Vec<StreamChoice>,
}

#[derive(Deserialize)]
struct StreamChoice {
    delta: Delta,
}

#[derive(Deserialize)]
struct Delta {
    content: Option<String>,
}

// ---------------------------------------------------------------------------
// LlmBackend impl
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl LlmBackend for LmStudioLlm {
    async fn generate(
        &self,
        messages: &[Message],
        max_tokens: usize,
        tx: mpsc::Sender<Token>,
        json_schema: Option<serde_json::Value>,
    ) -> Result<()> {
        let chat_messages: Vec<ChatMessage> = messages
            .iter()
            .map(|m| ChatMessage {
                role: &m.role,
                content: &m.content,
            })
            .collect();

        let response_format = json_schema
            .map(|schema| serde_json::json!({ "type": "json_schema", "json_schema": schema }));

        let req = ChatRequest {
            model: &self.model,
            messages: chat_messages,
            stream: true,
            max_tokens,
            temperature: 0.7,
            response_format,
        };

        let mut stream = self
            .client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .json(&req)
            .send()
            .await
            .with_context(|| {
                format!(
                    "calling LM Studio /v1/chat/completions at {}. \
                     Is LM Studio running with a chat model loaded?",
                    self.base_url
                )
            })?
            .error_for_status()
            .context("LM Studio chat completions API returned an error")?
            .bytes_stream();

        // Parse SSE stream: lines are "data: {...}" or "data: [DONE]", events
        // are separated by blank lines.
        let mut buffer = String::new();
        while let Some(chunk) = stream.next().await {
            let bytes = chunk.context("reading SSE byte chunk")?;
            buffer.push_str(&String::from_utf8_lossy(&bytes));

            // Consume complete SSE events (terminated by "\n\n").
            while let Some(pos) = buffer.find("\n\n") {
                let event = buffer[..pos].to_string();
                buffer.drain(..pos + 2);

                for line in event.lines() {
                    let data = match line.strip_prefix("data: ") {
                        Some(d) => d,
                        None => continue,
                    };
                    if data == "[DONE]" {
                        return Ok(());
                    }
                    if data.is_empty() {
                        continue;
                    }
                    match serde_json::from_str::<StreamChunk>(data) {
                        Ok(chunk) => {
                            for choice in chunk.choices {
                                if let Some(content) = choice.delta.content
                                    && !content.is_empty()
                                    && tx.send(content).await.is_err()
                                {
                                    // Receiver dropped — caller cancelled.
                                    return Ok(());
                                }
                            }
                        }
                        Err(e) => {
                            tracing::debug!("SSE parse error: {e} (data={data:?})");
                        }
                    }
                }
            }
        }

        Ok(())
    }
}
