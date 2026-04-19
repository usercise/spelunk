//! Lightweight secret scanner used during indexing.
//!
//! Chunks whose content matches known secret patterns are dropped before
//! embedding so credentials never enter the vector index.
//!
//! Patterns are deliberately conservative (high precision, some false
//! negatives) to avoid blocking legitimate code that discusses secrets
//! conceptually (e.g., documentation, tests with placeholder values).

use regex::Regex;
use std::sync::OnceLock;

/// Returns `true` if `text` appears to contain a secret that should not be indexed.
pub fn contains_secret(text: &str) -> bool {
    patterns().iter().any(|re| re.is_match(text))
}

static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();

fn patterns() -> &'static Vec<Regex> {
    PATTERNS.get_or_init(|| {
        let raw: &[&str] = &[
            // PEM / private key headers
            r"-----BEGIN (?:RSA |EC |DSA |OPENSSH )?PRIVATE KEY-----",
            // AWS access key IDs
            r"AKIA[0-9A-Z]{16}",
            // AWS secret access keys (base64, 40 chars after a key= / secret= assignment)
            r#"(?i)aws[_\-]?secret[_\-]?(?:access[_\-]?)?key["']?\s*[:=]\s*["']?[A-Za-z0-9+/]{40}"#,
            // Generic high-confidence key/secret assignments with a long value
            r#"(?i)(?:api[_\-]?key|auth[_\-]?token|secret[_\-]?key|access[_\-]?token|private[_\-]?key)\s*[:=]\s*["']?[A-Za-z0-9\-_.~+/]{32,}["']?"#,
            // Bearer tokens in HTTP headers
            r"(?i)Authorization:\s*Bearer\s+[A-Za-z0-9\-_.~+/]{20,}",
            // GitHub personal access tokens
            r"gh[pousr]_[A-Za-z0-9]{36,}",
            // Slack tokens
            r"xox[baprs]-[0-9A-Za-z\-]{10,}",
            // Generic JWT (three base64url segments)
            r"ey[A-Za-z0-9\-_]{10,}\.ey[A-Za-z0-9\-_]{10,}\.[A-Za-z0-9\-_]{10,}",
            // OpenAI API keys (sk- followed by 48+ alphanumeric/dash/underscore chars)
            r"sk-[A-Za-z0-9\-_]{48,}",
            // Anthropic API keys
            r"sk-ant-[A-Za-z0-9\-_]{40,}",
            // Stripe secret/test keys
            r"sk_(?:live|test)_[A-Za-z0-9]{24,}",
            // NPM automation tokens
            r"npm_[A-Za-z0-9]{36,}",
            // Database URLs with embedded passwords (postgres, mysql, mongodb)
            r"(?i)(?:postgres(?:ql)?|mysql|mongodb(?:\+srv)?)://[^:@\s]+:[^@\s]{8,}@",
        ];
        raw.iter()
            .map(|p| Regex::new(p).expect("invalid secret pattern"))
            .collect()
    })
}

// Call once at startup to compile regexes eagerly rather than on first chunk.
pub fn init() {
    let _ = patterns();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_pem_header() {
        assert!(contains_secret("-----BEGIN RSA PRIVATE KEY-----\nMIIE..."));
    }

    #[test]
    fn detects_aws_key_id() {
        assert!(contains_secret("key = AKIAIOSFODNN7EXAMPLE"));
    }

    #[test]
    fn detects_api_key_assignment() {
        assert!(contains_secret(
            r#"api_key = "sk-abcdefghijklmnopqrstuvwxyz012345""#
        ));
    }

    #[test]
    fn detects_github_pat() {
        assert!(contains_secret(
            "token = ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef123456789012"
        ));
    }

    #[test]
    fn clean_code_not_flagged() {
        assert!(!contains_secret(
            "fn verify_token(token: &str) -> bool { token.len() > 0 }"
        ));
    }

    #[test]
    fn placeholder_not_flagged() {
        assert!(!contains_secret("api_key = \"your_api_key_here\""));
    }

    #[test]
    fn detects_openai_key() {
        assert!(contains_secret(
            "OPENAI_API_KEY=sk-proj-ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuv"
        ));
    }

    #[test]
    fn detects_anthropic_key() {
        assert!(contains_secret(
            "key = sk-ant-api03-ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijkl"
        ));
    }

    #[test]
    fn detects_stripe_live_key() {
        // Concatenate to avoid triggering push-protection on an obviously fake key.
        let key = format!("stripe_key = sk_live_{}", "ABCDEFGHIJKLMNOPQRSTUVWXYZabcd");
        assert!(contains_secret(&key));
    }

    #[test]
    fn detects_stripe_test_key() {
        let key = format!("stripe_key = sk_test_{}", "ABCDEFGHIJKLMNOPQRSTUVWXYZabcd");
        assert!(contains_secret(&key));
    }

    #[test]
    fn detects_npm_token() {
        assert!(contains_secret(
            "NPM_TOKEN=npm_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijk"
        ));
    }

    #[test]
    fn detects_postgres_url_with_password() {
        assert!(contains_secret(
            "DATABASE_URL=postgresql://admin:s3cr3tpass@db.example.com/mydb"
        ));
    }

    #[test]
    fn detects_mongodb_url_with_password() {
        assert!(contains_secret(
            "url = mongodb+srv://user:mypassword123@cluster.mongodb.net/db"
        ));
    }

    #[test]
    fn db_url_without_password_not_flagged() {
        // No password segment (no colon before @)
        assert!(!contains_secret("postgresql://localhost/mydb"));
    }
}
