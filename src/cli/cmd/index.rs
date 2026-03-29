use anyhow::{Context, Result};
use futures_util::StreamExt as _;
use ignore::WalkBuilder;
use indicatif::{MultiProgress, ProgressBar};

use super::super::IndexArgs;
use super::ui::{is_tty, progress_style, short_path};
use crate::{
    config::Config,
    embeddings::{EmbeddingBackend as _, vec_to_blob},
    indexer::{
        docparser::parse_doc,
        graph::EdgeExtractor,
        parser::{
            SourceParser, detect_doc_language, detect_language, detect_text_language,
            is_binary_file,
        },
    },
    registry::Registry,
    search::tokens::estimate_tokens,
    storage::Database,
};

pub async fn index(args: IndexArgs, cfg: Config) -> Result<()> {
    // Compile secret-scanning regexes once before the hot loop.
    crate::indexer::secrets::init();

    // Default DB lives inside the indexed directory, scoping the index to the project.
    let db_path = args
        .db
        .clone()
        .unwrap_or_else(|| args.path.join(".spelunk").join("index.db"));
    let db = Database::open(&db_path)?;

    // --recount: backfill token_count for existing chunks, then exit.
    if args.recount {
        let updated = db.backfill_token_counts()?;
        println!("Backfilled token counts for {updated} chunk(s).");
        return Ok(());
    }

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
            detect_language(p).is_some()
                || detect_text_language(p).is_some()
                || detect_doc_language(p).is_some()
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

        // ── Binary document formats (DOCX, XLSX, …) ──────────────────────────
        // These cannot be read with read_to_string and have no call graph.
        if let Some(doc_lang) = detect_doc_language(path) {
            let bytes = match std::fs::read(path) {
                Ok(b) => b,
                Err(e) => {
                    tracing::warn!("read error for {path_str}: {e}");
                    parse_bar.inc(1);
                    continue;
                }
            };
            let hash = format!("{}", blake3::hash(&bytes));
            if !args.force
                && let Some(existing) = db.file_hash(&path_str)?
                && existing == hash
            {
                skipped += 1;
                parse_bar.inc(1);
                continue;
            }
            let chunks = parse_doc(&bytes, &path_str, doc_lang);
            let file_id = db.upsert_file(&path_str, Some(doc_lang), &hash)?;
            db.delete_embeddings_for_file(file_id)?;
            db.delete_chunks_for_file(file_id)?;
            for chunk in &chunks {
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
                let tc = estimate_tokens(&chunk.content);
                let chunk_id = db.insert_chunk(
                    file_id,
                    &chunk.kind.to_string(),
                    chunk.name.as_deref(),
                    chunk.start_line,
                    chunk.end_line,
                    &chunk.content,
                    Some(&metadata.to_string()),
                    tc,
                )?;
                chunk_ids_and_texts.push((chunk_id, chunk.embedding_text()));
            }
            indexed += 1;
            parse_bar.inc(1);
            continue;
        }

        // ── PDF documents (feature-gated) ─────────────────────────────────────
        #[cfg(feature = "pdf")]
        if detect_language(path) == Some("pdf") {
            let bytes = match std::fs::read(path) {
                Ok(b) => b,
                Err(e) => {
                    tracing::warn!("read error for {path_str}: {e}");
                    parse_bar.inc(1);
                    continue;
                }
            };
            let hash = format!("{}", blake3::hash(&bytes));
            if !args.force
                && let Some(existing) = db.file_hash(&path_str)?
                && existing == hash
            {
                skipped += 1;
                parse_bar.inc(1);
                continue;
            }
            match crate::indexer::pdf::extract_pdf_text(path) {
                Ok(pages) => {
                    let file_id = db.upsert_file(&path_str, Some("pdf"), &hash)?;
                    db.delete_embeddings_for_file(file_id)?;
                    db.delete_chunks_for_file(file_id)?;
                    for (page_num, text) in pages {
                        if crate::indexer::secrets::contains_secret(&text) {
                            tracing::warn!(
                                "skipping PDF page {page_num} in {path_str} (possible secret detected)",
                            );
                            continue;
                        }
                        let chunk = crate::indexer::Chunk {
                            file_path: path_str.to_string(),
                            language: "pdf".to_string(),
                            kind: crate::indexer::ChunkKind::Section,
                            name: Some(format!("page {page_num}")),
                            start_line: page_num as usize,
                            end_line: page_num as usize,
                            content: text,
                            docstring: None,
                            parent_scope: None,
                            summary: None,
                        };
                        let metadata = serde_json::json!({
                            "docstring": chunk.docstring,
                            "parent_scope": chunk.parent_scope,
                        });
                        let tc = estimate_tokens(&chunk.content);
                        let chunk_id = db.insert_chunk(
                            file_id,
                            &chunk.kind.to_string(),
                            chunk.name.as_deref(),
                            chunk.start_line,
                            chunk.end_line,
                            &chunk.content,
                            Some(&metadata.to_string()),
                            tc,
                        )?;
                        chunk_ids_and_texts.push((chunk_id, chunk.embedding_text()));
                    }
                    indexed += 1;
                }
                Err(e) => {
                    tracing::warn!("skipping PDF {}: {e}", path.display());
                }
            }
            parse_bar.inc(1);
            continue;
        }

        // ── Text / code formats ───────────────────────────────────────────────
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
            && existing == hash
        {
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
            let tc = estimate_tokens(&chunk.content);
            let chunk_id = db.insert_chunk(
                file_id,
                &chunk.kind.to_string(),
                chunk.name.as_deref(),
                chunk.start_line,
                chunk.end_line,
                &chunk.content,
                Some(&metadata.to_string()),
                tc,
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

    // Embed each chunk concurrently, keeping up to `concurrency` requests
    // in-flight at the same time. Each future carries the chunk_id so results
    // can be stored in the correct order after all tasks finish.
    let concurrency = batch_size;

    let results: Vec<(i64, Vec<f32>)> = futures_util::stream::iter(
        chunk_ids_and_texts
            .iter()
            .map(|(chunk_id, text)| (*chunk_id, text.clone())),
    )
    .map(|(chunk_id, text)| {
        let embedder = &embedder;
        let embed_bar = &embed_bar;
        async move {
            // Simple exponential-backoff retry for transient 429 / server errors.
            let mut delay_ms = 100u64;
            let mut last_err: anyhow::Error = anyhow::anyhow!("unreachable");
            for attempt in 0..3u32 {
                match embedder.embed(&[text.as_str()]).await {
                    Ok(mut vecs) => {
                        embed_bar.inc(1);
                        return Ok::<(i64, Vec<f32>), anyhow::Error>((chunk_id, vecs.remove(0)));
                    }
                    Err(e) => {
                        last_err = e;
                        if attempt < 2 {
                            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                            delay_ms *= 2;
                        }
                    }
                }
            }
            Err(last_err.context("generating embedding (3 attempts failed)"))
        }
    })
    .buffer_unordered(concurrency)
    .collect::<Vec<_>>()
    .await
    .into_iter()
    .collect::<Result<Vec<_>>>()?;

    // Write embeddings serially — rusqlite connections are not Send.
    for (chunk_id, embedding) in results {
        let blob = vec_to_blob(&embedding);
        db.insert_embedding(chunk_id, &blob)?;
    }

    embed_bar.finish_with_message(format!("{total_chunks} chunks embedded"));

    let stats = db.stats()?;
    println!(
        "\nIndex: {} files, {} chunks, {} embeddings",
        stats.file_count, stats.chunk_count, stats.embedding_count
    );

    // ── Phase 3: compute and store PageRank scores ────────────────────────────
    eprintln!("Computing graph rank…");
    let edges = db.graph_edges_all()?;
    if !edges.is_empty() {
        let pr_scores = crate::indexer::pagerank::compute_pagerank(&edges, 20, 0.85);
        let named_chunks = db.chunks_with_names()?;
        let updates: Vec<(i64, f32)> = named_chunks
            .into_iter()
            .filter_map(|(id, name)| name.and_then(|n| pr_scores.get(&n).copied().map(|s| (id, s))))
            .collect();
        if !updates.is_empty() {
            db.update_graph_ranks(&updates)?;
        }
    }

    // ── Phase 4: auto-discover spec files ────────────────────────────────────
    let mut specs_found = 0u32;
    for entry in &files {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        if super::spec::is_spec_file(path, &cfg.specs_dir) {
            let path_str = path.to_string_lossy().into_owned();
            let title = super::spec::extract_spec_title(path).unwrap_or_default();
            if let Err(e) = db.upsert_spec(&path_str, &title, true) {
                tracing::warn!("spec registration failed for {path_str}: {e}");
            } else {
                specs_found += 1;
            }
        }
    }
    if specs_found > 0 {
        eprintln!("Registered {specs_found} spec file(s).");
    }

    // ── Phase 5: generate LLM summaries ──────────────────────────────────────
    generate_summaries(&args, &cfg, &db).await?;

    // Register / update this project in the global registry.
    if let Ok(reg) = Registry::open() {
        let db_canonical = db_path.canonicalize().unwrap_or(db_path.clone());
        if let Err(e) = reg.register(&root_canonical, &db_canonical) {
            tracing::warn!("registry update failed: {e}");
        }
    }

    Ok(())
}

/// Run the optional LLM summary generation pass.
///
/// Fetches chunks without summaries in batches, calls the LLM, and stores results.
/// If no `llm_model` is configured or `--no-summaries` is set, prints a message and returns.
async fn generate_summaries(args: &IndexArgs, cfg: &Config, db: &Database) -> Result<()> {
    if args.no_summaries {
        return Ok(());
    }

    if cfg.llm_model.is_none() {
        eprintln!("  Skipping summaries (no llm_model configured)");
        return Ok(());
    }

    // Count total chunks needing summaries for progress reporting.
    let batch_size = args.summary_batch_size.max(1);
    let first_batch = db.chunks_without_summaries(1)?;
    if first_batch.is_empty() {
        return Ok(());
    }

    // Load the LLM backend.
    let llm = crate::backends::ActiveLlm::load(cfg)
        .await
        .with_context(|| {
            format!(
                "loading LLM model '{}'",
                cfg.llm_model.as_deref().unwrap_or("unknown")
            )
        })?;

    // Count pending chunks for progress display.
    let pending = db.chunks_without_summaries(usize::MAX)?;
    let total_chunks = pending.len();
    let total_batches = total_chunks.div_ceil(batch_size);

    eprintln!("Generating summaries ({total_chunks} chunks, batch size {batch_size})\u{2026}");

    let mut batch_num = 0usize;
    loop {
        let batch = db.chunks_without_summaries(batch_size)?;
        if batch.is_empty() {
            break;
        }
        batch_num += 1;
        eprintln!("  Summarising batch {batch_num}/{total_batches}\u{2026}");

        match crate::indexer::summariser::summarise_batch(&llm, &batch).await {
            Ok(summaries) => {
                let mut summarised_ids = std::collections::HashSet::new();
                for (chunk_id, summary) in summaries {
                    if let Err(e) = db.update_chunk_summary(chunk_id, &summary) {
                        tracing::warn!("failed to store summary for chunk {chunk_id}: {e}");
                    } else {
                        summarised_ids.insert(chunk_id);
                    }
                }
                // Mark chunks that received no summary with "" so they aren't
                // re-fetched on the next pass (chunks_without_summaries checks IS NULL).
                for (id, _, _, _) in &batch {
                    if !summarised_ids.contains(id) {
                        let _ = db.update_chunk_summary(*id, "");
                    }
                }
            }
            Err(e) => {
                tracing::warn!("summarise_batch failed: {e}");
                // Mark the batch as attempted so we don't loop forever.
                for (id, _, _, _) in &batch {
                    let _ = db.update_chunk_summary(*id, "");
                }
            }
        }
    }

    eprintln!("  Summarised {batch_num} batch(es).");
    Ok(())
}
