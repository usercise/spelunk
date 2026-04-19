use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use std::path::PathBuf;

#[derive(Args, Debug)]
pub struct SpecArgs {
    #[command(subcommand)]
    pub command: SpecCommand,

    /// Path to the SQLite database (overrides auto-detect)
    #[arg(long, global = true)]
    pub db: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
pub enum SpecCommand {
    /// Link a spec file to one or more code paths it governs
    Link(SpecLinkArgs),
    /// Remove a link between a spec and a code path
    Unlink(SpecUnlinkArgs),
    /// List all registered spec files and their links
    List(SpecListArgs),
    /// Show specs whose linked code has been re-indexed since the spec was last indexed
    Check(SpecCheckArgs),
}

#[derive(Args, Debug)]
pub struct SpecLinkArgs {
    /// Path to the spec file (markdown)
    pub spec: PathBuf,

    /// One or more file paths or directory prefixes this spec governs
    #[arg(required = true)]
    pub paths: Vec<String>,
}

#[derive(Args, Debug)]
pub struct SpecUnlinkArgs {
    /// Path to the spec file
    pub spec: PathBuf,

    /// Path prefix to remove (leave empty to remove all links for this spec)
    pub path: Option<String>,
}

#[derive(Args, Debug)]
pub struct SpecListArgs {
    /// Output format: text or json
    #[arg(long, default_value = "text")]
    pub format: String,
}

#[derive(Args, Debug)]
pub struct SpecCheckArgs {
    /// Output format: text or json
    #[arg(long, default_value = "text")]
    pub format: String,
}

use crate::{
    config::{Config, resolve_db},
    storage::Database,
};

pub fn spec(args: SpecArgs, cfg: Config) -> Result<()> {
    let db_path = resolve_db(args.db.as_deref(), &cfg.db_path);
    let db = Database::open(&db_path)?;
    match args.command {
        SpecCommand::Link(a) => spec_link(a, &db),
        SpecCommand::Unlink(a) => spec_unlink(a, &db),
        SpecCommand::List(a) => spec_list(a, &db),
        SpecCommand::Check(a) => spec_check(a, &db),
    }
}

fn spec_link(args: SpecLinkArgs, db: &Database) -> Result<()> {
    let spec_path = args
        .spec
        .to_str()
        .context("spec path is not valid UTF-8")?
        .to_owned();

    let title = extract_spec_title(&args.spec).unwrap_or_default();
    let spec_id = db.upsert_spec(&spec_path, &title, false)?;

    for path in &args.paths {
        db.add_spec_link(spec_id, path)?;
        println!("Linked  {spec_path}  →  {path}");
    }
    Ok(())
}

fn spec_unlink(args: SpecUnlinkArgs, db: &Database) -> Result<()> {
    let spec_path = args
        .spec
        .to_str()
        .context("spec path is not valid UTF-8")?
        .to_owned();

    let record = db
        .spec_by_path(&spec_path)?
        .with_context(|| format!("spec not found: {spec_path}"))?;

    match args.path {
        Some(p) => {
            db.remove_spec_link(record.id, &p)?;
            println!("Unlinked  {spec_path}  →  {p}");
        }
        None => {
            db.delete_spec(record.id)?;
            println!("Removed spec (and all links): {spec_path}");
        }
    }
    Ok(())
}

fn spec_list(args: SpecListArgs, db: &Database) -> Result<()> {
    let specs = db.all_specs()?;

    if specs.is_empty() {
        println!(
            "No specs registered. Run `spelunk spec link` or re-index a project with docs/specs/."
        );
        return Ok(());
    }

    #[derive(serde::Serialize)]
    struct SpecOut<'a> {
        path: &'a str,
        title: &'a str,
        is_auto: bool,
        links: Vec<String>,
    }

