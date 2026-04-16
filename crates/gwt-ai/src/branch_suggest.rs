//! Branch name suggestion via AI.
//!
//! Given a textual context (SPEC title, issue description, etc.), the AI
//! proposes 3-5 valid git branch name candidates.

use serde::Deserialize;

use crate::{
    client::{AIClient, ChatMessage},
    error::AIError,
};

const SYSTEM_PROMPT: &str = "\
You are a git branch naming assistant. Generate 3 to 5 branch name suggestions \
based on the user's description.\n\n\
Rules:\n\
- Each suggestion must include exactly one of these prefixes: feature/, bugfix/, hotfix/, release/\n\
- Use lowercase\n\
- Use hyphens for separators\n\
- Keep names concise (<= 50 characters including prefix)\n\n\
Respond with JSON only in this format:\n\
{\"suggestions\": [\"prefix/name-1\", \"prefix/name-2\", ...]}";

const VALID_PREFIXES: &[&str] = &["feature/", "bugfix/", "hotfix/", "release/"];

#[derive(Debug, Deserialize)]
struct SuggestionsResponse {
    suggestions: Vec<String>,
}

/// Validate and sanitize a single branch name candidate.
///
/// Returns `Some(sanitized)` if the name has a known prefix and non-empty
/// suffix after sanitization, otherwise `None`.
fn validate_branch_name(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let prefix = VALID_PREFIXES.iter().find(|p| trimmed.starts_with(**p))?;
    let suffix = &trimmed[prefix.len()..];

    let sanitized: String = suffix
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else if c == ' ' {
                '-'
            } else {
                '\0'
            }
        })
        .filter(|&c| c != '\0')
        .collect();

    // Collapse consecutive hyphens and trim leading/trailing hyphens.
    let collapsed = collapse_hyphens(&sanitized);
    if collapsed.is_empty() {
        return None;
    }

    let full = format!("{prefix}{collapsed}");
    if full.len() > 50 {
        return None;
    }
    Some(full)
}

fn collapse_hyphens(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_hyphen = false;
    for c in s.chars() {
        if c == '-' {
            if !prev_hyphen {
                out.push(c);
            }
            prev_hyphen = true;
        } else {
            out.push(c);
            prev_hyphen = false;
        }
    }
    out.trim_matches('-').to_string()
}

/// Parse the AI response JSON into a list of validated branch names.
pub fn parse_suggestions(response: &str) -> Result<Vec<String>, AIError> {
    let start = response
        .find('{')
        .ok_or_else(|| AIError::ParseError("No JSON object in response".into()))?;
    let end = response
        .rfind('}')
        .ok_or_else(|| AIError::ParseError("No JSON object in response".into()))?;
    if end <= start {
        return Err(AIError::ParseError("Invalid JSON bounds".into()));
    }

    let parsed: SuggestionsResponse = serde_json::from_str(&response[start..=end])
        .map_err(|e| AIError::ParseError(format!("Invalid suggestions JSON: {e}")))?;

    let mut valid: Vec<String> = parsed
        .suggestions
        .iter()
        .filter_map(|s| validate_branch_name(s))
        .collect();

    if valid.len() < 3 {
        return Err(AIError::ParseError(
            "Need at least 3 valid branch names in suggestions".into(),
        ));
    }

    valid.truncate(5);
    Ok(valid)
}

