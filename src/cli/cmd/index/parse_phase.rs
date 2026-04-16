use anyhow::{Context, Result};
use ignore::WalkBuilder;
use indicatif::{MultiProgress, ProgressBar};

use super::super::super::IndexArgs;
use super::super::ui::{is_tty, progress_style, short_path};
#[cfg(feature = "rich-formats")]
use crate::indexer::docparser::parse_doc;
use crate::{
    indexer::{
        graph::EdgeExtractor,
        parser::{
            SourceParser, detect_doc_language, detect_language, detect_text_language,
            is_binary_file,
        },
    },
    search::tokens::estimate_tokens,
    storage::Database,
};

pub(super) struct ParseResult {
    /// (chunk_id, embedding_text) pairs awaiting embedding.
    pub chunk_ids_and_texts: Vec<(i64, String)>,
    pub indexed: u64,
    pub removed: u64,
}

/// Collect source files from `root`, parse them, store chunks + graph edges,
/// then remove stale index records for files that no longer exist.
pub(super) fn run_parse_phase(
    root: &std::path::Path,
    db: &Database,
    args: &IndexArgs,
    mp: &MultiProgress,
) -> Result<ParseResult> {
    let files = collect_files(root)?;

    if files.is_empty() {
        println!("No supported source files found in {}", root.display());
        return Ok(ParseResult {
            chunk_ids_and_texts: vec![],
            indexed: 0,
            removed: 0,
        });
    }

    let parse_bar = if is_tty() && !crate::utils::is_agent_mode() {
        let bar = mp.add(ProgressBar::new(files.len() as u64));
        bar.set_style(progress_style("Parsing  "));
        bar
    } else {
        ProgressBar::hidden()
    };

    let mut chunk_ids_and_texts: Vec<(i64, String)> = Vec::new();
    let mut indexed = 0u64;
    let mut skipped = 0u64; // tracked for progress bar message only

    for entry in &files {
        let path = entry.path();
        // Store paths relative to the project root so the index is portable.
        let rel = path.strip_prefix(root).unwrap_or(path);
        let path_str = rel.to_string_lossy();
        parse_bar.set_message(short_path(&path_str));

        // ── Binary document formats (DOCX, XLSX, PDF, …) ─────────────────────
        #[cfg(feature = "rich-formats")]
        if let Some(doc_lang) = detect_doc_language(path) {
            if process_doc_file(
                path,
                &path_str,
                doc_lang,
                db,
                args,
                &mut chunk_ids_and_texts,
                &mut indexed,
                &mut skipped,
            )? {
                parse_bar.inc(1);
                continue;
            }
        }

        // ── PDF documents (feature-gated) ─────────────────────────────────────
        #[cfg(feature = "rich-formats")]
        if detect_language(path) == Some("pdf") {
            if process_pdf_file(
                path,
                &path_str,
                db,
                args,
                &mut chunk_ids_and_texts,
                &mut indexed,
            )? {
                parse_bar.inc(1);
                continue;
            }
        }

        // ── Text / code formats ───────────────────────────────────────────────
        process_text_file(
            path,
            &path_str,
            db,
            args,
            &mut chunk_ids_and_texts,
            &mut indexed,
            &mut skipped,
        )?;
        parse_bar.inc(1);
    }

    parse_bar.finish_with_message(format!(
        "{indexed} files parsed ({skipped} skipped, {indexed} new/changed)"
    ));

    let removed = remove_stale_files(&files, root, db)?;

    Ok(ParseResult {
        chunk_ids_and_texts,
        indexed,
        removed,
    })
}

// ── File collection ───────────────────────────────────────────────────────────

fn collect_files(root: &std::path::Path) -> Result<Vec<ignore::DirEntry>> {
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
    let mut walk = WalkBuilder::new(root);
    walk.standard_filters(true);
    let mut ob = ignore::overrides::OverrideBuilder::new(root);
    for pat in &sensitive_patterns {
        ob.add(pat).ok();
    }
    if let Ok(ov) = ob.build() {
        walk.overrides(ov);
    }

    Ok(walk
        .build()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|ft| ft.is_file()).unwrap_or(false))
        .filter(|e| {
            let p = e.path();
            detect_language(p).is_some()
                || detect_text_language(p).is_some()
                || detect_doc_language(p).is_some()
        })
        .collect())
}

// ── Per-file processors ───────────────────────────────────────────────────────

