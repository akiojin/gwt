//! Summary generation and cache

use super::client::{AIClient, AIError, ChatMessage};
use crate::git::CommitEntry;
use std::collections::HashMap;

pub const SYSTEM_PROMPT: &str = "You are a helpful assistant that summarizes git commit history.\nRespond with 2-3 bullet points in English, each starting with '- '.\nBe concise and focus on the main changes.\nDo not include commit hashes or dates in the summary.";

pub fn build_user_prompt(branch_name: &str, commit_list: &str) -> String {
    format!(
        "Summarize the following git commits for branch '{}':\n\n{}",
        branch_name, commit_list
    )
}

#[derive(Debug, Clone)]
pub struct SummaryRequest {
    pub branch_name: String,
    pub commits: Vec<CommitEntry>,
}

#[derive(Debug, Default, Clone)]
pub struct AISummaryCache {
    cache: HashMap<String, Vec<String>>,
}

impl AISummaryCache {
    pub fn get(&self, branch: &str) -> Option<&Vec<String>> {
        self.cache.get(branch)
    }

    pub fn set(&mut self, branch: String, summary: Vec<String>) {
        self.cache.insert(branch, summary);
    }
}

pub fn summarize_commits(
    client: &AIClient,
    branch_name: &str,
    commits: &[CommitEntry],
) -> Result<Vec<String>, AIError> {
    if commits.is_empty() {
        return Err(AIError::ParseError("No commits to summarize".to_string()));
    }

    let commit_list = commits
        .iter()
        .map(|commit| {
            if commit.message.is_empty() {
                commit.hash.clone()
            } else {
                format!("{} {}", commit.hash, commit.message)
            }
        })
        .collect::<Vec<String>>()
        .join("\n");

    let messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: SYSTEM_PROMPT.to_string(),
        },
        ChatMessage {
            role: "user".to_string(),
            content: build_user_prompt(branch_name, &commit_list),
        },
    ];

    let content = client.create_chat_completion(messages)?;
    parse_summary_lines(&content)
}

pub fn parse_summary_lines(content: &str) -> Result<Vec<String>, AIError> {
    let mut lines: Vec<String> = content
        .lines()
        .filter_map(normalize_line)
        .collect();

    if lines.is_empty() {
        let cleaned = content.trim();
        if cleaned.is_empty() {
            return Err(AIError::ParseError("Empty summary".to_string()));
        }
        for sentence in cleaned.split_terminator(". ") {
            let trimmed = sentence.trim();
            if trimmed.is_empty() {
                continue;
            }
            lines.push(format!("- {}", trimmed.trim_end_matches('.')));
            if lines.len() >= 3 {
                break;
            }
        }
    }

    if lines.is_empty() {
        return Err(AIError::ParseError("No summary lines".to_string()));
    }

    if lines.len() > 3 {
        lines.truncate(3);
    }

    Ok(lines)
}

fn normalize_line(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    let trimmed = if let Some(rest) = trimmed.strip_prefix("- ") {
        rest.trim()
    } else if let Some(rest) = trimmed.strip_prefix("-") {
        rest.trim()
    } else if let Some(rest) = trimmed.strip_prefix("* ") {
        rest.trim()
    } else if let Some(rest) = trimmed.strip_prefix("*") {
        rest.trim()
    } else if let Some(rest) = trimmed.strip_prefix("•") {
        rest.trim()
    } else if let Some(rest) = strip_ordered_prefix(trimmed) {
        rest.trim()
    } else {
        trimmed
    };

    if trimmed.is_empty() {
        return None;
    }

    Some(format!("- {}", trimmed))
}

fn strip_ordered_prefix(value: &str) -> Option<&str> {
    let mut chars = value.chars();
    let mut digit_count = 0usize;
    while let Some(ch) = chars.next() {
        if ch.is_ascii_digit() {
            digit_count += 1;
            continue;
        }
        if digit_count > 0 && (ch == '.' || ch == ')') {
            return Some(chars.as_str().trim_start());
        }
        break;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_summary_lines_bullets() {
        let content = "- Added login\n- Fixed bug\n- Updated docs";
        let lines = parse_summary_lines(content).expect("should parse");
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "- Added login");
    }

    #[test]
    fn test_parse_summary_lines_ordered() {
        let content = "1. Added login\n2) Fixed bug";
        let lines = parse_summary_lines(content).expect("should parse");
        assert_eq!(lines[0], "- Added login");
        assert_eq!(lines[1], "- Fixed bug");
    }
}
