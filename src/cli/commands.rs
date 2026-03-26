use std::io::IsTerminal as _;

use anyhow::{Context, Result};
use ignore::WalkBuilder;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

use super::{
    AskArgs, CheckArgs, ChunksArgs, GraphArgs, HooksArgs, HooksCommand, IndexArgs, LinkArgs,
    MemoryArgs, MemoryCommand, PlanArgs, PlanCommand, SearchArgs, StatusArgs, UnlinkArgs,
    VerifyArgs,
};
use crate::{
    config::{Config, resolve_db},
    embeddings::{EmbeddingBackend as _, vec_to_blob},
    indexer::{
        graph::EdgeExtractor,
        parser::{SourceParser, detect_language, detect_text_language, is_binary_file},
    },
    registry::Registry,
    search::SearchResult,
    storage::{Database, NoteInput, open_memory_backend},
};

fn is_tty() -> bool {
    std::io::stderr().is_terminal()
}

fn spinner(message: impl Into<std::borrow::Cow<'static, str>>) -> ProgressBar {
    if is_tty() && !crate::utils::is_agent_mode() {
        let sp = ProgressBar::new_spinner();
        sp.set_message(message);
        sp.enable_steady_tick(std::time::Duration::from_millis(80));
        sp
    } else {
        ProgressBar::hidden()
    }
}

pub async fn index(args: IndexArgs, cfg: Config) -> Result<()> {
    // Compile secret-scanning regexes once before the hot loop.
    crate::indexer::secrets::init();

    // Default DB lives inside the indexed directory, scoping the index to the project.
    let db_path = args
        .db
        .clone()
        .unwrap_or_else(|| args.path.join(".spelunk").join("index.db"));
    let db = Database::open(&db_path)?;

    // Canonicalise the root so symlinks don't create duplicate entries.
    let root_canonical = args
        .path
        .canonicalize()
        .unwrap_or_else(|_| args.path.clone());

    // ── Collect source files ─────────────────────────────────────────────────
    // WalkBuilder respects .gitignore, .ignore, and global gitignore rules.
    // The override below excludes sensitive files unconditionally — even when
    // no .gitignore is present or when they are explicitly un-ignored.
    let sensitive_patterns = [
        "!.env",
        "!.env.*",
        "!*.pem",
        "!*.key",
        "!*.p12",
        "!*.pfx",
        "!*.p8",
        "!*.cer",
        "!*.crt",
        "!*.der",
        "!id_rsa",
        "!id_ecdsa",
        "!id_ed25519",
        "!id_dsa",
        "!*.keystore",
        "!*.jks",
        "!.netrc",
        "!.npmrc",
    ];
    let mut walk = WalkBuilder::new(&args.path);
    walk.standard_filters(true);
    let mut ob = ignore::overrides::OverrideBuilder::new(&args.path);
    for pat in &sensitive_patterns {
        ob.add(pat).ok();
    }
    if let Ok(ov) = ob.build() {
        walk.overrides(ov);
    }

    let files: Vec<_> = walk
        .build()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|ft| ft.is_file()).unwrap_or(false))
        .filter(|e| {
            let p = e.path();
            detect_language(p).is_some() || detect_text_language(p).is_some()
        })
        .collect();

    if files.is_empty() {
        println!("No supported source files found in {}", args.path.display());
        return Ok(());
    }

    // ── Phase 1: parse + store chunks ────────────────────────────────────────
    let mp = MultiProgress::new();
    let parse_bar = if is_tty() && !crate::utils::is_agent_mode() {
        let bar = mp.add(ProgressBar::new(files.len() as u64));
        bar.set_style(progress_style("Parsing  "));
        bar
    } else {
        ProgressBar::hidden()
    };

    let mut chunk_ids_and_texts: Vec<(i64, String)> = Vec::new();
    let mut indexed = 0u64;
    let mut skipped = 0u64;

    for entry in &files {
        let path = entry.path();
        let path_str = path.to_string_lossy();
        parse_bar.set_message(short_path(&path_str));

        let language = detect_language(path)
            .or_else(|| detect_text_language(path))
            .unwrap(); // safe: files were filtered to only include detectable files

        // Skip binary files (e.g. compiled output with wrong extension)
        if matches!(language, "text" | "markdown") && is_binary_file(path) {
            parse_bar.inc(1);
            continue;
        }
        let source =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let hash = format!("{}", blake3::hash(source.as_bytes()));

        if !args.force
            && let Some(existing) = db.file_hash(&path_str)?
                && existing == hash {
                    skipped += 1;
                    parse_bar.inc(1);
                    continue;
                }

        let chunks = match SourceParser::parse(&source, &path_str, language) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("parse error for {path_str}: {e}");
                parse_bar.inc(1);
                continue;
            }
        };

        let file_id = db.upsert_file(&path_str, Some(language), &hash)?;
        db.delete_embeddings_for_file(file_id)?;
        db.delete_chunks_for_file(file_id)?;

        // Extract and store graph edges for this file.
        match EdgeExtractor::extract(&source, &path_str, language) {
            Ok(edges) => {
                if let Err(e) = db.replace_edges(&path_str, &edges) {
                    tracing::warn!("graph edge storage failed for {path_str}: {e}");
                }
            }
            Err(e) => tracing::warn!("graph extraction failed for {path_str}: {e}"),
        }

        for chunk in &chunks {
            // Skip chunks that appear to contain secrets.
            if crate::indexer::secrets::contains_secret(&chunk.content) {
                tracing::warn!(
                    "skipping chunk '{}' in {path_str} (possible secret detected)",
                    chunk.name.as_deref().unwrap_or("<anonymous>"),
                );
                continue;
            }

            let metadata = serde_json::json!({
                "docstring": chunk.docstring,
                "parent_scope": chunk.parent_scope,
            });
            let chunk_id = db.insert_chunk(
                file_id,
                &chunk.kind.to_string(),
                chunk.name.as_deref(),
                chunk.start_line,
                chunk.end_line,
                &chunk.content,
                Some(&metadata.to_string()),
            )?;
            chunk_ids_and_texts.push((chunk_id, chunk.embedding_text()));
        }

        indexed += 1;
        parse_bar.inc(1);
    }

    parse_bar.finish_with_message(format!(
        "{indexed} files parsed ({skipped} skipped, {indexed} new/changed)"
    ));

    // ── Stale file cleanup ────────────────────────────────────────────────────
    // Remove index records for files that no longer exist under the project root.
    let root_str = args.path.to_string_lossy().to_string();
    let visited: std::collections::HashSet<String> = files
        .iter()
        .map(|e| e.path().to_string_lossy().to_string())
        .collect();
    let all_indexed = db.file_paths_under(&root_str)?;
    let mut removed = 0u64;
    for (id, path) in all_indexed {
        if !visited.contains(&path) {
            db.delete_file(id, &path)?;
            removed += 1;
        }
    }
    if removed > 0 {
        eprintln!("Removed {removed} stale file(s) from index.");
    }

    if chunk_ids_and_texts.is_empty() {
        let stats = db.stats()?;
        println!(
            "Index: {} files, {} chunks, {} embeddings (nothing new to embed)",
            stats.file_count, stats.chunk_count, stats.embedding_count
        );
        return Ok(());
    }

    // ── Phase 2: embed chunks ────────────────────────────────────────────────
    eprintln!("Embedding via: {}", cfg.embedding_model);

    let embedder = crate::backends::ActiveEmbedder::load(&cfg)
        .await
        .with_context(|| format!("loading embedding model '{}'", cfg.embedding_model))?;

    let batch_size = args.batch_size.max(1);
    let total_chunks = chunk_ids_and_texts.len() as u64;

    let embed_bar = if is_tty() && !crate::utils::is_agent_mode() {
        let bar = mp.add(ProgressBar::new(total_chunks));
        bar.set_style(progress_style("Embedding"));
        bar
    } else {
        ProgressBar::hidden()
    };

    for batch in chunk_ids_and_texts.chunks(batch_size) {
        let texts: Vec<&str> = batch.iter().map(|(_, t)| t.as_str()).collect();
        let embeddings = embedder
            .embed(&texts)
            .await
            .context("generating embeddings")?;

        for ((chunk_id, _), embedding) in batch.iter().zip(embeddings.iter()) {
            let blob = vec_to_blob(embedding);
            db.insert_embedding(*chunk_id, &blob)?;
        }
        embed_bar.inc(batch.len() as u64);
    }

    embed_bar.finish_with_message(format!("{total_chunks} chunks embedded"));

    let stats = db.stats()?;
    println!(
        "\nIndex: {} files, {} chunks, {} embeddings",
        stats.file_count, stats.chunk_count, stats.embedding_count
    );

    // Register / update this project in the global registry.
    if let Ok(reg) = Registry::open() {
        let db_canonical = db_path.canonicalize().unwrap_or(db_path.clone());
        if let Err(e) = reg.register(&root_canonical, &db_canonical) {
            tracing::warn!("registry update failed: {e}");
        }
    }

    Ok(())
}