#[cfg(feature = "rich-formats")]
fn process_doc_file(
    path: &std::path::Path,
    path_str: &str,
    doc_lang: &'static str,
    db: &Database,
    args: &IndexArgs,
    out: &mut Vec<(i64, String)>,
    indexed: &mut u64,
    skipped: &mut u64,
) -> Result<bool> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("read error for {path_str}: {e}");
            return Ok(true);
        }
    };
    let hash = format!("{}", blake3::hash(&bytes));
    if !args.force
        && let Some(existing) = db.file_hash(path_str)?
        && existing == hash
    {
        *skipped += 1;
        return Ok(true);
    }
    let chunks = parse_doc(&bytes, path_str, doc_lang);
    let file_id = db.upsert_file(path_str, Some(doc_lang), &hash)?;
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
        let metadata =
            serde_json::json!({ "docstring": chunk.docstring, "parent_scope": chunk.parent_scope });
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
        out.push((chunk_id, chunk.embedding_text()));
    }
    *indexed += 1;
    Ok(true)
}

#[cfg(feature = "rich-formats")]
fn process_pdf_file(
    path: &std::path::Path,
    path_str: &str,
    db: &Database,
    args: &IndexArgs,
    out: &mut Vec<(i64, String)>,
    indexed: &mut u64,
) -> Result<bool> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("read error for {path_str}: {e}");
            return Ok(true);
        }
    };
    let hash = format!("{}", blake3::hash(&bytes));
    if !args.force
        && let Some(existing) = db.file_hash(path_str)?
        && existing == hash
    {
        return Ok(true);
    }
    match crate::indexer::pdf::extract_pdf_text(path) {
        Ok(pages) => {
            let file_id = db.upsert_file(path_str, Some("pdf"), &hash)?;
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
                let metadata = serde_json::json!({ "docstring": chunk.docstring, "parent_scope": chunk.parent_scope });
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
                out.push((chunk_id, chunk.embedding_text()));
            }
            *indexed += 1;
        }
        Err(e) => {
            tracing::warn!("skipping PDF {}: {e}", path.display());
        }
    }
    Ok(true)
}

fn process_text_file(
    path: &std::path::Path,
    path_str: &str,
    db: &Database,
    args: &IndexArgs,
    out: &mut Vec<(i64, String)>,
    indexed: &mut u64,
    skipped: &mut u64,
) -> Result<()> {
    let language = detect_language(path)
        .or_else(|| detect_text_language(path))
        .unwrap(); // safe: files were filtered to only include detectable files

    // Skip binary files (e.g. compiled output with wrong extension)
    if matches!(language, "text" | "markdown") && is_binary_file(path) {
        return Ok(());
    }
    let source =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let hash = format!("{}", blake3::hash(source.as_bytes()));

    if !args.force
        && let Some(existing) = db.file_hash(path_str)?
        && existing == hash
    {
        *skipped += 1;
        return Ok(());
    }

    let chunks = match SourceParser::parse(&source, path_str, language) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("parse error for {path_str}: {e}");
            return Ok(());
        }
    };

    let file_id = db.upsert_file(path_str, Some(language), &hash)?;
    db.delete_embeddings_for_file(file_id)?;
    db.delete_chunks_for_file(file_id)?;

    // Extract and store graph edges for this file.
    match EdgeExtractor::extract(&source, path_str, language) {
        Ok(edges) => {
            if let Err(e) = db.replace_edges(path_str, &edges) {
                tracing::warn!("graph edge storage failed for {path_str}: {e}");
            }
        }
        Err(e) => tracing::warn!("graph extraction failed for {path_str}: {e}"),
    }

    for chunk in &chunks {
        if crate::indexer::secrets::contains_secret(&chunk.content) {
            tracing::warn!(
                "skipping chunk '{}' in {path_str} (possible secret detected)",
                chunk.name.as_deref().unwrap_or("<anonymous>"),
            );
            continue;
        }
        let metadata =
            serde_json::json!({ "docstring": chunk.docstring, "parent_scope": chunk.parent_scope });
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
        out.push((chunk_id, chunk.embedding_text()));
    }

    *indexed += 1;
    Ok(())
}

// ── Stale file cleanup ────────────────────────────────────────────────────────

fn remove_stale_files(
    files: &[ignore::DirEntry],
    root: &std::path::Path,
    db: &Database,
) -> Result<u64> {
    // Paths in the DB are root-relative, so visited uses the same relative form.
    let visited: std::collections::HashSet<String> = files
        .iter()
        .map(|e| {
            let p = e.path();
            p.strip_prefix(root)
                .unwrap_or(p)
                .to_string_lossy()
                .to_string()
        })
        .collect();
    // Pass "" so file_paths_under returns all files in this DB (paths are relative).
    let all_indexed = db.file_paths_under("")?;
    let mut removed = 0u64;
    for (id, path) in all_indexed {
        if !visited.contains(&path) {
            db.delete_file(id, &path)?;
            removed += 1;
        }
    }
    Ok(removed)
}
