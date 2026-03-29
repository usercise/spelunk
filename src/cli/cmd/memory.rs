use anyhow::{Context, Result};

use super::super::{
    MemoryAddArgs, MemoryArchiveArgs, MemoryArgs, MemoryCommand, MemoryHarvestArgs, MemoryListArgs,
    MemoryPushArgs, MemorySearchArgs, MemoryShowArgs, MemorySupersededArgs,
};
use super::helpers::embed_query;
use super::status::format_age;
use super::ui::spinner;
use crate::{
    config::{Config, resolve_db},
    embeddings::{EmbeddingBackend as _, vec_to_blob},
    storage::{NoteInput, open_memory_backend},
};

pub async fn memory(args: MemoryArgs, cfg: Config) -> Result<()> {
    cfg.validate()?;
    let mem_path = args
        .db
        .clone()
        .unwrap_or_else(|| resolve_db(None, &cfg.db_path).with_file_name("memory.db"));
    match args.command {
        MemoryCommand::Add(a) => memory_add(a, &mem_path, &cfg).await,
        MemoryCommand::Search(a) => memory_search(a, &mem_path, &cfg).await,
        MemoryCommand::List(a) => memory_list(a, &mem_path, &cfg).await,
        MemoryCommand::Show(a) => memory_show(a, &mem_path, &cfg).await,
        MemoryCommand::Harvest(a) => memory_harvest(a, &mem_path, &cfg).await,
        MemoryCommand::Archive(a) => memory_archive(a, &mem_path, &cfg).await,
        MemoryCommand::Supersede(a) => memory_supersede(a, &mem_path, &cfg).await,
        MemoryCommand::Push(a) => memory_push(a, &mem_path, &cfg).await,
    }
}

async fn memory_add(args: MemoryAddArgs, mem_path: &std::path::Path, cfg: &Config) -> Result<()> {
    // Resolve title and body: from URL, explicit args, or editor.
    let (title, body) = if let Some(url) = &args.from_url {
        let (fetched_title, fetched_body) = fetch_url_content(url)
            .await
            .with_context(|| format!("fetching {url}"))?;
        let title = args.title.clone().unwrap_or(fetched_title);
        let body = args.body.clone().unwrap_or(fetched_body);
        (title, body)
    } else {
        let title = args
            .title
            .clone()
            .context("--title is required when --from-url is not provided")?;
        let body = match args.body.clone() {
            Some(b) => b,
            None => {
                let t = title.clone();
                tokio::task::spawn_blocking(move || open_editor_for_body(&t))
                    .await
                    .context("editor task panicked")?
                    .context("opening editor for body")?
            }
        };
        (title, body)
    };

    let tags: Vec<String> = args
        .tags
        .as_deref()
        .map(|s| s.split(',').map(|t| t.trim().to_string()).collect())
        .unwrap_or_default();

    let files: Vec<String> = args
        .files
        .as_deref()
        .map(|s| s.split(',').map(|f| f.trim().to_string()).collect())
        .unwrap_or_default();

    // Embed first so the vector is ready when we call the backend.
    let embed_text = format!("title: {title} | text: {body}");
    let sp = spinner("Embedding…");
    let embedder = crate::backends::ActiveEmbedder::load(cfg)
        .await
        .context("loading embedding model")?;
    let vecs = embedder.embed(&[&embed_text]).await?;
    sp.finish_and_clear();
    let embedding = vecs.first().map(|v| vec_to_blob(v));

    let backend = open_memory_backend(cfg, mem_path)?;
    let note_id = backend
        .add(NoteInput {
            kind: args.kind.clone(),
            title: title.clone(),
            body: body.clone(),
            tags,
            linked_files: files,
            embedding,
        })
        .await?;

    println!(
        "Stored [{kind}] #{id}: {title}",
        kind = args.kind,
        id = note_id
    );
    Ok(())
}