/// Ask the AI client to suggest branch names for the given context.
///
/// Returns 3-5 validated, git-safe branch name candidates.
pub fn suggest_branch_name(client: &AIClient, context: &str) -> Result<Vec<String>, AIError> {
    let context = context.trim();
    if context.is_empty() {
        return Err(AIError::ConfigError("Context is empty".into()));
    }

    let messages = vec![
        ChatMessage {
            role: "system".into(),
            content: SYSTEM_PROMPT.into(),
        },
        ChatMessage {
            role: "user".into(),
            content: context.into(),
        },
    ];

    let response = client.create_response(messages)?;
    parse_suggestions(&response)
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::*;

    // ── validate_branch_name ───────────────────────────────────────────

    #[test]
    fn validates_good_name() {
        assert_eq!(
            validate_branch_name("feature/add-login"),
            Some("feature/add-login".into())
        );
    }

    #[test]
    fn lowercases_suffix() {
        assert_eq!(
            validate_branch_name("bugfix/Fix-Crash"),
            Some("bugfix/fix-crash".into())
        );
    }

    #[test]
    fn replaces_spaces_with_hyphens() {
        assert_eq!(
            validate_branch_name("feature/add user auth"),
            Some("feature/add-user-auth".into())
        );
    }

    #[test]
    fn strips_invalid_chars() {
        assert_eq!(
            validate_branch_name("hotfix/fix@bug#1"),
            Some("hotfix/fixbug1".into())
        );
    }

    #[test]
    fn rejects_unknown_prefix() {
        assert_eq!(validate_branch_name("fix/something"), None);
    }

    #[test]
    fn rejects_empty_suffix() {
        assert_eq!(validate_branch_name("feature/!!!"), None);
    }

    #[test]
    fn rejects_too_long() {
        let long = format!("feature/{}", "a".repeat(50));
        assert_eq!(validate_branch_name(&long), None);
    }

    #[test]
    fn collapses_consecutive_hyphens() {
        assert_eq!(
            validate_branch_name("feature/a--b---c"),
            Some("feature/a-b-c".into())
        );
    }

    #[test]
    fn trims_leading_trailing_hyphens_from_suffix() {
        assert_eq!(
            validate_branch_name("feature/-leading-trailing-"),
            Some("feature/leading-trailing".into())
        );
    }

    // ── parse_suggestions ──────────────────────────────────────────────

    #[test]
    fn parses_valid_json() {
        let json = r#"{"suggestions": ["feature/add-auth", "bugfix/fix-crash", "hotfix/patch-1"]}"#;
        let result = parse_suggestions(json).unwrap();
        assert_eq!(
            result,
            vec!["feature/add-auth", "bugfix/fix-crash", "hotfix/patch-1"]
        );
        assert!(result.iter().all(|s| is_git_safe_branch_name(s)));
    }

    #[test]
    fn filters_invalid_suggestions() {
        let json =
            r#"{"suggestions": ["feature/ok", "bad/name", "hotfix/good", "bugfix/fix-this"]}"#;
        let result = parse_suggestions(json).unwrap();
        assert_eq!(result, vec!["feature/ok", "hotfix/good", "bugfix/fix-this"]);
        assert!(result.iter().all(|s| is_git_safe_branch_name(s)));
    }

    #[test]
    fn fails_on_all_invalid() {
        let json = r#"{"suggestions": ["bad/one", "worse/two"]}"#;
        assert!(parse_suggestions(json).is_err());
    }

    #[test]
    fn fails_when_fewer_than_three_valid_suggestions_remain() {
        let json = r#"{"suggestions": ["feature/ok", "bad/name", "hotfix/good"]}"#;
        let err = parse_suggestions(json).unwrap_err();
        assert!(matches!(err, AIError::ParseError(_)));
    }

    #[test]
    fn truncates_to_five_valid_suggestions() {
        let json = r#"{"suggestions": ["feature/one", "feature/two", "feature/three", "feature/four", "feature/five", "feature/six"]}"#;
        let result = parse_suggestions(json).unwrap();
        assert_eq!(result.len(), 5);
        assert_eq!(
            result,
            vec![
                "feature/one",
                "feature/two",
                "feature/three",
                "feature/four",
                "feature/five"
            ]
        );
        assert!(result.iter().all(|s| is_git_safe_branch_name(s)));
    }

    #[test]
    fn fails_on_no_json() {
        assert!(parse_suggestions("no json here").is_err());
    }

    #[test]
    fn handles_surrounding_text() {
        let text = r#"Here are suggestions: {"suggestions": ["release/v2", "release/v2-1", "release/v2-2"]} hope this helps"#;
        let result = parse_suggestions(text).unwrap();
        assert_eq!(result, vec!["release/v2", "release/v2-1", "release/v2-2"]);
    }

    // ── suggest_branch_name ────────────────────────────────────────────

    #[test]
    fn rejects_empty_context() {
        // We cannot call the real API, but we can test validation.
        let client = AIClient::new("https://api.example.com", "k", "m").unwrap();
        let err = suggest_branch_name(&client, "").unwrap_err();
        assert!(matches!(err, AIError::ConfigError(_)));
    }

    fn is_git_safe_branch_name(name: &str) -> bool {
        Command::new("git")
            .args(["check-ref-format", "--branch", name])
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }
}
