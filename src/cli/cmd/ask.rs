use anyhow::{Context, Result};
use std::io::Write;

use super::super::AskArgs;
use super::search::{resolve_project_and_deps, search_all_dbs};
use super::ui::spinner;
use crate::{
    config::{Config, resolve_db},
    embeddings::{EmbeddingBackend as _, vec_to_blob},
    storage::{Database, open_memory_backend},
};

pub async fn ask(args: AskArgs, cfg: Config) -> Result<()> {
    use crate::llm::LlmBackend;

    let (db_path, dep_dbs) = resolve_project_and_deps(args.db.as_ref(), &cfg)?;

    // ── Step 1: embed the question + search ──────────────────────────────────
    let sp = spinner("Loading embedding model…");

    let embedder = crate::backends::ActiveEmbedder::load(&cfg)
        .await
        .with_context(|| format!("loading embedding model '{}'", cfg.embedding_model))?;

    sp.set_message("Searching for relevant context…");
    let query_text = format!("task: question answering | query: {}", args.question);
    let vecs = embedder.embed(&[&query_text]).await?;
    let query_blob = vec_to_blob(vecs.first().context("no embedding")?);

    let mut results = search_all_dbs(
        &db_path,
        &dep_dbs,
        &query_blob,
        args.context_chunks.min(100),
    )?;
    sp.finish_and_clear();
    drop(embedder); // free GPU memory before loading the LLM

    if results.is_empty() {
        println!("No relevant code found in the index.");
        return Ok(());
    }

    // ── Step 1b: graph neighbour enrichment (primary DB only) ────────────────
    const MAX_GRAPH_EXTRA: usize = 5;
    if let Ok(primary_db) = Database::open(&db_path) {
        let seen_ids: std::collections::HashSet<i64> = results.iter().map(|r| r.chunk_id).collect();
        let names: Vec<&str> = results.iter().filter_map(|r| r.name.as_deref()).collect();
        if !names.is_empty()
            && let Ok(neighbor_ids) = primary_db.graph_neighbor_chunks(&names)
        {
            let new_ids: Vec<i64> = neighbor_ids
                .into_iter()
                .filter(|id| !seen_ids.contains(id))
                .take(MAX_GRAPH_EXTRA)
                .collect();
            if !new_ids.is_empty()
                && let Ok(extra) = primary_db.chunks_by_ids(&new_ids)
            {
                results.extend(extra);
            }
        }
    }

    // ── Step 2: assemble code context ───────────────────────────────────────
    let code_context = results
        .iter()
        .map(|r| {
            let name = r.name.as_deref().unwrap_or("<anonymous>");
            format!(
                "### {path}  [{kind}: {name}, lines {start}–{end}]\n```{lang}\n{code}\n```",
                path = r.file_path,
                kind = r.node_type,
                name = name,
                start = r.start_line,
                end = r.end_line,
                lang = r.language,
                code = r.content,
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    // ── Step 2b: memory context (decisions / requirements / background) ──────
    let mem_path = resolve_db(None, &cfg.db_path).with_file_name("memory.db");
    let memory_context: Option<String> = if let Ok(backend) = open_memory_backend(&cfg, &mem_path) {
        match backend.search(&query_blob, 5).await {
            Ok(notes) if !notes.is_empty() => {
                let text = notes
                    .iter()
                    .map(|n| {
                        let tags = if n.tags.is_empty() {
                            String::new()
                        } else {
                            format!("  [{}]", n.tags.join(", "))
                        };
                        format!(
                            "### [{kind}] {title}{tags}\n{body}",
                            kind = n.kind,
                            title = n.title,
                            tags = tags,
                            body = n.body
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n");
                Some(text)
            }
            _ => None,
        }
    } else {
        None
    };

    // ── Step 2c: prompt injection pre-flight ─────────────────────────────────
    const INJECTION_PATTERNS: &[&str] = &[
        "ignore previous instructions",
        "ignore all previous",
        "disregard your instructions",
        "disregard the above",
        "new instructions:",
        "system prompt:",
        "you are now",
        "pretend you are",
        "act as if you",
        "jailbreak",
    ];
    let question_lower = args.question.to_lowercase();
    if INJECTION_PATTERNS
        .iter()
        .any(|p| question_lower.contains(p))
    {
        anyhow::bail!("Question contains a disallowed pattern and cannot be processed.");
    }

    // ── Step 3: build chat messages ──────────────────────────────────────────
    const SYSTEM_BASE: &str = "\
You are an expert software analyst helping a developer understand a codebase.\n\
\n\
You have two sources of context:\n\
- Code context: excerpts from the source code showing HOW the system is built.\n\
  Reference specific file paths and line numbers when they are relevant.\n\
- Memory context: recorded decisions, requirements, and background explaining\n\
  WHAT was built and WHY those choices were made.\n\
  Reference these when they explain the reasoning behind the code.\n\
\n\
Use both sources together to give accurate, grounded answers. \
If the answer cannot be determined from the provided context, say so clearly rather than guessing.";

    let use_json = args.json || crate::utils::is_agent_mode();
    let (system_prompt, json_schema) = if use_json {
        (
            concat!(
                "You are an expert software analyst helping a developer understand a codebase.\n",
                "\n",
                "You have two sources of context:\n",
                "- Code context: source code excerpts showing HOW the system is built.\n",
                "- Memory context: recorded decisions and requirements explaining WHAT and WHY.\n",
                "\n",
                "Respond ONLY with a valid JSON object matching the provided schema. No other text.",
            ),
            Some(ask_json_schema()),
        )
    } else {
        (SYSTEM_BASE, None)
    };

    let user_message = if let Some(mem) = &memory_context {
        format!(
            "<code_context>\n{code}\n</code_context>\n\n\
             <memory_context>\n{mem}\n</memory_context>\n\n\
             <question>\n{q}\n</question>",
            code = code_context,
            mem = mem,
            q = args.question,
        )
    } else {
        format!(
            "<code_context>\n{code}\n</code_context>\n\n\
             <question>\n{q}\n</question>",
            code = code_context,
            q = args.question,
        )
    };

    let messages = vec![
        crate::llm::Message::system(system_prompt),
        crate::llm::Message::user(user_message),
    ];

    // ── Step 4: load LLM + stream answer ─────────────────────────────────────
    let llm_model_name = cfg.llm_model.as_deref().unwrap_or("<not configured>");
    let sp2 = spinner(format!("Loading LLM ({llm_model_name})…"));

    let llm = crate::backends::ActiveLlm::load(&cfg)
        .await
        .with_context(|| format!("loading LLM '{llm_model_name}'"))?;

    sp2.finish_and_clear();
    println!();

    let (tx, mut rx) = tokio::sync::mpsc::channel::<crate::llm::Token>(128);
    let generate = llm.generate(&messages, 1024, tx, json_schema);

    if use_json {
        // Collect all tokens then parse + pretty-print the JSON object.
        let collect = async move {
            let mut buf = String::new();
            while let Some(t) = rx.recv().await {
                buf.push_str(&t);
            }
            buf
        };
        let (_, raw) = tokio::try_join!(generate, async { Ok::<_, anyhow::Error>(collect.await) })?;
        // Sanitize before parsing: remove any ANSI escape sequences the model may emit.
        let raw = crate::utils::strip_ansi(&raw);
        match serde_json::from_str::<serde_json::Value>(&raw) {
            Ok(v) => println!("{}", serde_json::to_string_pretty(&v)?),
            Err(_) => print!("{raw}"),
        }
    } else {
        let print_tokens = async move {
            while let Some(token) = rx.recv().await {
                print!("{}", crate::utils::strip_ansi(&token));
                std::io::stdout().flush().ok();
            }
            println!("\n");
        };
        tokio::try_join!(generate, async {
            print_tokens.await;
            Ok(())
        })?;
    }

    Ok(())
}

fn ask_json_schema() -> serde_json::Value {
    serde_json::json!({
        "name": "code_answer",
        "strict": true,
        "schema": {
            "type": "object",
            "properties": {
                "answer": {
                    "type": "string",
                    "description": "The answer to the question about the codebase"
                },
                "relevant_files": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "File paths most relevant to the answer"
                },
                "confidence": {
                    "type": "string",
                    "enum": ["high", "medium", "low"],
                    "description": "Confidence level in the answer"
                }
            },
            "required": ["answer", "relevant_files", "confidence"],
            "additionalProperties": false
        }
    })
}
