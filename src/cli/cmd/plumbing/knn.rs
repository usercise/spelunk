use anyhow::{Context, Result};

use super::super::super::PlumbingKnnArgs;
use super::embed_query;
use crate::{config::Config, storage::Database};

pub(super) async fn knn(args: PlumbingKnnArgs, db: &Database, cfg: &Config) -> Result<()> {
    let embedder = crate::backends::ActiveEmbedder::load(cfg)
        .await
        .context("loading embedding model")?;

    let blob = embed_query(&embedder, "code retrieval", &args.query).await?;

    let mut results = db.search_similar(&blob, args.limit + 20)?;

    // Filter by min_score (distance ≤ 1 - min_score for cosine) and language.
    // sqlite-vec returns cosine distance; convert to similarity for --min-score.
    results.retain(|r| {
        let score = 1.0 - r.distance;
        if score < args.min_score {
            return false;
        }
        if let Some(ref lang) = args.lang
            && &r.language != lang
        {
            return false;
        }
        true
    });
    results.truncate(args.limit);

    if results.is_empty() {
        std::process::exit(1);
    }

    for r in &results {
        // Augment with a `score` field without changing SearchResult struct.
        let score = 1.0 - r.distance;
        let mut val = serde_json::to_value(r)?;
        if let serde_json::Value::Object(ref mut m) = val {
            m.insert("score".into(), serde_json::json!(score));
        }
        println!("{}", serde_json::to_string(&val)?);
    }
    Ok(())
}