pub async fn search(args: SearchArgs, cfg: Config) -> Result<()> {
    let (db_path, dep_dbs) = resolve_project_and_deps(args.db.as_ref(), &cfg)?;

    let sp = spinner("Loading model…");

    let embedder = crate::backends::ActiveEmbedder::load(&cfg)
        .await
        .with_context(|| format!("loading embedding model '{}'", cfg.embedding_model))?;

    sp.set_message("Embedding query…");
    let query_text = format!("task: code retrieval | query: {}", args.query);
    let vecs = embedder.embed(&[&query_text]).await?;
    let query_blob = vec_to_blob(vecs.first().context("no embedding returned")?);

    sp.set_message("Searching…");
    let mut results = search_all_dbs(&db_path, &dep_dbs, &query_blob, args.limit.min(100))?;
    sp.finish_and_clear();

    if results.is_empty() {
        println!("No results found. Make sure the index has embeddings (`spelunk index <path>`).");
        return Ok(());
    }

    // ── Graph-aware enrichment (primary DB only) ──────────────────────────────
    if args.graph
        && let Ok(primary_db) = Database::open(&db_path) {
            let seen_ids: std::collections::HashSet<i64> =
                results.iter().map(|r| r.chunk_id).collect();
            let names: Vec<&str> = results.iter().filter_map(|r| r.name.as_deref()).collect();

            if !names.is_empty()
                && let Ok(neighbor_ids) = primary_db.graph_neighbor_chunks(&names) {
                    let new_ids: Vec<i64> = neighbor_ids
                        .into_iter()
                        .filter(|id| !seen_ids.contains(id))
                        .take(args.graph_limit)
                        .collect();

                    if !new_ids.is_empty()
                        && let Ok(mut extra) = primary_db.chunks_by_ids(&new_ids) {
                            for r in &mut extra {
                                r.from_graph = true;
                            }
                            results.extend(extra);
                        }
                }
        }

    match crate::utils::effective_format(&args.format) {
        "json" => println!("{}", serde_json::to_string_pretty(&results)?),
        _ => print_results_text(&results),
    }

    Ok(())
}

pub async fn ask(args: AskArgs, cfg: Config) -> Result<()> {
    use crate::llm::LlmBackend;
    use std::io::Write;

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
            && let Ok(neighbor_ids) = primary_db.graph_neighbor_chunks(&names) {
                let new_ids: Vec<i64> = neighbor_ids
                    .into_iter()
                    .filter(|id| !seen_ids.contains(id))
                    .take(MAX_GRAPH_EXTRA)
                    .collect();
                if !new_ids.is_empty()
                    && let Ok(extra) = primary_db.chunks_by_ids(&new_ids) {
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
    let sp2 = spinner(format!("Loading LLM ({})…", cfg.llm_model));

    let llm = crate::backends::ActiveLlm::load(&cfg)
        .await
        .with_context(|| format!("loading LLM '{}'", cfg.llm_model))?;

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
        tokio::try_join!(generate, async { print_tokens.await;
        Ok(()) })?;
    }

    Ok(())
}

pub async fn status(args: StatusArgs, cfg: Config) -> Result<()> {
    let fmt = crate::utils::effective_format(&args.format);

    // JSON mode: current project stats only
    if fmt == "json" {
        let (db_path, _) = resolve_project_and_deps(None, &cfg)?;
        let db = Database::open(&db_path)?;
        let stats = db.stats()?;
        let drift = db.drift_candidates(30, 10).unwrap_or_default();
        let mem_path = resolve_db(None, &cfg.db_path).with_file_name("memory.db");
        let memory_count = match open_memory_backend(&cfg, &mem_path).ok() {
            Some(b) => b.count().await.unwrap_or(0),
            None => 0,
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "file_count": stats.file_count,
                "chunk_count": stats.chunk_count,
                "embedding_count": stats.embedding_count,
                "last_indexed_unix": stats.last_indexed,
                "memory_entry_count": memory_count,
                "drift_candidates": drift,
            }))?
        );
        return Ok(());
    }

    // --list implies --all
    let show_all = args.all || args.list;

    if show_all {
        let reg = Registry::open().context("opening registry")?;
        let projects = reg.all_projects()?;

        if projects.is_empty() {
            println!("No projects registered. Run `spelunk index <path>` to get started.");
            return Ok(());
        }

        if args.list {
            // Brief table: one line per project
            println!(
                "{:<6}  {:<8}  {:<10}  Root",
                "Files", "Chunks", "Embeddings"
            );
            println!("{}", "─".repeat(70));
            for p in &projects {
                let stats = Database::open(&p.db_path).and_then(|db| db.stats()).ok();
                let (files, chunks, embeddings) = stats
                    .map(|s| (s.file_count, s.chunk_count, s.embedding_count))
                    .unwrap_or((0, 0, 0));
                let exists = if p.root_path.exists() {
                    ""
                } else {
                    " [missing]"
                };
                println!(
                    "{:<6}  {:<8}  {:<10}  {}{}",
                    files,
                    chunks,
                    embeddings,
                    p.root_path.display(),
                    exists
                );
            }
        } else {
            // Detailed view per project
            for p in &projects {
                println!("\x1b[1m{}\x1b[0m", p.root_path.display());
                if !p.root_path.exists() {
                    println!("  \x1b[31m[root path missing from disk]\x1b[0m");
                }
                println!("  DB: {}", p.db_path.display());
                match Database::open(&p.db_path).and_then(|db| db.stats()) {
                    Ok(s) => {
                        println!(
                            "  Files: {}  Chunks: {}  Embeddings: {}",
                            s.file_count, s.chunk_count, s.embedding_count
                        );
                        if let Some(ts) = s.last_indexed {
                            println!("  Last indexed: {}", format_age(ts));
                        }
                    }
                    Err(_) => println!("  \x1b[2m(no index yet)\x1b[0m"),
                }
                let deps = reg.get_deps(p.id)?;
                if !deps.is_empty() {
                    println!("  Depends on:");
                    for dep in &deps {
                        println!("    → {}", dep.root_path.display());
                    }
                }
                println!();
            }
        }
        return Ok(());
    }

    // Current project only
    let reg = Registry::open().ok();
    let project = reg.as_ref().and_then(|r| {
        std::env::current_dir()
            .ok()
            .and_then(|cwd| r.find_project_for_path(&cwd).ok().flatten())
    });

    let db_path = match &project {
        Some(p) => p.db_path.clone(),
        None => resolve_db(None, &cfg.db_path),
    };

    if !db_path.exists() {
        println!("No index found for the current directory (checked parents too).");
        println!("Run `spelunk index <path>` to create one.");
        return Ok(());
    }

    let db = Database::open(&db_path)?;
    let s = db.stats()?;

    if let Some(p) = &project {
        println!("Project: \x1b[1m{}\x1b[0m", p.root_path.display());
    }
    println!("Index:      {}", db_path.display());
    println!("Files:      {}", s.file_count);
    println!("Chunks:     {}", s.chunk_count);
    println!("Embeddings: {}", s.embedding_count);
    if let Some(ts) = s.last_indexed {
        println!("Last index: {}", format_age(ts));
    }

    // Show dependencies
    if let (Some(reg), Some(p)) = (&reg, &project) {
        let deps = reg.get_deps(p.id)?;
        if !deps.is_empty() {
            println!("\nDependencies:");
            for dep in &deps {
                let dep_stats = Database::open(&dep.db_path).and_then(|db| db.stats()).ok();
                let summary = dep_stats
                    .map(|s| format!("{} files, {} chunks", s.file_count, s.chunk_count))
                    .unwrap_or_else(|| "not indexed".to_string());
                println!("  → {}  ({})", dep.root_path.display(), summary);
            }
        }
    }

    // Drift signals: files that haven't changed while the project has evolved
    let drift = db.drift_candidates(30, 5).unwrap_or_default();
    if !drift.is_empty() {
        println!("\n\x1b[33mDrift signals\x1b[0m  (unchanged while project evolved):");
        println!("  {:<6}  {:<8}  File", "Days", "Callers");
        println!("  {}", "─".repeat(60));
        for d in &drift {
            let callers = if d.caller_count > 0 {
                format!("{}", d.caller_count)
            } else {
                "—".to_string()
            };
            println!("  {:<6}  {:<8}  {}", d.days_behind, callers, d.path);
        }
        println!(
            "  \x1b[2mRun `spelunk search \"<topic>\"` to check if these are still relevant.\x1b[0m"
        );
    }

    Ok(())
}

