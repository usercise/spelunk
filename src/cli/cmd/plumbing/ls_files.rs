use anyhow::Result;
use serde::Serialize;

use super::super::super::PlumbingLsFilesArgs;
use crate::storage::Database;

#[derive(Serialize)]
struct FileEntry {
    path: String,
    language: Option<String>,
    chunk_count: usize,
    indexed_at: Option<i64>,
}

pub(super) fn ls_files(args: PlumbingLsFilesArgs, db: &Database) -> Result<()> {
    let prefix = args.prefix.as_deref().unwrap_or("");
    let paths = db.file_paths_under(prefix)?;

    if paths.is_empty() {
        std::process::exit(1);
    }

    // Fetch language + chunk count per file in one shot using stats.
    for (_id, path) in paths {
        let chunks = db.chunks_for_file(&path)?;
        let language = chunks.first().map(|c| c.language.clone());
        let entry = FileEntry {
            chunk_count: chunks.len(),
            language,
            path,
            indexed_at: None, // available via db.stats() but not per-file; omit for now
        };
        println!("{}", serde_json::to_string(&entry)?);
    }
    Ok(())
}