/// Fetch content from a URL.
///
/// Priority:
///   1. GitHub issue URLs → `gh api` for structured title + body (Markdown)
///   2. `~/scripts/web-to-md.ts` via bun → Readability + Turndown (clean Markdown)
///   3. Fallback: raw HTTP GET + naive HTML stripping
async fn fetch_url_content(url: &str) -> Result<(String, String)> {
    // ── 1. GitHub issue ───────────────────────────────────────────────────────
    let gh_issue_re =
        regex::Regex::new(r"https?://github\.com/([^/]+)/([^/]+)/issues/(\d+)").unwrap();

    if let Some(caps) = gh_issue_re.captures(url) {
        let owner = &caps[1];
        let repo = &caps[2];
        let num = &caps[3];
        let api_path = format!("repos/{owner}/{repo}/issues/{num}");
        let out = tokio::process::Command::new("gh")
            .args(["api", &api_path])
            .output()
            .await;
        if let Ok(out) = out
            && out.status.success()
        {
            let json: serde_json::Value =
                serde_json::from_slice(&out.stdout).context("parsing gh api response")?;
            let title = json["title"].as_str().unwrap_or("GitHub Issue").to_string();
            let body = json["body"].as_str().unwrap_or("").to_string();
            return Ok((title, body));
        }
        // gh missing or not authenticated — fall through
    }

    // ── 2. web-to-md.ts via bun ───────────────────────────────────────────────
    // The script outputs:  # <title>\n\n<markdown body>
    // Expand ~ manually so we don't rely on shell expansion.
    let script = dirs::home_dir()
        .map(|h| h.join("scripts/web-to-md.ts"))
        .filter(|p| p.exists());

    if let Some(script_path) = script {
        let out = tokio::process::Command::new("bun")
            .arg(&script_path)
            .arg(url)
            .output()
            .await;
        if let Ok(out) = out
            && out.status.success()
        {
            let md = String::from_utf8_lossy(&out.stdout);
            return parse_web_to_md_output(&md, url);
        }
        // bun missing or script errored — fall through
    }

    // ── 3. Fallback: raw HTTP + naive stripping ───────────────────────────────
    let client = reqwest::Client::builder()
        .user_agent("spelunk/0.1")
        .build()?;
    let html = client.get(url).send().await?.text().await?;

    let title_re = regex::Regex::new(r"(?i)<title[^>]*>([\s\S]*?)</title>").unwrap();
    let title = title_re
        .captures(&html)
        .and_then(|c| c.get(1))
        .map(|m| html_unescape(m.as_str().trim()))
        .unwrap_or_else(|| url.to_string());

    let no_script =
        regex::Regex::new(r"(?is)<(?:script|style)[^>]*>[\s\S]*?</(?:script|style)>").unwrap();
    let no_tags = regex::Regex::new(r"<[^>]+>").unwrap();
    let ws = regex::Regex::new(r"\s{3,}").unwrap();
    let stripped = no_script.replace_all(&html, " ");
    let stripped = no_tags.replace_all(&stripped, " ");
    let body = ws.replace_all(stripped.trim(), "\n\n").to_string();
    let body = if body.len() > 8192 {
        body[..8192].to_string()
    } else {
        body
    };

    Ok((title, body))
}

/// Parse the `# Title\n\n<body>` output produced by web-to-md.ts.
fn parse_web_to_md_output(md: &str, url: &str) -> Result<(String, String)> {
    let md = md.trim();
    if let Some(rest) = md.strip_prefix("# ") {
        let (title_line, body) = rest.split_once('\n').unwrap_or((rest, ""));
        Ok((title_line.trim().to_string(), body.trim_start().to_string()))
    } else {
        // Unexpected format — use the whole thing as body, URL as title
        Ok((url.to_string(), md.to_string()))
    }
}

fn html_unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

async fn memory_search(
    args: MemorySearchArgs,
    mem_path: &std::path::Path,
    cfg: &Config,
) -> Result<()> {
    let index_db_path = crate::config::resolve_db(None, &cfg.db_path);
    crate::storage::record_usage_at(&index_db_path, "memory search");

    let sp = spinner("Embedding query…");
    let embedder = crate::backends::ActiveEmbedder::load(cfg)
        .await
        .context("loading embedding model")?;
    let blob = embed_query(&embedder, "question answering", &args.query).await?;
    sp.finish_and_clear();

    let backend = open_memory_backend(cfg, mem_path)?;
    let notes = backend.search(&blob, args.limit).await?;

    if notes.is_empty() {
        println!("No memory entries found.");
        return Ok(());
    }

    match crate::utils::effective_format(&args.format) {
        "json" => println!("{}", serde_json::to_string_pretty(&notes)?),
        _ => {
            for n in &notes {
                print_note_summary(n);
            }
        }
    }
    Ok(())
}