pub fn graph(args: GraphArgs, cfg: Config) -> Result<()> {
    let db_path = resolve_db(args.db.as_deref(), &cfg.db_path);
    if !db_path.exists() {
        anyhow::bail!(
            "No index found (checked current directory and parents).\n\
             Run `spelunk index <path>` inside your project first."
        );
    }

    let db = Database::open(&db_path)?;
    let symbol = &args.symbol;

    // Decide whether the query looks like a file path or a symbol name.
    let mut edges = if symbol.contains('/')
        || symbol.contains('\\')
        || symbol.ends_with(".rs")
        || symbol.ends_with(".py")
        || symbol.ends_with(".go")
        || symbol.ends_with(".java")
        || symbol.ends_with(".ts")
        || symbol.ends_with(".js")
    {
        db.edges_for_file(symbol)?
    } else {
        db.edges_for_symbol(symbol)?
    };

    // Optional kind filter
    if let Some(kind) = &args.kind {
        edges.retain(|e| e.kind == *kind);
    }

    if edges.is_empty() {
        println!("No graph edges found for '{symbol}'.");
        return Ok(());
    }

    match crate::utils::effective_format(&args.format) {
        "json" => println!("{}", serde_json::to_string_pretty(&edges)?),
        _ => print_edges(&edges, symbol),
    }

    Ok(())
}

pub fn languages() -> Result<()> {
    let langs = crate::indexer::parser::SUPPORTED_LANGUAGES;
    println!("Supported languages:");
    for lang in langs {
        println!("  {lang}");
    }
    Ok(())
}

pub fn chunks(args: ChunksArgs, cfg: Config) -> Result<()> {
    let db_path = resolve_db(args.db.as_deref(), &cfg.db_path);
    if !db_path.exists() {
        anyhow::bail!(
            "No index found (checked current directory and parents).\n\
             Run `spelunk index <path>` inside your project first."
        );
    }

    let db = Database::open(&db_path)?;
    let results = db.chunks_for_file(&args.path)?;

    if results.is_empty() {
        println!("No chunks found for '{}'.", args.path);
        return Ok(());
    }

    match crate::utils::effective_format(&args.format) {
        "json" => println!("{}", serde_json::to_string_pretty(&results)?),
        _ => print_chunks_text(&results),
    }

    Ok(())
}

pub fn link(args: LinkArgs, _cfg: Config) -> Result<()> {
    let cwd = std::env::current_dir().context("getting current directory")?;
    let reg = Registry::open().context("opening registry")?;

    // Resolve current project
    let primary = reg.find_project_for_path(&cwd)?.with_context(|| {
        "No indexed project found for the current directory.\n\
             Run `spelunk index .` first.".to_string()
    })?;

    // Resolve target
    let target_path = if args.path.is_absolute() {
        args.path.clone()
    } else {
        cwd.join(&args.path)
    };
    let target_canonical = target_path
        .canonicalize()
        .unwrap_or_else(|_| target_path.clone());

    if target_canonical == primary.root_path {
        anyhow::bail!("A project cannot depend on itself.");
    }

    let dep = reg
        .find_project_for_path(&target_canonical)?
        .with_context(|| {
            format!(
                "No index found for '{}'.\n\
             Run `spelunk index {}` first.",
                target_canonical.display(),
                target_canonical.display()
            )
        })?;

    reg.add_dep(primary.id, dep.id)?;

    println!(
        "Linked: {} → {}",
        primary.root_path.display(),
        dep.root_path.display()
    );
    println!(
        "Searches from '{}' will now include results from '{}'.",
        primary.root_path.display(),
        dep.root_path.display()
    );
    Ok(())
}

pub fn unlink(args: UnlinkArgs, _cfg: Config) -> Result<()> {
    let cwd = std::env::current_dir().context("getting current directory")?;
    let reg = Registry::open().context("opening registry")?;

    let primary = reg
        .find_project_for_path(&cwd)?
        .with_context(|| "No indexed project found for the current directory.")?;

    let target_path = if args.path.is_absolute() {
        args.path.clone()
    } else {
        cwd.join(&args.path)
    };
    let target_canonical = target_path
        .canonicalize()
        .unwrap_or_else(|_| target_path.clone());

    let dep = reg.find_by_root(&target_canonical)?.with_context(|| {
        format!(
            "No registered project found at '{}'.",
            target_canonical.display()
        )
    })?;

    reg.remove_dep(primary.id, dep.id)?;

    println!(
        "Unlinked: {} ↛ {}",
        primary.root_path.display(),
        dep.root_path.display()
    );
    Ok(())
}

pub fn autoclean(_cfg: Config) -> Result<()> {
    let reg = Registry::open().context("opening registry")?;
    let removed = reg.autoclean()?;
    if removed.is_empty() {
        println!(
            "All {} registered project(s) have valid paths — nothing to clean.",
            reg.all_projects()?.len()
        );
    } else {
        println!("Removed {} stale project(s):", removed.len());
        for path in &removed {
            println!("  - {path}");
        }
    }
    Ok(())
}

// ── check ────────────────────────────────────────────────────────────────────

pub fn check(args: CheckArgs, cfg: Config) -> Result<()> {
    let db_path = resolve_db(args.db.as_deref(), &cfg.db_path);
    if !db_path.exists() {
        anyhow::bail!(
            "No index found (checked current directory and parents).\n\
             Run `spelunk index <path>` first."
        );
    }

    let db = Database::open(&db_path)?;
    let stored = db.all_file_hashes()?;

    let mut stale: Vec<String> = Vec::new();

    // Check every indexed file against its current on-disk hash.
    for (path, stored_hash) in &stored {
        match std::fs::read(path) {
            Ok(bytes) => {
                let current = format!("{}", blake3::hash(&bytes));
                if current != *stored_hash {
                    stale.push(path.clone());
                }
            }
            Err(_) => {
                // File deleted since last index.
                stale.push(path.clone());
            }
        }
    }

    let fmt = crate::utils::effective_format(&args.format);
    let fresh = stale.is_empty();

    if fmt == "json" {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "fresh": fresh,
                "indexed_files": stored.len(),
                "stale_files": stale.len(),
                "stale": stale,
            }))?
        );
    } else if fresh {
        println!("Index is up to date. ({} files indexed)", stored.len());
    } else {
        println!("{} file(s) changed since last index:", stale.len());
        for p in &stale {
            println!("  {p}");
        }
        println!("\nRun `spelunk index .` to update.");
    }

    if !fresh {
        std::process::exit(1);
    }
    Ok(())
}

// ── hooks ────────────────────────────────────────────────────────────────────

pub fn hooks(args: HooksArgs) -> Result<()> {
    match args.command {
        HooksCommand::Install(a) => hooks_install(a),
        HooksCommand::Uninstall => hooks_uninstall(),
    }
}

const POST_COMMIT_HOOK: &str = r#"#!/bin/sh
# spelunk post-commit hook — installed by `spelunk hooks install`
# Keeps the spelunk index in sync and harvests memory from new commits.
# Silently skips if `spelunk` is not in PATH, so teammates without spelunk are unaffected.

if ! command -v spelunk >/dev/null 2>&1; then
  exit 0
fi

PROJECT_ROOT="$(git rev-parse --show-toplevel 2>/dev/null)" || exit 0

spelunk index "$PROJECT_ROOT"
spelunk memory harvest --git-range HEAD~1..HEAD
"#;

const CI_STEP: &str = r#"# Add to your .github/workflows/ file:
- name: Update spelunk index
  run: |
    if command -v spelunk >/dev/null 2>&1; then
      spelunk index .
      spelunk memory harvest --git-range HEAD~1..HEAD
    fi
"#;

fn find_git_dir() -> Result<std::path::PathBuf> {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "--absolute-git-dir"])
        .output()
        .context("running git rev-parse --absolute-git-dir (is git installed?)")?;
    if !out.status.success() {
        anyhow::bail!("Not inside a git repository.");
    }
    let path = String::from_utf8(out.stdout).context("git output not UTF-8")?;
    Ok(std::path::PathBuf::from(path.trim()))
}