    let mut out: Vec<SpecOut<'_>> = Vec::with_capacity(specs.len());
    for s in &specs {
        let links = db.spec_links(s.id)?;
        out.push(SpecOut {
            path: &s.path,
            title: &s.title,
            is_auto: s.is_auto,
            links,
        });
    }

    match crate::utils::effective_format(&args.format) {
        "json" => println!("{}", serde_json::to_string_pretty(&out)?),
        _ => {
            for s in &out {
                let auto = if s.is_auto {
                    " \x1b[2m(auto)\x1b[0m"
                } else {
                    ""
                };
                let title = if s.title.is_empty() {
                    String::new()
                } else {
                    format!(" — {}", s.title)
                };
                println!("\x1b[1m{}\x1b[0m{}{}", s.path, title, auto);
                if s.links.is_empty() {
                    println!("  \x1b[2m(no links)\x1b[0m");
                }
                for l in &s.links {
                    println!("  → {l}");
                }
                println!();
            }
        }
    }
    Ok(())
}

fn spec_check(args: SpecCheckArgs, db: &Database) -> Result<()> {
    let stale = db.stale_specs()?;

    if stale.is_empty() {
        println!("All specs are up to date with their linked code.");
        return Ok(());
    }

    match crate::utils::effective_format(&args.format) {
        "json" => println!("{}", serde_json::to_string_pretty(&stale)?),
        _ => {
            println!(
                "\x1b[33mWarning:\x1b[0m {} spec(s) may be out of date:\n",
                stale.len()
            );
            let mut last_spec = "";
            for s in &stale {
                if s.spec_path != last_spec {
                    let title = if s.title.is_empty() {
                        String::new()
                    } else {
                        format!(" — {}", s.title)
                    };
                    println!("\x1b[1m{}\x1b[0m{}", s.spec_path, title);
                    last_spec = &s.spec_path;
                }
                let days = (s.code_indexed_at - s.spec_indexed_at) / 86400;
                println!(
                    "  → {} \x1b[2m(code re-indexed ~{} day(s) after spec)\x1b[0m",
                    s.linked_path, days
                );
            }
        }
    }
    Ok(())
}

/// Extract the first ATX heading (`# Title`) from a markdown file as a title string.
pub(super) fn extract_spec_title(path: &std::path::Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    content.lines().find_map(|line| {
        let stripped = line.strip_prefix("# ")?;
        let title = stripped.trim().to_owned();
        if title.is_empty() { None } else { Some(title) }
    })
}

/// Return true if a markdown file declares itself as a spec via frontmatter
/// (`spelunk_spec: true`) or lives under a conventional spec directory.
///
/// `specs_dir` is the configured specs directory (e.g. `docs/specs`).
/// In addition to `specs_dir`, the legacy `specs/` prefix is always matched
/// so that projects that haven't configured a custom value still work.
pub fn is_spec_file(path: &std::path::Path, specs_dir: &std::path::Path) -> bool {
    let path_str = path.to_string_lossy();
    // Check configured specs_dir first (both as prefix and as interior component).
    if path.starts_with(specs_dir) {
        return true;
    }
    let specs_dir_str = specs_dir.to_string_lossy();
    if path_str.contains(&format!("/{}/", specs_dir_str))
        || path_str.starts_with(&format!("{}/", specs_dir_str))
    {
        return true;
    }
    // Legacy fallback: bare `specs/` prefix (matches projects that have no config).
    if path_str.contains("/specs/") || path_str.starts_with("specs/") {
        return true;
    }

    if let Ok(content) = std::fs::read_to_string(path) {
        let mut in_frontmatter = false;
        for (i, line) in content.lines().enumerate() {
            if i == 0 && line.trim_end() == "---" {
                in_frontmatter = true;
                continue;
            }
            if in_frontmatter {
                if line.trim_end() == "---" || line.trim_end() == "..." {
                    break;
                }
                if line.trim() == "spelunk_spec: true" {
                    return true;
                }
            }
            if i >= 20 {
                break;
            }
        }
    }
    false
}
