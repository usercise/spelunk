use anyhow::Result;
use serde::Serialize;

use super::PlumbingLsFilesArgs;
use crate::storage::Database;

#[derive(Serialize)]
struct FileEntry {
    path: String,
    language: Option<String>,
    chunk_count: usize,
    indexed_at: Option<i64>,
    stale: bool,
}

pub(super) fn ls_files(args: PlumbingLsFilesArgs, db: &Database) -> Result<()> {
    let root = args
        .root
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let prefix = args.prefix.as_deref().unwrap_or("");
    let records = db.file_records_under(prefix)?;

    if records.is_empty() {
        std::process::exit(1);
    }

    let mut emitted = false;
    for record in records {
        // Paths in the DB are relative to the project root; join before hashing.
        let on_disk = root.join(&record.path);
        let stale = match std::fs::read(&on_disk) {
            Ok(bytes) => format!("{}", blake3::hash(&bytes)) != record.hash,
            Err(_) => true, // file missing on disk — treat as stale
        };

        if args.stale && !stale {
            continue;
        }

        let chunks = db.chunks_for_file(&record.path)?;
        let chunk_count = chunks.len();

        // language from DB record; fall back to first chunk's language if not stored
        let lang = record
            .language
            .or_else(|| chunks.first().map(|c| c.language.clone()));

        let entry = FileEntry {
            path: record.path,
            language: lang,
            chunk_count,
            indexed_at: Some(record.indexed_at),
            stale,
        };
        println!("{}", serde_json::to_string(&entry)?);
        emitted = true;
    }
    if !emitted {
        std::process::exit(1);
    }
    Ok(())
}
