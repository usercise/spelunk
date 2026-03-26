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

/// Walk up from `start` looking for `.spelunk/config.toml` (project-level config).
/// Stops at the filesystem root. Returns the path if found.
fn find_project_config(start: &Path) -> Option<PathBuf> {
    let mut dir = start.to_path_buf();
    loop {
        let candidate = dir.join(".spelunk").join("config.toml");
        if candidate.exists() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Fields that can be set in `.spelunk/config.toml` (project-level, checked-in).
/// Only contains fields safe to share with the team (no secrets).
#[derive(Debug, Default, Deserialize)]
struct ProjectConfig {
    memory_server_url: Option<String>,
    /// Shared API key — acceptable if the server is behind a VPN/firewall.
    /// For secrets, prefer `SPELUNK_SERVER_KEY` env var instead.
    memory_server_key: Option<String>,
    project_id: Option<String>,
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

    // ── Shared memory server (optional) ──────────────────────────────────────

    /// URL of the spelunk-server instance, e.g. `http://spelunk.internal:7777`.
    /// Set in `.spelunk/config.toml` (project-level) or via `SPELUNK_SERVER_URL`.
    /// If unset, memory is stored locally in memory.db.
    #[serde(default)]
    pub memory_server_url: Option<String>,

    /// Bearer token for spelunk-server auth.
    /// Set in `~/.config/spelunk/config.toml` (personal) or via `SPELUNK_SERVER_KEY`.
    /// Do NOT commit this to `.spelunk/config.toml`.
    #[serde(default)]
    pub memory_server_key: Option<String>,

    /// Project slug for the shared memory server (e.g. `my-awesome-app`).
    /// Required when `memory_server_url` is set.
    /// Set in `.spelunk/config.toml` (project-level) or via `SPELUNK_PROJECT_ID`.
    #[serde(default)]
    pub project_id: Option<String>,
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
            memory_server_url: None,
            memory_server_key: None,
            project_id: None,
        }
    }
}

impl Config {
    /// Load config with layered overrides:
    ///   1. Defaults
    ///   2. `~/.config/spelunk/config.toml` (global personal)
    ///   3. `.spelunk/config.toml` discovered by walking up from CWD (project-level, team-wide)
    ///   4. Environment variables: `SPELUNK_SERVER_URL`, `SPELUNK_SERVER_KEY`, `SPELUNK_PROJECT_ID`
    ///
    /// Pass `path` to override the global config location (used by `--config` flag).
    pub fn load(path: Option<&Path>) -> Result<Self> {
        // ── 1. Load global personal config ───────────────────────────────────
        let global_path = match path {
            Some(p) => p.to_path_buf(),
            None => dirs::config_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("spelunk")
                .join("config.toml"),
        };
        let mut cfg: Config = if global_path.exists() {
            let raw = std::fs::read_to_string(&global_path)
                .with_context(|| format!("reading config at {}", global_path.display()))?;
            toml::from_str(&raw).context("parsing config.toml")?
        } else {
            Config::default()
        };

        // ── 2. Merge project-level config (.spelunk/config.toml) ─────────────
        if let Ok(cwd) = std::env::current_dir() {
            if let Some(proj_path) = find_project_config(&cwd) {
                let raw = std::fs::read_to_string(&proj_path)
                    .with_context(|| format!("reading project config at {}", proj_path.display()))?;
                let proj: ProjectConfig = toml::from_str(&raw)
                    .context("parsing .spelunk/config.toml")?;
                if let Some(v) = proj.memory_server_url { cfg.memory_server_url = Some(v); }
                if let Some(v) = proj.memory_server_key { cfg.memory_server_key = Some(v); }
                if let Some(v) = proj.project_id        { cfg.project_id = Some(v); }
            }
        }

        // ── 3. Environment variable overrides ────────────────────────────────
        if let Ok(v) = std::env::var("SPELUNK_SERVER_URL")  { cfg.memory_server_url = Some(v); }
        if let Ok(v) = std::env::var("SPELUNK_SERVER_KEY")  { cfg.memory_server_key = Some(v); }
        if let Ok(v) = std::env::var("SPELUNK_PROJECT_ID")  { cfg.project_id = Some(v); }

        Ok(cfg)
    }

    /// Validate cross-field constraints. Call after `load()`.
    pub fn validate(&self) -> Result<()> {
        if self.memory_server_url.is_some() && self.project_id.is_none() {
            anyhow::bail!(
                "memory_server_url is set but project_id is missing.\n\
                 Add `project_id = \"my-project\"` to .spelunk/config.toml \
                 or set SPELUNK_PROJECT_ID."
            );
        }
        Ok(())
    }
}
