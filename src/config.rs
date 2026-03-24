use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Walk up from `start` looking for `.spelunk/index.db`.
/// Returns the first match found, or `None` if the filesystem root is reached.
pub fn find_project_db(start: &Path) -> Option<PathBuf> {
    let mut dir = start.to_path_buf();
    loop {
        let candidate = dir.join(".spelunk").join("index.db");
        if candidate.exists() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Resolve the database path.
///
/// Priority: explicit `--db` arg > project DB (walk up from CWD) > `cfg_default`.
pub fn resolve_db(explicit: Option<&PathBuf>, cfg_default: &PathBuf) -> PathBuf {
    if let Some(p) = explicit {
        return p.clone();
    }
    if let Ok(cwd) = std::env::current_dir() {
        if let Some(p) = find_project_db(&cwd) {
            return p;
        }
    }
    cfg_default.clone()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Path to the SQLite database file
    pub db_path: PathBuf,

    /// Directory where model weights are cached (used by backend-metal)
    pub models_dir: PathBuf,

    /// Model ID for embeddings.
    /// LM Studio: the model's API key shown in the LM Studio UI
    ///   (e.g. `text-embedding-embeddinggemma-300m-qat`).
    /// Metal: HuggingFace repo ID (e.g. `google/embeddinggemma-300m`).
    #[serde(default = "Config::default_embedding_model")]
    pub embedding_model: String,

    /// Model ID for the LLM used by `ask`.
    /// LM Studio: the model's API key (e.g. `google/gemma-3n-e4b`).
    /// Metal: HuggingFace repo ID (e.g. `google/gemma-3-1b-it`).
    #[serde(default = "Config::default_llm_model")]
    pub llm_model: String,

    /// Default embedding batch size
    pub batch_size: usize,

    /// Base URL for the LM Studio server (backend-lmstudio only).
    #[serde(default = "Config::default_lmstudio_base_url")]
    pub lmstudio_base_url: String,
}

impl Config {
    fn default_embedding_model() -> String {
        "text-embedding-embeddinggemma-300m-qat".to_string()
    }
    fn default_llm_model() -> String {
        "google/gemma-3n-e4b".to_string()
    }
    fn default_lmstudio_base_url() -> String {
        "http://127.0.0.1:1234".to_string()
    }
}

impl Default for Config {
    fn default() -> Self {
        let base = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("spelunk");

        Self {
            db_path: base.join("index.db"),
            models_dir: base.join("models"),
            embedding_model: Self::default_embedding_model(),
            llm_model: Self::default_llm_model(),
            batch_size: 32,
            lmstudio_base_url: Self::default_lmstudio_base_url(),
        }
    }
}

impl Config {
    /// Load config from file, falling back to defaults.
    /// If `path` is None, looks for `~/.config/spelunk/config.toml`.
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let config_path = match path {
            Some(p) => p.to_path_buf(),
            None => dirs::config_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("spelunk")
                .join("config.toml"),
        };

        if !config_path.exists() {
            return Ok(Self::default());
        }

        let raw = std::fs::read_to_string(&config_path)
            .with_context(|| format!("reading config at {}", config_path.display()))?;

        toml::from_str(&raw).context("parsing config.toml")
    }
}
