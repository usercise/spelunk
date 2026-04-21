#![no_main]

use libfuzzer_sys::fuzz_target;
use serde::Deserialize;
use std::collections::HashMap;

/// Fuzz deserialization of `ClaudeHistoryEntry` from `~/.claude/history.jsonl`.
///
/// Run with:
///   cargo +nightly fuzz run fuzz_cli_history_entry -- -max_total_time=600
///
/// `ClaudeHistoryEntry` and `PastedContent` in
/// `src/cli/cmd/memory/harvest_claude.rs` are private, so they are replicated
/// here.  Goal: confirm serde deserialization doesn't panic on arbitrary JSON.
#[derive(Deserialize)]
struct PastedContent {
    #[serde(default)]
    content: String,
}

#[derive(Deserialize)]
struct ClaudeHistoryEntry {
    #[serde(default)]
    display: String,
    #[serde(rename = "pastedContents", default)]
    pasted_contents: HashMap<String, PastedContent>,
    #[serde(default)]
    timestamp: i64,
    #[serde(default)]
    project: String,
    #[serde(rename = "sessionId", default)]
    session_id: String,
}

fuzz_target!(|data: &[u8]| {
    let Ok(s) = std::str::from_utf8(data) else { return };
    let _ = serde_json::from_str::<ClaudeHistoryEntry>(s);
});