fn hooks_install(args: super::HooksInstallArgs) -> Result<()> {
    if args.ci {
        print!("{CI_STEP}");
        return Ok(());
    }

    let git_dir = find_git_dir()?;
    let hooks_dir = git_dir.join("hooks");
    std::fs::create_dir_all(&hooks_dir)?;
    let hook_path = hooks_dir.join("post-commit");

    if hook_path.exists() {
        let existing = std::fs::read_to_string(&hook_path)?;
        if existing.contains("spelunk post-commit hook") {
            println!("Hook already installed at {}", hook_path.display());
            return Ok(());
        }
        anyhow::bail!(
            "A post-commit hook already exists at {}.\n\
             Inspect it and merge manually, or remove it first.",
            hook_path.display()
        );
    }

    std::fs::write(&hook_path, POST_COMMIT_HOOK)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&hook_path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&hook_path, perms)?;
    }

    println!("Installed post-commit hook at {}", hook_path.display());
    println!("After each commit, ca will:");
    println!("  - Re-index the project");
    println!("  - Harvest memory from the new commit");
    println!("Teammates without spelunk installed are unaffected.");
    Ok(())
}

fn hooks_uninstall() -> Result<()> {
    let git_dir = find_git_dir()?;
    let hook_path = git_dir.join("hooks").join("post-commit");

    if !hook_path.exists() {
        println!("No post-commit hook found.");
        return Ok(());
    }

    let content = std::fs::read_to_string(&hook_path)?;
    if !content.contains("spelunk post-commit hook") {
        anyhow::bail!(
            "The hook at {} was not installed by spelunk. Remove it manually.",
            hook_path.display()
        );
    }

    std::fs::remove_file(&hook_path)?;
    println!("Removed post-commit hook.");
    Ok(())
}

// ── memory ───────────────────────────────────────────────────────────────────

pub async fn memory(args: MemoryArgs, cfg: Config) -> Result<()> {
    cfg.validate()?;
    let mem_path = args
        .db
        .clone()
        .unwrap_or_else(|| resolve_db(None, &cfg.db_path).with_file_name("memory.db"));
    match args.command {
        MemoryCommand::Add(a) => memory_add(a, &mem_path, &cfg).await,
        MemoryCommand::Search(a) => memory_search(a, &mem_path, &cfg).await,
        MemoryCommand::List(a) => memory_list(a, &mem_path, &cfg).await,
        MemoryCommand::Show(a) => memory_show(a, &mem_path, &cfg).await,
        MemoryCommand::Harvest(a) => memory_harvest(a, &mem_path, &cfg).await,
        MemoryCommand::Archive(a) => memory_archive(a, &mem_path, &cfg).await,
        MemoryCommand::Supersede(a) => memory_supersede(a, &mem_path, &cfg).await,
        MemoryCommand::Push(a) => memory_push(a, &mem_path, &cfg).await,
    }
}

async fn memory_add(
    args: super::MemoryAddArgs,
    mem_path: &std::path::Path,
    cfg: &Config,
) -> Result<()> {
    // Resolve title and body: from URL, explicit args, or editor.
    let (title, body) = if let Some(url) = &args.from_url {
        let (fetched_title, fetched_body) = fetch_url_content(url)
            .await
            .with_context(|| format!("fetching {url}"))?;
        let title = args.title.clone().unwrap_or(fetched_title);
        let body = args.body.clone().unwrap_or(fetched_body);
        (title, body)
    } else {
        let title = args
            .title
            .clone()
            .context("--title is required when --from-url is not provided")?;
        let body = match args.body.clone() {
            Some(b) => b,
            None => {
                let t = title.clone();
                tokio::task::spawn_blocking(move || open_editor_for_body(&t))
                    .await
                    .context("editor task panicked")?
                    .context("opening editor for body")?
            }
        };
        (title, body)
    };

    let tags: Vec<String> = args
        .tags
        .as_deref()
        .map(|s| s.split(',').map(|t| t.trim().to_string()).collect())
        .unwrap_or_default();
    let files: Vec<String> = args
        .files
        .as_deref()
        .map(|s| s.split(',').map(|f| f.trim().to_string()).collect())
        .unwrap_or_default();

    // Embed first so the vector is ready when we call the backend.
    let embed_text = format!("title: {title} | text: {body}");
    let sp = spinner("Embedding…");
    let embedder = crate::backends::ActiveEmbedder::load(cfg)
        .await
        .context("loading embedding model")?;
    let vecs = embedder.embed(&[&embed_text]).await?;
    sp.finish_and_clear();
    let embedding = vecs.first().map(|v| vec_to_blob(v));

    let backend = open_memory_backend(cfg, mem_path)?;
    let note_id = backend
        .add(NoteInput {
            kind: args.kind.clone(),
            title: title.clone(),
            body: body.clone(),
            tags,
            linked_files: files,
            embedding,
        })
        .await?;

    println!(
        "Stored [{kind}] #{id}: {title}",
        kind = args.kind,
        id = note_id
    );
    Ok(())
}

/// Fetch content from a URL.
///
/// Priority:
///   1. GitHub issue URLs → `gh api` for structured title + body (Markdown)
///   2. `~/scripts/web-to-md.ts` via bun → Readability + Turndown (clean Markdown)
///   3. Fallback: raw HTTP GET + naive HTML stripping
async fn fetch_url_content(url: &str) -> Result<(String, String)> {
    // ── 1. GitHub issue ───────────────────────────────────────────────────────
    let gh_issue_re =
        regex::Regex::new(r"https?://github\.com/([^/]+)/([^/]+)/issues/(\d+)").unwrap();

    if let Some(caps) = gh_issue_re.captures(url) {
        let owner = &caps[1];
        let repo = &caps[2];
        let num = &caps[3];
        let api_path = format!("repos/{owner}/{repo}/issues/{num}");
        let out = tokio::process::Command::new("gh")
            .args(["api", &api_path])
            .output()
            .await;
        if let Ok(out) = out
            && out.status.success() {
                let json: serde_json::Value =
                    serde_json::from_slice(&out.stdout).context("parsing gh api response")?;
                let title = json["title"].as_str().unwrap_or("GitHub Issue").to_string();
                let body = json["body"].as_str().unwrap_or("").to_string();
                return Ok((title, body));
            }
        // gh missing or not authenticated — fall through
    }

    // ── 2. web-to-md.ts via bun ───────────────────────────────────────────────
    // The script outputs:  # <title>\n\n<markdown body>
    // Expand ~ manually so we don't rely on shell expansion.
    let script = dirs::home_dir()
        .map(|h| h.join("scripts/web-to-md.ts"))
        .filter(|p| p.exists());

    if let Some(script_path) = script {
        let out = tokio::process::Command::new("bun")
            .arg(&script_path)
            .arg(url)
            .output()
            .await;
        if let Ok(out) = out
            && out.status.success() {
                let md = String::from_utf8_lossy(&out.stdout);
                return parse_web_to_md_output(&md, url);
            }
        // bun missing or script errored — fall through
    }

    // ── 3. Fallback: raw HTTP + naive stripping ───────────────────────────────
    let client = reqwest::Client::builder()
        .user_agent("spelunk/0.1")
        .build()?;
    let html = client.get(url).send().await?.text().await?;

    let title_re = regex::Regex::new(r"(?i)<title[^>]*>([\s\S]*?)</title>").unwrap();
    let title = title_re
        .captures(&html)
        .and_then(|c| c.get(1))
        .map(|m| html_unescape(m.as_str().trim()))
        .unwrap_or_else(|| url.to_string());

    let no_script = regex::Regex::new(r"(?is)<(?:script|style)[^>]*>[\s\S]*?</(?:script|style)>").unwrap();
    let no_tags = regex::Regex::new(r"<[^>]+>").unwrap();
    let ws = regex::Regex::new(r"\s{3,}").unwrap();
    let stripped = no_script.replace_all(&html, " ");
    let stripped = no_tags.replace_all(&stripped, " ");
    let body = ws.replace_all(stripped.trim(), "\n\n").to_string();
    let body = if body.len() > 8192 {
        body[..8192].to_string()
    } else {
        body
    };

    Ok((title, body))
}

/// Parse the `# Title\n\n<body>` output produced by web-to-md.ts.
fn parse_web_to_md_output(md: &str, url: &str) -> Result<(String, String)> {
    let md = md.trim();
    if let Some(rest) = md.strip_prefix("# ") {
        let (title_line, body) = rest.split_once('\n').unwrap_or((rest, ""));
        Ok((title_line.trim().to_string(), body.trim_start().to_string()))
    } else {
        // Unexpected format — use the whole thing as body, URL as title
        Ok((url.to_string(), md.to_string()))
    }
}

