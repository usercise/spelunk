use anyhow::{Context, Result};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use walkdir::WalkDir;

use crate::{
    config::Config,
    embeddings::{candle::vec_to_blob, EmbeddingBackend},
    indexer::parser::{detect_language, SourceParser},
    storage::Database,
};
use super::{AskArgs, IndexArgs, SearchArgs};

pub async fn index(args: IndexArgs, cfg: Config) -> Result<()> {
    let db_path = args.db.as_ref().unwrap_or(&cfg.db_path);
    let db = Database::open(db_path)?;

    // ── Collect source files ─────────────────────────────────────────────────
    let files: Vec<_> = WalkDir::new(&args.path)
        .into_iter()
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
    let parse_bar = mp.add(ProgressBar::new(files.len() as u64));
    parse_bar.set_style(progress_style("Parsing  "));

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
        "{} files parsed ({} skipped)",
        indexed, skipped
    ));

    if chunk_ids_and_texts.is_empty() {
        println!("Nothing new to embed.");
        return Ok(());
    }

    // ── Phase 2: embed chunks ────────────────────────────────────────────────
    println!("Loading embedding model: {}", cfg.embedding_model);

    let models_dir = cfg.models_dir.clone();
    let model_id = cfg.embedding_model.clone();
    let embedder = crate::embeddings::candle::CandleEmbedder::load(&model_id, &models_dir)
        .await
        .with_context(|| format!("loading embedding model '{model_id}'"))?;

    let batch_size = args.batch_size.max(1);
    let total_chunks = chunk_ids_and_texts.len() as u64;

    let embed_bar = mp.add(ProgressBar::new(total_chunks));
    embed_bar.set_style(progress_style("Embedding"));

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

pub async fn search(_args: SearchArgs, _cfg: Config) -> Result<()> {
    todo!("Phase 4: vector search")
}

pub async fn ask(_args: AskArgs, _cfg: Config) -> Result<()> {
    todo!("Phase 5: RAG pipeline with Gemma 3n")
}

pub async fn status(cfg: Config) -> Result<()> {
    let db_path = &cfg.db_path;
    if !db_path.exists() {
        println!(
            "No index found at {}.\nRun `ca index <path>` to create one.",
            db_path.display()
        );
        return Ok(());
    }
    let db = Database::open(db_path)?;
    let s = db.stats()?;
    println!("Index:      {}", db_path.display());
    println!("Files:      {}", s.file_count);
    println!("Chunks:     {}", s.chunk_count);
    println!("Embeddings: {}", s.embedding_count);
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

// ── helpers ──────────────────────────────────────────────────────────────────

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
