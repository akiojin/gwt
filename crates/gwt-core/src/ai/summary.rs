//! Session summary generation and cache.

use super::client::{AIClient, AIError, ChatMessage};
use super::session_parser::{MessageRole, ParsedSession, SessionMessage};
use std::collections::HashMap;
use std::time::SystemTime;

pub const SESSION_SYSTEM_PROMPT_BASE: &str = "You are a helpful assistant summarizing a coding agent session.\nReturn Markdown only with the following format and headings, in this exact order:\n\n## 目的\n<1 sentence>\n\n## 要約\n<1-2 sentences>\n\n## ハイライト\n- <bullet 1>\n- <bullet 2>\n- <bullet 3>\n\nDetect the response language from the session content and respond in that language.\nIf the session contains multiple languages, use the language used by the user messages.\nDo not output JSON, code fences, or any extra text.";

const MAX_MESSAGE_CHARS: usize = 220;
const MAX_TOOL_ITEMS: usize = 8;
const MAX_PROMPT_CHARS: usize = 8000;

#[derive(Debug, Clone, Default)]
pub struct SessionSummary {
    pub task_overview: Option<String>,
    pub short_summary: Option<String>,
    pub bullet_points: Vec<String>,
    pub markdown: Option<String>,
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
        let mut used_chars = lines.join("\n").chars().count();
        let mut truncated = false;
        for (index, message) in parsed.messages.iter().enumerate() {
            let role = match message.role {
                MessageRole::User => "user",
                MessageRole::Assistant => "assistant",
            };
            let mut content = message.content.trim().to_string();
            if content.chars().count() > MAX_MESSAGE_CHARS {
                content = format!(
                    "{}...",
                    content
                        .chars()
                        .take(MAX_MESSAGE_CHARS - 3)
                        .collect::<String>()
                );
            }
            let line = format!("{}. {}: {}", index + 1, role, content);
            let line_len = line.chars().count() + 1; // +1 for newline
            if used_chars + line_len > MAX_PROMPT_CHARS {
                truncated = true;
                break;
            }
            lines.push(line);
            used_chars += line_len;
        }

        if truncated {
            let notice = "Messages truncated due to length.";
            if used_chars + notice.chars().count() < MAX_PROMPT_CHARS {
                lines.push(notice.to_string());
            }
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
        let tool_line = format!("Tool usage: {}", summary);
        let current_len = lines.join("\n").chars().count();
        if current_len + tool_line.chars().count() < MAX_PROMPT_CHARS {
            lines.push(tool_line);
        } else if current_len > MAX_PROMPT_CHARS {
            // Ensure we never exceed the cap even if previous sections were already too long.
            let truncated = lines.join("\n");
            let shortened = truncated.chars().take(MAX_PROMPT_CHARS).collect::<String>();
            lines.clear();
            lines.push(shortened);
        }
    }

    let mut user_prompt = lines.join("\n");
    if user_prompt.chars().count() > MAX_PROMPT_CHARS {
        user_prompt = user_prompt
            .chars()
            .take(MAX_PROMPT_CHARS)
            .collect::<String>();
    }

