use anyhow::{Context, Result};

use super::super::{MemoryArgs, MemoryCommand};
use super::status::format_age;

mod add;
mod archive;
mod graph_cmd;
mod harvest;
mod list;
mod push;
mod search;
mod show;
mod supersede;
mod timeline;

pub async fn memory(args: MemoryArgs, cfg: crate::config::Config) -> Result<()> {
    cfg.validate()?;
    let mem_path = args.db.clone().unwrap_or_else(|| {
        crate::config::resolve_db(None, &cfg.db_path).with_file_name("memory.db")
    });
    match args.command {
        MemoryCommand::Add(a) => add::memory_add(a, &mem_path, &cfg).await,
        MemoryCommand::Search(a) => search::memory_search(a, &mem_path, &cfg).await,
        MemoryCommand::List(a) => list::memory_list(a, &mem_path, &cfg).await,
        MemoryCommand::Show(a) => show::memory_show(a, &mem_path, &cfg).await,
        MemoryCommand::Harvest(a) => harvest::memory_harvest(a, &mem_path, &cfg).await,
        MemoryCommand::Archive(a) => archive::memory_archive(a, &mem_path, &cfg).await,
        MemoryCommand::Supersede(a) => supersede::memory_supersede(a, &mem_path, &cfg).await,
        MemoryCommand::Push(a) => push::memory_push(a, &mem_path, &cfg).await,
        MemoryCommand::Timeline(a) => timeline::memory_timeline(a, &mem_path, &cfg).await,
        MemoryCommand::Graph(a) => graph_cmd::memory_graph(a, &mem_path, &cfg).await,
    }
}

// ── Shared display helpers ────────────────────────────────────────────────────

pub(super) fn print_note_summary(n: &crate::storage::memory::Note) {
    let dist = if let Some(s) = n.score {
        format!("  score: {s:.4}")
    } else {
        n.distance
            .map(|d| format!("  dist: {d:.4}"))
            .unwrap_or_default()
    };
    let archived_badge = if n.status == "archived" {
        " \x1b[31m[archived]\x1b[0m"
    } else {
        ""
    };
    println!(
        "\x1b[1m#{id}\x1b[0m  \x1b[33m[{kind}]\x1b[0m  {title}{archived}{dist_fmt}",
        id = n.id,
        kind = n.kind,
        title = n.title,
        archived = archived_badge,
        dist_fmt = if dist.is_empty() {
            String::new()
        } else {
            format!("\x1b[2m{dist}\x1b[0m")
        },
    );
    println!("     \x1b[2m{}\x1b[0m", format_age(n.created_at));
    if let Some(valid_at) = n.valid_at {
        println!("     \x1b[2mvalid_at: {}\x1b[0m", format_age(valid_at));
    }
    if !n.tags.is_empty() {
        println!("     tags: {}", n.tags.join(", "));
    }
    if !n.linked_files.is_empty() {
        println!("     files: {}", n.linked_files.join(", "));
    }
    if let Some(sup) = n.superseded_by {
        println!("     \x1b[2msuperseded by #{sup}\x1b[0m");
    }
    if !matches!(n.kind.as_str(), "question" | "answer") {
        let preview: Vec<&str> = n.body.lines().take(2).collect();
        for line in &preview {
            println!("     \x1b[2m{line}\x1b[0m");
        }
        if n.body.lines().count() > 2 {
            println!("     \x1b[2m…\x1b[0m");
        }
    } else {
        println!(
            "     \x1b[2m(use `spelunk memory show {}` to read body)\x1b[0m",
            n.id
        );
    }
    println!();
}

/// Open $EDITOR (or $VISUAL, then vi) for the user to write a memory body.
pub(super) fn open_editor_for_body(title: &str) -> Result<String> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());

    let tmp = std::env::temp_dir().join(format!("ca_memory_{}.md", std::process::id()));
    std::fs::write(
        &tmp,
        format!(
            "# {title}\n\n\
         # Write your memory entry below. Lines starting with # are ignored.\n\
         # Save and close the editor when done.\n\n"
        ),
    )?;

    let status = std::process::Command::new(&editor)
        .arg(&tmp)
        .status()
        .with_context(|| format!("could not open editor '{editor}'"))?;

    let content = std::fs::read_to_string(&tmp)?;
    std::fs::remove_file(&tmp).ok();

    if !status.success() {
        anyhow::bail!("Editor exited with a non-zero status; entry not saved.");
    }

    let body: String = content
        .lines()
        .filter(|l| !l.starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();

    if body.is_empty() {
        anyhow::bail!("Body is empty; entry not saved.");
    }
    Ok(body)
}

// Re-export from the shared dates module for use within this submodule tree.
pub(super) use crate::utils::dates::parse_as_of;
