use anyhow::{Context, Result};

use super::super::VerifyArgs;
use super::helpers::load_embedder;
use super::search::resolve_project_and_deps;
use super::ui::spinner;
use crate::{
    config::Config,
    embeddings::{EmbeddingBackend as _, vec_to_blob},
    storage::Database,
};

pub async fn verify(args: VerifyArgs, cfg: Config) -> Result<()> {
    use crate::utils::effective_format;
    let fmt = effective_format(&args.format);

    let (db_path, _dep_paths) = resolve_project_and_deps(args.db.as_ref(), &cfg)?;
    let db = Database::open(&db_path)?;

    // Find chunks matching the target (file suffix or symbol name).
    let target = &args.target;
    let all_chunks = db.chunks_for_file(target)?;
    if all_chunks.is_empty() {
        anyhow::bail!("No indexed chunks found for '{target}'. Try `spelunk index` first.");
    }

    // Build embedder and re-embed each chunk's current content.
    let embedder = load_embedder(&cfg).await?;
    let sp = spinner(format!("Verifying {target}…"));

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
