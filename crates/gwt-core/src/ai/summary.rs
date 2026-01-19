//! Session summary generation and cache.

use super::client::{AIClient, AIError, ChatMessage};
use super::session_parser::{MessageRole, ParsedSession, SessionMessage};
use std::collections::HashMap;
use std::time::SystemTime;

pub const SESSION_SYSTEM_PROMPT: &str = "You are a helpful assistant summarizing a coding agent session.\nReturn JSON only with keys: task_overview, short_summary, bullets.\n- task_overview: current task and progress in 1 sentence.\n- short_summary: 1-2 sentence summary.\n- bullets: 2-3 concise bullet points (no leading dash).\nUse English, no markdown, no extra text.";

const MAX_MESSAGE_CHARS: usize = 220;
const MAX_TOOL_ITEMS: usize = 8;

#[derive(Debug, Clone, Default)]
pub struct SessionSummary {
    pub task_overview: Option<String>,
    pub short_summary: Option<String>,
    pub bullet_points: Vec<String>,
    pub metrics: SessionMetrics,
    pub last_updated: Option<SystemTime>,
}

#[derive(Debug, Clone, Default)]
pub struct SessionMetrics {
    pub token_count: Option<usize>,
    pub tool_execution_count: usize,
    pub elapsed_seconds: Option<u64>,
    pub turn_count: usize,
}

#[derive(Debug, Default, Clone)]
pub struct SessionSummaryCache {
    cache: HashMap<String, SessionSummary>,
    last_modified: HashMap<String, SystemTime>,
    session_ids: HashMap<String, String>,
}

impl SessionSummaryCache {
    pub fn get(&self, branch: &str) -> Option<&SessionSummary> {
        self.cache.get(branch)
    }

    pub fn set(
        &mut self,
        branch: String,
        session_id: String,
        summary: SessionSummary,
        mtime: SystemTime,
    ) {
        self.cache.insert(branch.clone(), summary);
        self.last_modified.insert(branch.clone(), mtime);
        self.session_ids.insert(branch, session_id);
    }

    pub fn is_stale(&self, branch: &str, session_id: &str, current_mtime: SystemTime) -> bool {
        if let Some(cached_session_id) = self.session_ids.get(branch) {
            if cached_session_id != session_id {
                return true;
            }
        } else {
            return true;
        }

        self.last_modified
            .get(branch)
            .map(|&cached| cached < current_mtime)
            .unwrap_or(true)
    }
}

#[derive(Debug, Default)]
struct SessionSummaryFields {
    task_overview: Option<String>,
    short_summary: Option<String>,
    bullet_points: Vec<String>,
}

pub fn build_session_prompt(parsed: &ParsedSession) -> Vec<ChatMessage> {
    let mut lines = Vec::new();
    lines.push(format!(
        "Agent: {} (session_id: {})",
        parsed.agent_type.display_name(),
        parsed.session_id
    ));

    if parsed.messages.is_empty() {
        lines.push("No messages recorded.".to_string());
    } else {
        lines.push("Messages (sampled):".to_string());
        for (index, message) in parsed.messages.iter().enumerate() {
            let role = match message.role {
                MessageRole::User => "user",
                MessageRole::Assistant => "assistant",
            };
            let mut content = message.content.trim().to_string();
            if content.chars().count() > MAX_MESSAGE_CHARS {
                content = format!("{}...", content.chars().take(MAX_MESSAGE_CHARS - 3).collect::<String>());
            }
            lines.push(format!("{}. {}: {}", index + 1, role, content));
        }
    }

    if !parsed.tool_executions.is_empty() {
        let mut counts: HashMap<String, usize> = HashMap::new();
        for tool in &parsed.tool_executions {
            let key = tool.tool_name.clone();
            *counts.entry(key).or_insert(0) += 1;
        }
        let mut entries: Vec<(String, usize)> = counts.into_iter().collect();
        entries.sort_by(|a, b| b.1.cmp(&a.1));
        let summary = entries
            .into_iter()
            .take(MAX_TOOL_ITEMS)
            .map(|(name, count)| format!("{} x{}", name, count))
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(format!("Tool usage: {}", summary));
    }

    let user_prompt = lines.join("\n");

    vec![
        ChatMessage {
            role: "system".to_string(),
            content: SESSION_SYSTEM_PROMPT.to_string(),
        },
        ChatMessage {
            role: "user".to_string(),
            content: user_prompt,
        },
    ]
}

