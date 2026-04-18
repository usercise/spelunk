use anyhow::Result;

use super::MemoryListArgs;
use super::{parse_as_of, print_note_summary};
use crate::{config::Config, storage::open_memory_backend};

pub(super) async fn memory_list(
    args: MemoryListArgs,
    mem_path: &std::path::Path,
    cfg: &Config,
) -> Result<()> {
    let backend = open_memory_backend(cfg, mem_path)?;
    let as_of = parse_as_of(args.as_of.as_deref())?;
    let notes = if let Some(ref sha_prefix) = args.source_ref {
        backend
            .list_by_source_ref(sha_prefix, args.limit, args.archived, as_of)
            .await?
    } else {
        backend
            .list(args.kind.as_deref(), args.limit, args.archived, as_of)
            .await?
    };

    if notes.is_empty() {
        println!("No memory entries found.");
        return Ok(());
    }

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
