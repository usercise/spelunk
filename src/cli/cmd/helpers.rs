use anyhow::{Context, Result};

use super::ui::spinner;
use crate::{
    config::{Config, resolve_db},
    embeddings::{EmbeddingBackend as _, vec_to_blob},
    storage::Database,
};

/// Resolve the DB path via `resolve_db`, error if not found, open and return
/// both the path and the opened database.
pub(crate) fn open_project_db(
    db: Option<&std::path::Path>,
    cfg_path: &std::path::Path,
) -> Result<(std::path::PathBuf, Database)> {
    let db_path = resolve_db(db, cfg_path);
    if !db_path.exists() {
        anyhow::bail!(
            "No index found (checked current directory and parents).\n\
             Run `spelunk index <path>` inside your project first."
        );
    }
    let database = Database::open(&db_path)?;
    Ok((db_path, database))
}

/// Show a "Loading embedding model…" spinner, load `ActiveEmbedder`, clear
/// the spinner, and return the embedder.
pub(crate) async fn load_embedder(cfg: &Config) -> Result<crate::backends::ActiveEmbedder> {
    let sp = spinner("Loading embedding model…");
    let embedder = crate::backends::ActiveEmbedder::load(cfg)
        .await
        .with_context(|| format!("loading embedding model '{}'", cfg.embedding_model))?;
    sp.finish_and_clear();
    Ok(embedder)
}

/// Show a "Loading LLM (…)…" spinner, load `ActiveLlm`, clear the spinner,
/// and return the LLM.
pub(crate) async fn load_llm(cfg: &Config) -> Result<crate::backends::ActiveLlm> {
    let model_name = cfg.llm_model.as_deref().unwrap_or("<not configured>");
    let sp = spinner(format!("Loading LLM ({model_name})…"));
    let llm = crate::backends::ActiveLlm::load(cfg)
        .await
        .with_context(|| format!("loading LLM '{model_name}'"))?;
    sp.finish_and_clear();
    Ok(llm)
}

/// Embed a query with the given task prefix and return the blob bytes suitable
/// for KNN search.
pub(crate) async fn embed_query(
    embedder: &crate::backends::ActiveEmbedder,
    task: &str,
    query: &str,
) -> Result<Vec<u8>> {
    let query_text = format!("task: {task} | query: {query}");
    let vecs = embedder.embed(&[&query_text]).await?;
    let blob = vec_to_blob(vecs.first().context("no embedding returned")?);
    Ok(blob)
}

/// Return the final path component of `path` as a display name, falling back
/// to the full path string if there is no file name component.
pub(crate) fn project_display_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}
