use gwt_core::ai::{AIClient, AIResponse, ChatMessage, ToolCall};
use gwt_core::config::ProfilesConfig;
use serde::Serialize;

use crate::agent_tools::{builtin_tool_definitions, execute_tool_call};
use crate::state::AppState;

const MAX_TOOL_CALL_LOOPS: usize = 3;
const SYSTEM_PROMPT: &str = "You are the master agent for gwt. Use ReAct and tool calls to send instructions to agent panes and capture output when needed. Keep instructions concise and in English.\n\nReAct format:\nThought: <short reasoning>\nAction: <tool name + short params summary>\nObservation: <tool result>\n\nRules:\n- Use tool calls for actions.\n- Do not fabricate observations; observations come from tool results.\n- Keep Thought to 2-4 lines.\n- When delegating to sub-agents, include a clear task and request a short completion summary.";

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
}

impl AgentModeState {
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
            ai_ready: false,
            ai_error: None,
            last_error: None,
            is_waiting: false,
            session_name: Some("Agent Mode".to_string()),
            llm_call_count: 0,
            estimated_tokens: 0,
        }
    }
}

pub fn get_agent_mode_state(state: &AppState, window_label: &str) -> AgentModeState {
    let guard = match state.window_agent_modes.lock() {
        Ok(g) => g,
        Err(_) => return AgentModeState::new(),
    };
    guard
        .get(window_label)
        .cloned()
        .unwrap_or_else(AgentModeState::new)
}

fn save_agent_mode_state(state: &AppState, window_label: &str, mode_state: &AgentModeState) {
    if let Ok(mut guard) = state.window_agent_modes.lock() {
        guard.insert(window_label.to_string(), mode_state.clone());
    }
}

pub fn send_agent_message(state: &AppState, window_label: &str, input: &str) -> AgentModeState {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return get_agent_mode_state(state, window_label);
    }

    let mut working = get_agent_mode_state(state, window_label);
    working.last_error = None;
    working.is_waiting = true;
    if working.messages.is_empty() {
        push_message(&mut working, "system", "message", SYSTEM_PROMPT);
    }
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
        let tool_observations = execute_tool_calls(state, &response.tool_calls);
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

fn execute_tool_calls(state: &AppState, tool_calls: &[ToolCall]) -> Vec<String> {
    let mut results = Vec::new();
    for call in tool_calls {
        let result = execute_tool_call(state, call)
            .map(|r| r)
            .unwrap_or_else(|e| format!("error: {e}"));
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
    messages
        .iter()
        .map(|m| ChatMessage {
            role: m.role.clone(),
            content: m.content.clone(),
        })
        .collect()
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

    #[test]
    fn build_chat_messages_preserves_roles() {
        let input = vec![
            AgentModeMessage {
                role: "user".to_string(),
                kind: "message".to_string(),
                content: "hello".to_string(),
                timestamp: 0,
            },
            AgentModeMessage {
                role: "assistant".to_string(),
                kind: "message".to_string(),
                content: "hi".to_string(),
                timestamp: 1,
            },
        ];
        let out = build_chat_messages(&input);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].role, "user");
        assert_eq!(out[1].role, "assistant");
    }
}
