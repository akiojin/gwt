//! Issue branch prefix classifier (AI).
//!
//! Given an issue title, labels, and body, this module uses an AI model to
//! determine the appropriate branch prefix (feature, bugfix, hotfix, release).

use super::client::{AIClient, AIError, ChatMessage};

pub const ISSUE_CLASSIFY_SYSTEM_PROMPT: &str = "You are a branch prefix classifier for GitHub issues. Based on the issue title, labels, and body, respond with exactly ONE word indicating the branch type.\n\nValid responses: feature, bugfix, hotfix, release\n\n- bugfix: Bug reports, crashes, errors, broken functionality, regressions\n- hotfix: Critical production issues requiring immediate fix\n- feature: New features, enhancements, improvements\n- release: Release preparation, version bumps\n\nRespond with only the single word. No explanation.";

const VALID_PREFIXES: &[&str] = &["feature", "bugfix", "hotfix", "release"];
const ISSUE_BODY_MAX_CHARS: usize = 500;

fn truncate_to_chars(input: &str, max_chars: usize) -> &str {
    match input.char_indices().nth(max_chars) {
        Some((idx, _)) => &input[..idx],
        None => input,
    }
}

/// Parse the AI response text into a valid branch prefix.
///
/// Trims whitespace, lowercases, and searches for a known prefix keyword.
pub fn parse_classify_response(response: &str) -> Result<String, AIError> {
    let lower = response.trim().to_lowercase();
    if lower.is_empty() {
        return Err(AIError::ParseError(
            "Empty classification response".to_string(),
        ));
    }

    for &prefix in VALID_PREFIXES {
        if lower.contains(prefix) {
            return Ok(prefix.to_string());
        }
    }

    Err(AIError::ParseError(format!(
        "No valid prefix found in response: {lower}"
    )))
}

/// Classify a GitHub issue into a branch prefix using AI.
///
/// Returns one of: "feature", "bugfix", "hotfix", "release".
pub fn classify_issue_prefix(
    client: &AIClient,
    title: &str,
    labels: &[String],
    body: Option<&str>,
) -> Result<String, AIError> {
    let title = title.trim();
    if title.is_empty() {
        return Err(AIError::ConfigError(
            "Issue title is empty".to_string(),
        ));
    }

    let labels_str = if labels.is_empty() {
        "(none)".to_string()
    } else {
        labels.join(", ")
    };

    let body_str = match body {
        Some(b) if !b.trim().is_empty() => {
            let trimmed = b.trim();
            truncate_to_chars(trimmed, ISSUE_BODY_MAX_CHARS)
        }
        _ => "(none)",
    };

    let user_message = format!("Title: {title}\nLabels: {labels_str}\nBody: {body_str}");

    let messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: ISSUE_CLASSIFY_SYSTEM_PROMPT.to_string(),
        },
        ChatMessage {
            role: "user".to_string(),
            content: user_message,
        },
    ];

    let response = client.create_response(messages)?;
    parse_classify_response(&response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_classify_response_returns_feature() {
        assert_eq!(parse_classify_response("feature").unwrap(), "feature");
    }

    #[test]
    fn parse_classify_response_returns_bugfix() {
        assert_eq!(parse_classify_response("bugfix").unwrap(), "bugfix");
    }

    #[test]
    fn parse_classify_response_returns_hotfix() {
        assert_eq!(parse_classify_response("hotfix").unwrap(), "hotfix");
    }

    #[test]
    fn parse_classify_response_returns_release() {
        assert_eq!(parse_classify_response("release").unwrap(), "release");
    }

    #[test]
    fn parse_classify_response_extracts_from_surrounding_text() {
        assert_eq!(
            parse_classify_response("The prefix should be bugfix").unwrap(),
            "bugfix"
        );
    }

    #[test]
    fn parse_classify_response_handles_whitespace() {
        assert_eq!(parse_classify_response("  feature  ").unwrap(), "feature");
    }

    #[test]
    fn parse_classify_response_fails_on_invalid_value() {
        assert!(parse_classify_response("fix").is_err());
    }

    #[test]
    fn parse_classify_response_fails_on_empty() {
        assert!(parse_classify_response("").is_err());
    }

    #[test]
    fn parse_classify_response_fails_on_enhancement() {
        assert!(parse_classify_response("enhancement").is_err());
    }

    #[test]
    fn truncate_to_chars_keeps_utf8_boundary() {
        let input = "あ".repeat(501);
        let truncated = truncate_to_chars(&input, 500);
        assert_eq!(truncated.chars().count(), 500);
        assert!(std::str::from_utf8(truncated.as_bytes()).is_ok());
    }

    #[test]
    fn truncate_to_chars_returns_original_when_shorter_than_limit() {
        let input = "emoji😀text";
        let truncated = truncate_to_chars(input, ISSUE_BODY_MAX_CHARS);
        assert_eq!(truncated, input);
    }
}
