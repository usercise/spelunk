use anyhow::{Context, Result};

use super::MemoryWatchArgs;
use crate::config::Config;

pub(super) async fn memory_watch(args: MemoryWatchArgs, cfg: &Config) -> Result<()> {
    let base_url = cfg.memory_server_url.as_deref().ok_or_else(|| {
        anyhow::anyhow!(
            "`memory_server_url` is not configured. \
             Set it in `.spelunk/config.toml` or via `SPELUNK_SERVER_URL`."
        )
    })?;
    let project_id = cfg.project_id.as_deref().ok_or_else(|| {
        anyhow::anyhow!(
            "`project_id` is not configured. \
             Set it in `.spelunk/config.toml` or via `SPELUNK_PROJECT_ID`."
        )
    })?;

    let url = format!(
        "{}/v1/projects/{}/memory/stream",
        base_url.trim_end_matches('/'),
        project_id,
    );

    eprintln!("Watching {url} — press Ctrl-C to stop.");

    let client = reqwest::Client::new();
    let mut req = client.get(&url);
    if let Some(key) = cfg.memory_server_key.as_deref() {
        req = req.header("Authorization", format!("Bearer {key}"));
    }

    let resp = req.send().await.context("connecting to /memory/stream")?;
    resp.error_for_status_ref()
        .context("server returned error for GET /memory/stream")?;

    let is_json = matches!(crate::utils::effective_format(&args.format), "json");
    let mut stream = resp.bytes_stream();

    use futures_util::StreamExt;

    let mut buf = String::new();
    loop {
        match stream.next().await {
            None => {
                // Server closed the connection.
                eprintln!("Stream closed by server.");
                break;
            }
            Some(Err(e)) => {
                eprintln!("Stream error: {e}");
                break;
            }
            Some(Ok(chunk)) => {
                let text = String::from_utf8_lossy(&chunk);
                buf.push_str(&text);
                // Process all complete SSE lines in the buffer.
                while let Some(pos) = buf.find('\n') {
                    let line = buf[..pos].to_string();
                    buf = buf[pos + 1..].to_string();
                    let line = line.trim_end_matches('\r');
                    if let Some(data) = line.strip_prefix("data: ") {
                        if data.is_empty() {
                            continue;
                        }
                        if is_json {
                            // Pretty-print the raw JSON.
                            if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                                println!("{}", serde_json::to_string_pretty(&v)?);
                            } else {
                                println!("{data}");
                            }
                        } else {
                            print_sse_note(data);
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

fn print_sse_note(data: &str) {
    #[derive(serde::Deserialize)]
    struct Slim {
        id: i64,
        kind: String,
        title: String,
        created_at: i64,
        #[serde(default)]
        tags: Vec<String>,
    }
    if let Ok(n) = serde_json::from_str::<Slim>(data) {
        println!(
            "\x1b[1m#{id}\x1b[0m  \x1b[33m[{kind}]\x1b[0m  {title}",
            id = n.id,
            kind = n.kind,
            title = n.title,
        );
        println!(
            "     \x1b[2m{}\x1b[0m",
            super::super::status::format_age(n.created_at)
        );
        if !n.tags.is_empty() {
            println!("     tags: {}", n.tags.join(", "));
        }
        println!();
    } else {
        println!("{data}");
    }
}
