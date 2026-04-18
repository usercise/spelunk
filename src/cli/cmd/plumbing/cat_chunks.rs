use anyhow::Result;

use super::PlumbingCatChunksArgs;
use crate::{config::Config, storage::Database};

pub(super) fn cat_chunks(args: PlumbingCatChunksArgs, db: &Database, _cfg: &Config) -> Result<()> {
    let chunks = db.chunks_for_file(&args.file)?;
    if chunks.is_empty() {
        eprintln!("No indexed chunks for '{}'", args.file);
        std::process::exit(1);
    }
    for c in chunks {
        println!("{}", serde_json::to_string(&c)?);
    }
    Ok(())
}
