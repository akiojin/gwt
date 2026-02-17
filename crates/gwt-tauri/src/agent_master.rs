use gwt_core::ai::{AIClient, AIResponse, ChatMessage, ToolCall};
use gwt_core::config::{ProfilesConfig, Settings};
use gwt_core::git::{
    find_spec_issue_by_spec_id, sync_issue_to_project, upsert_spec_issue, SpecIssueSections,
    SpecProjectPhase,
};
use serde::Serialize;
use std::path::Path;

use crate::agent_tools::{builtin_tool_definitions, execute_tool_call};
use crate::commands::project::resolve_repo_path_for_project_root;
use crate::state::AppState;

const MAX_TOOL_CALL_LOOPS: usize = 3;
const SYSTEM_PROMPT: &str = "You are the master agent for gwt. Use ReAct and tool calls to send instructions to agent panes and capture output when needed. Keep instructions concise and in English.\n\nReAct format:\nThought: <short reasoning>\nAction: <tool name + short params summary>\nObservation: <tool result>\n\nRules:\n- Use tool calls for actions.\n- Do not fabricate observations; observations come from tool results.\n- Keep Thought to 2-4 lines.\n- Keep spec artifacts in GitHub Issues (Issue-first). Do not generate local spec markdown files.\n- Maintain the full bundle: spec, plan, tasks, tdd, research, data-model, quickstart, contracts, checklists.\n- Keep contracts/checklists as issue comments via artifact tools.\n- When delegating to sub-agents, include a clear task and request a short completion summary.";

#[derive(Debug, Clone, Serialize)]
pub struct AgentModeMessage {
    pub role: String,
    pub kind: String,
    pub content: String,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentModeState {
    pub messages: Vec<AgentModeMessage>,
    pub ai_ready: bool,
    pub ai_error: Option<String>,
    pub last_error: Option<String>,
    pub is_waiting: bool,
    pub session_name: Option<String>,
    pub llm_call_count: u64,
    pub estimated_tokens: u64,
    pub active_spec_id: Option<String>,
    pub active_spec_issue_number: Option<u64>,
    pub active_spec_issue_url: Option<String>,
    pub active_spec_issue_etag: Option<String>,
}

#[derive(Debug, Clone)]
struct IssueSpecPreparation {
    spec_id: String,
    issue_number: u64,
    issue_url: String,
    etag: String,
    created: bool,
}

impl AgentModeState {
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
            ai_ready: false,
            ai_error: None,
            last_error: None,
            is_waiting: false,
            session_name: Some("Master Agent".to_string()),
            llm_call_count: 0,
            estimated_tokens: 0,
            active_spec_id: None,
            active_spec_issue_number: None,
            active_spec_issue_url: None,
            active_spec_issue_etag: None,
        }
    }
}

pub fn get_agent_mode_state(state: &AppState, window_label: &str) -> AgentModeState {
    let guard = match state.window_agent_modes.lock() {
        Ok(g) => g,
        Err(_) => return AgentModeState::new(),
    };
    let mut mode = guard
        .get(window_label)
        .cloned()
        .unwrap_or_else(initial_agent_mode_state);
    mode.messages.retain(|m| m.role != "system");
    mode
}

fn save_agent_mode_state(state: &AppState, window_label: &str, mode_state: &AgentModeState) {
    if let Ok(mut guard) = state.window_agent_modes.lock() {
        guard.insert(window_label.to_string(), mode_state.clone());
    }
}

fn initial_agent_mode_state() -> AgentModeState {
    let mut state = AgentModeState::new();
    match ProfilesConfig::load() {
        Ok(profiles) => {
            let ai = profiles.resolve_active_ai_settings();
            if ai.resolved.is_some() {
                state.ai_ready = true;
                state.ai_error = None;
            } else {
                state.ai_ready = false;
                state.ai_error = Some("AI settings are required.".to_string());
            }
        }
        Err(e) => {
            state.ai_ready = false;
            state.ai_error = Some(e.to_string());
        }
    }
    state
}

