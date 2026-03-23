use std::io::IsTerminal as _;

use anyhow::{Context, Result};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use walkdir::WalkDir;

use crate::{
    config::{resolve_db, Config},
    embeddings::{candle::vec_to_blob, EmbeddingBackend as _},
    indexer::{graph::EdgeExtractor, parser::{detect_language, SourceParser}},
    storage::Database,
};
use super::{AskArgs, ChunksArgs, GraphArgs, IndexArgs, SearchArgs};

fn is_tty() -> bool {
    std::io::stderr().is_terminal()
}

fn spinner(message: impl Into<std::borrow::Cow<'static, str>>) -> ProgressBar {
    if is_tty() {
        let sp = ProgressBar::new_spinner();
        sp.set_message(message);
        sp.enable_steady_tick(std::time::Duration::from_millis(80));
        sp
    } else {
        ProgressBar::hidden()
    }
}

pub async fn index(args: IndexArgs, cfg: Config) -> Result<()> {
    // Default DB lives inside the indexed directory, scoping the index to the project.
    let db_path = args.db.clone().unwrap_or_else(|| {
        args.path.join(".codeanalysis").join("index.db")
    });
    let db = Database::open(&db_path)?;

    // ── Collect source files ─────────────────────────────────────────────────
    let files: Vec<_> = WalkDir::new(&args.path)
        .into_iter()
        .filter_entry(|e| !is_ignored_dir(e))
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| detect_language(e.path()).is_some())
        .collect();

    if files.is_empty() {
        println!("No supported source files found in {}", args.path.display());
        return Ok(());
    }

    // ── Phase 1: parse + store chunks ────────────────────────────────────────
    let mp = MultiProgress::new();
    let parse_bar = if is_tty() {
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

        let language = detect_language(path).unwrap();
        let source = std::fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        let hash = format!("{}", blake3::hash(source.as_bytes()));

        if !args.force {
            if let Some(existing) = db.file_hash(&path_str)? {
                if existing == hash {
                    skipped += 1;
                    parse_bar.inc(1);
                    continue;
                }
            }
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
    eprintln!("Loading embedding model: {}", cfg.embedding_model);

    let models_dir = cfg.models_dir.clone();
    let model_id = cfg.embedding_model.clone();
    let embedder = crate::backends::ActiveEmbedder::load(&model_id, &models_dir)
        .await
        .with_context(|| format!("loading embedding model '{model_id}'"))?;

    let batch_size = args.batch_size.max(1);
    let total_chunks = chunk_ids_and_texts.len() as u64;

    let embed_bar = if is_tty() {
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

    Ok(())
}

pub async fn search(args: SearchArgs, cfg: Config) -> Result<()> {
    let db_path = resolve_db(args.db.as_ref(), &cfg.db_path);

    if !db_path.exists() {
        anyhow::bail!(
            "No index found (checked current directory and parents).\n\
             Run `ca index <path>` inside your project first."
        );
    }

    let sp = spinner("Loading model…");

    let embedder =
        crate::backends::ActiveEmbedder::load(&cfg.embedding_model, &cfg.models_dir)
            .await
            .with_context(|| format!("loading embedding model '{}'", cfg.embedding_model))?;

    sp.set_message("Embedding query…");
    // Asymmetric search prefix: document side uses "Represent this code: …",
    // query side uses a search-oriented prefix for better retrieval quality.
    let query_text = format!("Represent this query for searching code: {}", args.query);
    let vecs = embedder.embed(&[&query_text]).await?;
    let query_blob = vec_to_blob(vecs.first().context("no embedding returned")?);

    sp.set_message("Searching…");
    let db = Database::open(&db_path)?;
    let mut results = db.search_similar(&query_blob, args.limit)?;
    sp.finish_and_clear();

    if results.is_empty() {
        println!("No results found. Make sure the index has embeddings (`ca index <path>`).");
        return Ok(());
    }

    // ── Graph-aware enrichment ────────────────────────────────────────────────
    if args.graph {
        let seen_ids: std::collections::HashSet<i64> =
            results.iter().map(|r| r.chunk_id).collect();
        let names: Vec<&str> = results.iter().filter_map(|r| r.name.as_deref()).collect();

        if !names.is_empty() {
            if let Ok(neighbor_ids) = db.graph_neighbor_chunks(&names) {
                let new_ids: Vec<i64> = neighbor_ids
                    .into_iter()
                    .filter(|id| !seen_ids.contains(id))
                    .take(args.graph_limit)
                    .collect();

                if !new_ids.is_empty() {
                    if let Ok(mut extra) = db.chunks_by_ids(&new_ids) {
                        for r in &mut extra {
                            r.from_graph = true;
                        }
                        results.extend(extra);
                    }
                }
            }
        }
    }

    match args.format.as_str() {
        "json" => println!("{}", serde_json::to_string_pretty(&results)?),
        _ => print_results_text(&results),
    }

    Ok(())
}

pub async fn ask(args: AskArgs, cfg: Config) -> Result<()> {
    use crate::llm::LlmBackend;
    use std::io::Write;

    let db_path = resolve_db(args.db.as_ref(), &cfg.db_path);
    if !db_path.exists() {
        anyhow::bail!(
            "No index found (checked current directory and parents).\n\
             Run `ca index <path>` inside your project first."
        );
    }

    // ── Step 1: embed the question + search ──────────────────────────────────
    let sp = spinner("Loading embedding model…");

    let embedder =
        crate::backends::ActiveEmbedder::load(&cfg.embedding_model, &cfg.models_dir)
            .await
            .with_context(|| format!("loading embedding model '{}'", cfg.embedding_model))?;

    sp.set_message("Searching for relevant context…");
    let query_text = format!("Represent this query for searching code: {}", args.question);
    let vecs = embedder.embed(&[&query_text]).await?;
    let query_blob = vec_to_blob(vecs.first().context("no embedding")?);

    let db = Database::open(&db_path)?;
    let mut results = db.search_similar(&query_blob, args.context_chunks)?;
    sp.finish_and_clear();
    drop(embedder); // free GPU memory before loading the LLM

    if results.is_empty() {
        println!("No relevant code found in the index.");
        return Ok(());
    }

    // ── Step 1b: graph neighbour enrichment ───────────────────────────────────
    // Fetch 1-hop call neighbours for each named result and append new chunks.
    // Capped at 5 extra chunks to keep the LLM prompt within Metal's attention
    // buffer limits (Gemma 3 1B has 4 heads; global attention needs ~32×L² bytes,
    // which exceeds Metal's ~4 GB single-buffer limit around L = 11 500 tokens).
    const MAX_GRAPH_EXTRA: usize = 5;
    let seen_ids: std::collections::HashSet<i64> = results.iter().map(|r| r.chunk_id).collect();
    let names: Vec<&str> = results.iter().filter_map(|r| r.name.as_deref()).collect();
    if !names.is_empty() {
        if let Ok(neighbor_ids) = db.graph_neighbor_chunks(&names) {
            let new_ids: Vec<i64> = neighbor_ids
                .into_iter()
                .filter(|id| !seen_ids.contains(id))
                .take(MAX_GRAPH_EXTRA)
                .collect();
            if !new_ids.is_empty() {
                if let Ok(extra) = db.chunks_by_ids(&new_ids) {
                    results.extend(extra);
                }
            }
        }
    }

    // ── Step 2: assemble context ─────────────────────────────────────────────
    let context = results
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

    // ── Step 3: build prompt ─────────────────────────────────────────────────
    let prompt = format!(
        "<bos><start_of_turn>user\n\
         You are an expert code analyst. \
         Answer the user's question concisely using the code context below.\n\n\
         {context}\n\n\
         Question: {question}<end_of_turn>\n\
         <start_of_turn>model\n",
        context = context,
        question = args.question,
    );

    // ── Step 4: load LLM + stream answer ─────────────────────────────────────
    let sp2 = spinner(format!("Loading LLM ({})…", cfg.llm_model));

    let llm = crate::backends::ActiveLlm::load(&cfg.llm_model, &cfg.models_dir)
        .await
        .with_context(|| format!("loading LLM '{}'. \
            If gated, run: huggingface-cli login", cfg.llm_model))?;

    sp2.finish_and_clear();
    println!();

    let (tx, mut rx) = tokio::sync::mpsc::channel::<crate::llm::Token>(128);

    // Stream tokens in a background task
    let generate = llm.generate(&prompt, 512, tx);

    let print_tokens = async move {
        while let Some(token) = rx.recv().await {
            print!("{token}");
            std::io::stdout().flush().ok();
        }
        println!("\n");
    };

    tokio::try_join!(generate, async { Ok(print_tokens.await) })?;

    Ok(())
}

pub async fn status(cfg: Config) -> Result<()> {
    let db_path = resolve_db(None, &cfg.db_path);
    if !db_path.exists() {
        println!("No index found (checked current directory and parents).");
        println!("Run `ca index <path>` inside your project first.");
        return Ok(());
    }
    let db = Database::open(&db_path)?;
    let s = db.stats()?;
    println!("Index:      {}", db_path.display());
    println!("Files:      {}", s.file_count);
    println!("Chunks:     {}", s.chunk_count);
    println!("Embeddings: {}", s.embedding_count);
    if let Some(ts) = s.last_indexed {
        use std::time::{Duration, UNIX_EPOCH};
        if let Ok(t) = UNIX_EPOCH.checked_add(Duration::from_secs(ts as u64)).ok_or(()) {
            if let Ok(elapsed) = std::time::SystemTime::now().duration_since(t) {
                let secs = elapsed.as_secs();
                let age = if secs < 60 { format!("{secs}s ago") }
                    else if secs < 3600 { format!("{}m ago", secs / 60) }
                    else if secs < 86400 { format!("{}h ago", secs / 3600) }
                    else { format!("{}d ago", secs / 86400) };
                println!("Last index: {age}");
            }
        }
    }
    Ok(())
}

pub fn graph(args: GraphArgs, cfg: Config) -> Result<()> {
    let db_path = resolve_db(args.db.as_ref(), &cfg.db_path);
    if !db_path.exists() {
        anyhow::bail!(
            "No index found (checked current directory and parents).\n\
             Run `ca index <path>` inside your project first."
        );
    }

    let db = Database::open(&db_path)?;
    let symbol = &args.symbol;

    // Decide whether the query looks like a file path or a symbol name.
    let mut edges = if symbol.contains('/') || symbol.contains('\\') || symbol.ends_with(".rs")
        || symbol.ends_with(".py") || symbol.ends_with(".go") || symbol.ends_with(".java")
        || symbol.ends_with(".ts") || symbol.ends_with(".js")
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

    match args.format.as_str() {
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
    let db_path = resolve_db(args.db.as_ref(), &cfg.db_path);
    if !db_path.exists() {
        anyhow::bail!(
            "No index found (checked current directory and parents).\n\
             Run `ca index <path>` inside your project first."
        );
    }

    let db = Database::open(&db_path)?;
    let results = db.chunks_for_file(&args.path)?;

    if results.is_empty() {
        println!("No chunks found for '{}'.", args.path);
        return Ok(());
    }

    match args.format.as_str() {
        "json" => println!("{}", serde_json::to_string_pretty(&results)?),
        _ => print_chunks_text(&results),
    }

    Ok(())
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn progress_style(prefix: &str) -> ProgressStyle {
    ProgressStyle::with_template(&format!(
        "{{spinner:.cyan}} {prefix} [{{bar:38.cyan/blue}}] {{pos}}/{{len}} {{wide_msg}}"
    ))
    .unwrap()
    .progress_chars("=>-")
}

/// Returns true for directory entries that should be skipped entirely.
fn is_ignored_dir(entry: &walkdir::DirEntry) -> bool {
    if !entry.file_type().is_dir() {
        return false;
    }
    matches!(
        entry.file_name().to_string_lossy().as_ref(),
        "target" | "node_modules" | ".git" | "__pycache__" | "venv" | ".venv"
            | "dist" | "build" | ".gradle" | ".idea" | ".next" | "vendor"
            | ".tox" | "out" | ".svn" | ".hg" | "coverage" | ".cache"
    )
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
            println!("    \x1b[2m… ({} more lines)\x1b[0m", lines.len() - preview_lines);
        }
        println!();
    }
}

fn print_chunks_text(chunks: &[crate::search::SearchResult]) {
    for (i, c) in chunks.iter().enumerate() {
        let name = c.name.as_deref().unwrap_or("<anonymous>");
        println!(
            "{:2}. \x1b[2m{}:{}-{}\x1b[0m  \x1b[33m[{}: {}]\x1b[0m",
            i + 1, c.language, c.start_line, c.end_line, c.node_type, name,
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

fn print_edges(edges: &[crate::storage::db::GraphEdge], query: &str) {
    // Group into outgoing (source) and incoming (target) edges.
    let outgoing: Vec<_> = edges.iter().filter(|e| {
        e.source_name.as_deref() == Some(query) || e.source_file == query
    }).collect();
    let incoming: Vec<_> = edges.iter().filter(|e| e.target_name == query).collect();
    let other: Vec<_> = edges.iter().filter(|e| {
        e.source_name.as_deref() != Some(query)
            && e.source_file != query
            && e.target_name != query
    }).collect();

    if !outgoing.is_empty() {
        println!("\x1b[1mOutgoing from '{query}':\x1b[0m");
        for e in &outgoing {
            let loc = e.source_name.as_deref().unwrap_or(&e.source_file);
            println!("  \x1b[33m{}\x1b[0m  {}  \x1b[2m({}:{})\x1b[0m",
                e.kind, e.target_name, loc, e.line);
        }
        println!();
    }
    if !incoming.is_empty() {
        println!("\x1b[1mIncoming to '{query}':\x1b[0m");
        for e in &incoming {
            let loc = e.source_name.as_deref().unwrap_or(&e.source_file);
            println!("  \x1b[36m{}\x1b[0m  {}  \x1b[2m({}:{})\x1b[0m",
                e.kind, e.source_file, loc, e.line);
        }
        println!();
    }
    if !other.is_empty() {
        println!("\x1b[1mRelated edges:\x1b[0m");
        for e in &other {
            let loc = e.source_name.as_deref().unwrap_or(&e.source_file);
            println!("  {} -- \x1b[33m{}\x1b[0m --> {}  \x1b[2m({}:{})\x1b[0m",
                loc, e.kind, e.target_name, e.source_file, e.line);
        }
    }
}
