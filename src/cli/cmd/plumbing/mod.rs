use anyhow::Result;

use super::super::{PlumbingArgs, PlumbingCommand};
use crate::config::Config;

mod cat_chunks;
mod embed_cmd;
mod graph_edges;
mod hash_file;
mod knn;
mod ls_files;
mod parse_file;
mod read_memory;

pub async fn plumbing(args: PlumbingArgs, cfg: Config) -> Result<()> {
    // Most plumbing commands need the project DB; open it once here.
    // `embed` and `parse-file` do not need it but it's cheap to open.
    let db_path = args
        .db
        .as_deref()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| crate::config::resolve_db(None, &cfg.db_path));

    match args.command {
        PlumbingCommand::ParseFile(a) => return parse_file::parse_file(a),
        PlumbingCommand::Embed(a) => return embed_cmd::embed_cmd(&cfg, a.query).await,
        _ => {}
    }

    // Commands below require the index DB.
    if !db_path.exists() {
        anyhow::bail!(
            "No index found (checked current directory and parents).\n\
             Run `spelunk index <path>` inside your project first."
        );
    }
    let db = crate::storage::Database::open(&db_path)?;

    match args.command {
        PlumbingCommand::CatChunks(a) => cat_chunks::cat_chunks(a, &db, &cfg),
        PlumbingCommand::LsFiles(a) => ls_files::ls_files(a, &db),
        PlumbingCommand::HashFile(a) => hash_file::hash_file(a, &db),
        PlumbingCommand::Knn(a) => knn::knn(a, &db).await,
        PlumbingCommand::GraphEdges(a) => graph_edges::graph_edges(a, &db),
        PlumbingCommand::ReadMemory(a) => {
            let mem_path = db_path.with_file_name("memory.db");
            read_memory::read_memory(a, &mem_path, &cfg).await
        }
        // Already handled above but Rust requires exhaustive match.
        PlumbingCommand::ParseFile(_) | PlumbingCommand::Embed(_) => unreachable!(),
    }
}