fn html_unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

async fn memory_search(
    args: super::MemorySearchArgs,
    mem_path: &std::path::Path,
    cfg: &Config,
) -> Result<()> {
    let query_text = format!("task: question answering | query: {}", args.query);
    let sp = spinner("Embedding query…");
    let embedder = crate::backends::ActiveEmbedder::load(cfg)
        .await
        .context("loading embedding model")?;
    let vecs = embedder.embed(&[&query_text]).await?;
    sp.finish_and_clear();

    let blob = vec_to_blob(vecs.first().context("no embedding returned")?);
    let backend = open_memory_backend(cfg, mem_path)?;
    let notes = backend.search(&blob, args.limit).await?;

    if notes.is_empty() {
        println!("No memory entries found.");
        return Ok(());
    }

    match crate::utils::effective_format(&args.format) {
        "json" => println!("{}", serde_json::to_string_pretty(&notes)?),
        _ => {
            for n in &notes {
                print_note_summary(n);
            }
        }
    }
    Ok(())
}

async fn memory_list(
    args: super::MemoryListArgs,
    mem_path: &std::path::Path,
    cfg: &Config,
) -> Result<()> {
    let backend = open_memory_backend(cfg, mem_path)?;
    let notes = backend
        .list(args.kind.as_deref(), args.limit, args.archived)
        .await?;

    if notes.is_empty() {
        println!("No memory entries found.");
        return Ok(());
    }

    match crate::utils::effective_format(&args.format) {
        "json" => println!("{}", serde_json::to_string_pretty(&notes)?),
        _ => {
            for n in &notes {
                print_note_summary(n);
            }
        }
    }
    Ok(())
}

async fn memory_show(
    args: super::MemoryShowArgs,
    mem_path: &std::path::Path,
    cfg: &Config,
) -> Result<()> {
    let backend = open_memory_backend(cfg, mem_path)?;
    match backend.get(args.id).await? {
        None => anyhow::bail!("No memory entry with id {}.", args.id),
        Some(n) => match crate::utils::effective_format(&args.format) {
            "json" => println!("{}", serde_json::to_string_pretty(&n)?),
            _ => {
                println!("\x1b[1m#{} [{}] {}\x1b[0m", n.id, n.kind, n.title);
                println!("\x1b[2m{}\x1b[0m", format_age(n.created_at));
                if !n.tags.is_empty() {
                    println!("tags: {}", n.tags.join(", "));
                }
                if !n.linked_files.is_empty() {
                    println!("files: {}", n.linked_files.join(", "));
                }
                println!();
                println!("{}", n.body);
            }
        },
    }
    Ok(())
}

fn print_note_summary(n: &crate::storage::memory::Note) {
    let dist = n
        .distance
        .map(|d| format!("  dist: {d:.4}"))
        .unwrap_or_default();
    let archived_badge = if n.status == "archived" {
        " \x1b[31m[archived]\x1b[0m"
    } else {
        ""
    };
    println!(
        "\x1b[1m#{id}\x1b[0m  \x1b[33m[{kind}]\x1b[0m  {title}{archived}{dist_fmt}",
        id = n.id,
        kind = n.kind,
        title = n.title,
        archived = archived_badge,
        dist_fmt = if dist.is_empty() {
            String::new()
        } else {
            format!("\x1b[2m{dist}\x1b[0m")
        },
    );
    println!("     \x1b[2m{}\x1b[0m", format_age(n.created_at));
    if !n.tags.is_empty() {
        println!("     tags: {}", n.tags.join(", "));
    }
    if !n.linked_files.is_empty() {
        println!("     files: {}", n.linked_files.join(", "));
    }
    if let Some(sup) = n.superseded_by {
        println!("     \x1b[2msuperseded by #{sup}\x1b[0m");
    }
    // For question/answer kinds: titles-only list — use `spelunk memory show <id>` for body.
    // For other kinds: show first 2 lines of body as preview.
    if !matches!(n.kind.as_str(), "question" | "answer") {
        let preview: Vec<&str> = n.body.lines().take(2).collect();
        for line in &preview {
            println!("     \x1b[2m{line}\x1b[0m");
        }
        if n.body.lines().count() > 2 {
            println!("     \x1b[2m…\x1b[0m");
        }
    } else {
        println!(
            "     \x1b[2m(use `spelunk memory show {}` to read body)\x1b[0m",
            n.id
        );
    }
    println!();
}

/// Open $EDITOR (or $VISUAL, then vi) for the user to write a memory body.
fn open_editor_for_body(title: &str) -> Result<String> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());

    let tmp = std::env::temp_dir().join(format!("ca_memory_{}.md", std::process::id()));
    std::fs::write(
        &tmp,
        format!(
            "# {title}\n\n\
         # Write your memory entry below. Lines starting with # are ignored.\n\
         # Save and close the editor when done.\n\n"
        ),
    )?;

    let status = std::process::Command::new(&editor)
        .arg(&tmp)
        .status()
        .with_context(|| format!("could not open editor '{editor}'"))?;

    let content = std::fs::read_to_string(&tmp)?;
    std::fs::remove_file(&tmp).ok();

    if !status.success() {
        anyhow::bail!("Editor exited with a non-zero status; entry not saved.");
    }

    let body: String = content
        .lines()
        .filter(|l| !l.starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();

    if body.is_empty() {
        anyhow::bail!("Body is empty; entry not saved.");
    }
    Ok(body)
}

async fn memory_push(
    args: super::MemoryPushArgs,
    mem_path: &std::path::Path,
    cfg: &Config,
) -> Result<()> {
    if cfg.memory_server_url.is_none() {
        anyhow::bail!(
            "memory_server_url is not configured.\n\
             Set it in .spelunk/config.toml or via SPELUNK_SERVER_URL."
        );
    }

    let src_path = args.source.as_deref().unwrap_or(mem_path);
    let local = crate::storage::MemoryStore::open(src_path)
        .with_context(|| format!("opening local memory at {}", src_path.display()))?;

    let notes = local.list(None, 10_000, args.include_archived)?;
    if notes.is_empty() {
        println!("No local memory entries to push.");
        return Ok(());
    }

    let remote = open_memory_backend(cfg, mem_path)?;

    println!(
        "Pushing {} entries to {}…",
        notes.len(),
        cfg.memory_server_url.as_deref().unwrap_or("?")
    );
    let mut pushed = 0usize;
    let mut skipped = 0usize;

    // Read local embeddings from the DB for each note.
    for note in &notes {
        // Fetch the raw embedding blob from local store.
        let blob = local.get_embedding(note.id)?;
        let result = remote
            .add(NoteInput {
                kind: note.kind.clone(),
                title: note.title.clone(),
                body: note.body.clone(),
                tags: note.tags.clone(),
                linked_files: note.linked_files.clone(),
                embedding: blob,
            })
            .await;
        match result {
            Ok(_) => {
                pushed += 1;
            }
            Err(e) => {
                eprintln!("  [skip] #{}: {e}", note.id);
                skipped += 1;
            }
        }
    }
    println!("Done. Pushed: {pushed}, skipped: {skipped}.");
    Ok(())
}

async fn memory_archive(
    args: super::MemoryArchiveArgs,
    mem_path: &std::path::Path,
    cfg: &Config,
) -> Result<()> {
    let backend = open_memory_backend(cfg, mem_path)?;
    if backend.archive(args.id).await? {
        println!("Archived memory entry #{}.", args.id);
    } else {
        anyhow::bail!("No active memory entry with id {}.", args.id);
    }
    Ok(())
}

async fn memory_supersede(
    args: super::MemorySupersededArgs,
    mem_path: &std::path::Path,
    cfg: &Config,
) -> Result<()> {
    let backend = open_memory_backend(cfg, mem_path)?;
    // Verify the new entry exists.
    if backend.get(args.new_id).await?.is_none() {
        anyhow::bail!("No memory entry with id {} (new).", args.new_id);
    }
    if backend.supersede(args.old_id, args.new_id).await? {
        println!(
            "Archived #{old} → superseded by #{new}.",
            old = args.old_id,
            new = args.new_id
        );
    } else {
        anyhow::bail!("No active memory entry with id {} (old).", args.old_id);
    }
    Ok(())
}

