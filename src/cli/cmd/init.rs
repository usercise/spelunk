use anyhow::Result;

use super::super::InitArgs;
use crate::{config::Config, registry::Registry, storage::Database};

pub async fn init(args: InitArgs, cfg: Config) -> Result<()> {
    // ── 1. Detect project root ────────────────────────────────────────────────
    let cwd = std::env::current_dir()?;
    let git_root = find_git_root(&cwd);

    let project_root = match &git_root {
        Some(root) => root.clone(),
        None => {
            eprintln!(
                "Warning: not inside a git repository. Using current directory as project root."
            );
            cwd.clone()
        }
    };

    let project_name = project_root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| project_root.to_string_lossy().into_owned());

    let db_path = project_root.join(".spelunk").join("index.db");

    // ── 2. Check if already initialised ──────────────────────────────────────
    let already_exists = db_path.exists();
    if already_exists {
        println!(
            "Note: spelunk is already initialised for '{}' (DB exists at {}).",
            project_name,
            db_path.display()
        );
        println!("Re-running init is safe — it will update the registry and optionally re-index.");
    }

    // ── 3. Register in global registry ───────────────────────────────────────
    let root_canonical = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.clone());

    if let Ok(reg) = Registry::open() {
        // We register with the expected db_path even if it doesn't exist yet —
        // the index step below will create it.
        let db_canonical = if db_path.exists() {
            db_path.canonicalize().unwrap_or_else(|_| db_path.clone())
        } else {
            db_path.clone()
        };
        if let Err(e) = reg.register(&root_canonical, &db_canonical) {
            eprintln!("Warning: registry update failed: {e}");
        }
    }

    // ── 4. Install hook (if requested) ───────────────────────────────────────
    let hook_status = if args.hook {
        match install_hook_for_init() {
            Ok(msg) => msg,
            Err(e) => format!("failed: {e}"),
        }
    } else {
        "not installed  (run `spelunk hooks install` to add)".to_string()
    };

    // ── 5. Run initial index (unless --no-index) ──────────────────────────────
    let (file_count, chunk_count) = if args.no_index {
        println!("Skipping index (--no-index). Run `spelunk index .` when ready.");
        // If the DB exists already, read its stats; otherwise report zeros.
        if db_path.exists() {
            match Database::open(&db_path) {
                Ok(db) => match db.stats() {
                    Ok(stats) => (stats.file_count, stats.chunk_count),
                    Err(_) => (0, 0),
                },
                Err(_) => (0, 0),
            }
        } else {
            (0, 0)
        }
    } else {
        // Delegate to the real index command logic.
        let index_args = super::super::IndexArgs {
            path: project_root.clone(),
            db: None,
            batch_size: 32,
            force: false,
            recount: false,
            no_summaries: true,
            summary_batch_size: 10,
            background_phases: false,
        };
        super::index::index(index_args, cfg).await?;

        // Read fresh stats from the just-created DB.
        match Database::open(&db_path) {
            Ok(db) => match db.stats() {
                Ok(stats) => (stats.file_count, stats.chunk_count),
                Err(_) => (0, 0),
            },
            Err(_) => (0, 0),
        }
    };

    // ── 6. Print success summary ──────────────────────────────────────────────
    println!();
    println!("spelunk initialised for {}", project_name);
    println!();
    println!("  Index:   {} files, {} chunks", file_count, chunk_count);
    println!("  DB:      {}", db_path.display());
    println!("  Hook:    {}", hook_status);
    println!();
    println!("Next steps:");
    println!("  spelunk search \"your query\"");
    println!("  spelunk ask \"how does X work?\"");

    Ok(())
}

/// Walk up from `start` to find the nearest `.git` directory.
/// Returns the directory containing `.git`, not the `.git` directory itself.
fn find_git_root(start: &std::path::Path) -> Option<std::path::PathBuf> {
    let mut dir = start.to_path_buf();
    loop {
        if dir.join(".git").exists() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Install the git post-commit hook, returning a short status string.
fn install_hook_for_init() -> Result<String> {
    // Re-use the hook installation logic from hooks.rs by calling the same
    // underlying helper used there: replicate it inline to avoid making private
    // functions pub, keeping the hook module self-contained.
    let out = std::process::Command::new("git")
        .args(["rev-parse", "--absolute-git-dir"])
        .output()?;
    if !out.status.success() {
        anyhow::bail!("not inside a git repository");
    }
    let git_dir_str = String::from_utf8(out.stdout)?;
    let git_dir = std::path::PathBuf::from(git_dir_str.trim());
    let hooks_dir = git_dir.join("hooks");
    std::fs::create_dir_all(&hooks_dir)?;
    let hook_path = hooks_dir.join("post-commit");

    if hook_path.exists() {
        let existing = std::fs::read_to_string(&hook_path)?;
        if existing.contains("spelunk post-commit hook") {
            return Ok(format!("already installed at {}", hook_path.display()));
        }
        anyhow::bail!(
            "a post-commit hook already exists at {} and was not installed by spelunk; \
             merge manually or remove it first",
            hook_path.display()
        );
    }

    const POST_COMMIT_HOOK: &str = r#"#!/bin/sh
# spelunk post-commit hook — installed by `spelunk hooks install`
# Keeps the spelunk index in sync and harvests memory from new commits.
# Silently skips if `spelunk` is not in PATH, so teammates without spelunk are unaffected.

if ! command -v spelunk >/dev/null 2>&1; then
  exit 0
fi

PROJECT_ROOT="$(git rev-parse --show-toplevel 2>/dev/null)" || exit 0

spelunk index "$PROJECT_ROOT"
spelunk memory harvest --git-range HEAD~1..HEAD
"#;

    std::fs::write(&hook_path, POST_COMMIT_HOOK)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&hook_path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&hook_path, perms)?;
    }

    Ok(format!("installed at {}", hook_path.display()))
}
