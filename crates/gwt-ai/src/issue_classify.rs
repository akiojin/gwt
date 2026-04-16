//! Issue classification via AI.
//!
//! Determines the branch-type category prefix for a GitHub issue
//! (feature, bugfix, hotfix, or release) based on title and body.

use crate::{
    client::{AIClient, ChatMessage},
    error::AIError,
};

const SYSTEM_PROMPT: &str = "\
You are a branch prefix classifier for GitHub issues. Based on the issue title \
and body, respond with exactly ONE word indicating the branch type.\n\n\
Valid responses: feature, bugfix, hotfix, release\n\n\
- bugfix: Bug reports, crashes, errors, broken functionality, regressions\n\
- hotfix: Critical production issues requiring immediate fix\n\
- feature: New features, enhancements, improvements\n\
- release: Release preparation, version bumps\n\n\
Respond with only the single word. No explanation.";

const VALID_PREFIXES: &[&str] = &["feature", "bugfix", "hotfix", "release"];
const BODY_MAX_CHARS: usize = 500;

fn truncate_chars(input: &str, max: usize) -> &str {
    match input.char_indices().nth(max) {
        Some((idx, _)) => &input[..idx],
        None => input,
    }
}

/// Parse the AI response text into a valid category prefix.
///
/// Lowercases and searches for the first known keyword.
pub fn parse_classify_response(response: &str) -> Result<String, AIError> {
    let lower = response.trim().to_lowercase();
    if lower.is_empty() {
        return Err(AIError::ParseError("Empty classification response".into()));
    }

    let earliest = VALID_PREFIXES
        .iter()
        .filter_map(|&p| lower.find(p).map(|idx| (idx, p)))
        .min_by_key(|(idx, _)| *idx);

    match earliest {
        Some((_, prefix)) => Ok(prefix.to_string()),
        None => Err(AIError::ParseError(format!(
            "No valid prefix found in response: {lower}"
        ))),
    }
}

/// Classify a GitHub issue into a branch prefix using AI.
///
/// Returns one of: `"feature"`, `"bugfix"`, `"hotfix"`, `"release"`.
pub fn classify_issue(client: &AIClient, title: &str, body: &str) -> Result<String, AIError> {
    let title = title.trim();
    if title.is_empty() {
        return Err(AIError::ConfigError("Issue title is empty".into()));
    }

    let body_str = if body.trim().is_empty() {
        "(none)"
    } else {
        truncate_chars(body.trim(), BODY_MAX_CHARS)
    };

    let user_message = format!("Title: {title}\nBody: {body_str}");

    let messages = vec![
        ChatMessage {
            role: "system".into(),
            content: SYSTEM_PROMPT.into(),
        },
        ChatMessage {
            role: "user".into(),
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
    fn returns_feature() {
        assert_eq!(parse_classify_response("feature").unwrap(), "feature");
    }

    #[test]
    fn returns_bugfix() {
        assert_eq!(parse_classify_response("bugfix").unwrap(), "bugfix");
    }

    #[test]
    fn returns_hotfix() {
        assert_eq!(parse_classify_response("hotfix").unwrap(), "hotfix");
    }

    #[test]
    fn returns_release() {
        assert_eq!(parse_classify_response("release").unwrap(), "release");
    }

    #[test]
    fn extracts_from_surrounding_text() {
        assert_eq!(
            parse_classify_response("The prefix should be bugfix").unwrap(),
            "bugfix"
        );
    }

    #[test]
    fn picks_earliest_match() {
        assert_eq!(
            parse_classify_response("hotfix, not bugfix").unwrap(),
            "hotfix"
        );
    }

    #[test]
    fn handles_whitespace() {
        assert_eq!(parse_classify_response("  feature  ").unwrap(), "feature");
    }

    #[test]
    fn fails_on_invalid_value() {
        assert!(parse_classify_response("fix").is_err());
    }

    #[test]
    fn fails_on_empty() {
        assert!(parse_classify_response("").is_err());
    }

    #[test]
    fn truncate_preserves_utf8() {
        let input = "a".repeat(600);
        let truncated = truncate_chars(&input, BODY_MAX_CHARS);
        assert_eq!(truncated.chars().count(), 500);
    }

    #[test]
    fn truncate_short_input_unchanged() {
        let input = "short";
        assert_eq!(truncate_chars(input, BODY_MAX_CHARS), "short");
    }

    #[test]
    fn classify_rejects_empty_title() {
        let client = AIClient::new("https://api.example.com", "k", "m").unwrap();
        let err = classify_issue(&client, "", "body").unwrap_err();
        assert!(matches!(err, AIError::ConfigError(_)));
    }
}
