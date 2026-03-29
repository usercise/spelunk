//! Batch LLM summarisation for indexed chunks.
use anyhow::Result;
use tokio::sync::mpsc;

use crate::llm::{LlmBackend, Message};

/// Summarise up to `chunks.len()` chunks in a single LLM call.
///
/// Each entry is `(chunk_id, name, kind, content)`.
/// Returns a `Vec` of `(chunk_id, summary)` in the same order as the input.
/// The prompt packs all chunks separated by `===CHUNK {id}===` delimiters
/// and asks for one-sentence summaries returned as a JSON array:
/// `[{"id": N, "summary": "..."}]`.
///
/// Partial results are handled gracefully — unparseable entries are skipped.
pub async fn summarise_batch(
    llm: &dyn LlmBackend,
    chunks: &[(i64, String, String, String)],
) -> Result<Vec<(i64, String)>> {
    if chunks.is_empty() {
        return Ok(vec![]);
    }

    // Build the prompt body: one block per chunk.
    let mut body = String::new();
    for (id, name, kind, content) in chunks {
        body.push_str(&format!("===CHUNK {id}===\n"));
        body.push_str(&format!("name: {name}\nkind: {kind}\n"));
        // Truncate very long chunks to avoid blowing the context window.
        // Use floor_char_boundary so we never split a multi-byte codepoint.
        let trimmed = if content.len() > 1500 {
            let boundary = content.floor_char_boundary(1500);
            &content[..boundary]
        } else {
            content.as_str()
        };
        body.push_str(trimmed);
        body.push_str("\n\n");
    }

    let user_msg = format!(
        "Summarise each code chunk in one sentence. Focus on what it does, not how.\n\
         Reply ONLY with a JSON array: [{{\"id\": <id>, \"summary\": \"...\"}}]\n\n\
         {body}"
    );

    let messages = [
        Message::system(
            "You are a code documentation assistant. \
             You output concise, accurate one-sentence summaries of code chunks.",
        ),
        Message::user(user_msg),
    ];

    // JSON schema for structured output.
    let schema = serde_json::json!({
        "name": "chunk_summaries",
        "schema": {
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "id":      { "type": "integer" },
                    "summary": { "type": "string" }
                },
                "required": ["id", "summary"],
                "additionalProperties": false
            }
        }
    });

    let (tx, mut rx) = mpsc::channel::<String>(256);
    let generate_fut = llm.generate(&messages, 2048, tx, Some(schema));

    let mut raw = String::new();
    let (gen_result, _) = tokio::join!(generate_fut, async {
        while let Some(token) = rx.recv().await {
            raw.push_str(&token);
        }
    });

    if let Err(e) = gen_result {
        tracing::warn!("LLM summarisation failed: {e}");
        return Ok(vec![]);
    }

    // Strip ANSI codes that some backends emit.
    let cleaned = crate::utils::strip_ansi(&raw);

    // Try to parse the JSON array; gracefully handle partial / wrapped responses.
    let trimmed = cleaned.trim();

    // Find the JSON array boundaries in case the LLM wrapped the output in prose.
    let json_str = if let (Some(start), Some(end)) = (trimmed.find('['), trimmed.rfind(']')) {
        &trimmed[start..=end]
    } else {
        trimmed
    };

    let parsed: Vec<serde_json::Value> = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("failed to parse summariser JSON response: {e}");
            return Ok(vec![]);
        }
    };

    let results = parsed
        .into_iter()
        .filter_map(|entry| {
            let id = entry.get("id")?.as_i64()?;
            let summary = entry.get("summary")?.as_str()?.to_owned();
            if summary.trim().is_empty() {
                return None;
            }
            Some((id, summary))
        })
        .collect();

    Ok(results)
}