pub fn send_agent_message(state: &AppState, window_label: &str, input: &str) -> AgentModeState {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return get_agent_mode_state(state, window_label);
    }

    let mut working = get_agent_mode_state(state, window_label);
    working.messages.retain(|m| m.role != "system");
    working.last_error = None;
    working.is_waiting = true;
    push_message(&mut working, "user", "message", trimmed);
    save_agent_mode_state(state, window_label, &working);

    let profiles = match ProfilesConfig::load() {
        Ok(p) => p,
        Err(e) => {
            working.ai_ready = false;
            working.ai_error = Some(e.to_string());
            working.is_waiting = false;
            save_agent_mode_state(state, window_label, &working);
            return working;
        }
    };

    let ai = profiles.resolve_active_ai_settings();
    let Some(settings) = ai.resolved else {
        working.ai_ready = false;
        working.ai_error = Some("AI settings are required.".to_string());
        working.is_waiting = false;
        save_agent_mode_state(state, window_label, &working);
        return working;
    };
    working.ai_ready = true;
    working.ai_error = None;
    save_agent_mode_state(state, window_label, &working);

    let prep = match prepare_issue_spec_for_window(
        state,
        window_label,
        trimmed,
        working.active_spec_id.as_deref(),
    ) {
        Ok(p) => p,
        Err(e) => {
            working.last_error = Some(e);
            working.is_waiting = false;
            save_agent_mode_state(state, window_label, &working);
            return working;
        }
    };
    working.active_spec_id = Some(prep.spec_id.clone());
    working.active_spec_issue_number = Some(prep.issue_number);
    working.active_spec_issue_url = Some(prep.issue_url.clone());
    working.active_spec_issue_etag = Some(prep.etag.clone());
    let note = if prep.created {
        format!(
            "Prepared issue-first spec {} as #{} ({})",
            prep.spec_id, prep.issue_number, prep.issue_url
        )
    } else {
        format!(
            "Updated issue-first spec {} on #{} ({})",
            prep.spec_id, prep.issue_number, prep.issue_url
        )
    };
    push_message(&mut working, "assistant", "observation", &note);
    save_agent_mode_state(state, window_label, &working);

    let client = match AIClient::new(settings) {
        Ok(c) => c,
        Err(e) => {
            working.last_error = Some(e.to_string());
            working.is_waiting = false;
            save_agent_mode_state(state, window_label, &working);
            return working;
        }
    };

    let mut loops = 0usize;
    let mut messages = build_chat_messages(&working.messages);

    loop {
        loops += 1;
        let response =
            match client.create_response_with_tools(messages.clone(), builtin_tool_definitions()) {
                Ok(r) => r,
                Err(e) => {
                    working.last_error = Some(e.to_string());
                    working.is_waiting = false;
                    save_agent_mode_state(state, window_label, &working);
                    return working;
                }
            };

        let has_tools = !response.tool_calls.is_empty();
        let has_action = apply_ai_response(&mut working, &response, !has_tools);
        save_agent_mode_state(state, window_label, &working);

        if response.tool_calls.is_empty() {
            break;
        }

        if !has_action {
            for call in &response.tool_calls {
                push_message(&mut working, "assistant", "action", &format_tool_call(call));
            }
        }
        let tool_observations = execute_tool_calls(state, window_label, &response.tool_calls);
        for obs in tool_observations {
            push_message(&mut working, "tool", "observation", &obs);
        }
        save_agent_mode_state(state, window_label, &working);
        messages = build_chat_messages(&working.messages);

        if loops >= MAX_TOOL_CALL_LOOPS {
            break;
        }
    }

    working.is_waiting = false;
    save_agent_mode_state(state, window_label, &working);
    working
}

fn apply_ai_response(
    state: &mut AgentModeState,
    response: &AIResponse,
    allow_observation: bool,
) -> bool {
    let parsed = parse_react_sections(&response.text, allow_observation);
    let mut has_action = false;
    if parsed.is_empty() && !response.text.trim().is_empty() {
        push_message(state, "assistant", "message", response.text.trim());
    } else {
        for section in parsed {
            if section.kind == "action" {
                has_action = true;
            }
            push_message(state, "assistant", section.kind, &section.content);
        }
    }
    if let Some(tokens) = response.usage_tokens {
        state.estimated_tokens = state.estimated_tokens.saturating_add(tokens);
    }
    state.llm_call_count = state.llm_call_count.saturating_add(1);
    has_action
}

fn execute_tool_calls(
    state: &AppState,
    window_label: &str,
    tool_calls: &[ToolCall],
) -> Vec<String> {
    let mut results = Vec::new();
    for call in tool_calls {
        let result =
            execute_tool_call(state, window_label, call).unwrap_or_else(|e| format!("error: {e}"));
        results.push(format!("{} => {}", call.name, result));
    }
    results
}

fn push_message(state: &mut AgentModeState, role: &str, kind: &str, content: &str) {
    state.messages.push(AgentModeMessage {
        role: role.to_string(),
        kind: kind.to_string(),
        content: content.to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
    });
}

