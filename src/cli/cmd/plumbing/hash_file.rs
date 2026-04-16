use anyhow::{Context, Result};
use serde::Serialize;

use super::super::super::PlumbingHashFileArgs;
use crate::storage::Database;

#[derive(Serialize)]
struct HashEntry {
    path: String,
    hash: String,
    indexed_hash: Option<String>,
    is_current: bool,
}

pub(super) fn hash_file(args: PlumbingHashFileArgs, db: &Database) -> Result<()> {
    let path = &args.file;
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let hash = format!("{}", blake3::hash(&bytes));

    let path_str = path.to_string_lossy().to_string();
    let indexed_hash = db.file_hash(&path_str)?;
    let is_current = indexed_hash.as_deref() == Some(&hash);

    println!(
        "{}",
        serde_json::to_string(&HashEntry {
            path: path_str,
            hash,
            is_current,
            indexed_hash,
        })?
    );
    Ok(())
}
