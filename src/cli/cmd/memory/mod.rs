use anyhow::{Context, Result};

use super::super::{MemoryArgs, MemoryCommand};
use super::status::format_age;

mod add;
mod graph_cmd;
mod harvest;
mod list;
mod push;
mod search;
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
        MemoryCommand::Show(a) => list::memory_show(a, &mem_path, &cfg).await,
        MemoryCommand::Harvest(a) => harvest::memory_harvest(a, &mem_path, &cfg).await,
        MemoryCommand::Archive(a) => push::memory_archive(a, &mem_path, &cfg).await,
        MemoryCommand::Supersede(a) => push::memory_supersede(a, &mem_path, &cfg).await,
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

// ── Date/time helpers ─────────────────────────────────────────────────────────

/// Parse an optional `--as-of` argument into a Unix timestamp.
pub(super) fn parse_as_of(s: Option<&str>) -> Result<Option<i64>> {
    match s {
        None => Ok(None),
        Some(v) => parse_iso8601_to_epoch(v)
            .with_context(|| {
                format!(
                    "parsing --as-of '{v}': expected ISO 8601 (e.g. 2026-03-15 or 2026-03-15T10:00:00)"
                )
            })
            .map(Some),
    }
}

/// Parse an ISO 8601 date or datetime string to a unix epoch (seconds, UTC).
fn parse_iso8601_to_epoch(s: &str) -> Result<i64> {
    let s = s.trim_end_matches('Z');

    let (date_part, time_part) =
        if let Some((d, t)) = s.split_once('T').or_else(|| s.split_once(' ')) {
            (d, Some(t))
        } else {
            (s, None)
        };

    let parse_u32 = |v: &str, label: &str| -> Result<u32> {
        v.parse::<u32>()
            .with_context(|| format!("invalid {label} in '{s}'"))
    };

    let date_parts: Vec<&str> = date_part.split('-').collect();
    if date_parts.len() != 3 {
        anyhow::bail!("expected YYYY-MM-DD in '{s}'");
    }
    let year = parse_u32(date_parts[0], "year")?;
    let month = parse_u32(date_parts[1], "month")?;
    let day = parse_u32(date_parts[2], "day")?;

    let (hour, minute, second) = if let Some(t) = time_part {
        let parts: Vec<&str> = t.split(':').collect();
        if parts.len() < 2 {
            anyhow::bail!("expected HH:MM or HH:MM:SS in '{s}'");
        }
        let h = parse_u32(parts[0], "hour")?;
        let m = parse_u32(parts[1], "minute")?;
        let sec = if parts.len() >= 3 {
            parse_u32(parts[2], "second")?
        } else {
            0
        };
        (h, m, sec)
    } else {
        (0, 0, 0)
    };

    if month == 0 || month > 12 {
        anyhow::bail!("month out of range in '{s}'");
    }
    if day == 0 || day > 31 {
        anyhow::bail!("day out of range in '{s}'");
    }
    if hour > 23 || minute > 59 || second > 59 {
        anyhow::bail!("time out of range in '{s}'");
    }

    let y = year as i64;
    let m = month as i64;
    let d = day as i64;
    let days = days_since_epoch(y, m, d)?;
    let epoch = days * 86400 + (hour as i64) * 3600 + (minute as i64) * 60 + (second as i64);
    Ok(epoch)
}

fn days_since_epoch(year: i64, month: i64, day: i64) -> Result<i64> {
    let y = if month <= 2 { year - 1 } else { year };
    let m = if month <= 2 { month + 9 } else { month - 3 };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * m + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let rd = era * 146097 + doe;
    Ok(rd - 719_468)
}
