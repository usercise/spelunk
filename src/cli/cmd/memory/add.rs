use anyhow::{Context, Result};

use super::super::super::MemoryAddArgs;
use crate::{
    config::Config,
    embeddings::{EmbeddingBackend as _, vec_to_blob},
    storage::{NoteInput, open_memory_backend},
};

pub(super) async fn memory_add(
    args: MemoryAddArgs,
    mem_path: &std::path::Path,
    cfg: &Config,
) -> Result<()> {
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
                tokio::task::spawn_blocking(move || super::open_editor_for_body(&t))
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

    let embed_text = format!("title: {title} | text: {body}");
    let sp = super::super::ui::spinner("Embedding…");
    let embedder = crate::backends::ActiveEmbedder::load(cfg)
        .await
        .context("loading embedding model")?;
    let vecs = embedder
        .embed(&[&embed_text])
        .await
        .context("embedding memory entry")?;
    let blob = vecs
        .into_iter()
        .next()
        .map(|v| vec_to_blob(&v))
        .context("embedding returned empty")?;
    sp.finish_and_clear();

    let backend = open_memory_backend(cfg, mem_path)?;
    let id = backend
        .add(NoteInput {
            kind: args.kind.clone(),
            title: title.clone(),
            body: body.clone(),
            tags: tags.clone(),
            linked_files: files.clone(),
            embedding: Some(blob),
            source_ref: None,
            valid_at: args
                .valid_at
                .and_then(|s| super::parse_as_of(Some(&s)).ok().flatten()),
            supersedes: args.supersedes,
        })
        .await?;

    println!("Stored [{kind}] #{id}: {title}", kind = args.kind);
    Ok(())
}

async fn fetch_url_content(url: &str) -> Result<(String, String)> {
    let gh_issue_re =
        regex::Regex::new(r"https?://github\.com/([^/]+)/([^/]+)/(?:issues|pull)/(\d+)").unwrap();

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
    }

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
    }

    let client = reqwest::Client::builder()
        .user_agent(concat!("spelunk/", env!("CARGO_PKG_VERSION")))
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

fn parse_web_to_md_output(md: &str, url: &str) -> Result<(String, String)> {
    let md = md.trim();
    if let Some(rest) = md.strip_prefix("# ") {
        let (title_line, body) = rest.split_once('\n').unwrap_or((rest, ""));
        Ok((title_line.trim().to_string(), body.trim_start().to_string()))
    } else {
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
