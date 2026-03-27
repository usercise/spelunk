pub mod backend;
pub mod db;
pub mod memory;
pub mod remote;

pub use backend::{LocalMemoryBackend, MemoryBackend, NoteInput};
pub use db::Database;
pub use memory::MemoryStore;
pub use remote::RemoteMemoryBackend;

use anyhow::Result;
use std::path::Path;

/// Open the appropriate memory backend based on config.
///
/// - If `memory_server_url` is set in config, returns a `RemoteMemoryBackend`.
///   `project_id` must also be set (validated by `Config::validate()`).
/// - Otherwise, opens local SQLite at `mem_path`.
pub fn open_memory_backend(
    cfg: &crate::config::Config,
    mem_path: &Path,
) -> Result<Box<dyn MemoryBackend + Send>> {
    if let Some(url) = &cfg.memory_server_url {
        let project_id = cfg.project_id.clone().expect(
            "project_id must be set when memory_server_url is configured; \
             call Config::validate() before open_memory_backend()",
        );
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()?;
        Ok(Box::new(RemoteMemoryBackend {
            client,
            base_url: url.clone(),
            project_id,
            api_key: cfg.memory_server_key.clone(),
        }))
    } else {
        Ok(Box::new(LocalMemoryBackend::new(MemoryStore::open(
            mem_path,
        )?)))
    }
}
