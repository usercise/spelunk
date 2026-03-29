use anyhow::{Context, Result};
use std::collections::HashSet;
use std::path::PathBuf;

use super::tools::{ToolCall, parse_tool_call, tool_call_schema};
use crate::{
    embeddings::{EmbeddingBackend, vec_to_blob},
    llm::{LlmBackend, Message},
    storage::Database,
};

/// A single tool invocation recorded during exploration.
#[derive(Debug, serde::Serialize)]
pub struct ExploreStep {
    pub step: usize,
    pub tool: String,
    pub args_summary: String,
    pub result_preview: String,
}

/// The final result returned by `Explorer::explore`.
#[derive(Debug, serde::Serialize)]
pub struct ExploreResult {
    pub answer: String,
    pub sources: Vec<String>,
    pub steps: Vec<ExploreStep>,
}

/// Drives the tool-use loop for `spelunk explore`.
///
/// Stores `db_path` and re-opens the database per tool call so that no
/// `&Database` borrow crosses an `.await` point (keeping futures `Send`).
pub struct Explorer<'a> {
    db_path: PathBuf,
    embedder: &'a (dyn EmbeddingBackend + 'a),
    llm: &'a (dyn LlmBackend + 'a),
    max_steps: usize,
    verbose: bool,
}

impl<'a> Explorer<'a> {
    pub fn new(
        db_path: PathBuf,
        embedder: &'a (dyn EmbeddingBackend + 'a),
        llm: &'a (dyn LlmBackend + 'a),
        max_steps: usize,
        verbose: bool,
    ) -> Self {
        Self {
            db_path,
            embedder,
            llm,
            max_steps,
            verbose,
        }
    }

    pub async fn explore(&self, question: &str) -> Result<ExploreResult> {
        let schema = tool_call_schema();
        let mut messages = vec![
            Message::system(SYSTEM_PROMPT),
            Message::user(format!(
                "Question: {question}\n\n\
                 Begin exploring. Use tools to find relevant code, then call done with your answer."
            )),
        ];
        let mut steps: Vec<ExploreStep> = Vec::new();
        let mut sources: HashSet<String> = HashSet::new();

        for step_num in 1..=self.max_steps {
            let raw = self.call_llm(&messages, &schema).await?;
            let raw = crate::utils::strip_ansi(&raw);

            if self.verbose {
                eprintln!("\n\x1b[2m[step {step_num}] {}\x1b[0m", raw.trim());
            }

            messages.push(Message {
                role: "assistant".into(),
                content: raw.clone(),
            });

            let tool_call = match parse_tool_call(&raw) {
                Some(tc) => tc,
                None => {
                    // Unparseable output — treat as final answer.
                    return Ok(ExploreResult {
                        answer: raw.trim().to_string(),
                        sources: sorted(sources),
                        steps,
                    });
                }
            };

            if let ToolCall::Done { answer } = tool_call {
                return Ok(ExploreResult {
                    answer,
                    sources: sorted(sources),
                    steps,
                });
            }

            let tool_name = tool_call.name();
            let (args_summary, result) = self.execute(&tool_call, &mut sources).await?;
            let result_preview: String = result.chars().take(200).collect();

            if self.verbose {
                eprintln!("\x1b[2m  → {result_preview}\x1b[0m");
            }

            steps.push(ExploreStep {
                step: step_num,
                tool: tool_name.to_string(),
                args_summary,
                result_preview,
            });

            messages.push(Message::user(format!(
                "<tool_result name=\"{tool_name}\" step=\"{step_num}\">\n{result}\n</tool_result>\n\n\
                 Continue exploring or call done when you have enough information."
            )));
        }

        // Max steps reached — request a final answer.
        messages.push(Message::user(
            "You have reached the maximum number of steps. \
             Call done with your best answer based on what you found so far."
                .to_string(),
        ));
        let raw = self.call_llm(&messages, &schema).await?;
        let raw = crate::utils::strip_ansi(&raw);
        let answer = parse_tool_call(&raw)
            .and_then(|tc| {
                if let ToolCall::Done { answer } = tc {
                    Some(answer)
                } else {
                    None
                }
            })
            .unwrap_or_else(|| raw.trim().to_string());

        Ok(ExploreResult {
            answer,
            sources: sorted(sources),
            steps,
        })
    }

    async fn call_llm(&self, messages: &[Message], schema: &serde_json::Value) -> Result<String> {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<crate::llm::Token>(256);
        let generate = self.llm.generate(messages, 512, tx, Some(schema.clone()));
        let collect = async {
            let mut buf = String::new();
            while let Some(t) = rx.recv().await {
                buf.push_str(&t);
            }
            buf
        };
        let (gen_result, raw) = tokio::join!(generate, collect);
        gen_result?;
        Ok(raw)
    }

    async fn execute(
        &self,
        tool: &ToolCall,
        sources: &mut HashSet<String>,
    ) -> Result<(String, String)> {
        match tool {
            ToolCall::Search { query, limit } => {
                // Async: embed the query.
                let query_text = format!("task: code retrieval | query: {query}");
                let vecs = self.embedder.embed(&[&query_text]).await?;
                let blob = vec_to_blob(vecs.first().context("no embedding returned")?);

                // Sync: open DB, query, drop before next await.
                let db = Database::open(&self.db_path)?;
                let results = db.search_similar(&blob, *limit)?;
                drop(db);

                for r in &results {
                    sources.insert(r.file_path.clone());
                }
                let result = if results.is_empty() {
                    "No results found.".to_string()
                } else {
                    results
                        .iter()
                        .map(|r| {
                            let name = r.name.as_deref().unwrap_or("<anonymous>");
                            let preview: String = r.content.chars().take(400).collect();
                            format!(
                                "chunk_id={} {}:{}-{} [{}: {name}]\n{preview}",
                                r.chunk_id, r.file_path, r.start_line, r.end_line, r.node_type,
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n\n---\n\n")
                };
                Ok((format!("query={query:?} limit={limit}"), result))
            }

            ToolCall::Graph { symbol } => {
                let db = Database::open(&self.db_path)?;
                let edges = if symbol.contains('/')
                    || symbol.contains('\\')
                    || symbol.ends_with(".rs")
                    || symbol.ends_with(".py")
                    || symbol.ends_with(".go")
                    || symbol.ends_with(".ts")
                    || symbol.ends_with(".js")
                {
                    db.edges_for_file(symbol)?
                } else {
                    db.edges_for_symbol(symbol)?
                };
                drop(db);

                let result = if edges.is_empty() {
                    format!("No graph edges found for '{symbol}'.")
                } else {
                    edges
                        .iter()
                        .map(|e| {
                            let src = e.source_name.as_deref().unwrap_or(&e.source_file);
                            format!(
                                "{src} --[{}]--> {} ({}:{})",
                                e.kind, e.target_name, e.source_file, e.line
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                };
                Ok((format!("symbol={symbol:?}"), result))
            }

            ToolCall::ReadChunk { chunk_id } => {
                let db = Database::open(&self.db_path)?;
                let chunks = db.chunks_by_ids(&[*chunk_id])?;
                drop(db);

                let result = chunks
                    .first()
                    .map(|c| {
                        sources.insert(c.file_path.clone());
                        format!(
                            "{}:{}-{}\n```{}\n{}\n```",
                            c.file_path, c.start_line, c.end_line, c.language, c.content
                        )
                    })
                    .unwrap_or_else(|| format!("Chunk {chunk_id} not found."));
                Ok((format!("chunk_id={chunk_id}"), result))
            }

            ToolCall::ReadFile {
                path,
                start_line,
                end_line,
            } => {
                sources.insert(path.clone());
                let content = std::fs::read_to_string(path)
                    .with_context(|| format!("reading file '{path}'"))?;
                let lines: Vec<&str> = content.lines().collect();
                let from = start_line.map(|n| n.saturating_sub(1)).unwrap_or(0);
                let to = end_line
                    .map(|n| n.min(lines.len()))
                    .unwrap_or_else(|| (from + 80).min(lines.len()));
                let slice = lines.get(from..to).unwrap_or(&[]);
                let result = format!("{}:{}-{}\n{}", path, from + 1, to, slice.join("\n"));
                Ok((format!("path={path:?} lines={}-{}", from + 1, to), result))
            }

            ToolCall::Done { .. } => unreachable!("Done handled in caller"),
        }
    }
}

fn sorted(set: HashSet<String>) -> Vec<String> {
    let mut v: Vec<String> = set.into_iter().collect();
    v.sort();
    v
}

const SYSTEM_PROMPT: &str = "\
You are an expert code analyst exploring a codebase to answer a developer's question.\n\
\n\
You have access to these tools. Always respond with exactly one JSON object — no prose, no code fences.\n\
\n\
{\"tool\": \"search\", \"args\": {\"query\": \"<semantic query>\", \"limit\": 5}}\n\
  Semantically search the code index. Returns chunks with chunk_id, file path, line range, content.\n\
\n\
{\"tool\": \"graph\", \"args\": {\"symbol\": \"<function name or file path>\"}}\n\
  Get call/import graph edges for a symbol or file.\n\
\n\
{\"tool\": \"read_chunk\", \"args\": {\"chunk_id\": 42}}\n\
  Read the full content of a specific chunk by id (from search results).\n\
\n\
{\"tool\": \"read_file\", \"args\": {\"path\": \"src/foo.rs\", \"start_line\": 10, \"end_line\": 50}}\n\
  Read lines from a file. Omit start_line/end_line to read from line 1.\n\
\n\
{\"tool\": \"done\", \"args\": {\"answer\": \"<your final answer>\"}}\n\
  Call this when you have enough information to answer the question fully.\n\
\n\
Strategy: start with search, use read_chunk/graph to go deeper, call done when confident.";