async fn memory_harvest(
    args: super::MemoryHarvestArgs,
    mem_path: &std::path::Path,
    cfg: &Config,
) -> Result<()> {
    use crate::llm::LlmBackend;

    // ── Step 1: collect commits via git log ───────────────────────────────────
    let git_out = std::process::Command::new("git")
        .args(["log", &args.git_range, "--format=%H%x00%s%x00%b%x00---"])
        .output()
        .context("running git log (is git installed and are we in a git repo?)")?;

    if !git_out.status.success() {
        let msg = String::from_utf8_lossy(&git_out.stderr);
        anyhow::bail!("git log failed: {msg}");
    }

    let raw = String::from_utf8(git_out.stdout).context("git log output not UTF-8")?;
    let commits: Vec<(String, String, String)> = raw
        .split("---\n")
        .filter(|s| !s.trim().is_empty())
        .filter_map(|entry| {
            let parts: Vec<&str> = entry.splitn(4, '\x00').collect();
            if parts.len() < 3 {
                return None;
            }
            Some((
                parts[0].trim().to_string(),
                parts[1].trim().to_string(),
                parts[2].trim().to_string(),
            ))
        })
        .collect();

    if commits.is_empty() {
        println!("No commits found in range '{}'.", args.git_range);
        return Ok(());
    }

    // ── Step 2: skip already-harvested SHAs ──────────────────────────────────
    let backend = open_memory_backend(cfg, mem_path)?;
    let known_shas = backend.harvested_shas().await?;
    let new_commits: Vec<_> = commits
        .iter()
        .filter(|(sha, _, _)| !known_shas.contains(sha))
        .collect();

    if new_commits.is_empty() {
        println!("All {} commits already harvested.", commits.len());
        return Ok(());
    }

    println!(
        "Analysing {} new commit(s) in '{}'…",
        new_commits.len(),
        args.git_range
    );

    // ── Step 3: ask LLM to classify the commits ───────────────────────────────
    let commit_list = new_commits
        .iter()
        .map(|(sha, subject, body)| {
            if body.is_empty() {
                format!("COMMIT {sha}\n{subject}")
            } else {
                format!("COMMIT {sha}\n{subject}\n\n{body}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n---\n\n");

    let system = "You help build a project memory store from git history. \
        Respond ONLY with valid JSON matching the provided schema. No other text.";

    let user = format!(
        "Review these git commit messages. Identify commits that represent:\n\
         - \"decision\": A significant architectural or design choice and reasoning\n\
         - \"context\": Background about requirements, constraints, or project goals\n\
         - \"requirement\": A hard constraint the codebase must satisfy\n\
         - \"note\": A surprising or non-obvious discovery\n\n\
         Skip routine commits (version bumps, typo fixes, dependency updates \
         unless they reveal a constraint, formatting).\n\n\
         For each significant commit write: sha (first 8 chars), kind, title \
         (one sentence, past tense for decisions), body (include why, \
         what alternatives were considered), tags (2-4 keywords).\n\n\
         Commits:\n{commit_list}"
    );

    let schema = serde_json::json!({
        "name": "harvest_result",
        "strict": true,
        "schema": {
            "type": "object",
            "properties": {
                "entries": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "sha": {"type": "string"},
                            "kind": {"type": "string", "enum": ["decision","context","requirement","note"]},
                            "title": {"type": "string"},
                            "body": {"type": "string"},
                            "tags": {"type": "array", "items": {"type": "string"}}
                        },
                        "required": ["sha","kind","title","body","tags"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["entries"],
            "additionalProperties": false
        }
    });

    let sp = spinner("Loading LLM for harvest…");
    let llm = crate::backends::ActiveLlm::load(cfg)
        .await
        .context("loading LLM")?;
    sp.finish_and_clear();

    let messages = vec![
        crate::llm::Message::system(system),
        crate::llm::Message::user(user),
    ];

    let (tx, mut rx) = tokio::sync::mpsc::channel::<crate::llm::Token>(256);
    let generate = llm.generate(&messages, 2048, tx, Some(schema));
    let collect = async move {
        let mut buf = String::new();
        while let Some(t) = rx.recv().await {
            buf.push_str(&t);
        }
        buf
    };
    let (_, raw_json) =
        tokio::try_join!(generate, async { Ok::<_, anyhow::Error>(collect.await) })?;
    let raw_json = crate::utils::strip_ansi(&raw_json);

    // ── Step 4: parse entries, embed, store ───────────────────────────────────
    let parsed: serde_json::Value = serde_json::from_str(&raw_json)
        .with_context(|| format!("parsing LLM harvest response:\n{raw_json}"))?;

    let entries = parsed["entries"]
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or(&[]);

    if entries.is_empty() {
        println!("No significant commits found in this range.");
        return Ok(());
    }

    println!("Embedding {} entries…", entries.len());
    let embedder = crate::backends::ActiveEmbedder::load(cfg)
        .await
        .context("loading embedding model")?;

    let mut stored = 0usize;
    for entry in entries {
        let sha = entry["sha"].as_str().unwrap_or("").to_string();
        let kind = entry["kind"].as_str().unwrap_or("note");
        let title = entry["title"].as_str().unwrap_or("").to_string();
        let body = entry["body"].as_str().unwrap_or("").to_string();
        let tags_val = entry["tags"].as_array();

        let mut tags: Vec<String> = tags_val
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        // Store the full SHA as a tag for dedup on future harvests.
        // Find the full SHA from our commit list.
        let full_sha = new_commits
            .iter()
            .find(|(s, _, _)| s.starts_with(&sha))
            .map(|(s, _, _)| s.clone())
            .unwrap_or(sha.clone());
        tags.push(format!("git:{full_sha}"));

        let embed_text = format!("title: {title} | text: {body}");
        let vecs = embedder.embed(&[&embed_text]).await?;
        let embedding = vecs.first().map(|v| vec_to_blob(v));

        let note_id = backend
            .add(NoteInput {
                kind: kind.to_string(),
                title: title.clone(),
                body: body.clone(),
                tags: tags.clone(),
                linked_files: vec![],
                embedding,
            })
            .await?;

        println!("  + [{kind}] #{note_id}: {title}");
        stored += 1;
    }

    let skipped = new_commits.len().saturating_sub(stored);
    println!("\nStored {stored} memory entries. Skipped {skipped} routine commits.");
    Ok(())
}

// ── verify ───────────────────────────────────────────────────────────────────

pub async fn verify(args: VerifyArgs, cfg: Config) -> Result<()> {
    use crate::utils::effective_format;
    let fmt = effective_format(&args.format);

    let (db_path, _dep_paths) = resolve_project_and_deps(args.db.as_ref(), &cfg)?;
    let db = Database::open(&db_path)?;

    // Find chunks matching the target (file suffix or symbol name).
    let target = &args.target;
    let all_chunks = db.chunks_for_file(target)?;
    if all_chunks.is_empty() {
        anyhow::bail!(
            "No indexed chunks found for '{target}'. Try `spelunk index` first."
        );
    }

    // Build embedder and re-embed each chunk's current content.
    let sp = spinner(format!("Verifying {target}…"));
    let embedder = crate::backends::ActiveEmbedder::load(&cfg)
        .await
        .context("loading embedder")?;

    let mut results: Vec<serde_json::Value> = Vec::new();

    for chunk in &all_chunks {
        let title = chunk.name.as_deref().unwrap_or("none");
        let embed_text = format!("title: {title} | text: {}", chunk.content);
        let vecs = embedder
            .embed(&[&embed_text])
            .await
            .context("embedding chunk")?;
        let Some(vec) = vecs.first() else { continue };
        let blob = vec_to_blob(vec);

        // KNN search for this chunk's embedding.
        let neighbours_raw = db.search_similar(&blob, args.neighbours + 1)?;
        // Drop the chunk itself (distance ≈ 0).
        let neighbours: Vec<_> = neighbours_raw
            .into_iter()
            .filter(|r| r.chunk_id != chunk.chunk_id)
            .take(args.neighbours)
            .collect();

        if fmt == "json" {
            results.push(serde_json::json!({
                "chunk_id": chunk.chunk_id,
                "name": chunk.name,
                "file": chunk.file_path,
                "lines": format!("{}-{}", chunk.start_line, chunk.end_line),
                "neighbours": neighbours.iter().map(|n| serde_json::json!({
                    "chunk_id": n.chunk_id,
                    "name": n.name,
                    "file": n.file_path,
                    "distance": n.distance,
                })).collect::<Vec<_>>()
            }));
        } else {
            let name = chunk.name.as_deref().unwrap_or("<anonymous>");
            let loc = format!(
                "{}:{}-{}",
                chunk.file_path, chunk.start_line, chunk.end_line
            );
            println!("\x1b[1m{name}\x1b[0m  \x1b[2m{loc}\x1b[0m");
            for (i, n) in neighbours.iter().enumerate() {
                let nname = n.name.as_deref().unwrap_or("<anonymous>");
                println!(
                    "  {}. \x1b[33m{:.4}\x1b[0m  {} \x1b[2m({}:{}-{})\x1b[0m",
                    i + 1,
                    n.distance,
                    nname,
                    n.file_path,
                    n.start_line,
                    n.end_line,
                );
            }
            println!();
        }
    }

    sp.finish_and_clear();

    if fmt == "json" {
        println!("{}", serde_json::to_string_pretty(&results)?);
    }

    Ok(())
}

// ── plan ─────────────────────────────────────────────────────────────────────

pub async fn plan(args: PlanArgs, cfg: Config) -> Result<()> {
    match args.command {
        PlanCommand::Create(a) => plan_create(a, args.db.as_ref(), &cfg).await,
        PlanCommand::Status(a) => plan_status(a),
    }
}

async fn plan_create(
    args: super::PlanCreateArgs,
    explicit_db: Option<&std::path::PathBuf>,
    cfg: &Config,
) -> Result<()> {
    let (db_path, dep_paths) = resolve_project_and_deps(explicit_db, cfg)?;

    // Gather context: search for relevant chunks using the description as query.
    let sp = spinner("Gathering codebase context…");
    let embedder = crate::backends::ActiveEmbedder::load(cfg)
        .await
        .context("loading embedder")?;
    let query_text = format!("task: question answering | query: {}", args.description);
    let vecs = embedder.embed(&[&query_text]).await?;
    let blob = vec_to_blob(vecs.first().context("no embedding")?);

    let chunks = search_all_dbs(&db_path, &dep_paths, &blob, 15)?;
    sp.finish_and_clear();

    // Gather memory context if available.
    let mem_path = resolve_db(None, &cfg.db_path).with_file_name("memory.db");
    let memory_context = {
        let mblob = vec_to_blob(vecs.first().context("no embedding")?);
        match open_memory_backend(cfg, &mem_path).ok() {
            Some(b) => b.search(&mblob, 5).await.ok().and_then(|notes| {
                if notes.is_empty() {
                    None
                } else {
                    Some(
                        notes
                            .iter()
                            .map(|n| format!("[{}] {}: {}", n.kind, n.title, n.body))
                            .collect::<Vec<_>>()
                            .join("\n"),
                    )
                }
            }),
            None => None,
        }
    };

    // Build LLM prompt.
    let code_ctx = chunks
        .iter()
        .map(|c| {
            let name = c.name.as_deref().unwrap_or("<anonymous>");
            format!("// {name} ({})\n{}", c.file_path, c.content)
        })
        .collect::<Vec<_>>()
        .join("\n\n---\n\n");

    let memory_section = memory_context
        .map(|m| format!("\n<memory_context>\n{m}\n</memory_context>"))
        .unwrap_or_default();

    let system = "You are a senior software engineer creating an implementation plan as a markdown checklist. \
        Output ONLY the markdown document — no explanations before or after. \
        Structure: a brief summary paragraph, then a checklist of concrete implementation steps using `- [ ]` syntax. \
        Each step should be a single actionable task.";

    let user_msg = format!(
        "<code_context>\n{code_ctx}\n</code_context>{memory_section}\n\n\
        <task>\nCreate an implementation plan for: {desc}\n</task>",
        desc = args.description,
    );

    let sp2 = spinner("Generating plan…");
    let llm = crate::backends::ActiveLlm::load(cfg)
        .await
        .context("loading LLM")?;
    let messages = vec![
        crate::llm::Message::system(system),
        crate::llm::Message::user(user_msg),
    ];
    let (tx, mut rx) = tokio::sync::mpsc::channel::<crate::llm::Token>(128);
    use crate::llm::LlmBackend as _;
    let generate = llm.generate(&messages, 2048, tx, None);
    let collect = async move {
        let mut buf = String::new();
        while let Some(t) = rx.recv().await {
            buf.push_str(&t);
        }
        buf
    };
    let (_, plan_content) =
        tokio::try_join!(generate, async { Ok::<_, anyhow::Error>(collect.await) })?;
    sp2.finish_and_clear();

    // Determine output path under docs/plans/.
    let project_root = {
        let db_parent = db_path.parent().unwrap_or(std::path::Path::new("."));
        // Walk up to find the project root (where .git lives, or fall back).
        let mut p = db_parent;
        loop {
            if p.join(".git").exists() {
                break p.to_path_buf();
            }
            match p.parent() {
                Some(pp) => p = pp,
                None => break db_parent.to_path_buf(),
            }
        }
    };

    let slug = args.name.unwrap_or_else(|| {
        args.description
            .to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '-' })
            .collect::<String>()
            .split('-')
            .filter(|s| !s.is_empty())
            .take(6)
            .collect::<Vec<_>>()
            .join("-")
    });
    let plan_dir = project_root.join("docs").join("plans");
    std::fs::create_dir_all(&plan_dir)?;
    let plan_file = plan_dir.join(format!("{slug}.md"));

    // Prepend a YAML-lite header if the LLM didn't.
    let date = {
        use std::time::{SystemTime, UNIX_EPOCH};
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let days = secs / 86400;
        // Simplified ISO date from epoch (accurate for dates 1970+).
        let mut y = 1970u32;
        let mut remaining = days;
        loop {
            let leap = if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) {
                366
            } else {
                365
            };
            if remaining < leap {
                break;
            }
            remaining -= leap;
            y += 1;
        }
        let month_days = [
            31u32,
            if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) {
                29
            } else {
                28
            },
            31,
            30,
            31,
            30,
            31,
            31,
            30,
            31,
            30,
            31,
        ];
        let mut m = 1u32;
        for md in &month_days {
            if remaining < *md as u64 {
                break;
            }
            remaining -= *md as u64;
            m += 1;
        }
        let d = remaining + 1;
        format!("{y:04}-{m:02}-{d:02}")
    };
    let header = format!(
        "# Plan: {desc}\n\n> Created: {date}\n\n",
        desc = args.description,
    );
    let full_content = if plan_content.trim_start().starts_with('#') {
        plan_content
    } else {
        format!("{header}{plan_content}")
    };

    std::fs::write(&plan_file, &full_content)?;
    println!("Plan written to {}", plan_file.display());
    println!("\nPreview:");
    for line in full_content.lines().take(20) {
        println!("  {line}");
    }
    if full_content.lines().count() > 20 {
        println!("  \x1b[2m…\x1b[0m");
    }

    Ok(())
}

