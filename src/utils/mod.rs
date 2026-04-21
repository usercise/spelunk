pub mod dates;

/// Returns true when the process is running in agent mode (`AGENT=true`).
///
/// In agent mode all output defaults to structured JSON and progress spinners
/// are suppressed so that stdout is machine-readable.
pub fn is_agent_mode() -> bool {
    std::env::var("AGENT").as_deref() == Ok("true")
}

/// Return the effective output format string.
///
/// When agent mode is active, overrides `"text"` with `"json"` so that every
/// command with a `--format` flag produces machine-readable output without the
/// caller needing to pass `--format json` explicitly.
pub fn effective_format(format: &str) -> &str {
    if is_agent_mode() && format == "text" {
        "json"
    } else {
        format
    }
}

/// Strip ANSI escape sequences and unsafe control characters from a string.
///
/// Allows newline, carriage return, and tab. Strips all other C0 control
/// characters, DEL, and ANSI/VT escape sequences (CSI, OSC, two-char).
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '\x1b' => {
                match chars.peek().copied() {
                    Some('[') => {
                        // CSI sequence: ESC [ <params> <final 0x40–0x7E>
                        chars.next();
                        for c2 in chars.by_ref() {
                            if ('\x40'..='\x7e').contains(&c2) {
                                break;
                            }
                        }
                    }
                    Some(']') => {
                        // OSC sequence: ESC ] <text> ST  (ST = BEL or ESC \)
                        chars.next();
                        loop {
                            match chars.next() {
                                None | Some('\x07') => break,
                                Some('\x1b') => {
                                    chars.next();
                                    break;
                                }
                                _ => {}
                            }
                        }
                    }
                    _ => {
                        // Two-char sequence: ESC <char>
                        chars.next();
                    }
                }
            }
            '\n' | '\r' | '\t' => out.push(c),
            c if (c as u32) < 0x20 || c == '\x7f' => { /* drop */ }
            c => out.push(c),
        }
    }
    out
}

/// Format a Unix timestamp as a human-readable age string (e.g. "3 min ago").
pub fn format_age(created_at: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let secs = (now - created_at).max(0) as u64;
    if secs < 90 {
        format!("{secs} sec ago")
    } else if secs < 3600 {
        format!("{} min ago", secs / 60)
    } else if secs < 86400 {
        format!("{} hr ago", secs / 3600)
    } else {
        format!("{} days ago", secs / 86400)
    }
}

/// Collect files modified or untracked relative to HEAD using git.
/// Returns an empty set on any error (graceful degradation).
pub fn worktree_modified_files() -> std::collections::HashSet<String> {
    let mut files = std::collections::HashSet::new();

    if let Ok(out) = std::process::Command::new("git")
        .args(["diff", "--name-only", "HEAD"])
        .output()
    {
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let s = line.trim();
            if !s.is_empty() {
                files.insert(s.to_string());
            }
        }
    }

    if let Ok(out) = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .output()
    {
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let s = line.trim();
            if s.len() > 3 {
                let path = s[3..].trim();
                if !path.is_empty() {
                    files.insert(path.to_string());
                }
            }
        }
    }

    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_csi_colour() {
        assert_eq!(strip_ansi("\x1b[1;32mhello\x1b[0m"), "hello");
    }

    #[test]
    fn strips_osc() {
        assert_eq!(strip_ansi("\x1b]0;title\x07text"), "text");
    }

    #[test]
    fn preserves_newlines_and_tabs() {
        assert_eq!(strip_ansi("line1\nline2\ttabbed"), "line1\nline2\ttabbed");
    }

    #[test]
    fn strips_lone_c0_controls() {
        assert_eq!(strip_ansi("a\x01\x08b"), "ab");
    }

    #[test]
    fn clean_string_unchanged() {
        let s = "hello, world! 123";
        assert_eq!(strip_ansi(s), s);
    }
}
