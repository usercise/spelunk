use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Walk up from `start` looking for `.codeanalysis/index.db`.
/// Returns the first match found, or `None` if the filesystem root is reached.
pub fn find_project_db(start: &Path) -> Option<PathBuf> {
    let mut dir = start.to_path_buf();
    loop {
        let candidate = dir.join(".codeanalysis").join("index.db");
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

    /// Directory where model weights are cached
    pub models_dir: PathBuf,

    /// HuggingFace model ID for embeddings
    pub embedding_model: String,

    /// HuggingFace model ID for the LLM (ask command)
    pub llm_model: String,

    /// Default embedding batch size
    pub batch_size: usize,
}

impl Default for Config {
    fn default() -> Self {
        let base = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("codeanalysis");

        Self {
            db_path: base.join("index.db"),
            models_dir: base.join("models"),
            // EmbeddingGemma 300M — bidirectional Gemma 3 encoder, 768-dim output.
            // Requires huggingface-cli login (gated model).
            // Falls back to BAAI/bge-base-en-v1.5 (BERT) if you prefer an ungated model.
            embedding_model: "google/embeddinggemma-300m".to_string(),
            // Gemma 3 1B instruction-tuned. Requires candle-transformers >=0.9
            // which added sliding-window and per-layer RoPE support for Gemma 3.
            // Gemma 3n (e2b/e4b) has a non-transformer architecture not yet
            // implemented in candle — use a standard Gemma 3 model for now.
            // All Gemma models require: huggingface-cli login (accept licence first).
            llm_model: "google/gemma-3-1b-it".to_string(),
            batch_size: 32,
        }
    }
}

impl Config {
    /// Load config from file, falling back to defaults.
    /// If `path` is None, looks for `~/.config/codeanalysis/config.toml`.
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let config_path = match path {
            Some(p) => p.to_path_buf(),
            None => dirs::config_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("codeanalysis")
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