pub fn summarize_session(
    client: &AIClient,
    parsed: &ParsedSession,
) -> Result<SessionSummary, AIError> {
    let messages = build_session_prompt(parsed);
    let content = client.create_chat_completion(messages)?;
    let fields = parse_session_summary_fields(&content)?;

    let metrics = build_metrics(parsed);

    Ok(SessionSummary {
        task_overview: fields.task_overview,
        short_summary: fields.short_summary,
        bullet_points: fields.bullet_points,
        metrics,
        last_updated: Some(SystemTime::now()),
    })
}

fn build_metrics(parsed: &ParsedSession) -> SessionMetrics {
    let token_count = estimate_token_count(&parsed.messages);
    let elapsed_seconds = match (parsed.started_at, parsed.last_updated_at) {
        (Some(start), Some(end)) => {
            let duration = end.signed_duration_since(start);
            duration.num_seconds().max(0) as u64
        }
        _ => 0,
    };

    SessionMetrics {
        token_count: if token_count > 0 { Some(token_count) } else { None },
        tool_execution_count: parsed.tool_executions.len(),
        elapsed_seconds: if elapsed_seconds > 0 { Some(elapsed_seconds) } else { None },
        turn_count: if parsed.total_turns > 0 {
            parsed.total_turns
        } else {
            parsed.messages.len()
        },
    }
}

fn estimate_token_count(messages: &[SessionMessage]) -> usize {
    let total_chars: usize = messages.iter().map(|m| m.content.chars().count()).sum();
    if total_chars == 0 {
        return 0;
    }
    (total_chars / 4).max(1)
}

fn parse_session_summary_fields(content: &str) -> Result<SessionSummaryFields, AIError> {
    if let Some(fields) = parse_json_summary(content) {
        return Ok(fields);
    }

    let bullet_points = parse_summary_lines(content).unwrap_or_default();
    let short_summary = bullet_points
        .get(0)
        .map(|line| line.trim_start_matches("- ").to_string());

    Ok(SessionSummaryFields {
        task_overview: None,
        short_summary,
        bullet_points,
    })
}

fn parse_json_summary(content: &str) -> Option<SessionSummaryFields> {
    let candidate = content.trim();
    if candidate.starts_with('{') {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(candidate) {
            return extract_fields_from_json(&value);
        }
    }

    if let Some((start, end)) = find_json_bounds(candidate) {
        let slice = &candidate[start..=end];
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(slice) {
            return extract_fields_from_json(&value);
        }
    }

    None
}

fn extract_fields_from_json(value: &serde_json::Value) -> Option<SessionSummaryFields> {
    let task_overview = value
        .get("task_overview")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let short_summary = value
        .get("short_summary")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let bullets_value = value
        .get("bullets")
        .or_else(|| value.get("bullet_points"))
        .or_else(|| value.get("bulletPoints"));

    let mut bullet_points = Vec::new();
    if let Some(bullets) = bullets_value {
        if let Some(arr) = bullets.as_array() {
            for item in arr {
                if let Some(text) = item.as_str() {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        bullet_points.push(normalize_bullet(trimmed));
                    }
                }
            }
        } else if let Some(text) = bullets.as_str() {
            if let Ok(lines) = parse_summary_lines(text) {
                bullet_points = lines;
            }
        }
    }

    if bullet_points.len() > 3 {
        bullet_points.truncate(3);
    }

    Some(SessionSummaryFields {
        task_overview,
        short_summary,
        bullet_points,
    })
}

fn normalize_bullet(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.starts_with("- ") {
        trimmed.to_string()
    } else {
        format!("- {}", trimmed)
    }
}

fn find_json_bounds(value: &str) -> Option<(usize, usize)> {
    let start = value.find('{')?;
    let end = value.rfind('}')?;
    if start < end {
        Some((start, end))
    } else {
        None
    }
}

pub fn parse_summary_lines(content: &str) -> Result<Vec<String>, AIError> {
    let mut lines: Vec<String> = content.lines().filter_map(normalize_line).collect();

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
    } else if let Some(rest) = trimmed.strip_prefix('-') {
        rest.trim()
    } else if let Some(rest) = trimmed.strip_prefix("* ") {
        rest.trim()
    } else if let Some(rest) = trimmed.strip_prefix('*') {
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

    #[test]
    fn test_session_summary_cache_stale_session_id() {
        let mut cache = SessionSummaryCache::default();
        let summary = SessionSummary::default();
        let now = SystemTime::now();
        cache.set("main".to_string(), "sess-1".to_string(), summary, now);
        assert!(cache.is_stale("main", "sess-2", now));
    }
}