fn build_chat_messages(messages: &[AgentModeMessage]) -> Vec<ChatMessage> {
    let mut out = Vec::with_capacity(messages.len() + 1);
    out.push(ChatMessage {
        role: "system".to_string(),
        content: SYSTEM_PROMPT.to_string(),
    });
    out.extend(
        messages
            .iter()
            .filter(|m| m.role != "system")
            .map(|m| ChatMessage {
                role: m.role.clone(),
                content: m.content.clone(),
            }),
    );
    out
}

fn prepare_issue_spec_for_window(
    state: &AppState,
    window_label: &str,
    user_input: &str,
    preferred_spec_id: Option<&str>,
) -> Result<IssueSpecPreparation, String> {
    let Some(project_path) = state.project_for_window(window_label) else {
        return Err("Open a project before using Master Agent.".to_string());
    };
    prepare_issue_spec(Path::new(&project_path), user_input, preferred_spec_id)
}

fn prepare_issue_spec(
    project_root: &Path,
    user_input: &str,
    preferred_spec_id: Option<&str>,
) -> Result<IssueSpecPreparation, String> {
    let repo_path = resolve_repo_path_for_project_root(project_root)?;
    let spec_id = extract_spec_id(user_input)
        .or_else(|| preferred_spec_id.map(str::to_string))
        .unwrap_or_else(generate_spec_id);
    let existing = find_spec_issue_by_spec_id(&repo_path, &spec_id)?;
    let created = existing.is_none();
    let title = build_issue_title(
        &spec_id,
        user_input,
        existing.as_ref().map(|e| e.title.as_str()),
    );
    let sections = if let Some(current) = existing.as_ref() {
        let mut sections = current.sections.clone();
        sections.spec = merge_spec_section(&sections.spec, user_input);
        sections
    } else {
        SpecIssueSections {
            spec: collapse_whitespace(user_input),
            plan: String::new(),
            tasks: String::new(),
            tdd: String::new(),
            research: String::new(),
            data_model: String::new(),
            quickstart: String::new(),
            contracts: String::new(),
            checklists: String::new(),
        }
    };

    let detail = upsert_spec_issue(
        &repo_path,
        &spec_id,
        &title,
        &sections,
        existing.as_ref().map(|e| e.etag.as_str()),
    )?;

    let settings = Settings::load(project_root).unwrap_or_default();
    if let Some(project_id) = settings.agent.github_project_id {
        let _ = sync_issue_to_project(
            &repo_path,
            detail.number,
            project_id.trim(),
            SpecProjectPhase::Draft,
        );
    }

    Ok(IssueSpecPreparation {
        spec_id,
        issue_number: detail.number,
        issue_url: detail.url,
        etag: detail.etag,
        created,
    })
}

fn extract_spec_id(input: &str) -> Option<String> {
    for token in input.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-')) {
        if token.len() != 13 {
            continue;
        }
        if !token[..5].eq_ignore_ascii_case("SPEC-") {
            continue;
        }
        let suffix = &token[5..];
        if suffix.chars().all(|c| c.is_ascii_hexdigit()) {
            return Some(format!("SPEC-{}", suffix.to_ascii_lowercase()));
        }
    }
    None
}

fn generate_spec_id() -> String {
    let raw = uuid::Uuid::new_v4().simple().to_string();
    format!("SPEC-{}", &raw[..8])
}

