//! Branch naming assistant (AI).
//!
//! This module generates and parses branch name suggestions with fixed prefixes.

use super::client::{AIClient, AIError, ChatMessage};
use serde::Deserialize;

pub const BRANCH_SUGGEST_SYSTEM_PROMPT: &str = "You are a git branch naming assistant. Generate exactly 3 branch name suggestions based on the user's description.\n\nRules:\n- Each suggestion must include exactly one of these prefixes: feature/, bugfix/, hotfix/, release/\n- Use lowercase\n- Use hyphens for separators\n- Keep names concise (<= 50 characters including prefix)\n\nRespond with JSON only in this format: {\"suggestions\": [\"prefix/name-1\", \"prefix/name-2\", \"prefix/name-3\"]}";

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
struct BranchSuggestionsResponse {
    suggestions: Vec<String>,
}

/// Parse AI response text into sanitized branch name suggestions.
///
/// Expected JSON: {"suggestions": ["feature/foo", "bugfix/bar", "feature/baz"]}
pub fn parse_branch_suggestions(response: &str) -> Result<Vec<String>, AIError> {
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

    let parsed: BranchSuggestionsResponse = serde_json::from_str(json)
        .map_err(|e| AIError::ParseError(format!("Invalid suggestions JSON: {e}")))?;

    if parsed.suggestions.len() != 3 {
        return Err(AIError::ParseError(
            "Expected exactly 3 suggestions".to_string(),
        ));
    }

    let mut out = Vec::with_capacity(3);
    for raw in parsed.suggestions {
        let trimmed = raw.trim();
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
        out.push(format!("{}{}", t.prefix(), sanitized));
    }

    Ok(out)
}

/// Generate 3 branch name suggestions for a human description.
///
/// Note: This performs a network request via `AIClient`.
pub fn suggest_branch_names(client: &AIClient, description: &str) -> Result<Vec<String>, AIError> {
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
    parse_branch_suggestions(&response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_branch_suggestions_success_sanitizes_suffix_only() {
        let response =
            r#"{"suggestions": ["feature/Add Login", "bugfix/fix-crash!", "release/v1.2.3"]}"#;
        let out = parse_branch_suggestions(response).unwrap();
        assert_eq!(out.len(), 3);
        assert_eq!(out[0], "feature/add-login");
        assert_eq!(out[1], "bugfix/fix-crash");
        assert_eq!(out[2], "release/v1-2-3");
    }

    #[test]
    fn parse_branch_suggestions_requires_json_object() {
        let err = parse_branch_suggestions("not-json").unwrap_err();
        assert!(matches!(err, AIError::ParseError(_)));
    }

    #[test]
    fn parse_branch_suggestions_requires_exactly_three() {
        let err = parse_branch_suggestions(r#"{"suggestions":["feature/a"]}"#).unwrap_err();
        assert!(matches!(err, AIError::ParseError(_)));
    }

    #[test]
    fn parse_branch_suggestions_fails_on_invalid_prefix() {
        let err = parse_branch_suggestions(r#"{"suggestions":["foo/bar","feature/a","bugfix/b"]}"#)
            .unwrap_err();
        assert!(matches!(err, AIError::ParseError(_)));
    }

    #[test]
    fn parse_branch_suggestions_fails_when_sanitized_suffix_empty() {
        let err =
            parse_branch_suggestions(r#"{"suggestions":["feature/!!!","bugfix/ok","hotfix/ok2"]}"#)
                .unwrap_err();
        assert!(matches!(err, AIError::ParseError(_)));
    }
}
