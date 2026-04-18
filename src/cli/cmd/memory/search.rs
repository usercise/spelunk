use anyhow::{Context, Result};

use super::super::helpers::embed_query;
use super::MemorySearchArgs;
use super::{parse_as_of, print_note_summary};
use crate::{config::Config, storage::open_memory_backend};

pub(super) async fn memory_search(
    args: MemorySearchArgs,
    mem_path: &std::path::Path,
    cfg: &Config,
) -> Result<()> {
    let index_db_path = crate::config::resolve_db(None, &cfg.db_path);
    crate::storage::record_usage_at(&index_db_path, "memory search");

    let mode = args.mode.as_str();
    let backend = open_memory_backend(cfg, mem_path)?;
    let as_of = parse_as_of(args.as_of.as_deref())?;

    let notes = if mode == "text" {
        let sp = super::super::ui::spinner("Searching (text)…");
        let result = backend.search_text(&args.query, args.limit, as_of).await?;
        sp.finish_and_clear();
        result
    } else {
        let sp = super::super::ui::spinner("Embedding query…");
        let embedder = crate::backends::ActiveEmbedder::load(cfg)
            .await
            .context("loading embedding model")?;
        let blob = embed_query(&embedder, "question answering", &args.query).await?;
        sp.finish_and_clear();

        if mode == "semantic" {
            backend.search(&blob, args.limit, as_of).await?
        } else {
            backend
                .search_hybrid(&blob, &args.query, args.limit, as_of)
                .await?
        }
    };

    if notes.is_empty() {
        println!("No memory entries found.");
        return Ok(());
    }

    let notes = if args.expand_graph {
        let mut seen: std::collections::HashSet<i64> = notes.iter().map(|n| n.id).collect();
        let mut expanded = notes;
        let mut neighbours = vec![];
        for n in &expanded {
            let (outgoing, incoming) = backend.get_edges(n.id).await?;
            for e in outgoing.iter().chain(incoming.iter()) {
                if e.kind != "relates_to" {
                    continue;
                }
                let neighbour_id = if e.from_id == n.id {
                    e.to_id
                } else {
                    e.from_id
                };
                if seen.insert(neighbour_id)
                    && let Some(nb) = backend.get(neighbour_id).await?
                {
                    neighbours.push(nb);
                }
            }
        }
        expanded.extend(neighbours);
        expanded
    } else {
        notes
    };

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