fn collapse_whitespace(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn merge_spec_section(existing: &str, user_input: &str) -> String {
    let normalized = collapse_whitespace(user_input);
    if normalized.is_empty() {
        return existing.to_string();
    }
    let trimmed_existing = existing.trim();
    if trimmed_existing.is_empty() || trimmed_existing == "_TODO_" {
        return normalized;
    }
    if trimmed_existing.contains(&normalized) {
        return trimmed_existing.to_string();
    }
    format!("{trimmed_existing}\n\n- {normalized}")
}

fn build_issue_title(spec_id: &str, user_input: &str, existing_title: Option<&str>) -> String {
    if let Some(existing) = existing_title {
        if !existing.trim().is_empty() {
            return existing.trim().to_string();
        }
    }
    let base = collapse_whitespace(user_input);
    let mut title = if base.is_empty() {
        "Master Agent Task".to_string()
    } else {
        base
    };
    if title.chars().count() > 72 {
        title = title.chars().take(72).collect::<String>();
        title = title.trim_end().to_string();
    }
    format!("[{}] {}", spec_id, title)
}

struct ReactSection {
    kind: &'static str,
    content: String,
}

fn parse_react_sections(text: &str, allow_observation: bool) -> Vec<ReactSection> {
    let mut sections: Vec<ReactSection> = Vec::new();
    let mut current_kind: Option<&'static str> = None;
    let mut current_lines: Vec<String> = Vec::new();

    let flush =
        |kind: Option<&'static str>, lines: &mut Vec<String>, out: &mut Vec<ReactSection>| {
            if let Some(k) = kind {
                let content = lines.join("\n").trim().to_string();
                if !content.is_empty() {
                    out.push(ReactSection { kind: k, content });
                }
            }
            lines.clear();
        };

    for raw in text.lines() {
        let line = raw.trim_end();
        if let Some(rest) = line.strip_prefix("Thought:") {
            flush(current_kind, &mut current_lines, &mut sections);
            current_kind = Some("thought");
            current_lines.push(rest.trim().to_string());
            continue;
        }
        if let Some(rest) = line.strip_prefix("Action:") {
            flush(current_kind, &mut current_lines, &mut sections);
            current_kind = Some("action");
            current_lines.push(rest.trim().to_string());
            continue;
        }
        if allow_observation {
            if let Some(rest) = line.strip_prefix("Observation:") {
                flush(current_kind, &mut current_lines, &mut sections);
                current_kind = Some("observation");
                current_lines.push(rest.trim().to_string());
                continue;
            }
        }
        if line.strip_prefix("Observation:").is_some() {
            flush(current_kind, &mut current_lines, &mut sections);
            current_kind = None;
            continue;
        }
        if current_kind.is_some() {
            current_lines.push(line.to_string());
        }
    }
    flush(current_kind, &mut current_lines, &mut sections);
    sections
}

fn format_tool_call(call: &ToolCall) -> String {
    let args = match serde_json::to_string(&call.arguments) {
        Ok(s) => s,
        Err(_) => "{}".to_string(),
    };
    format!("{} {}", call.name, args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;

    #[test]
    fn build_chat_messages_adds_system_prompt_and_filters_system_messages() {
        let input = vec![
            AgentModeMessage {
                role: "system".to_string(),
                kind: "message".to_string(),
                content: "legacy system".to_string(),
                timestamp: 0,
            },
            AgentModeMessage {
                role: "user".to_string(),
                kind: "message".to_string(),
                content: "hello".to_string(),
                timestamp: 1,
            },
            AgentModeMessage {
                role: "assistant".to_string(),
                kind: "message".to_string(),
                content: "hi".to_string(),
                timestamp: 2,
            },
        ];
        let out = build_chat_messages(&input);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].role, "system");
        assert_eq!(out[0].content, SYSTEM_PROMPT);
        assert_eq!(out[1].role, "user");
        assert_eq!(out[2].role, "assistant");
    }

    #[test]
    fn extract_spec_id_parses_embedded_id() {
        let out = extract_spec_id("continue work on SPEC-ba3f610c please");
        assert_eq!(out, Some("SPEC-ba3f610c".to_string()));
    }

    #[test]
    fn prepare_issue_spec_for_window_requires_open_project() {
        let state = AppState::new();
        let err = prepare_issue_spec_for_window(&state, "main", "implement auth", None);
        assert!(err.is_err());
        assert!(err
            .unwrap_err()
            .contains("Open a project before using Master Agent."));
    }

    #[test]
    fn build_issue_title_prefers_existing_title() {
        let title = build_issue_title("SPEC-deadbeef", "new input", Some("Existing"));
        assert_eq!(title, "Existing");
    }

    #[test]
    fn build_issue_title_contains_spec_id_prefix() {
        let title = build_issue_title("SPEC-cafebabe", "Implement authentication flow", None);
        assert!(title.starts_with("[SPEC-cafebabe] "));
    }

    #[test]
    fn build_issue_title_handles_multibyte_without_panic() {
        // 72 chars total but >72 bytes (71 ASCII + 1 multibyte char)
        let input = format!("{}あ", "a".repeat(71));
        let title = build_issue_title("SPEC-cafebabe", &input, None);
        assert!(title.contains('あ'));
    }

    #[test]
    fn generate_spec_id_has_expected_shape() {
        let id = generate_spec_id();
        assert_eq!(id.len(), 13);
        assert!(id.starts_with("SPEC-"));
    }

    #[test]
    fn merge_spec_section_replaces_todo() {
        let merged = merge_spec_section("_TODO_", "implement oauth flow");
        assert_eq!(merged, "implement oauth flow");
    }

    #[test]
    fn merge_spec_section_appends_unique_input() {
        let merged = merge_spec_section("existing context", "new requirement");
        assert_eq!(merged, "existing context\n\n- new requirement");
    }
}
