use anyhow::{Context, Result};

use super::super::{PlanArgs, PlanCommand};
use super::helpers::embed_query;
use super::search::{resolve_project_and_deps, search_all_dbs};
use super::ui::spinner;
use crate::{
    config::{Config, resolve_db},
    storage::open_memory_backend,
};

pub async fn plan(args: PlanArgs, cfg: Config) -> Result<()> {
    match args.command {
        PlanCommand::Create(a) => plan_create(a, args.db.as_ref(), &cfg).await,
        PlanCommand::Status(a) => plan_status(a, &cfg),
    }
}

async fn plan_create(
    args: super::super::PlanCreateArgs,
    explicit_db: Option<&std::path::PathBuf>,
    cfg: &Config,
) -> Result<()> {
    let (db_path, dep_paths) = resolve_project_and_deps(explicit_db, cfg)?;

    // Gather context: search for relevant chunks using the description as query.
    let sp = spinner("Gathering codebase context…");
    let embedder = crate::backends::ActiveEmbedder::load(cfg)
        .await
        .context("loading embedder")?;
    let blob = embed_query(&embedder, "question answering", &args.description).await?;

    let chunks = search_all_dbs(&db_path, &dep_paths, &blob, 15)?;
    sp.finish_and_clear();

    // Gather memory context if available.
    let mem_path = resolve_db(None, &cfg.db_path).with_file_name("memory.db");
    let memory_context = {
        let mblob = blob.clone();
        match open_memory_backend(cfg, &mem_path).ok() {
            Some(b) => b.search(&mblob, 5).await.ok().and_then(|notes| {
                if notes.is_empty() {
                    None
                } else {
                    Some(
                        notes
                            .iter()
                            .map(|n| format!("[{}] {}: {}", n.kind, n.title, n.body))
                            .collect::<Vec<_>>()
                            .join("\n"),
                    )
                }
            }),
            None => None,
        }
    };

    // Build LLM prompt.
    let code_ctx = chunks
        .iter()
        .map(|c| {
            let name = c.name.as_deref().unwrap_or("<anonymous>");
            format!("// {name} ({})\n{}", c.file_path, c.content)
        })
        .collect::<Vec<_>>()
        .join("\n\n---\n\n");

    let memory_section = memory_context
        .map(|m| format!("\n<memory_context>\n{m}\n</memory_context>"))
        .unwrap_or_default();

    let system = "You are a senior software engineer creating an implementation plan as a markdown checklist. \
        Output ONLY the markdown document — no explanations before or after. \
        Structure: a brief summary paragraph, then a checklist of concrete implementation steps using `- [ ]` syntax. \
        Each step should be a single actionable task.";

    let user_msg = format!(
        "<code_context>\n{code_ctx}\n</code_context>{memory_section}\n\n\
        <task>\nCreate an implementation plan for: {desc}\n</task>",
        desc = args.description,
    );

    let sp2 = spinner("Generating plan…");
    let llm = crate::backends::ActiveLlm::load(cfg)
        .await
        .context("loading LLM")?;
    let messages = vec![
        crate::llm::Message::system(system),
        crate::llm::Message::user(user_msg),
    ];
    let (tx, mut rx) = tokio::sync::mpsc::channel::<crate::llm::Token>(128);
    use crate::llm::LlmBackend as _;
    let generate = llm.generate(&messages, 2048, tx, None);
    let collect = async move {
        let mut buf = String::new();
        while let Some(t) = rx.recv().await {
            buf.push_str(&t);
        }
        buf
    };
    let (_, plan_content) =
        tokio::try_join!(generate, async { Ok::<_, anyhow::Error>(collect.await) })?;
    sp2.finish_and_clear();

    // Determine output path under docs/plans/.
    let project_root = {
        let db_parent = db_path.parent().unwrap_or(std::path::Path::new("."));
        // Walk up to find the project root (where .git lives, or fall back).
        let mut p = db_parent;
        loop {
            if p.join(".git").exists() {
                break p.to_path_buf();
            }
            match p.parent() {
                Some(pp) => p = pp,
                None => break db_parent.to_path_buf(),
            }
        }
    };

    let slug = args.name.unwrap_or_else(|| {
        args.description
            .to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '-' })
            .collect::<String>()
            .split('-')
            .filter(|s| !s.is_empty())
            .take(6)
            .collect::<Vec<_>>()
            .join("-")
    });
    let plan_dir = project_root.join(&cfg.plans_dir);
    std::fs::create_dir_all(&plan_dir)?;
    let plan_file = plan_dir.join(format!("{slug}.md"));

    // Prepend a YAML-lite header if the LLM didn't.
    let date = {
        use std::time::{SystemTime, UNIX_EPOCH};
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let days = secs / 86400;
        // Simplified ISO date from epoch (accurate for dates 1970+).
        let mut y = 1970u32;
        let mut remaining = days;
        loop {
            let leap = if y.is_multiple_of(4) && (!y.is_multiple_of(100) || y.is_multiple_of(400)) {
                366
            } else {
                365
            };
            if remaining < leap {
                break;
            }
            remaining -= leap;
            y += 1;
        }
        let month_days = [
            31u32,
            if y.is_multiple_of(4) && (!y.is_multiple_of(100) || y.is_multiple_of(400)) {
                29
            } else {
                28
            },
            31,
            30,
            31,
            30,
            31,
            31,
            30,
            31,
            30,
            31,
        ];
        let mut m = 1u32;
        for md in &month_days {
            if remaining < *md as u64 {
                break;
            }
            remaining -= *md as u64;
            m += 1;
        }
        let d = remaining + 1;
        format!("{y:04}-{m:02}-{d:02}")
    };
    let header = format!(
        "# Plan: {desc}\n\n> Created: {date}\n\n",
        desc = args.description,
    );
    let full_content = if plan_content.trim_start().starts_with('#') {
        plan_content
    } else {
        format!("{header}{plan_content}")
    };

    std::fs::write(&plan_file, &full_content)?;
    println!("Plan written to {}", plan_file.display());
    println!("\nPreview:");
    for line in full_content.lines().take(20) {
        println!("  {line}");
    }
    if full_content.lines().count() > 20 {
        println!("  \x1b[2m…\x1b[0m");
    }

    Ok(())
}