fn plan_status(args: super::PlanStatusArgs) -> Result<()> {
    use crate::utils::effective_format;
    let fmt = effective_format(&args.format);

    // Find docs/plans/ relative to cwd or git root.
    let plan_dir = {
        let cwd = std::env::current_dir()?;
        let candidate = cwd.join("docs").join("plans");
        if candidate.exists() {
            candidate
        } else {
            // Walk up for git root.
            let mut p = cwd.as_path();
            loop {
                if p.join(".git").exists() {
                    break p.join("docs").join("plans");
                }
                match p.parent() {
                    Some(pp) => p = pp,
                    None => break candidate,
                }
            }
        }
    };

    if !plan_dir.exists() {
        println!(
            "No plans directory found (expected {}).",
            plan_dir.display()
        );
        println!("Create a plan with: ca plan create \"<description>\"");
        return Ok(());
    }

    let mut plans: Vec<serde_json::Value> = Vec::new();
    let entries = std::fs::read_dir(&plan_dir)?;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        if let Some(name_filter) = &args.name {
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            if stem != name_filter {
                continue;
            }
        }
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();

        // Count checklist items.
        let total = content
            .lines()
            .filter(|l| l.trim_start().starts_with("- ["))
            .count();
        let done = content
            .lines()
            .filter(|l| l.trim_start().starts_with("- [x]") || l.trim_start().starts_with("- [X]"))
            .count();

        // Extract title from first `# ` line.
        let title = content
            .lines()
            .find(|l| l.starts_with("# "))
            .map(|l| l.trim_start_matches("# ").trim().to_string())
            .unwrap_or_else(|| stem.clone());

        if fmt == "json" {
            plans.push(serde_json::json!({
                "name": stem,
                "title": title,
                "done": done,
                "total": total,
                "file": path.display().to_string(),
            }));
        } else {
            let pct = if total > 0 { done * 100 / total } else { 0 };
            let bar = {
                let filled = (pct / 10) as usize;
                format!("[{}{}]", "#".repeat(filled), ".".repeat(10 - filled))
            };
            println!("\x1b[1m{title}\x1b[0m  \x1b[2m{stem}\x1b[0m");
            println!("  {bar} {done}/{total} ({pct}%)  {}", path.display());
            println!();
        }
    }

    if fmt == "json" {
        println!("{}", serde_json::to_string_pretty(&plans)?);
    } else if plans.is_empty() && args.name.is_none() {
        println!("No plans found in {}.", plan_dir.display());
    }

    Ok(())
}

