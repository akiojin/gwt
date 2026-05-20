//! Secret redaction for process console lines.
//!
//! `ProcessConsoleHub` runs every stdout / stderr line through
//! [`redact_line`] before pushing it to the ring buffer or broadcasting
//! to subscribers (SPEC-1924 FR-041). The redaction targets the token
//! shapes that gh, git, and docker can echo into diagnostic output
//! when running in verbose / debug mode:
//!
//! - `Authorization: <anything>` headers (case-insensitive)
//! - `token=<value>` URL parameters
//! - GitHub Personal Access Token prefixes (`gh_` / `ghp_` / `ghs_` / `ghu_`) followed by 16+ alphanumerics
//!
//! Replacement is `***redacted***`. The function returns the raw line
//! unchanged when it does not match any pattern, so callers can compare
//! the input and output cheaply.

use std::sync::OnceLock;

use regex::Regex;

/// Replacement string used for every redacted pattern.
pub const REDACTED: &str = "***redacted***";

/// Apply every redaction pattern to `line`.
///
/// Returns a `String` because the regex crate cannot guarantee an
/// in-place replace. When no pattern matches, the result is byte-equal
/// to the input.
pub fn redact_line(line: &str) -> String {
    let mut out = line.to_string();
    out = authorization_re().replace_all(&out, REDACTED).into_owned();
    out = url_token_re().replace_all(&out, REDACTED).into_owned();
    out = github_token_re().replace_all(&out, REDACTED).into_owned();
    out
}

fn authorization_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)Authorization\s*:\s*[^\r\n]+").expect("authorization regex"))
}

fn url_token_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)\btoken=[^\s&\r\n]+").expect("url token regex"))
}

fn github_token_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"gh[pous]_[A-Za-z0-9]{16,}").expect("github token regex"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_authorization_header() {
        let line = "Authorization: Bearer ghp_abcdef0123456789abcdef";
        let out = redact_line(line);
        assert_eq!(out, REDACTED);
    }

    #[test]
    fn redacts_lowercase_authorization_header() {
        let line = "  authorization: bearer x  ";
        let out = redact_line(line);
        assert!(out.contains(REDACTED));
        assert!(!out.contains("bearer x"));
    }

    #[test]
    fn redacts_url_token_param() {
        let line = "GET /api/v3/user?token=secretvalue123 HTTP/1.1";
        let out = redact_line(line);
        assert!(out.contains(REDACTED));
        assert!(!out.contains("secretvalue123"));
        assert!(out.contains("/api/v3/user"));
    }

    #[test]
    fn redacts_github_personal_access_token() {
        let line = "got ghp_abcdef0123456789ABCDEF from env";
        let out = redact_line(line);
        assert!(out.contains(REDACTED));
        assert!(!out.contains("ghp_abcdef0123456789ABCDEF"));
    }

    #[test]
    fn redacts_other_github_token_prefixes() {
        for prefix in ["gho_", "ghs_", "ghu_"] {
            let token = format!("{prefix}{}", "X".repeat(20));
            let line = format!("token is {token} here");
            let out = redact_line(&line);
            assert!(
                out.contains(REDACTED),
                "{prefix} should be redacted, got: {out}"
            );
            assert!(!out.contains(&token));
        }
    }

    #[test]
    fn passes_through_clean_line() {
        let line = "fatal: could not find object";
        assert_eq!(redact_line(line), line);
    }

    #[test]
    fn does_not_redact_short_gh_words() {
        // `gh_` followed by <16 chars should not match.
        let line = "ghi_short and gh_abc are not tokens";
        let out = redact_line(line);
        assert_eq!(out, line);
    }

    #[test]
    fn redacts_multiple_secrets_in_one_line() {
        let line =
            "Authorization: Bearer ghp_abcdef0123456789abcdef; token=secretvalue123 ghs_short";
        let out = redact_line(line);
        assert!(!out.contains("ghp_abcdef0123456789abcdef"));
        assert!(!out.contains("secretvalue123"));
    }
}