    vec![
        ChatMessage {
            role: "system".to_string(),
            content: SESSION_SYSTEM_PROMPT_BASE.to_string(),
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
    let content = client.create_response(messages)?;
    let fields = parse_session_summary_fields(&content).unwrap_or_default();
    let markdown = normalize_session_summary_markdown(&content, &fields)?;
    validate_session_summary_markdown(&markdown)?;

    let metrics = build_metrics(parsed);

    Ok(SessionSummary {
        task_overview: fields.task_overview,
        short_summary: fields.short_summary,
        bullet_points: fields.bullet_points,
        markdown: Some(markdown),
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
        token_count: if token_count > 0 {
            Some(token_count)
        } else {
            None
        },
        tool_execution_count: parsed.tool_executions.len(),
        elapsed_seconds: if elapsed_seconds > 0 {
            Some(elapsed_seconds)
        } else {
            None
        },
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
        .first()
        .map(|line| line.trim_start_matches("- ").to_string());

    Ok(SessionSummaryFields {
        task_overview: None,
        short_summary,
        bullet_points,
    })
}

fn normalize_session_summary_markdown(
    content: &str,
    fields: &SessionSummaryFields,
) -> Result<String, AIError> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err(AIError::ParseError("Empty summary".to_string()));
    }

    if parse_json_summary(trimmed).is_some() {
        return Ok(build_markdown_from_fields(fields));
    }

    if looks_like_markdown(trimmed) {
        return Ok(trimmed.to_string());
    }

    Ok(build_markdown_from_fields(fields))
}

fn validate_session_summary_markdown(markdown: &str) -> Result<(), AIError> {
    let mut stage = SummaryStage::Start;
    let mut has_bullet = false;

    for line in markdown.lines() {
        let trimmed = line.trim();
        if let Some(title) = trimmed.strip_prefix("## ") {
            let title = title.trim();
            if heading_matches(title, "目的") {
                if stage != SummaryStage::Start {
                    return Err(AIError::IncompleteSummary);
                }
                stage = SummaryStage::Purpose;
                continue;
            }
            if heading_matches(title, "要約") {
                if stage != SummaryStage::Purpose {
                    return Err(AIError::IncompleteSummary);
                }
                stage = SummaryStage::Summary;
                continue;
            }
            if heading_matches(title, "ハイライト") {
                if stage != SummaryStage::Summary {
                    return Err(AIError::IncompleteSummary);
                }
                stage = SummaryStage::Highlight;
                continue;
            }
            if stage == SummaryStage::Highlight {
                stage = SummaryStage::Done;
            }
            continue;
        }

        if stage == SummaryStage::Highlight && is_bullet_line(trimmed) {
            has_bullet = true;
        }
    }

    if stage == SummaryStage::Highlight || stage == SummaryStage::Done {
        if has_bullet {
            return Ok(());
        }
    }

    Err(AIError::IncompleteSummary)
}

fn heading_matches(title: &str, expected: &str) -> bool {
    let trimmed = title.trim();
    if trimmed == expected {
        return true;
    }
    let Some(rest) = trimmed.strip_prefix(expected) else {
        return false;
    };
    let rest = rest.trim_start();
    rest.is_empty()
        || rest.starts_with('(')
        || rest.starts_with('（')
        || rest.starts_with(':')
        || rest.starts_with('：')
}

fn is_bullet_line(line: &str) -> bool {
    if line.starts_with("- ") || line.starts_with("* ") || line.starts_with("•") {
        return true;
    }
    strip_ordered_prefix(line).is_some()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SummaryStage {
    Start,
    Purpose,
    Summary,
    Highlight,
    Done,
}

fn looks_like_markdown(content: &str) -> bool {
    content.contains("## ")
        || content.contains("\n- ")
        || content.contains("\n* ")
        || content.contains("\n1.")
        || content.contains("\n1)")
}

fn build_markdown_from_fields(fields: &SessionSummaryFields) -> String {
    let purpose = fields
        .task_overview
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or("(Not available)");
    let summary = fields
        .short_summary
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or("(Not available)");

    let mut out = String::new();
    out.push_str("## 目的\n");
    out.push_str(purpose);
    out.push_str("\n\n## 要約\n");
    out.push_str(summary);
    out.push_str("\n\n## ハイライト\n");
    if fields.bullet_points.is_empty() {
        out.push_str("- (No highlights)\n");
    } else {
        for bullet in fields.bullet_points.iter().take(3) {
            let line = bullet.trim_start_matches("- ").trim();
            out.push_str("- ");
            out.push_str(line);
            out.push('\n');
        }
    }
    out
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

    #[test]
    fn test_build_session_prompt_caps_length() {
        let long_text = "a".repeat(2000);
        let messages = (0..200)
            .map(|_| SessionMessage {
                role: MessageRole::User,
                content: long_text.clone(),
                timestamp: None,
            })
            .collect::<Vec<_>>();
        let parsed = ParsedSession {
            session_id: "sess-1".to_string(),
            agent_type: crate::ai::AgentType::CodexCli,
            messages,
            tool_executions: vec![],
            started_at: None,
            last_updated_at: None,
            total_turns: 200,
        };

        let prompt = build_session_prompt(&parsed);
        let user_prompt = prompt
            .iter()
            .find(|msg| msg.role == "user")
            .expect("user prompt")
            .content
            .clone();

        assert!(user_prompt.chars().count() <= MAX_PROMPT_CHARS);
    }

    #[test]
    fn test_normalize_session_summary_markdown_from_json() {
        let content =
            r#"{"task_overview":"目的文","short_summary":"要約文","bullets":["項目1","項目2"]}"#;
        let fields = parse_session_summary_fields(content).expect("parse fields");
        let markdown = normalize_session_summary_markdown(content, &fields).expect("markdown");
        assert!(markdown.contains("## 目的"));
        assert!(markdown.contains("目的文"));
        assert!(markdown.contains("## 要約"));
        assert!(markdown.contains("要約文"));
        assert!(markdown.contains("## ハイライト"));
        assert!(markdown.contains("- 項目1"));
    }

    #[test]
    fn test_normalize_session_summary_markdown_passthrough() {
        let content = "## 目的\nA\n\n## 要約\nB\n\n## ハイライト\n- C";
        let fields = SessionSummaryFields::default();
        let markdown = normalize_session_summary_markdown(content, &fields).expect("markdown");
        assert_eq!(markdown, content);
    }

    #[test]
    fn test_validate_session_summary_markdown_accepts_complete() {
        let content = "## 目的\nA\n\n## 要約\nB\n\n## ハイライト\n- C";
        assert!(validate_session_summary_markdown(content).is_ok());
    }

    #[test]
    fn test_validate_session_summary_markdown_rejects_missing_highlight() {
        let content = "## 目的\nA\n\n## 要約\nB\n";
        let result = validate_session_summary_markdown(content);
        assert!(matches!(result, Err(AIError::IncompleteSummary)));
    }

    #[test]
    fn test_validate_session_summary_markdown_rejects_missing_bullets() {
        let content = "## 目的\nA\n\n## 要約\nB\n\n## ハイライト\n";
        let result = validate_session_summary_markdown(content);
        assert!(matches!(result, Err(AIError::IncompleteSummary)));
    }
}