async fn memory_list(args: MemoryListArgs, mem_path: &std::path::Path, cfg: &Config) -> Result<()> {
    let backend = open_memory_backend(cfg, mem_path)?;
    let notes = backend
        .list(args.kind.as_deref(), args.limit, args.archived)
        .await?;

    if notes.is_empty() {
        println!("No memory entries found.");
        return Ok(());
    }

    match crate::utils::effective_format(&args.format) {
        "json" => println!("{}", serde_json::to_string_pretty(&notes)?),
        _ => {
            for n in &notes {
                print_note_summary(n);
            }
        }
    }
    Ok(())
}

async fn memory_show(args: MemoryShowArgs, mem_path: &std::path::Path, cfg: &Config) -> Result<()> {
    let backend = open_memory_backend(cfg, mem_path)?;
    match backend.get(args.id).await? {
        None => anyhow::bail!("No memory entry with id {}.", args.id),
        Some(n) => match crate::utils::effective_format(&args.format) {
            "json" => println!("{}", serde_json::to_string_pretty(&n)?),
            _ => {
                println!("\x1b[1m#{} [{}] {}\x1b[0m", n.id, n.kind, n.title);
                println!("\x1b[2m{}\x1b[0m", format_age(n.created_at));
                if !n.tags.is_empty() {
                    println!("tags: {}", n.tags.join(", "));
                }
                if !n.linked_files.is_empty() {
                    println!("files: {}", n.linked_files.join(", "));
                }
                println!();
                println!("{}", n.body);
            }
        },
    }
    Ok(())
}

