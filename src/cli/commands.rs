use anyhow::{Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use walkdir::WalkDir;

use crate::{
    config::Config,
    indexer::parser::{detect_language, SourceParser},
    storage::Database,
};
use super::{AskArgs, IndexArgs, SearchArgs};

pub async fn index(args: IndexArgs, cfg: Config) -> Result<()> {
    let db_path = args.db.unwrap_or(cfg.db_path);
    let db = Database::open(&db_path)?;

    // Collect all files upfront so we can show a total in the progress bar
    let files: Vec<_> = WalkDir::new(&args.path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| detect_language(e.path()).is_some())
        .collect();

    let total = files.len() as u64;
    if total == 0 {
        println!("No supported source files found in {}", args.path.display());
        return Ok(());
    }

    let bar = ProgressBar::new(total);
    bar.set_style(
        ProgressStyle::with_template(
            "{bar:40.cyan/blue} {pos}/{len} {wide_msg}",
        )
        .unwrap(),
    );

    let mut indexed = 0u64;
    let mut skipped = 0u64;
    let mut total_chunks = 0usize;

    for entry in &files {
        let path = entry.path();
        let path_str = path.to_string_lossy();
        bar.set_message(path_str.to_string());

        let language = detect_language(path).unwrap(); // filtered above

        // Read and hash the file
        let source = std::fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        let hash = format!("{}", blake3::hash(source.as_bytes()));

        // Skip unchanged files unless --force
        if !args.force {
            if let Some(existing) = db.file_hash(&path_str)? {
                if existing == hash {
                    skipped += 1;
                    bar.inc(1);
                    continue;
                }
            }
        }

        // Parse into chunks
        let chunks = match SourceParser::parse(&source, &path_str, language) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("parse error for {path_str}: {e}");
                bar.inc(1);
                continue;
            }
        };

        // Store file + chunks
        let file_id = db.upsert_file(&path_str, Some(language), &hash)?;
        db.delete_chunks_for_file(file_id)?;

        for chunk in &chunks {
            let metadata = serde_json::json!({
                "docstring": chunk.docstring,
                "parent_scope": chunk.parent_scope,
            });
            db.insert_chunk(
                file_id,
                &chunk.kind.to_string(),
                chunk.name.as_deref(),
                chunk.start_line,
                chunk.end_line,
                &chunk.content,
                Some(&metadata.to_string()),
            )?;
        }

        total_chunks += chunks.len();
        indexed += 1;
        bar.inc(1);
    }

    bar.finish_and_clear();

    println!(
        "Indexed {} files ({} chunks extracted). {} files skipped (unchanged).",
        indexed, total_chunks, skipped
    );

    // Phase 3: embed all chunks and write to sqlite-vec
    if indexed > 0 {
        println!("Note: embedding not yet implemented (Phase 3). Chunks are stored; vector search will be available after Phase 3.");
    }

    Ok(())
}

pub async fn search(_args: SearchArgs, _cfg: Config) -> Result<()> {
    todo!("Phase 4: vector search")
}

pub async fn ask(_args: AskArgs, _cfg: Config) -> Result<()> {
    todo!("Phase 5: RAG pipeline with Gemma 3n")
}

pub async fn status(cfg: Config) -> Result<()> {
    let db_path = cfg.db_path;
    if !db_path.exists() {
        println!("No index found at {}. Run `ca index <path>` to create one.", db_path.display());
        return Ok(());
    }

    let db = Database::open(&db_path)?;
    let stats = db.stats()?;
    println!("Index: {}", db_path.display());
    println!("  Files:  {}", stats.file_count);
    println!("  Chunks: {}", stats.chunk_count);
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