// ── helpers ──────────────────────────────────────────────────────────────────

/// Resolve the primary DB path and any dep DB paths via the registry.
/// Falls back to `resolve_db` if the registry can't find a project.
fn resolve_project_and_deps(
    explicit_db: Option<&std::path::PathBuf>,
    cfg: &Config,
) -> Result<(std::path::PathBuf, Vec<std::path::PathBuf>)> {
    // Explicit --db skips registry entirely.
    if let Some(p) = explicit_db {
        if !p.exists() {
            anyhow::bail!(
                "Database not found at '{}'. Run `spelunk index <path>` first.",
                p.display()
            );
        }
        return Ok((p.clone(), vec![]));
    }

    // Try registry first.
    if let Ok(reg) = Registry::open()
        && let Ok(cwd) = std::env::current_dir()
            && let Ok(Some(project)) = reg.find_project_for_path(&cwd)
                && project.db_path.exists() {
                    let deps = reg
                        .get_deps(project.id)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|d| d.db_path)
                        .filter(|p| p.exists())
                        .collect();
                    return Ok((project.db_path, deps));
                }

    // Fallback: filesystem walk-up.
    let db_path = resolve_db(None, &cfg.db_path);
    if !db_path.exists() {
        anyhow::bail!(
            "No index found (checked current directory and parents).\n\
             Run `spelunk index <path>` inside your project first."
        );
    }
    Ok((db_path, vec![]))
}

/// Search a primary DB and any dep DBs, merge results by distance, return top `limit`.
fn search_all_dbs(
    primary_db_path: &std::path::Path,
    dep_db_paths: &[std::path::PathBuf],
    query_blob: &[u8],
    limit: usize,
) -> Result<Vec<SearchResult>> {
    let primary_db = Database::open(primary_db_path)?;
    // Over-fetch from each DB so we have enough candidates after merging.
    let fetch = (limit * 2).max(limit + 10);
    let mut all = primary_db.search_similar(query_blob, fetch)?;

    for dep_path in dep_db_paths {
        match Database::open(dep_path) {
            Ok(dep_db) => match dep_db.search_similar(query_blob, fetch) {
                Ok(mut dep_results) => all.append(&mut dep_results),
                Err(e) => tracing::warn!("search failed on dep {}: {e}", dep_path.display()),
            },
            Err(e) => tracing::warn!("could not open dep DB {}: {e}", dep_path.display()),
        }
    }

    // Sort by distance (ascending), deduplicate by (file_path, start_line, end_line).
    all.sort_by(|a, b| {
        a.distance
            .partial_cmp(&b.distance)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut seen = std::collections::HashSet::new();
    all.retain(|r| seen.insert((r.file_path.clone(), r.start_line, r.end_line)));
    all.truncate(limit);
    Ok(all)
}

fn format_age(unix_ts: i64) -> String {
    use std::time::{Duration, UNIX_EPOCH};
    if let Ok(t) = UNIX_EPOCH
        .checked_add(Duration::from_secs(unix_ts as u64))
        .ok_or(())
        && let Ok(elapsed) = std::time::SystemTime::now().duration_since(t) {
            let secs = elapsed.as_secs();
            return if secs < 60 {
                format!("{secs}s ago")
            } else if secs < 3600 {
                format!("{}m ago", secs / 60)
            } else if secs < 86400 {
                format!("{}h ago", secs / 3600)
            } else {
                format!("{}d ago", secs / 86400)
            };
        }
    "unknown".to_string()
}

fn progress_style(prefix: &str) -> ProgressStyle {
    ProgressStyle::with_template(&format!(
        "{{spinner:.cyan}} {prefix} [{{bar:38.cyan/blue}}] {{pos}}/{{len}} {{wide_msg}}"
    ))
    .unwrap()
    .progress_chars("=>-")
}

fn short_path(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

fn print_results_text(results: &[crate::search::SearchResult]) {
    let knn_count = results.iter().filter(|r| !r.from_graph).count();
    let has_graph = results.iter().any(|r| r.from_graph);
    let mut printed_graph_header = false;
    let mut display_idx = 0usize;

    for r in results {
        if r.from_graph && !printed_graph_header {
            println!("\x1b[2m── Graph neighbours ─────────────────────────────────────────\x1b[0m");
            println!();
            display_idx = 0;
            printed_graph_header = true;
            let _ = (knn_count, has_graph); // suppress unused warnings
        }

        display_idx += 1;
        let name = r.name.as_deref().unwrap_or("<anonymous>");
        let suffix = if r.from_graph {
            "\x1b[2m [via graph]\x1b[0m".to_string()
        } else {
            format!("  dist: {:.4}", r.distance)
        };

        println!(
            "{:2}. \x1b[1m{}\x1b[0m  \x1b[2m{}:{}-{}\x1b[0m  \x1b[33m[{}: {}]\x1b[0m{}",
            display_idx,
            r.file_path,
            r.language,
            r.start_line,
            r.end_line,
            r.node_type,
            name,
            suffix,
        );

        let lines: Vec<&str> = r.content.lines().collect();
        let preview_lines = lines.len().min(6);
        for line in &lines[..preview_lines] {
            println!("    {line}");
        }
        if lines.len() > preview_lines {
            println!(
                "    \x1b[2m… ({} more lines)\x1b[0m",
                lines.len() - preview_lines
            );
        }
        println!();
    }
}

fn print_chunks_text(chunks: &[crate::search::SearchResult]) {
    for (i, c) in chunks.iter().enumerate() {
        let name = c.name.as_deref().unwrap_or("<anonymous>");
        println!(
            "{:2}. \x1b[2m{}:{}-{}\x1b[0m  \x1b[33m[{}: {}]\x1b[0m",
            i + 1,
            c.language,
            c.start_line,
            c.end_line,
            c.node_type,
            name,
        );
        let lines: Vec<&str> = c.content.lines().collect();
        let preview = lines.len().min(6);
        for line in &lines[..preview] {
            println!("    {line}");
        }
        if lines.len() > preview {
            println!("    \x1b[2m… ({} more lines)\x1b[0m", lines.len() - preview);
        }
        println!();
    }
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

fn print_edges(edges: &[crate::storage::db::GraphEdge], query: &str) {
    // Group into outgoing (source) and incoming (target) edges.
    let outgoing: Vec<_> = edges
        .iter()
        .filter(|e| e.source_name.as_deref() == Some(query) || e.source_file == query)
        .collect();
    let incoming: Vec<_> = edges.iter().filter(|e| e.target_name == query).collect();
    let other: Vec<_> = edges
        .iter()
        .filter(|e| {
            e.source_name.as_deref() != Some(query)
                && e.source_file != query
                && e.target_name != query
        })
        .collect();

    if !outgoing.is_empty() {
        println!("\x1b[1mOutgoing from '{query}':\x1b[0m");
        for e in &outgoing {
            let loc = e.source_name.as_deref().unwrap_or(&e.source_file);
            println!(
                "  \x1b[33m{}\x1b[0m  {}  \x1b[2m({}:{})\x1b[0m",
                e.kind, e.target_name, loc, e.line
            );
        }
        println!();
    }
    if !incoming.is_empty() {
        println!("\x1b[1mIncoming to '{query}':\x1b[0m");
        for e in &incoming {
            let loc = e.source_name.as_deref().unwrap_or(&e.source_file);
            println!(
                "  \x1b[36m{}\x1b[0m  {}  \x1b[2m({}:{})\x1b[0m",
                e.kind, e.source_file, loc, e.line
            );
        }
        println!();
    }
    if !other.is_empty() {
        println!("\x1b[1mRelated edges:\x1b[0m");
        for e in &other {
            let loc = e.source_name.as_deref().unwrap_or(&e.source_file);
            println!(
                "  {} -- \x1b[33m{}\x1b[0m --> {}  \x1b[2m({}:{})\x1b[0m",
                loc, e.kind, e.target_name, e.source_file, e.line
            );
        }
    }
}