fn plan_status(args: super::super::PlanStatusArgs, cfg: &Config) -> Result<()> {
    use crate::utils::effective_format;
    let fmt = effective_format(&args.format);

    // Find plans dir relative to cwd or git root.
    let plan_dir = {
        let cwd = std::env::current_dir()?;
        let candidate = cwd.join(&cfg.plans_dir);
        if candidate.exists() {
            candidate
        } else {
            // Walk up for git root.
            let mut p = cwd.as_path();
            loop {
                if p.join(".git").exists() {
                    break p.join(&cfg.plans_dir);
                }
                match p.parent() {
                    Some(pp) => p = pp,
                    None => break candidate,
                }
            }
        }
    };

    if !plan_dir.exists() {
        println!(
            "No plans directory found (expected {}).",
            plan_dir.display()
        );
        println!("Create a plan with: spelunk plan create \"<description>\"");
        return Ok(());
    }

    let mut plans: Vec<serde_json::Value> = Vec::new();
    let entries = std::fs::read_dir(&plan_dir)?;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        if let Some(name_filter) = &args.name {
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            if stem != name_filter {
                continue;
            }
        }
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();

        // Count checklist items.
        let total = content
            .lines()
            .filter(|l| l.trim_start().starts_with("- ["))
            .count();
        let done = content
            .lines()
            .filter(|l| l.trim_start().starts_with("- [x]") || l.trim_start().starts_with("- [X]"))
            .count();

        // Extract title from first `# ` line.
        let title = content
            .lines()
            .find(|l| l.starts_with("# "))
            .map(|l| l.trim_start_matches("# ").trim().to_string())
            .unwrap_or_else(|| stem.clone());

        if fmt == "json" {
            plans.push(serde_json::json!({
                "name": stem,
                "title": title,
                "done": done,
                "total": total,
                "file": path.display().to_string(),
            }));
        } else {
            let pct = if total > 0 { done * 100 / total } else { 0 };
            let bar = {
                let filled = pct / 10;
                format!("[{}{}]", "#".repeat(filled), ".".repeat(10 - filled))
            };
            println!("\x1b[1m{title}\x1b[0m  \x1b[2m{stem}\x1b[0m");
            println!("  {bar} {done}/{total} ({pct}%)  {}", path.display());
            println!();
        }
    }

    if fmt == "json" {
        println!("{}", serde_json::to_string_pretty(&plans)?);
    } else if plans.is_empty() && args.name.is_none() {
        println!("No plans found in {}.", plan_dir.display());
    }

    Ok(())
}
