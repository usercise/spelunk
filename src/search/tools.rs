use serde::{Deserialize, Serialize};

/// A tool call the LLM can make during an explore session.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "tool", content = "args", rename_all = "snake_case")]
pub enum ToolCall {
    Search {
        query: String,
        #[serde(default = "default_limit")]
        limit: usize,
    },
    Graph {
        symbol: String,
    },
    ReadChunk {
        chunk_id: i64,
    },
    ReadFile {
        path: String,
        start_line: Option<usize>,
        end_line: Option<usize>,
    },
    Done {
        answer: String,
    },
}

fn default_limit() -> usize {
    5
}

impl ToolCall {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Search { .. } => "search",
            Self::Graph { .. } => "graph",
            Self::ReadChunk { .. } => "read_chunk",
            Self::ReadFile { .. } => "read_file",
            Self::Done { .. } => "done",
        }
    }
}

/// JSON schema for the tool call response format (passed as response_format to LLM).
pub fn tool_call_schema() -> serde_json::Value {
    serde_json::json!({
        "name": "tool_call",
        "schema": {
            "type": "object",
            "properties": {
                "tool": {
                    "type": "string",
                    "enum": ["search", "graph", "read_chunk", "read_file", "done"]
                },
                "args": {
                    "type": "object"
                }
            },
            "required": ["tool", "args"],
            "additionalProperties": false
        }
    })
}

/// Extract the first balanced JSON object from potentially-prose LLM output.
fn extract_json(raw: &str) -> Option<&str> {
    let start = raw.find('{')?;
    let mut depth = 0usize;
    let mut end = None;
    for (i, ch) in raw[start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(start + i);
                    break;
                }
            }
            _ => {}
        }
    }
    let end = end?;
    Some(&raw[start..=end])
}

/// Parse a ToolCall from raw LLM output, tolerating prose wrappers.
pub fn parse_tool_call(raw: &str) -> Option<ToolCall> {
    serde_json::from_str(extract_json(raw)?).ok()
}
