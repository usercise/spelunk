use anyhow::{Context, Result};

use super::super::{HooksArgs, HooksCommand};

pub fn hooks(args: HooksArgs) -> Result<()> {
    match args.command {
        HooksCommand::Install(a) => hooks_install(a),
        HooksCommand::Uninstall => hooks_uninstall(),
    }
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

const CI_STEP: &str = r#"# Add to your .github/workflows/ file:
- name: Update spelunk index
  run: |
    if command -v spelunk >/dev/null 2>&1; then
      spelunk index .
      spelunk memory harvest --git-range HEAD~1..HEAD
    fi
"#;

fn find_git_dir() -> Result<std::path::PathBuf> {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "--absolute-git-dir"])
        .output()
        .context("running git rev-parse --absolute-git-dir (is git installed?)")?;
    if !out.status.success() {
        anyhow::bail!("Not inside a git repository.");
    }
    let path = String::from_utf8(out.stdout).context("git output not UTF-8")?;
    Ok(std::path::PathBuf::from(path.trim()))
}

fn hooks_install(args: super::super::HooksInstallArgs) -> Result<()> {
    if args.ci {
        print!("{CI_STEP}");
        return Ok(());
    }

    let git_dir = find_git_dir()?;
    let hooks_dir = git_dir.join("hooks");
    std::fs::create_dir_all(&hooks_dir)?;
    let hook_path = hooks_dir.join("post-commit");

    if hook_path.exists() {
        let existing = std::fs::read_to_string(&hook_path)?;
        if existing.contains("spelunk post-commit hook") {
            println!("Hook already installed at {}", hook_path.display());
            return Ok(());
        }
        anyhow::bail!(
            "A post-commit hook already exists at {}.\n\
             Inspect it and merge manually, or remove it first.",
            hook_path.display()
        );
    }

    std::fs::write(&hook_path, POST_COMMIT_HOOK)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&hook_path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&hook_path, perms)?;
    }

    println!("Installed post-commit hook at {}", hook_path.display());
    println!("After each commit, ca will:");
    println!("  - Re-index the project");
    println!("  - Harvest memory from the new commit");
    println!("Teammates without spelunk installed are unaffected.");
    Ok(())
}

fn hooks_uninstall() -> Result<()> {
    let git_dir = find_git_dir()?;
    let hook_path = git_dir.join("hooks").join("post-commit");

    if !hook_path.exists() {
        println!("No post-commit hook found.");
        return Ok(());
    }

    let content = std::fs::read_to_string(&hook_path)?;
    if !content.contains("spelunk post-commit hook") {
        anyhow::bail!(
            "The hook at {} was not installed by spelunk. Remove it manually.",
            hook_path.display()
        );
    }

    std::fs::remove_file(&hook_path)?;
    println!("Removed post-commit hook.");
    Ok(())
}
