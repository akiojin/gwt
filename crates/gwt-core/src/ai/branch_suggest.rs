//! Branch naming assistant (AI).
//!
//! This module generates and parses a single branch name suggestion with a fixed prefix.

use serde::Deserialize;

use super::client::{AIClient, AIError, ChatMessage};

pub const BRANCH_SUGGEST_SYSTEM_PROMPT: &str = "You are a git branch naming assistant. Generate exactly 1 branch name suggestion based on the user's description.\n\nRules:\n- The suggestion must include exactly one of these prefixes: feature/, bugfix/, hotfix/, release/\n- Use lowercase\n- Use hyphens for separators\n- Keep names concise (<= 50 characters including prefix)\n\nRespond with JSON only in this format: {\"suggestion\": \"prefix/name\"}";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BranchType {
    Feature,
    Bugfix,
    Hotfix,
    Release,
}

impl BranchType {
    fn prefix(&self) -> &'static str {
        match self {
            BranchType::Feature => "feature/",
            BranchType::Bugfix => "bugfix/",
            BranchType::Hotfix => "hotfix/",
            BranchType::Release => "release/",
        }
    }

    fn all() -> &'static [BranchType] {
        &[
            BranchType::Feature,
            BranchType::Bugfix,
            BranchType::Hotfix,
            BranchType::Release,
        ]
    }

    fn from_prefix(name: &str) -> Option<(BranchType, &str)> {
        let trimmed = name.trim();
        for t in BranchType::all() {
            if let Some(rest) = trimmed.strip_prefix(t.prefix()) {
                return Some((*t, rest));
            }
        }
        None
    }
}

#[derive(Debug, Deserialize)]
struct BranchSuggestionResponse {
    suggestion: String,
}

/// Parse AI response text into a sanitized branch name suggestion.
///
/// Expected JSON: `{"suggestion": "prefix/name"}`
pub fn parse_branch_suggestion(response: &str) -> Result<String, AIError> {
    // Be resilient to pre/post text and extract the first JSON object.
    let start = response
        .find('{')
        .ok_or_else(|| AIError::ParseError("No JSON object found in response".to_string()))?;
    let end = response
        .rfind('}')
        .ok_or_else(|| AIError::ParseError("No JSON object found in response".to_string()))?;
    if end <= start {
        return Err(AIError::ParseError(
            "Invalid JSON object bounds in response".to_string(),
        ));
    }
    let json = &response[start..=end];

    let parsed: BranchSuggestionResponse = serde_json::from_str(json)
        .map_err(|e| AIError::ParseError(format!("Invalid suggestion JSON: {e}")))?;

    let trimmed = parsed.suggestion.trim();
    let Some((t, rest)) = BranchType::from_prefix(trimmed) else {
        return Err(AIError::ParseError(format!(
            "Suggestion missing/invalid prefix: {trimmed}"
        )));
    };

    // Sanitize suffix only; keep prefix.
    let sanitized = crate::agent::worktree::sanitize_branch_name(rest);
    if sanitized.is_empty() {
        return Err(AIError::ParseError(format!(
            "Suggestion suffix is empty after sanitization: {trimmed}"
        )));
    }

    Ok(format!("{}{}", t.prefix(), sanitized))
}

/// Generate a single branch name suggestion for a human description.
///
/// Note: This performs a network request via `AIClient`.
pub fn suggest_branch_name(client: &AIClient, description: &str) -> Result<String, AIError> {
    let description = description.trim();
    if description.is_empty() {
        return Err(AIError::ConfigError("Description is empty".to_string()));
    }

    let messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: BRANCH_SUGGEST_SYSTEM_PROMPT.to_string(),
        },
        ChatMessage {
            role: "user".to_string(),
            content: description.to_string(),
        },
    ];

    let response = client.create_response(messages)?;
    parse_branch_suggestion(&response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_branch_suggestion_success_sanitizes_suffix() {
        let response = r#"{"suggestion": "feature/Add Login"}"#;
        let out = parse_branch_suggestion(response).unwrap();
        assert_eq!(out, "feature/add-login");
    }

    #[test]
    fn parse_branch_suggestion_requires_json_object() {
        let err = parse_branch_suggestion("not-json").unwrap_err();
        assert!(matches!(err, AIError::ParseError(_)));
    }

    #[test]
    fn parse_branch_suggestion_fails_on_invalid_prefix() {
        let err = parse_branch_suggestion(r#"{"suggestion": "foo/bar"}"#).unwrap_err();
        assert!(matches!(err, AIError::ParseError(_)));
    }

    #[test]
    fn parse_branch_suggestion_fails_when_sanitized_suffix_empty() {
        let err = parse_branch_suggestion(r#"{"suggestion": "feature/!!!"}"#).unwrap_err();
        assert!(matches!(err, AIError::ParseError(_)));
    }
}
