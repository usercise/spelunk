use std::io::IsTerminal as _;

use anyhow::{Context, Result};
use ignore::WalkBuilder;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

use crate::{
    config::{resolve_db, Config},
    embeddings::{vec_to_blob, EmbeddingBackend as _},
    indexer::{graph::EdgeExtractor, parser::{detect_language, detect_text_language, is_binary_file, SourceParser}},
    registry::Registry,
    search::SearchResult,
    storage::{Database, MemoryStore},
};
use super::{
    AskArgs, ChunksArgs, GraphArgs, IndexArgs, LinkArgs, MemoryArgs, MemoryCommand,
    SearchArgs, StatusArgs, UnlinkArgs,
};

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
    // Compile secret-scanning regexes once before the hot loop.
    crate::indexer::secrets::init();

    // Default DB lives inside the indexed directory, scoping the index to the project.
    let db_path = args.db.clone().unwrap_or_else(|| {
        args.path.join(".codeanalysis").join("index.db")
    });
    let db = Database::open(&db_path)?;

    // Canonicalise the root so symlinks don't create duplicate entries.
    let root_canonical = args.path.canonicalize().unwrap_or_else(|_| args.path.clone());

    // ── Collect source files ─────────────────────────────────────────────────
    // WalkBuilder respects .gitignore, .ignore, and global gitignore rules.
    // The override below excludes sensitive files unconditionally — even when
    // no .gitignore is present or when they are explicitly un-ignored.
    let sensitive_patterns = [
        "!.env", "!.env.*",
        "!*.pem", "!*.key", "!*.p12", "!*.pfx", "!*.p8",
        "!*.cer", "!*.crt", "!*.der",
        "!id_rsa", "!id_ecdsa", "!id_ed25519", "!id_dsa",
        "!*.keystore", "!*.jks",
        "!.netrc", "!.npmrc",
    ];
    let mut walk = WalkBuilder::new(&args.path);
    walk.standard_filters(true);
    let mut ob = ignore::overrides::OverrideBuilder::new(&args.path);
    for pat in &sensitive_patterns { ob.add(pat).ok(); }
    if let Ok(ov) = ob.build() { walk.overrides(ov); }

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

        let language = detect_language(path)
            .or_else(|| detect_text_language(path))
            .unwrap(); // safe: files were filtered to only include detectable files

        // Skip binary files (e.g. compiled output with wrong extension)
        if matches!(language, "text" | "markdown") && is_binary_file(path) {
            parse_bar.inc(1);
            continue;
        }
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
        println!("No results found. Make sure the index has embeddings (`ca index <path>`).");
        return Ok(());
    }

    // ── Graph-aware enrichment (primary DB only) ──────────────────────────────
    if args.graph {
        if let Ok(primary_db) = Database::open(&db_path) {
            let seen_ids: std::collections::HashSet<i64> =
                results.iter().map(|r| r.chunk_id).collect();
            let names: Vec<&str> = results.iter().filter_map(|r| r.name.as_deref()).collect();

            if !names.is_empty() {
                if let Ok(neighbor_ids) = primary_db.graph_neighbor_chunks(&names) {
                    let new_ids: Vec<i64> = neighbor_ids
                        .into_iter()
                        .filter(|id| !seen_ids.contains(id))
                        .take(args.graph_limit)
                        .collect();

                    if !new_ids.is_empty() {
                        if let Ok(mut extra) = primary_db.chunks_by_ids(&new_ids) {
                            for r in &mut extra {
                                r.from_graph = true;
                            }
                            results.extend(extra);
                        }
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

    let mut results = search_all_dbs(&db_path, &dep_dbs, &query_blob, args.context_chunks.min(100))?;
    sp.finish_and_clear();
    drop(embedder); // free GPU memory before loading the LLM

    if results.is_empty() {
        println!("No relevant code found in the index.");
        return Ok(());
    }

    // ── Step 1b: graph neighbour enrichment (primary DB only) ────────────────
    const MAX_GRAPH_EXTRA: usize = 5;
    if let Ok(primary_db) = Database::open(&db_path) {
        let seen_ids: std::collections::HashSet<i64> =
            results.iter().map(|r| r.chunk_id).collect();
        let names: Vec<&str> = results.iter().filter_map(|r| r.name.as_deref()).collect();
        if !names.is_empty() {
            if let Ok(neighbor_ids) = primary_db.graph_neighbor_chunks(&names) {
                let new_ids: Vec<i64> = neighbor_ids
                    .into_iter()
                    .filter(|id| !seen_ids.contains(id))
                    .take(MAX_GRAPH_EXTRA)
                    .collect();
                if !new_ids.is_empty() {
                    if let Ok(extra) = primary_db.chunks_by_ids(&new_ids) {
                        results.extend(extra);
                    }
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

    // ── Step 2b: prompt injection pre-flight ────────────────────────────────
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
    if INJECTION_PATTERNS.iter().any(|p| question_lower.contains(p)) {
        anyhow::bail!("Question contains a disallowed pattern and cannot be processed.");
    }

    // ── Step 3: build chat messages ──────────────────────────────────────────
    let (system_prompt, json_schema) = if args.json {
        (
            "You are an expert code analyst. \
             Answer the user's question using the code context provided. \
             Respond ONLY with a JSON object. Do not include any other text.",
            Some(ask_json_schema()),
        )
    } else {
        (
            "You are an expert code analyst. \
             Answer the user's question concisely using the code context provided.",
            None,
        )
    };

    let messages = vec![
        crate::llm::Message::system(system_prompt),
        crate::llm::Message::user(format!(
            "<code_context>\n{context}\n</code_context>\n\n<question>\n{question}\n</question>",
            context = context,
            question = args.question,
        )),
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

    if args.json {
        // Collect all tokens then parse + pretty-print the JSON object.
        let collect = async move {
            let mut buf = String::new();
            while let Some(t) = rx.recv().await { buf.push_str(&t); }
            buf
        };
        let (_, raw) = tokio::try_join!(generate, async { Ok::<_, anyhow::Error>(collect.await) })?;
        // Sanitize before parsing: remove any ANSI escape sequences the model may emit.
        let raw = crate::utils::strip_ansi(&raw);
        match serde_json::from_str::<serde_json::Value>(&raw) {
            Ok(v)  => println!("{}", serde_json::to_string_pretty(&v)?),
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
        tokio::try_join!(generate, async { Ok(print_tokens.await) })?;
    }

    Ok(())
}

pub async fn status(args: StatusArgs, cfg: Config) -> Result<()> {
    // --list implies --all
    let show_all = args.all || args.list;

    if show_all {
        let reg = Registry::open().context("opening registry")?;
        let projects = reg.all_projects()?;

        if projects.is_empty() {
            println!("No projects registered. Run `ca index <path>` to get started.");
            return Ok(());
        }

        if args.list {
            // Brief table: one line per project
            println!("{:<6}  {:<8}  {:<10}  {}", "Files", "Chunks", "Embeddings", "Root");
            println!("{}", "─".repeat(70));
            for p in &projects {
                let stats = Database::open(&p.db_path)
                    .and_then(|db| db.stats())
                    .ok();
                let (files, chunks, embeddings) = stats
                    .map(|s| (s.file_count, s.chunk_count, s.embedding_count))
                    .unwrap_or((0, 0, 0));
                let exists = if p.root_path.exists() { "" } else { " [missing]" };
                println!(
                    "{:<6}  {:<8}  {:<10}  {}{}",
                    files, chunks, embeddings,
                    p.root_path.display(), exists
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
                        println!("  Files: {}  Chunks: {}  Embeddings: {}",
                            s.file_count, s.chunk_count, s.embedding_count);
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
    let project = reg
        .as_ref()
        .and_then(|r| {
            std::env::current_dir().ok()
                .and_then(|cwd| r.find_project_for_path(&cwd).ok().flatten())
        });

    let db_path = match &project {
        Some(p) => p.db_path.clone(),
        None => resolve_db(None, &cfg.db_path),
    };

    if !db_path.exists() {
        println!("No index found for the current directory (checked parents too).");
        println!("Run `ca index <path>` to create one.");
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
                let dep_stats = Database::open(&dep.db_path)
                    .and_then(|db| db.stats()).ok();
                let summary = dep_stats
                    .map(|s| format!("{} files, {} chunks", s.file_count, s.chunk_count))
                    .unwrap_or_else(|| "not indexed".to_string());
                println!("  → {}  ({})", dep.root_path.display(), summary);
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

pub fn link(args: LinkArgs, _cfg: Config) -> Result<()> {
    let cwd = std::env::current_dir().context("getting current directory")?;
    let reg = Registry::open().context("opening registry")?;

    // Resolve current project
    let primary = reg.find_project_for_path(&cwd)?
        .with_context(|| format!(
            "No indexed project found for the current directory.\n\
             Run `ca index .` first."
        ))?;

    // Resolve target
    let target_path = if args.path.is_absolute() {
        args.path.clone()
    } else {
        cwd.join(&args.path)
    };
    let target_canonical = target_path.canonicalize()
        .unwrap_or_else(|_| target_path.clone());

    if target_canonical == primary.root_path {
        anyhow::bail!("A project cannot depend on itself.");
    }

    let dep = reg.find_project_for_path(&target_canonical)?
        .with_context(|| format!(
            "No index found for '{}'.\n\
             Run `ca index {}` first.",
            target_canonical.display(),
            target_canonical.display()
        ))?;

    reg.add_dep(primary.id, dep.id)?;

    println!(
        "Linked: {} → {}",
        primary.root_path.display(),
        dep.root_path.display()
    );
    println!("Searches from '{}' will now include results from '{}'.",
        primary.root_path.display(),
        dep.root_path.display()
    );
    Ok(())
}

pub fn unlink(args: UnlinkArgs, _cfg: Config) -> Result<()> {
    let cwd = std::env::current_dir().context("getting current directory")?;
    let reg = Registry::open().context("opening registry")?;

    let primary = reg.find_project_for_path(&cwd)?
        .with_context(|| "No indexed project found for the current directory.")?;

    let target_path = if args.path.is_absolute() {
        args.path.clone()
    } else {
        cwd.join(&args.path)
    };
    let target_canonical = target_path.canonicalize()
        .unwrap_or_else(|_| target_path.clone());

    let dep = reg.find_by_root(&target_canonical)?
        .with_context(|| format!(
            "No registered project found at '{}'.",
            target_canonical.display()
        ))?;

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
        println!("All {} registered project(s) have valid paths — nothing to clean.",
            reg.all_projects()?.len());
    } else {
        println!("Removed {} stale project(s):", removed.len());
        for path in &removed {
            println!("  - {path}");
        }
    }
    Ok(())
}

// ── memory ───────────────────────────────────────────────────────────────────

pub async fn memory(args: MemoryArgs, cfg: Config) -> Result<()> {
    let mem_path = args.db.clone().unwrap_or_else(|| {
        resolve_db(None, &cfg.db_path).with_file_name("memory.db")
    });
    match args.command {
        MemoryCommand::Add(a)    => memory_add(a, &mem_path, &cfg).await,
        MemoryCommand::Search(a) => memory_search(a, &mem_path, &cfg).await,
        MemoryCommand::List(a)   => memory_list(a, &mem_path),
        MemoryCommand::Show(a)   => memory_show(a, &mem_path),
    }
}

async fn memory_add(
    args: super::MemoryAddArgs,
    mem_path: &std::path::Path,
    cfg: &Config,
) -> Result<()> {
    let store = MemoryStore::open(mem_path)?;

    let tags: Vec<&str> = args.tags.as_deref()
        .map(|s| s.split(',').map(str::trim).collect())
        .unwrap_or_default();
    let files: Vec<&str> = args.files.as_deref()
        .map(|s| s.split(',').map(str::trim).collect())
        .unwrap_or_default();

    let note_id = store.add_note(&args.kind, &args.title, &args.body, &tags, &files)?;

    // Embed and store so the note is immediately searchable.
    let embed_text = format!("title: {} | text: {}", args.title, args.body);
    let sp = spinner("Embedding…");
    let embedder = crate::backends::ActiveEmbedder::load(cfg)
        .await
        .context("loading embedding model")?;
    let vecs = embedder.embed(&[&embed_text]).await?;
    sp.finish_and_clear();

    if let Some(vec) = vecs.first() {
        store.insert_embedding(note_id, &vec_to_blob(vec))?;
    }

    println!("Stored [{kind}] #{id}: {title}",
        kind = args.kind, id = note_id, title = args.title);
    Ok(())
}

async fn memory_search(
    args: super::MemorySearchArgs,
    mem_path: &std::path::Path,
    cfg: &Config,
) -> Result<()> {
    let store = MemoryStore::open(mem_path)?;

    let query_text = format!("task: question answering | query: {}", args.query);
    let sp = spinner("Embedding query…");
    let embedder = crate::backends::ActiveEmbedder::load(cfg)
        .await
        .context("loading embedding model")?;
    let vecs = embedder.embed(&[&query_text]).await?;
    sp.finish_and_clear();

    let blob = vec_to_blob(vecs.first().context("no embedding returned")?);
    let notes = store.search(&blob, args.limit)?;

    if notes.is_empty() {
        println!("No memory entries found.");
        return Ok(());
    }

    match args.format.as_str() {
        "json" => println!("{}", serde_json::to_string_pretty(&notes)?),
        _ => {
            for n in &notes {
                print_note_summary(n);
            }
        }
    }
    Ok(())
}

fn memory_list(args: super::MemoryListArgs, mem_path: &std::path::Path) -> Result<()> {
    let store = MemoryStore::open(mem_path)?;
    let notes = store.list(args.kind.as_deref(), args.limit)?;

    if notes.is_empty() {
        println!("No memory entries found.");
        return Ok(());
    }

    match args.format.as_str() {
        "json" => println!("{}", serde_json::to_string_pretty(&notes)?),
        _ => {
            for n in &notes {
                print_note_summary(n);
            }
        }
    }
    Ok(())
}

fn memory_show(args: super::MemoryShowArgs, mem_path: &std::path::Path) -> Result<()> {
    let store = MemoryStore::open(mem_path)?;
    match store.get(args.id)? {
        None => anyhow::bail!("No memory entry with id {}.", args.id),
        Some(n) => match args.format.as_str() {
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
        }
    }
    Ok(())
}

fn print_note_summary(n: &crate::storage::memory::Note) {
    let dist = n.distance.map(|d| format!("  dist: {d:.4}")).unwrap_or_default();
    println!(
        "\x1b[1m#{id}\x1b[0m  \x1b[33m[{kind}]\x1b[0m  {title}\x1b[2m{dist}\x1b[0m",
        id = n.id, kind = n.kind, title = n.title, dist = dist,
    );
    if !n.tags.is_empty() {
        println!("     tags: {}", n.tags.join(", "));
    }
    if !n.linked_files.is_empty() {
        println!("     files: {}", n.linked_files.join(", "));
    }
    // Show first 2 lines of body as preview
    let preview: Vec<&str> = n.body.lines().take(2).collect();
    for line in &preview {
        println!("     \x1b[2m{line}\x1b[0m");
    }
    if n.body.lines().count() > 2 {
        println!("     \x1b[2m…\x1b[0m");
    }
    println!();
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
                "Database not found at '{}'. Run `ca index <path>` first.",
                p.display()
            );
        }
        return Ok((p.clone(), vec![]));
    }

    // Try registry first.
    if let Ok(reg) = Registry::open() {
        if let Ok(cwd) = std::env::current_dir() {
            if let Ok(Some(project)) = reg.find_project_for_path(&cwd) {
                if project.db_path.exists() {
                    let deps = reg.get_deps(project.id)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|d| d.db_path)
                        .filter(|p| p.exists())
                        .collect();
                    return Ok((project.db_path, deps));
                }
            }
        }
    }

    // Fallback: filesystem walk-up.
    let db_path = resolve_db(None, &cfg.db_path);
    if !db_path.exists() {
        anyhow::bail!(
            "No index found (checked current directory and parents).\n\
             Run `ca index <path>` inside your project first."
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
            Ok(dep_db) => {
                match dep_db.search_similar(query_blob, fetch) {
                    Ok(mut dep_results) => all.append(&mut dep_results),
                    Err(e) => tracing::warn!("search failed on dep {}: {e}", dep_path.display()),
                }
            }
            Err(e) => tracing::warn!("could not open dep DB {}: {e}", dep_path.display()),
        }
    }

    // Sort by distance (ascending), deduplicate by (file_path, start_line, end_line).
    all.sort_by(|a, b| a.distance.partial_cmp(&b.distance).unwrap_or(std::cmp::Ordering::Equal));
    let mut seen = std::collections::HashSet::new();
    all.retain(|r| seen.insert((r.file_path.clone(), r.start_line, r.end_line)));
    all.truncate(limit);
    Ok(all)
}

fn format_age(unix_ts: i64) -> String {
    use std::time::{Duration, UNIX_EPOCH};
    if let Ok(t) = UNIX_EPOCH.checked_add(Duration::from_secs(unix_ts as u64)).ok_or(()) {
        if let Ok(elapsed) = std::time::SystemTime::now().duration_since(t) {
            let secs = elapsed.as_secs();
            return if secs < 60 { format!("{secs}s ago") }
                else if secs < 3600 { format!("{}m ago", secs / 60) }
                else if secs < 86400 { format!("{}h ago", secs / 3600) }
                else { format!("{}d ago", secs / 86400) };
        }
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