fn print_note_summary(n: &crate::storage::memory::Note) {
    let dist = n
        .distance
        .map(|d| format!("  dist: {d:.4}"))
        .unwrap_or_default();
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
    if !n.tags.is_empty() {
        println!("     tags: {}", n.tags.join(", "));
    }
    if !n.linked_files.is_empty() {
        println!("     files: {}", n.linked_files.join(", "));
    }
    if let Some(sup) = n.superseded_by {
        println!("     \x1b[2msuperseded by #{sup}\x1b[0m");
    }
    // For question/answer kinds: titles-only list — use `spelunk memory show <id>` for body.
    // For other kinds: show first 2 lines of body as preview.
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
fn open_editor_for_body(title: &str) -> Result<String> {
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

async fn memory_push(args: MemoryPushArgs, mem_path: &std::path::Path, cfg: &Config) -> Result<()> {
    if cfg.memory_server_url.is_none() {
        anyhow::bail!(
            "memory_server_url is not configured.\n\
             Set it in .spelunk/config.toml or via SPELUNK_SERVER_URL."
        );
    }

    let src_path = args.source.as_deref().unwrap_or(mem_path);
    let local = crate::storage::MemoryStore::open(src_path)
        .with_context(|| format!("opening local memory at {}", src_path.display()))?;

    let notes = local.list(None, 10_000, args.include_archived)?;
    if notes.is_empty() {
        println!("No local memory entries to push.");
        return Ok(());
    }

    let remote = open_memory_backend(cfg, mem_path)?;

    println!(
        "Pushing {} entries to {}…",
        notes.len(),
        cfg.memory_server_url.as_deref().unwrap_or("?")
    );
    let mut pushed = 0usize;
    let mut skipped = 0usize;

    // Read local embeddings from the DB for each note.
    for note in &notes {
        // Fetch the raw embedding blob from local store.
        let blob = local.get_embedding(note.id)?;
        let result = remote
            .add(NoteInput {
                kind: note.kind.clone(),
                title: note.title.clone(),
                body: note.body.clone(),
                tags: note.tags.clone(),
                linked_files: note.linked_files.clone(),
                embedding: blob,
            })
            .await;
        match result {
            Ok(_) => {
                pushed += 1;
            }
            Err(e) => {
                eprintln!("  [skip] #{}: {e}", note.id);
                skipped += 1;
            }
        }
    }
    println!("Done. Pushed: {pushed}, skipped: {skipped}.");
    Ok(())
}

async fn memory_archive(
    args: MemoryArchiveArgs,
    mem_path: &std::path::Path,
    cfg: &Config,
) -> Result<()> {
    let backend = open_memory_backend(cfg, mem_path)?;
    if backend.archive(args.id).await? {
        println!("Archived memory entry #{}.", args.id);
    } else {
        anyhow::bail!("No active memory entry with id {}.", args.id);
    }
    Ok(())
}

async fn memory_supersede(
    args: MemorySupersededArgs,
    mem_path: &std::path::Path,
    cfg: &Config,
) -> Result<()> {
    let backend = open_memory_backend(cfg, mem_path)?;
    // Verify the new entry exists.
    if backend.get(args.new_id).await?.is_none() {
        anyhow::bail!("No memory entry with id {} (new).", args.new_id);
    }
    if backend.supersede(args.old_id, args.new_id).await? {
        println!(
            "Archived #{old} → superseded by #{new}.",
            old = args.old_id,
            new = args.new_id
        );
    } else {
        anyhow::bail!("No active memory entry with id {} (old).", args.old_id);
    }
    Ok(())
}

async fn memory_harvest(
    args: MemoryHarvestArgs,
    mem_path: &std::path::Path,
    cfg: &Config,
) -> Result<()> {
    use crate::llm::LlmBackend;

    // ── Step 1: collect commits via git log ───────────────────────────────────
    // --branch passes the branch name directly to `git log <branch>` which
    // traverses the full history. --git-range uses git's A..B range syntax.
    let (git_ref, range_label) = match &args.branch {
        Some(branch) => (branch.clone(), format!("full history of '{branch}'")),
        None => (args.git_range.clone(), format!("'{}'", args.git_range)),
    };

    let git_out = std::process::Command::new("git")
        .args(["log", &git_ref, "--format=%H%x00%s%x00%b%x00---"])
        .output()
        .context("running git log (is git installed and are we in a git repo?)")?;

    if !git_out.status.success() {
        let msg = String::from_utf8_lossy(&git_out.stderr);
        anyhow::bail!("git log failed: {msg}");
    }

    let raw = String::from_utf8(git_out.stdout).context("git log output not UTF-8")?;
    let commits: Vec<(String, String, String)> = raw
        .split("---\n")
        .filter(|s| !s.trim().is_empty())
        .filter_map(|entry| {
            let parts: Vec<&str> = entry.splitn(4, '\x00').collect();
            if parts.len() < 3 {
                return None;
            }
            Some((
                parts[0].trim().to_string(),
                parts[1].trim().to_string(),
                parts[2].trim().to_string(),
            ))
        })
        .collect();

    if commits.is_empty() {
        println!("No commits found in {range_label}.");
        return Ok(());
    }

    // ── Step 2: skip already-harvested SHAs ──────────────────────────────────
    let backend = open_memory_backend(cfg, mem_path)?;
    let known_shas = backend.harvested_shas().await?;
    let new_commits: Vec<_> = commits
        .iter()
        .filter(|(sha, _, _)| !known_shas.contains(sha))
        .collect();

    if new_commits.is_empty() {
        println!("All {} commits already harvested.", commits.len());
        return Ok(());
    }

    let batch_size = args.batch_size.max(1);
    let total = new_commits.len();
    let num_batches = total.div_ceil(batch_size);
    println!(
        "Analysing {} new commit(s) in '{}' ({} batch(es) of up to {})…",
        total, range_label, num_batches, batch_size
    );

    // ── Step 3: load LLM + embedder once, then process commits in batches ────
    let system = "You help build a project memory store from git history. \
        Respond ONLY with valid JSON matching the provided schema. No other text.";

    let schema = serde_json::json!({
        "name": "harvest_result",
        "strict": true,
        "schema": {
            "type": "object",
            "properties": {
                "entries": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "sha": {"type": "string"},
                            "kind": {"type": "string", "enum": ["decision","context","requirement","note"]},
                            "title": {"type": "string"},
                            "body": {"type": "string"},
                            "tags": {"type": "array", "items": {"type": "string"}}
                        },
                        "required": ["sha","kind","title","body","tags"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["entries"],
            "additionalProperties": false
        }
    });

    let sp = spinner("Loading LLM for harvest…");
    let llm = crate::backends::ActiveLlm::load(cfg)
        .await
        .context("loading LLM")?;
    sp.finish_and_clear();

    let embedder = crate::backends::ActiveEmbedder::load(cfg)
        .await
        .context("loading embedding model")?;

    let mut stored = 0usize;

    // ── Step 4: process each batch ────────────────────────────────────────────
    for (batch_idx, batch) in new_commits.chunks(batch_size).enumerate() {
        if num_batches > 1 {
            println!(
                "\nBatch {}/{} ({} commits)…",
                batch_idx + 1,
                num_batches,
                batch.len()
            );
        }

        let commit_list = batch
            .iter()
            .map(|(sha, subject, body)| {
                if body.is_empty() {
                    format!("COMMIT {sha}\n{subject}")
                } else {
                    format!("COMMIT {sha}\n{subject}\n\n{body}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n\n---\n\n");

        let user = format!(
            "Review these git commit messages. Identify commits that represent:\n\
             - \"decision\": A significant architectural or design choice and reasoning\n\
             - \"context\": Background about requirements, constraints, or project goals\n\
             - \"requirement\": A hard constraint the codebase must satisfy\n\
             - \"note\": A surprising or non-obvious discovery\n\n\
             Skip routine commits (version bumps, typo fixes, dependency updates \
             unless they reveal a constraint, formatting).\n\n\
             For each significant commit write: sha (first 8 chars), kind, title \
             (one sentence, past tense for decisions), body (include why, \
             what alternatives were considered), tags (2-4 keywords).\n\n\
             Commits:\n{commit_list}"
        );

        let messages = vec![
            crate::llm::Message::system(system),
            crate::llm::Message::user(user),
        ];

        // Scale max_tokens with batch size so larger batches get more room.
        let max_tokens = (batch.len() * 150).clamp(512, 4096);

        let (tx, mut rx) = tokio::sync::mpsc::channel::<crate::llm::Token>(256);
        let generate = llm.generate(&messages, max_tokens, tx, Some(schema.clone()));
        let collect = async move {
            let mut buf = String::new();
            while let Some(t) = rx.recv().await {
                buf.push_str(&t);
            }
            buf
        };
        let (_, raw_json) =
            tokio::try_join!(generate, async { Ok::<_, anyhow::Error>(collect.await) })?;
        let raw_json = crate::utils::strip_ansi(&raw_json);

        let parsed: serde_json::Value = serde_json::from_str(&raw_json).with_context(|| {
            format!(
                "parsing LLM harvest response (batch {}):\n{raw_json}",
                batch_idx + 1
            )
        })?;

        let entries = parsed["entries"].as_array().cloned().unwrap_or_default();

        if entries.is_empty() {
            println!("  No significant commits in this batch.");
            continue;
        }

        println!("Embedding {} entries…", entries.len());
        for entry in &entries {
            let sha = entry["sha"].as_str().unwrap_or("").to_string();
            let kind = entry["kind"].as_str().unwrap_or("note");
            let title = entry["title"].as_str().unwrap_or("").to_string();
            let body = entry["body"].as_str().unwrap_or("").to_string();
            let tags_val = entry["tags"].as_array();

            let mut tags: Vec<String> = tags_val
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            // Store the full SHA as a tag for dedup on future harvests.
            let full_sha = batch
                .iter()
                .find(|(s, _, _)| s.starts_with(&sha))
                .map(|(s, _, _)| s.clone())
                .unwrap_or(sha.clone());
            tags.push(format!("git:{full_sha}"));

            let embed_text = format!("title: {title} | text: {body}");
            let vecs = embedder.embed(&[&embed_text]).await?;
            let embedding = vecs.first().map(|v| vec_to_blob(v));

            let note_id = backend
                .add(NoteInput {
                    kind: kind.to_string(),
                    title: title.clone(),
                    body: body.clone(),
                    tags: tags.clone(),
                    linked_files: vec![],
                    embedding,
                })
                .await?;

            println!("  + [{kind}] #{note_id}: {title}");
            stored += 1;
        }
    } // end batch loop

    let skipped = new_commits.len().saturating_sub(stored);
    println!("\nStored {stored} memory entries. Skipped {skipped} routine commits.");
    Ok(())
}
