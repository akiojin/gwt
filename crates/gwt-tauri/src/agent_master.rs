// Many functions are implemented ahead of their call sites (wired in Phase 9-12).
#![allow(dead_code)]

use gwt_core::agent::conversation::MessageRole;
use gwt_core::agent::developer::AgentType;
use gwt_core::agent::lead::{LeadMessage, LeadStatus, MessageKind};
use gwt_core::agent::session::{ProjectModeSession, SessionStatus};
use gwt_core::agent::session_store::SessionStoreError;
use gwt_core::agent::task::{PullRequestRef, Task, TestStatus, TestVerification};
use gwt_core::agent::types::SessionId;
use gwt_core::ai::{AIClient, AIResponse, ChatMessage, ToolCall};
use gwt_core::config::ProfilesConfig;
use gwt_core::git::{
    get_spec_issue_detail, sync_issue_to_project, upsert_spec_issue, SpecIssueSections,
    SpecProjectPhase,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::agent_tools::{builtin_tool_definitions, execute_tool_call};
use crate::commands::project::resolve_repo_path_for_project_root;
use crate::state::AppState;

const MAX_TOOL_CALL_LOOPS: usize = 3;
const SYSTEM_PROMPT: &str = "You are the master agent for gwt. Use ReAct and tool calls to send instructions to agent panes and capture output when needed. Keep instructions concise and in English.\n\nReAct format:\nThought: <short reasoning>\nAction: <tool name + short params summary>\nObservation: <tool result>\n\nRules:\n- Use tool calls for actions.\n- Do not fabricate observations; observations come from tool results.\n- Keep Thought to 2-4 lines.\n- Keep spec artifacts in GitHub Issues (Issue-first) only.\n- Maintain the full bundle: spec, plan, tasks, tdd, research, data-model, quickstart, contracts, checklists.\n- Keep contracts/checklists as issue comments via artifact tools.\n- When delegating to sub-agents, include a clear task and request a short completion summary.";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectModeMessage {
    pub role: String,
    pub kind: String,
    pub content: String,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectModeState {
    pub messages: Vec<ProjectModeMessage>,
    pub ai_ready: bool,
    pub ai_error: Option<String>,
    pub last_error: Option<String>,
    pub is_waiting: bool,
    pub session_name: Option<String>,
    pub llm_call_count: u64,
    pub estimated_tokens: u64,
    pub active_spec_issue_number: Option<u64>,
    pub active_spec_issue_url: Option<String>,
    pub active_spec_issue_etag: Option<String>,
    /// Project Mode session ID (None when in Branch Mode)
    #[serde(default)]
    pub project_mode_session_id: Option<String>,
    /// Lead AI status indicator for Project Mode
    #[serde(default)]
    pub lead_status: Option<String>,
}

#[derive(Debug, Clone)]
struct IssueSpecPreparation {
    issue_number: u64,
    issue_url: String,
    etag: String,
    created: bool,
}

impl ProjectModeState {
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
            ai_ready: false,
            ai_error: None,
            last_error: None,
            is_waiting: false,
            session_name: Some("Project Mode".to_string()),
            llm_call_count: 0,
            estimated_tokens: 0,
            active_spec_issue_number: None,
            active_spec_issue_url: None,
            active_spec_issue_etag: None,
            project_mode_session_id: None,
            lead_status: None,
        }
    }
}

pub fn get_project_mode_state(state: &AppState, window_label: &str) -> ProjectModeState {
    let guard = match state.window_project_modes.lock() {
        Ok(g) => g,
        Err(_) => return ProjectModeState::new(),
    };
    let mut mode = guard
        .get(window_label)
        .cloned()
        .unwrap_or_else(initial_project_mode_state);
    mode.messages.retain(|m| m.role != "system");
    mode
}

fn save_window_project_mode_state(
    state: &AppState,
    window_label: &str,
    mode_state: &ProjectModeState,
) {
    if let Ok(mut guard) = state.window_project_modes.lock() {
        guard.insert(window_label.to_string(), mode_state.clone());
    }
}

fn initial_project_mode_state() -> ProjectModeState {
    let mut state = ProjectModeState::new();
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

fn send_project_mode_message_legacy(
    state: &AppState,
    window_label: &str,
    input: &str,
) -> ProjectModeState {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return get_project_mode_state(state, window_label);
    }

    let mut working = get_project_mode_state(state, window_label);
    working.messages.retain(|m| m.role != "system");
    working.last_error = None;
    working.is_waiting = true;
    push_message(&mut working, "user", "message", trimmed);
    save_window_project_mode_state(state, window_label, &working);

    let profiles = match ProfilesConfig::load() {
        Ok(p) => p,
        Err(e) => {
            working.ai_ready = false;
            working.ai_error = Some(e.to_string());
            working.is_waiting = false;
            save_window_project_mode_state(state, window_label, &working);
            return working;
        }
    };

    let ai = profiles.resolve_active_ai_settings();
    let Some(settings) = ai.resolved else {
        working.ai_ready = false;
        working.ai_error = Some("AI settings are required.".to_string());
        working.is_waiting = false;
        save_window_project_mode_state(state, window_label, &working);
        return working;
    };
    working.ai_ready = true;
    working.ai_error = None;
    save_window_project_mode_state(state, window_label, &working);

    let prep = match prepare_issue_spec_for_window(
        state,
        window_label,
        trimmed,
        working.active_spec_issue_number,
    ) {
        Ok(p) => p,
        Err(e) => {
            working.last_error = Some(e);
            working.is_waiting = false;
            save_window_project_mode_state(state, window_label, &working);
            return working;
        }
    };
    working.active_spec_issue_number = Some(prep.issue_number);
    working.active_spec_issue_url = Some(prep.issue_url.clone());
    working.active_spec_issue_etag = Some(prep.etag.clone());
    let note = if prep.created {
        format!(
            "Prepared issue-first spec #{} ({})",
            prep.issue_number, prep.issue_url
        )
    } else {
        format!(
            "Updated issue-first spec #{} ({})",
            prep.issue_number, prep.issue_url
        )
    };
    push_message(&mut working, "assistant", "observation", &note);
    save_window_project_mode_state(state, window_label, &working);

    let client = match AIClient::new(settings) {
        Ok(c) => c,
        Err(e) => {
            working.last_error = Some(e.to_string());
            working.is_waiting = false;
            save_window_project_mode_state(state, window_label, &working);
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
                    save_window_project_mode_state(state, window_label, &working);
                    return working;
                }
            };

        let has_tools = !response.tool_calls.is_empty();
        let has_action = apply_ai_response(&mut working, &response, !has_tools);
        save_window_project_mode_state(state, window_label, &working);

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
        save_window_project_mode_state(state, window_label, &working);
        messages = build_chat_messages(&working.messages);

        if loops >= MAX_TOOL_CALL_LOOPS {
            break;
        }
    }

    working.is_waiting = false;
    save_window_project_mode_state(state, window_label, &working);
    working
}

const PROJECT_MODE_SYSTEM_PROMPT: &str = "You are the Lead AI agent for gwt Project Mode. You orchestrate Coordinators and Developers to implement features. Use ReAct and tool calls when needed. Keep instructions concise and in English.

ReAct format:
Thought: <short reasoning>
Action: <tool name + short params summary>
Observation: <tool result>

Rules:
- Use tool calls for actions.
- Do not fabricate observations; observations come from tool results.
- Keep Thought to 2-4 lines.
- When delegating to sub-agents, include a clear task and request a short completion summary.

## Issue-First Spec Management

All specifications must be stored as GitHub Issues using the issue_spec tools:
- `upsert_spec_issue` - Create or update a spec issue with sections
- `get_spec_issue` - Read current spec issue content
- `upsert_spec_issue_artifact` - Add contract/checklist artifacts as comments
- `list_spec_issue_artifacts` - List artifact comments on an issue
- `delete_spec_issue_artifact` - Remove an artifact comment
- `sync_spec_issue_project` - Sync issue to GitHub Project board

## Workflow

Follow this workflow for every feature request:

1. **Clarify**: Ask the user to clarify ambiguous requirements. Gather enough context before proceeding.
2. **Create GitHub Issue**: Use `upsert_spec_issue` to create a new spec issue. Use the issue number as the only spec identifier and set a clear title.
3. **Write 4 required sections**: Update the issue with all 4 gate sections via `upsert_spec_issue`:
   - `spec` - Functional requirements and acceptance scenarios
   - `plan` - Implementation plan with approach and architecture decisions
   - `tasks` - Breakdown of work items with clear deliverables
   - `tdd` - Test-first definitions and test scenarios
4. **Gate check**: All 4 sections (spec, plan, tasks, tdd) must be non-empty before requesting user approval.
5. **Present plan for approval**: Summarize the plan to the user and ask for explicit approval. Wait for the user to approve before starting Coordinators.
6. **On approval**: Transition to Orchestrating and start Coordinators for each task.
7. **On rejection**: Revise the plan based on user feedback and re-present.

Do not start Coordinators or Developers until the user has explicitly approved the plan.";

pub fn send_project_mode_message(
    state: &AppState,
    window_label: &str,
    input: &str,
) -> ProjectModeState {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return get_project_mode_state(state, window_label);
    }

    let mut working = get_project_mode_state(state, window_label);
    working.messages.retain(|m| m.role != "system");
    working.last_error = None;
    working.is_waiting = true;
    working.lead_status = Some("thinking".to_string());
    push_message(&mut working, "user", "message", trimmed);
    save_window_project_mode_state(state, window_label, &working);

    // Resolve project path for session persistence
    let project_path = match state.project_for_window(window_label) {
        Some(p) => PathBuf::from(p),
        None => {
            working.last_error = Some("Open a project before using Project Mode.".to_string());
            working.is_waiting = false;
            working.lead_status = Some("idle".to_string());
            save_window_project_mode_state(state, window_label, &working);
            return working;
        }
    };

    // Load or create ProjectModeSession
    let session_id = working
        .project_mode_session_id
        .clone()
        .unwrap_or_else(|| SessionId::new().0);
    let sid = SessionId(session_id.clone());

    let mut session = match state.session_store.load_project_mode(&sid) {
        Ok(session) => session,
        Err(SessionStoreError::NotFound) => {
            ProjectModeSession::new(sid.clone(), project_path, "main", AgentType::Claude)
        }
        Err(e) => {
            working.last_error = Some(format!("Failed to load project mode session: {e}"));
            working.is_waiting = false;
            working.lead_status = Some("idle".to_string());
            save_window_project_mode_state(state, window_label, &working);
            return working;
        }
    };

    working.project_mode_session_id = Some(session_id);
    save_window_project_mode_state(state, window_label, &working);

    activate_session_for_message(&mut session, trimmed);
    session.touch();
    let _ = state.session_store.save_project_mode(&session);

    // Load AI settings
    let profiles = match ProfilesConfig::load() {
        Ok(p) => p,
        Err(e) => {
            working.ai_ready = false;
            working.ai_error = Some(e.to_string());
            working.is_waiting = false;
            working.lead_status = Some("idle".to_string());
            session.lead.status = LeadStatus::Idle;
            let _ = state.session_store.save_project_mode(&session);
            save_window_project_mode_state(state, window_label, &working);
            return working;
        }
    };

    let ai = profiles.resolve_active_ai_settings();
    let Some(settings) = ai.resolved else {
        working.ai_ready = false;
        working.ai_error = Some("AI settings are required.".to_string());
        working.is_waiting = false;
        working.lead_status = Some("idle".to_string());
        session.lead.status = LeadStatus::Idle;
        let _ = state.session_store.save_project_mode(&session);
        save_window_project_mode_state(state, window_label, &working);
        return working;
    };
    working.ai_ready = true;
    working.ai_error = None;

    let client = match AIClient::new(settings) {
        Ok(c) => c,
        Err(e) => {
            working.last_error = Some(e.to_string());
            working.is_waiting = false;
            working.lead_status = Some("idle".to_string());
            session.lead.status = LeadStatus::Idle;
            let _ = state.session_store.save_project_mode(&session);
            save_window_project_mode_state(state, window_label, &working);
            return working;
        }
    };

    // Build chat messages from working state with Project Mode system prompt
    let mut messages = build_project_mode_chat_messages(&working.messages);

    let mut loops = 0usize;
    loop {
        loops += 1;
        let response =
            match client.create_response_with_tools(messages.clone(), builtin_tool_definitions()) {
                Ok(r) => r,
                Err(e) => {
                    working.last_error = Some(e.to_string());
                    working.is_waiting = false;
                    working.lead_status = Some("idle".to_string());
                    session.lead.status = LeadStatus::Idle;
                    let _ = state.session_store.save_project_mode(&session);
                    save_window_project_mode_state(state, window_label, &working);
                    return working;
                }
            };

        let has_tools = !response.tool_calls.is_empty();
        let has_action = apply_ai_response(&mut working, &response, !has_tools);
        save_window_project_mode_state(state, window_label, &working);

        // Update session lead state with the response
        if let Some(tokens) = response.usage_tokens {
            session.lead.estimated_tokens = session.lead.estimated_tokens.saturating_add(tokens);
        }
        session.lead.llm_call_count = session.lead.llm_call_count.saturating_add(1);

        if !response.text.trim().is_empty() {
            session.lead.conversation.push(LeadMessage::new(
                MessageRole::Assistant,
                MessageKind::Message,
                response.text.trim(),
            ));
        }

        if response.tool_calls.is_empty() {
            break;
        }

        let tool_observations = execute_tool_calls(state, window_label, &response.tool_calls);
        push_tool_turns_to_state_and_session(
            &mut working,
            &mut session,
            &response.tool_calls,
            has_action,
            &tool_observations,
        );
        save_window_project_mode_state(state, window_label, &working);
        messages = build_project_mode_chat_messages(&working.messages);

        if loops >= MAX_TOOL_CALL_LOOPS {
            break;
        }
    }

    // Finalize: Thinking → Idle
    session.lead.status = LeadStatus::Idle;
    session.touch();
    let _ = state.session_store.save_project_mode(&session);

    working.is_waiting = false;
    working.lead_status = Some("idle".to_string());
    save_window_project_mode_state(state, window_label, &working);
    working
}

fn build_project_mode_chat_messages(messages: &[ProjectModeMessage]) -> Vec<ChatMessage> {
    let mut out = Vec::with_capacity(messages.len() + 1);
    out.push(ChatMessage {
        role: "system".to_string(),
        content: PROJECT_MODE_SYSTEM_PROMPT.to_string(),
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

fn apply_ai_response(
    state: &mut ProjectModeState,
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

fn push_tool_turns_to_state_and_session(
    working: &mut ProjectModeState,
    session: &mut ProjectModeSession,
    tool_calls: &[ToolCall],
    has_action: bool,
    tool_observations: &[String],
) {
    if !has_action {
        for call in tool_calls {
            let action = format_tool_call(call);
            push_message(working, "assistant", "action", &action);
            session.lead.conversation.push(LeadMessage::new(
                MessageRole::Assistant,
                MessageKind::Action,
                action,
            ));
        }
    }

    for obs in tool_observations {
        push_message(working, "tool", "observation", obs);
        session.lead.conversation.push(LeadMessage::new(
            MessageRole::Assistant,
            MessageKind::Observation,
            obs.clone(),
        ));
    }
}

fn activate_session_for_message(session: &mut ProjectModeSession, user_message: &str) {
    session.status = SessionStatus::Active;
    session.lead.status = LeadStatus::Thinking;
    session.lead.conversation.push(LeadMessage::new(
        MessageRole::User,
        MessageKind::Message,
        user_message,
    ));
}

fn push_message(state: &mut ProjectModeState, role: &str, kind: &str, content: &str) {
    state.messages.push(ProjectModeMessage {
        role: role.to_string(),
        kind: kind.to_string(),
        content: content.to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
    });
}

fn build_chat_messages(messages: &[ProjectModeMessage]) -> Vec<ChatMessage> {
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
    preferred_issue_number: Option<u64>,
) -> Result<IssueSpecPreparation, String> {
    let Some(project_path) = state.project_for_window(window_label) else {
        return Err("Open a project before using Project Mode.".to_string());
    };
    prepare_issue_spec(Path::new(&project_path), user_input, preferred_issue_number)
}

fn prepare_issue_spec(
    project_root: &Path,
    user_input: &str,
    preferred_issue_number: Option<u64>,
) -> Result<IssueSpecPreparation, String> {
    let repo_path = resolve_repo_path_for_project_root(project_root)?;
    let issue_number = extract_issue_number(user_input).or(preferred_issue_number);
    let existing = issue_number
        .map(|number| get_spec_issue_detail(&repo_path, number))
        .transpose()?;
    let created = existing.is_none();
    let title = build_issue_title(user_input, existing.as_ref().map(|e| e.title.as_str()));
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
        issue_number,
        &title,
        &sections,
        existing.as_ref().map(|e| e.etag.as_str()),
    )?;

    let _ = sync_issue_to_project(&repo_path, detail.number, "", SpecProjectPhase::Draft);

    Ok(IssueSpecPreparation {
        issue_number: detail.number,
        issue_url: detail.url,
        etag: detail.etag,
        created,
    })
}

fn extract_issue_number(input: &str) -> Option<u64> {
    for token in input.split_whitespace() {
        let token =
            token.trim_matches(|c: char| matches!(c, ',' | '.' | ';' | ':' | '(' | ')' | '[' | ']'));

        if let Some(rest) = token.strip_prefix('#') {
            if let Ok(number) = rest.parse::<u64>() {
                return Some(number);
            }
        }

        if let Some((_, rest)) = token.rsplit_once('#') {
            if token.contains('/') {
                if let Ok(number) = rest.parse::<u64>() {
                    return Some(number);
                }
            }
        }

        if let Some((_, rest)) = token.rsplit_once("/issues/") {
            if let Ok(number) = rest.parse::<u64>() {
                return Some(number);
            }
        }
    }
    None
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

fn build_issue_title(user_input: &str, existing_title: Option<&str>) -> String {
    if let Some(existing) = existing_title {
        if !existing.trim().is_empty() {
            return existing.trim().to_string();
        }
    }
    let base = collapse_whitespace(user_input);
    let mut title = if base.is_empty() {
        "Project Mode Task".to_string()
    } else {
        base
    };
    if title.chars().count() > 72 {
        title = title.chars().take(72).collect::<String>();
        title = title.trim_end().to_string();
    }
    title
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

/// Check that the 4 gate sections (spec, plan, tasks, tdd) are all non-empty.
/// Returns Ok(()) if complete, Err with list of missing section names otherwise.
pub fn check_spec_sections_complete(sections: &SpecIssueSections) -> Result<(), Vec<String>> {
    let mut missing = Vec::new();
    if sections.spec.trim().is_empty() {
        missing.push("spec".to_string());
    }
    if sections.plan.trim().is_empty() {
        missing.push("plan".to_string());
    }
    if sections.tasks.trim().is_empty() {
        missing.push("tasks".to_string());
    }
    if sections.tdd.trim().is_empty() {
        missing.push("tdd".to_string());
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(missing)
    }
}

/// Check if the user message is an approval pattern.
pub fn is_approval_message(input: &str) -> bool {
    let lower = input.trim().to_lowercase();
    matches!(
        lower.as_str(),
        "approve" | "approved" | "yes" | "ok" | "lgtm" | "go ahead" | "proceed" | "go"
    ) || lower.starts_with("yes,")
        || lower.starts_with("yes ")
        || lower.starts_with("ok,")
        || lower.starts_with("ok ")
        || lower.starts_with("approve ")
}

/// Register a GitHub Issue as a ProjectIssue in the session.
/// Skips if the issue number is already registered.
pub fn register_project_issue(
    session: &mut ProjectModeSession,
    issue_number: u64,
    issue_url: &str,
    title: &str,
) {
    use gwt_core::agent::issue::{IssueStatus, ProjectIssue};
    if session
        .issues
        .iter()
        .any(|i| i.github_issue_number == issue_number)
    {
        return;
    }
    session.issues.push(ProjectIssue {
        id: format!("issue-{}", issue_number),
        github_issue_number: issue_number,
        github_issue_url: issue_url.to_string(),
        title: title.to_string(),
        status: IssueStatus::Planned,
        coordinator: None,
        tasks: Vec::new(),
    });
}

/// Format scrollback output for inclusion in Lead conversation.
/// Truncates to `max_len` bytes with a truncation marker.
pub fn format_scrollback_for_lead(scrollback: &str, max_len: usize) -> String {
    if scrollback.len() <= max_len {
        return scrollback.to_string();
    }
    let truncated: String = scrollback.chars().take(max_len).collect();
    format!("{}...(truncated)", truncated)
}

/// Polling interval in seconds for the Lead hybrid resident loop.
/// Between events the Lead stays idle; this interval triggers a status check.
pub const POLLING_INTERVAL_SECS: u64 = 120;

/// Actions that the Lead can perform autonomously without user approval.
const AUTONOMOUS_ACTIONS: &[&str] = &["task_reorder", "parallel_degree", "retry"];

/// Classify whether an action requires user approval.
/// Autonomous actions (task reorder, parallel degree adjustment, retry) return false.
/// All other actions (strategy change, new issue creation, PR merge, etc.) return true.
pub fn requires_approval(action: &str) -> bool {
    !AUTONOMOUS_ACTIONS.contains(&action)
}

/// Determine whether the Lead should perform a polling check.
/// Returns true if never polled or if the polling interval has elapsed.
pub fn should_poll(last_poll_at: Option<chrono::DateTime<chrono::Utc>>) -> bool {
    match last_poll_at {
        None => true,
        Some(last) => {
            let elapsed = chrono::Utc::now() - last;
            elapsed.num_seconds() >= POLLING_INTERVAL_SECS as i64
        }
    }
}

fn format_tool_call(call: &ToolCall) -> String {
    let args = match serde_json::to_string(&call.arguments) {
        Ok(s) => s,
        Err(_) => "{}".to_string(),
    };
    format!("{} {}", call.name, args)
}

fn lead_status_to_api_string(status: LeadStatus) -> &'static str {
    match status {
        LeadStatus::Idle => "idle",
        LeadStatus::Thinking => "thinking",
        LeadStatus::WaitingApproval => "waiting_approval",
        LeadStatus::Orchestrating => "orchestrating",
        LeadStatus::Error => "error",
    }
}

// ===========================================================================
// Phase 7: US5 — Artifact Verification and Integration (T701-T706)
// ===========================================================================

/// Verify task artifacts by running a test command and returning the result.
/// In the current pure-function form, this constructs a TestVerification from
/// a pre-captured test output and pass/fail flag.
pub fn verify_task_artifacts(test_command: &str, output: &str, passed: bool) -> TestVerification {
    TestVerification {
        status: if passed {
            TestStatus::Passed
        } else {
            TestStatus::Failed
        },
        command: test_command.to_string(),
        output: Some(output.to_string()),
        attempt: 1,
    }
}

/// Generate a PR title from a task name and issue number.
pub fn generate_pr_title(task_name: &str, issue_number: u64) -> String {
    format!("feat: {} (#{issue_number})", task_name.trim())
}

/// Generate a PR body from a task and its associated issue number.
pub fn generate_pr_body(task: &Task, issue_number: u64) -> String {
    format!(
        "## Summary\n\n{}\n\nCloses #{issue_number}\n\n## Task\n\n- **ID**: {}\n- **Name**: {}",
        task.description, task.id.0, task.name
    )
}

/// CI status as observed from a PullRequestRef.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CiStatus {
    Pending,
    Success,
    Failure,
}

/// Check CI status for a PR. This is a stub that derives status from the
/// existence and number of a PullRequestRef. In production this would query
/// the GitHub API.
pub fn check_ci_status(pr: &Option<PullRequestRef>) -> Option<CiStatus> {
    pr.as_ref().map(|_| CiStatus::Pending)
}

/// Determine whether we should retry a CI fix. Max 3 retries (0..=2 attempts
/// means retries 0, 1, 2 are OK; 3+ stops).
pub fn should_retry_ci_fix(retry_count: u8) -> bool {
    retry_count < 3
}

/// Format a prompt instructing a developer to fix CI failures.
pub fn format_ci_fix_prompt(ci_output: &str, task: &Task) -> String {
    format!(
        "CI failed for task '{}'. Please fix the following issues and push again:\n\n```\n{}\n```",
        task.name, ci_output
    )
}

/// Format a git merge command string for a developer to execute.
pub fn format_merge_command(source_branch: &str, target_branch: &str) -> String {
    format!("git checkout {target_branch} && git merge {source_branch}")
}

/// Detect whether command output contains merge conflict markers.
pub fn detect_merge_conflict(output: &str) -> bool {
    output.contains("CONFLICT")
        || output.contains("<<<<<<<")
        || output.contains(">>>>>>>")
        || output.contains("Merge conflict")
        || output.contains("merge conflict")
}

// ===========================================================================
// Phase 8: US6 — Failure Handling and Layer Independence (T801-T804)
// ===========================================================================

/// Scope of a failure — which layer is affected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FailureScope {
    Lead,
    Coordinator,
    Developer,
}

/// Check if a layer status string indicates a healthy/operational state.
pub fn is_layer_healthy(status: &str) -> bool {
    let lower = status.to_lowercase();
    matches!(
        lower.as_str(),
        "idle" | "thinking" | "orchestrating" | "running" | "waiting_approval" | "ready"
    )
}

/// Classify a failure error message into the appropriate layer scope.
pub fn classify_failure(error: &str) -> FailureScope {
    let lower = error.to_lowercase();
    if lower.contains("lead") || lower.contains("ai settings") || lower.contains("api key") {
        FailureScope::Lead
    } else if lower.contains("coordinator") || lower.contains("pane") || lower.contains("session") {
        FailureScope::Coordinator
    } else {
        FailureScope::Developer
    }
}

/// Determine whether a coordinator should be restarted after a crash.
pub fn should_restart_coordinator(crash_count: u8, max_crashes: u8) -> bool {
    crash_count < max_crashes
}

/// Calculate exponential backoff delay in milliseconds for coordinator restarts.
/// Pattern: 1000ms, 2000ms, 4000ms, ...
pub fn coordinator_restart_delay_ms(crash_count: u8) -> u64 {
    1000u64 * 2u64.pow(crash_count as u32)
}

// ===========================================================================
// Phase 5: US3 — Developer Launch and Implementation (T501-T510)
// ===========================================================================

/// Launch a coordinator for a given project issue.
///
/// Creates a CoordinatorState with status Starting, assigns a pane_id, and
/// returns the state. The actual terminal pane creation is handled by the caller
/// using the pane_id.
pub fn launch_coordinator(
    issue: &gwt_core::agent::issue::ProjectIssue,
    pane_id: &str,
) -> gwt_core::agent::coordinator::CoordinatorState {
    gwt_core::agent::coordinator::CoordinatorState {
        pane_id: pane_id.to_string(),
        pid: None,
        status: gwt_core::agent::coordinator::CoordinatorStatus::Starting,
        started_at: chrono::Utc::now(),
        github_issue_number: issue.github_issue_number,
        crash_count: 0,
    }
}

/// Build the coordinator prompt for a given GitHub issue.
pub fn build_coordinator_prompt(issue_number: u64, issue_title: &str) -> String {
    format!(
        "You are a Coordinator agent. Implement GitHub Issue #{} ({}).\n\
         Break down the work into tasks, create developer worktrees, and track progress.\n\
         Report GWT_TASK_DONE when all tasks are complete.",
        issue_number, issue_title
    )
}

/// Assign developers to a task, creating DeveloperState entries.
///
/// Creates `count` DeveloperState entries with status Starting and the given
/// agent type. Worktree references use placeholder paths that should be updated
/// after actual worktree creation.
pub fn assign_developers_to_task(
    task: &mut gwt_core::agent::task::Task,
    agent_type: gwt_core::agent::developer::AgentType,
    count: usize,
) {
    use gwt_core::agent::developer::{DeveloperState, DeveloperStatus};
    use gwt_core::agent::types::SubAgentId;
    use gwt_core::agent::worktree::WorktreeRef;

    for i in 0..count {
        let dev_id = SubAgentId::new();
        let pane_id = format!("dev-{}-{}", task.id.0, i);
        let sanitized_name = task.name.replace(' ', "-").to_lowercase();
        let branch_name = format!("agent/{}-dev-{}", sanitized_name, i);
        let worktree = WorktreeRef::new(
            &branch_name,
            PathBuf::from(format!(".worktrees/{}", branch_name.replace('/', "-"))),
            vec![task.id.clone()],
        );
        task.developers.push(DeveloperState {
            id: dev_id,
            agent_type,
            pane_id,
            pid: None,
            status: DeveloperStatus::Starting,
            worktree,
            started_at: chrono::Utc::now(),
            completed_at: None,
            completion_source: None,
        });
    }
}

/// Create a developer worktree reference using the existing helpers.
///
/// Returns a WorktreeRef with the generated branch name and path.
/// Note: This does not actually create the git worktree on disk;
/// the caller must use WorktreeManager for that.
pub fn create_developer_worktree(
    repo_path: &Path,
    task_name: &str,
    existing_branches: &[String],
) -> gwt_core::agent::worktree::WorktreeRef {
    use gwt_core::agent::worktree::{create_agent_branch_name, worktree_path, WorktreeRef};

    let branch_name = create_agent_branch_name(task_name, existing_branches);
    let path = worktree_path(repo_path, &branch_name);

    WorktreeRef::new(branch_name, path, vec![])
}

/// Resolve the auto-mode flag for a given agent type.
///
/// - Claude: `--dangerously-skip-permissions`
/// - Codex: `--full-auto`
/// - Gemini: `auto`
pub fn auto_mode_flag(agent_type: gwt_core::agent::developer::AgentType) -> &'static str {
    use gwt_core::agent::developer::AgentType;
    match agent_type {
        AgentType::Claude => "--dangerously-skip-permissions",
        AgentType::Codex => "--full-auto",
        AgentType::Gemini => "auto",
    }
}

/// Resolve the agent command name for a given agent type.
pub fn agent_command_name(agent_type: gwt_core::agent::developer::AgentType) -> &'static str {
    use gwt_core::agent::developer::AgentType;
    match agent_type {
        AgentType::Claude => "claude",
        AgentType::Codex => "codex",
        AgentType::Gemini => "gemini",
    }
}

/// Build the developer task prompt.
pub fn build_developer_prompt(task_name: &str, task_description: &str) -> String {
    format!(
        "Task: {}\n\n{}\n\nWhen finished, output GWT_TASK_DONE on a new line.",
        task_name, task_description
    )
}

// ===========================================================================
// Phase 6: US4 — Developer Completion Detection (T601-T604)
// ===========================================================================

/// Sources of completion detection, checked in priority order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionDetection {
    /// Hook stop event detected
    HookStop,
    /// Output pattern (GWT_TASK_DONE) found in scrollback
    OutputPattern,
    /// Process has exited
    ProcessExit,
}

/// The output pattern that signals task completion.
pub const COMPLETION_PATTERN: &str = "GWT_TASK_DONE";

/// Detect developer completion from scrollback output.
///
/// Checks for the GWT_TASK_DONE pattern in the scrollback text.
pub fn detect_output_pattern(scrollback: &str) -> bool {
    scrollback.contains(COMPLETION_PATTERN)
}

/// Check if a pane process has exited by examining its status.
///
/// Returns Some(exit_code) if the pane has completed, None if still running.
pub fn check_pane_exit(state: &AppState, pane_id: &str) -> Option<i32> {
    use gwt_core::terminal::pane::PaneStatus;

    let manager = state.pane_manager.lock().ok()?;
    let pane = manager.panes().iter().find(|p| p.pane_id() == pane_id)?;
    match pane.status() {
        PaneStatus::Completed(code) => Some(*code),
        _ => None,
    }
}

/// Detect developer completion using composite detection.
///
/// Check order: 1) Hook Stop event, 2) Output pattern, 3) Process exit.
/// Returns None if the developer is still running.
pub fn detect_developer_completion(
    state: &AppState,
    pane_id: &str,
    hook_stopped: bool,
    scrollback: &str,
) -> Option<CompletionDetection> {
    // 1) Hook stop
    if hook_stopped {
        return Some(CompletionDetection::HookStop);
    }

    // 2) Output pattern
    if detect_output_pattern(scrollback) {
        return Some(CompletionDetection::OutputPattern);
    }

    // 3) Process exit
    if check_pane_exit(state, pane_id).is_some() {
        return Some(CompletionDetection::ProcessExit);
    }

    None
}

/// Format scrollback output as a progress report.
///
/// Extracts the last `max_lines` non-empty lines from the scrollback for
/// display in the Lead chat. No LLM call needed.
pub fn format_progress_report(scrollback: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = scrollback
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect();

    let start = lines.len().saturating_sub(max_lines);
    lines[start..].join("\n")
}

// ===========================================================================
// Phase 9: US7 — Session Persistence / Restore / Force Stop (T901-T906)
// ===========================================================================

/// Persist the current ProjectModeSession state to disk.
///
/// Should be called after each state change (message send, status change,
/// task update) to ensure session data is not lost on crash or restart.
pub fn save_project_mode_state(
    state: &AppState,
    session: &ProjectModeSession,
) -> Result<(), String> {
    state
        .session_store
        .save_project_mode(session)
        .map_err(|e| format!("Failed to save project mode session: {e}"))
}

/// Restore the most recent active ProjectModeSession and reconstruct
/// the ProjectModeState from it.
///
/// Loads the session by ID and rebuilds the in-memory state from the
/// persisted lead conversation and metadata.
pub fn restore_project_mode_session(
    state: &AppState,
    session_id: &str,
) -> Result<(ProjectModeSession, ProjectModeState), String> {
    let sid = SessionId(session_id.to_string());
    let session = state
        .session_store
        .load_project_mode(&sid)
        .map_err(|e| format!("Failed to load project mode session: {e}"))?;

    let mut mode = ProjectModeState::new();
    mode.project_mode_session_id = Some(session_id.to_string());
    mode.lead_status = Some(lead_status_to_api_string(session.lead.status).to_string());
    mode.llm_call_count = session.lead.llm_call_count;
    mode.estimated_tokens = session.lead.estimated_tokens;
    mode.ai_ready = true;

    // Reconstruct messages from lead conversation
    for msg in &session.lead.conversation {
        let role = match msg.role {
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            MessageRole::System => "system",
        };
        let kind = match msg.kind {
            MessageKind::Message => "message",
            MessageKind::Thought => "thought",
            MessageKind::Action => "action",
            MessageKind::Observation => "observation",
            MessageKind::Error => "error",
            MessageKind::Progress => "progress",
        };
        mode.messages.push(ProjectModeMessage {
            role: role.to_string(),
            kind: kind.to_string(),
            content: msg.content.clone(),
            timestamp: msg.timestamp.timestamp_millis(),
        });
    }

    Ok((session, mode))
}

/// List all ProjectModeSession summaries from the session store.
pub fn list_project_mode_sessions(
    state: &AppState,
) -> Result<Vec<ProjectModeSessionSummary>, String> {
    let summaries = state
        .session_store
        .list_project_mode_sessions()
        .map_err(|e| format!("Failed to list project mode sessions: {e}"))?;

    Ok(summaries
        .into_iter()
        .map(|s| ProjectModeSessionSummary {
            session_id: s.session_id.0,
            status: format!("{:?}", s.status).to_lowercase(),
            updated_at: s.updated_at.map(|dt| dt.timestamp_millis()),
        })
        .collect())
}

/// A lightweight summary for listing Project Mode sessions on the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectModeSessionSummary {
    pub session_id: String,
    pub status: String,
    pub updated_at: Option<i64>,
}

/// Force-stop a running ProjectModeSession by pausing it and saving.
///
/// Sets the session status to Paused and the lead status to Idle,
/// then persists the session. Returns a user-friendly status message.
pub fn force_stop_project_mode(state: &AppState, session_id: &str) -> Result<String, String> {
    let sid = SessionId(session_id.to_string());
    let mut session = state
        .session_store
        .load_project_mode(&sid)
        .map_err(|e| format!("Failed to load project mode session: {e}"))?;

    session.status = SessionStatus::Paused;
    session.lead.status = LeadStatus::Idle;
    session.touch();

    state
        .session_store
        .save_project_mode(&session)
        .map_err(|e| format!("Failed to save paused session: {e}"))?;

    Ok(format!("Session {} has been paused.", session_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;

    #[test]
    fn build_chat_messages_adds_system_prompt_and_filters_system_messages() {
        let input = vec![
            ProjectModeMessage {
                role: "system".to_string(),
                kind: "message".to_string(),
                content: "legacy system".to_string(),
                timestamp: 0,
            },
            ProjectModeMessage {
                role: "user".to_string(),
                kind: "message".to_string(),
                content: "hello".to_string(),
                timestamp: 1,
            },
            ProjectModeMessage {
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
    fn extract_issue_number_parses_hash_prefixed_issue() {
        let out = extract_issue_number("continue work on #1438 please");
        assert_eq!(out, Some(1438));
    }

    #[test]
    fn extract_issue_number_ignores_plain_number_token() {
        let out = extract_issue_number("continue work on 1438 please");
        assert_eq!(out, None);
    }

    #[test]
    fn extract_issue_number_parses_repo_scoped_reference() {
        let out = extract_issue_number("continue work on akiojin/gwt#1438 please");
        assert_eq!(out, Some(1438));
    }

    #[test]
    fn extract_issue_number_parses_issue_url() {
        let out = extract_issue_number("https://github.com/akiojin/gwt/issues/1438");
        assert_eq!(out, Some(1438));
    }

    #[test]
    fn prepare_issue_spec_for_window_requires_open_project() {
        let state = AppState::new();
        let err = prepare_issue_spec_for_window(&state, "main", "implement auth", None);
        assert!(err.is_err());
        assert!(err
            .unwrap_err()
            .contains("Open a project before using Project Mode."));
    }

    #[test]
    fn build_issue_title_prefers_existing_title() {
        let title = build_issue_title("new input", Some("Existing"));
        assert_eq!(title, "Existing");
    }

    #[test]
    fn build_issue_title_uses_plain_user_input() {
        let title = build_issue_title("Implement authentication flow", None);
        assert_eq!(title, "Implement authentication flow");
    }

    #[test]
    fn build_issue_title_handles_multibyte_without_panic() {
        // 72 chars total but >72 bytes (71 ASCII + 1 multibyte char)
        let input = format!("{}あ", "a".repeat(71));
        let title = build_issue_title(&input, None);
        assert!(title.contains('あ'));
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

    // --- ProjectModeState new fields tests ---

    #[test]
    fn project_mode_state_new_has_none_project_mode_fields() {
        let state = ProjectModeState::new();
        assert!(state.project_mode_session_id.is_none());
        assert!(state.lead_status.is_none());
    }

    #[test]
    fn project_mode_state_serde_backward_compatible() {
        // JSON without the new fields should deserialize with defaults
        let json = r#"{
            "messages": [],
            "ai_ready": true,
            "ai_error": null,
            "last_error": null,
            "is_waiting": false,
            "session_name": "Project Mode",
            "llm_call_count": 5,
            "estimated_tokens": 1000,
            "active_spec_issue_number": null,
            "active_spec_issue_url": null,
            "active_spec_issue_etag": null
        }"#;
        let state: ProjectModeState = serde_json::from_str(json).unwrap();
        assert!(state.project_mode_session_id.is_none());
        assert!(state.lead_status.is_none());
        assert!(state.ai_ready);
        assert_eq!(state.llm_call_count, 5);
    }

    #[test]
    fn project_mode_state_serde_with_project_mode_fields() {
        let mut state = ProjectModeState::new();
        state.project_mode_session_id = Some("pt-session-123".to_string());
        state.lead_status = Some("thinking".to_string());

        let json = serde_json::to_string(&state).unwrap();
        let deserialized: ProjectModeState = serde_json::from_str(&json).unwrap();
        assert_eq!(
            deserialized.project_mode_session_id,
            Some("pt-session-123".to_string())
        );
        assert_eq!(deserialized.lead_status, Some("thinking".to_string()));
    }

    #[test]
    fn project_mode_state_roundtrip_preserves_all_fields() {
        let mut state = ProjectModeState::new();
        state.ai_ready = true;
        state.session_name = Some("Test Session".to_string());
        state.project_mode_session_id = Some("pt-abc".to_string());
        state.lead_status = Some("orchestrating".to_string());
        state.llm_call_count = 42;
        state.estimated_tokens = 5000;
        push_message(&mut state, "user", "message", "hello");

        let json = serde_json::to_string(&state).unwrap();
        let deserialized: ProjectModeState = serde_json::from_str(&json).unwrap();

        assert!(deserialized.ai_ready);
        assert_eq!(deserialized.session_name, Some("Test Session".to_string()));
        assert_eq!(
            deserialized.project_mode_session_id,
            Some("pt-abc".to_string())
        );
        assert_eq!(deserialized.lead_status, Some("orchestrating".to_string()));
        assert_eq!(deserialized.llm_call_count, 42);
        assert_eq!(deserialized.estimated_tokens, 5000);
        assert_eq!(deserialized.messages.len(), 1);
        assert_eq!(deserialized.messages[0].content, "hello");
    }

    // --- LeadState status transition tests ---

    #[test]
    fn lead_status_idle_to_thinking_transition() {
        use gwt_core::agent::lead::LeadState;
        let mut lead = LeadState::default();
        assert_eq!(lead.status, LeadStatus::Idle);

        lead.status = LeadStatus::Thinking;
        assert_eq!(lead.status, LeadStatus::Thinking);

        lead.status = LeadStatus::Idle;
        assert_eq!(lead.status, LeadStatus::Idle);
    }

    #[test]
    fn lead_message_creation_and_conversation_append() {
        use gwt_core::agent::lead::LeadState;
        let mut lead = LeadState::default();
        assert!(lead.conversation.is_empty());

        lead.conversation.push(LeadMessage::new(
            MessageRole::User,
            MessageKind::Message,
            "implement login feature",
        ));
        assert_eq!(lead.conversation.len(), 1);
        assert_eq!(lead.conversation[0].content, "implement login feature");
        assert_eq!(lead.conversation[0].kind, MessageKind::Message);

        lead.conversation.push(LeadMessage::new(
            MessageRole::Assistant,
            MessageKind::Thought,
            "analyzing requirements...",
        ));
        assert_eq!(lead.conversation.len(), 2);
        assert_eq!(lead.conversation[1].kind, MessageKind::Thought);
    }

    // --- Project Mode message flow tests ---

    #[test]
    fn send_project_mode_message_requires_open_project() {
        let state = AppState::new();
        let result = send_project_mode_message(&state, "main", "implement auth");
        assert!(result.last_error.is_some());
        assert!(result
            .last_error
            .unwrap()
            .contains("Open a project before using Project Mode."));
        assert_eq!(result.lead_status, Some("idle".to_string()));
        assert!(!result.is_waiting);
    }

    #[test]
    fn send_project_mode_message_empty_input_returns_current_state() {
        let state = AppState::new();
        let result = send_project_mode_message(&state, "main", "   ");
        // Empty input should just return current state without error
        assert!(result.last_error.is_none());
    }

    #[test]
    fn send_project_mode_message_load_error_does_not_recreate_session() {
        let (state, _dir) = make_test_app_state_with_store();
        state
            .claim_project_for_window_with_identity(
                "main",
                "/repo".to_string(),
                "repo-id".to_string(),
            )
            .unwrap();

        if let Ok(mut guard) = state.window_project_modes.lock() {
            let mut mode = ProjectModeState::new();
            mode.project_mode_session_id = Some("pt-broken-1".to_string());
            guard.insert("main".to_string(), mode);
        }

        let session_path = state
            .session_store
            .sessions_dir()
            .join("pt-pt-broken-1.json");
        std::fs::write(&session_path, "{ this is invalid json ").unwrap();

        let result = send_project_mode_message(&state, "main", "resume");
        assert_eq!(result.lead_status, Some("idle".to_string()));
        assert!(!result.is_waiting);
        let err = result.last_error.unwrap();
        assert!(err.contains("Failed to load project mode session"));
        assert!(err.contains("Parse error"));

        let broken_path = state
            .session_store
            .sessions_dir()
            .join("pt-pt-broken-1.json.broken");
        assert!(broken_path.exists());
        assert!(!session_path.exists());
    }

    #[test]
    fn build_project_mode_chat_messages_uses_project_mode_prompt() {
        let messages = vec![ProjectModeMessage {
            role: "user".to_string(),
            kind: "message".to_string(),
            content: "hello".to_string(),
            timestamp: 1,
        }];
        let out = build_project_mode_chat_messages(&messages);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].role, "system");
        assert_eq!(out[0].content, PROJECT_MODE_SYSTEM_PROMPT);
        assert_eq!(out[1].role, "user");
        assert_eq!(out[1].content, "hello");
    }

    #[test]
    fn push_tool_turns_to_state_and_session_persists_action_and_observation() {
        let mut working = ProjectModeState::new();
        let mut session = ProjectModeSession::new(
            SessionId("pt-tools-1".to_string()),
            PathBuf::from("/repo"),
            "main",
            AgentType::Claude,
        );
        let tool_calls = vec![ToolCall {
            name: "run_cmd".to_string(),
            arguments: serde_json::json!({"cmd":"echo hello"}),
            call_id: Some("call-1".to_string()),
        }];
        let observations = vec!["run_cmd => ok".to_string()];

        push_tool_turns_to_state_and_session(
            &mut working,
            &mut session,
            &tool_calls,
            false,
            &observations,
        );

        assert_eq!(working.messages.len(), 2);
        assert_eq!(working.messages[0].role, "assistant");
        assert_eq!(working.messages[0].kind, "action");
        assert_eq!(working.messages[1].role, "tool");
        assert_eq!(working.messages[1].kind, "observation");

        assert_eq!(session.lead.conversation.len(), 2);
        assert_eq!(session.lead.conversation[0].role, MessageRole::Assistant);
        assert_eq!(session.lead.conversation[0].kind, MessageKind::Action);
        assert_eq!(
            session.lead.conversation[0].content,
            "run_cmd {\"cmd\":\"echo hello\"}"
        );
        assert_eq!(session.lead.conversation[1].role, MessageRole::Assistant);
        assert_eq!(session.lead.conversation[1].kind, MessageKind::Observation);
        assert_eq!(session.lead.conversation[1].content, "run_cmd => ok");
    }

    #[test]
    fn activate_session_for_message_resumes_paused_session() {
        let mut session = ProjectModeSession::new(
            SessionId("pt-resume-1".to_string()),
            PathBuf::from("/repo"),
            "main",
            AgentType::Claude,
        );
        session.status = SessionStatus::Paused;

        activate_session_for_message(&mut session, "resume work");

        assert_eq!(session.status, SessionStatus::Active);
        assert_eq!(session.lead.status, LeadStatus::Thinking);
        assert_eq!(session.lead.conversation.len(), 1);
        assert_eq!(session.lead.conversation[0].role, MessageRole::User);
        assert_eq!(session.lead.conversation[0].kind, MessageKind::Message);
        assert_eq!(session.lead.conversation[0].content, "resume work");
    }

    // --- T401: issue_spec tools Lead integration ---

    #[test]
    fn project_mode_tool_definitions_include_all_spec_tools() {
        let defs = builtin_tool_definitions();
        let names: Vec<String> = defs.iter().map(|d| d.function.name.clone()).collect();
        // All spec tools must be available to the Project Mode Lead
        assert!(names.contains(&"upsert_spec_issue".to_string()));
        assert!(names.contains(&"get_spec_issue".to_string()));
        assert!(names.contains(&"upsert_spec_issue_artifact".to_string()));
        assert!(names.contains(&"list_spec_issue_artifacts".to_string()));
        assert!(names.contains(&"delete_spec_issue_artifact".to_string()));
        assert!(names.contains(&"sync_spec_issue_project".to_string()));
    }

    #[test]
    fn project_mode_system_prompt_mentions_spec_tools() {
        // The system prompt should instruct the Lead to use issue_spec tools
        assert!(PROJECT_MODE_SYSTEM_PROMPT.contains("upsert_spec_issue"));
        assert!(PROJECT_MODE_SYSTEM_PROMPT.contains("get_spec_issue"));
        assert!(PROJECT_MODE_SYSTEM_PROMPT.contains("spec"));
    }

    // --- T403: GitHub Issue spec management workflow ---

    #[test]
    fn project_mode_system_prompt_has_workflow_instructions() {
        // The prompt should describe the spec workflow: clarify → create issue → write sections
        assert!(PROJECT_MODE_SYSTEM_PROMPT.contains("clarif"));
        assert!(PROJECT_MODE_SYSTEM_PROMPT.contains("GitHub Issue"));
        assert!(PROJECT_MODE_SYSTEM_PROMPT.contains("plan"));
        assert!(PROJECT_MODE_SYSTEM_PROMPT.contains("tasks"));
        assert!(PROJECT_MODE_SYSTEM_PROMPT.contains("tdd"));
    }

    #[test]
    fn project_mode_system_prompt_has_section_gate_instruction() {
        // The prompt should mention that all 4 sections are required before Coordinator
        assert!(PROJECT_MODE_SYSTEM_PROMPT.contains("spec"));
        assert!(PROJECT_MODE_SYSTEM_PROMPT.contains("plan"));
        assert!(PROJECT_MODE_SYSTEM_PROMPT.contains("tasks"));
        assert!(PROJECT_MODE_SYSTEM_PROMPT.contains("tdd"));
        assert!(
            PROJECT_MODE_SYSTEM_PROMPT.contains("approval")
                || PROJECT_MODE_SYSTEM_PROMPT.contains("approve")
        );
    }

    // --- T405: 4-section gate check ---

    #[test]
    fn check_spec_sections_complete_all_present() {
        let sections = SpecIssueSections {
            spec: "User login spec".to_string(),
            plan: "Implementation plan".to_string(),
            tasks: "- Task 1\n- Task 2".to_string(),
            tdd: "Test cases: ...".to_string(),
            research: String::new(),
            data_model: String::new(),
            quickstart: String::new(),
            contracts: String::new(),
            checklists: String::new(),
        };
        assert!(check_spec_sections_complete(&sections).is_ok());
    }

    #[test]
    fn check_spec_sections_complete_missing_plan() {
        let sections = SpecIssueSections {
            spec: "User login spec".to_string(),
            plan: String::new(),
            tasks: "- Task 1".to_string(),
            tdd: "Test cases".to_string(),
            research: String::new(),
            data_model: String::new(),
            quickstart: String::new(),
            contracts: String::new(),
            checklists: String::new(),
        };
        let err = check_spec_sections_complete(&sections).unwrap_err();
        assert_eq!(err.len(), 1);
        assert!(err[0].contains("plan"));
    }

    #[test]
    fn check_spec_sections_complete_missing_all_four() {
        let sections = SpecIssueSections {
            spec: String::new(),
            plan: String::new(),
            tasks: String::new(),
            tdd: String::new(),
            research: String::new(),
            data_model: String::new(),
            quickstart: String::new(),
            contracts: String::new(),
            checklists: String::new(),
        };
        let err = check_spec_sections_complete(&sections).unwrap_err();
        assert_eq!(err.len(), 4);
    }

    #[test]
    fn check_spec_sections_complete_whitespace_only_counts_as_missing() {
        let sections = SpecIssueSections {
            spec: "  \n  ".to_string(),
            plan: "Real plan".to_string(),
            tasks: "Real tasks".to_string(),
            tdd: "Real tdd".to_string(),
            research: String::new(),
            data_model: String::new(),
            quickstart: String::new(),
            contracts: String::new(),
            checklists: String::new(),
        };
        let err = check_spec_sections_complete(&sections).unwrap_err();
        assert_eq!(err.len(), 1);
        assert!(err[0].contains("spec"));
    }

    // --- T407: Plan presentation and approval flow ---

    #[test]
    fn lead_status_waiting_approval_transition() {
        use gwt_core::agent::lead::LeadState;
        let mut lead = LeadState {
            status: LeadStatus::Thinking,
            ..Default::default()
        };
        lead.status = LeadStatus::WaitingApproval;
        assert_eq!(lead.status, LeadStatus::WaitingApproval);

        // Approve → Orchestrating
        lead.status = LeadStatus::Orchestrating;
        assert_eq!(lead.status, LeadStatus::Orchestrating);
    }

    #[test]
    fn lead_status_waiting_approval_rejection_returns_to_thinking() {
        use gwt_core::agent::lead::LeadState;
        let mut lead = LeadState {
            status: LeadStatus::WaitingApproval,
            ..Default::default()
        };

        // Reject → back to Thinking
        lead.status = LeadStatus::Thinking;
        assert_eq!(lead.status, LeadStatus::Thinking);
    }

    #[test]
    fn is_approval_message_recognizes_approve_patterns() {
        assert!(is_approval_message("approve"));
        assert!(is_approval_message("APPROVE"));
        assert!(is_approval_message("yes"));
        assert!(is_approval_message("Yes, proceed"));
        assert!(is_approval_message("ok"));
        assert!(is_approval_message("OK"));
        assert!(is_approval_message("go ahead"));
        assert!(is_approval_message("lgtm"));
    }

    #[test]
    fn is_approval_message_rejects_non_approval() {
        assert!(!is_approval_message("reject"));
        assert!(!is_approval_message("no"));
        assert!(!is_approval_message("revise the plan"));
        assert!(!is_approval_message("change the tasks section"));
    }

    // --- T409: GitHub Issue creation and Project registration ---

    #[test]
    fn register_project_issue_creates_entry() {
        use gwt_core::agent::developer::AgentType;
        let mut session = ProjectModeSession::new(
            SessionId("pt-test".to_string()),
            PathBuf::from("/repo"),
            "main",
            AgentType::Claude,
        );
        assert!(session.issues.is_empty());

        register_project_issue(
            &mut session,
            42,
            "https://github.com/org/repo/issues/42",
            "Login feature",
        );
        assert_eq!(session.issues.len(), 1);
        assert_eq!(session.issues[0].github_issue_number, 42);
        assert_eq!(
            session.issues[0].status,
            gwt_core::agent::issue::IssueStatus::Planned
        );
        assert_eq!(session.issues[0].title, "Login feature");
    }

    #[test]
    fn register_project_issue_does_not_duplicate() {
        use gwt_core::agent::developer::AgentType;
        let mut session = ProjectModeSession::new(
            SessionId("pt-test".to_string()),
            PathBuf::from("/repo"),
            "main",
            AgentType::Claude,
        );

        register_project_issue(
            &mut session,
            42,
            "https://github.com/org/repo/issues/42",
            "Login feature",
        );
        register_project_issue(
            &mut session,
            42,
            "https://github.com/org/repo/issues/42",
            "Login feature v2",
        );
        assert_eq!(session.issues.len(), 1);
    }

    // --- T411: Coordinator→Lead hybrid communication ---

    #[test]
    fn format_scrollback_for_lead_truncates_long_output() {
        let long_text = "x".repeat(5000);
        let formatted = format_scrollback_for_lead(&long_text, 500);
        assert!(formatted.len() <= 600); // some overhead for prefix
        assert!(formatted.contains("...(truncated)"));
    }

    #[test]
    fn format_scrollback_for_lead_passes_short_output() {
        let short_text = "Build succeeded\nAll tests passed";
        let formatted = format_scrollback_for_lead(short_text, 500);
        assert!(formatted.contains("Build succeeded"));
        assert!(formatted.contains("All tests passed"));
        assert!(!formatted.contains("truncated"));
    }

    // --- T311: Lead delegation logic (requires_approval) ---

    #[test]
    fn requires_approval_autonomous_actions() {
        assert!(!requires_approval("task_reorder"));
        assert!(!requires_approval("parallel_degree"));
        assert!(!requires_approval("retry"));
    }

    #[test]
    fn requires_approval_approval_required_actions() {
        assert!(requires_approval("strategy_change"));
        assert!(requires_approval("create_issue"));
        assert!(requires_approval("pr_merge"));
    }

    #[test]
    fn requires_approval_unknown_action_defaults_to_required() {
        assert!(requires_approval("unknown_action"));
        assert!(requires_approval("deploy"));
    }

    // --- T313: Lead hybrid polling ---

    #[test]
    fn polling_interval_is_120_seconds() {
        assert_eq!(POLLING_INTERVAL_SECS, 120);
    }

    #[test]
    fn should_poll_returns_true_when_never_polled() {
        assert!(should_poll(None));
    }

    #[test]
    fn should_poll_returns_false_when_recently_polled() {
        let recent = chrono::Utc::now() - chrono::Duration::seconds(10);
        assert!(!should_poll(Some(recent)));
    }

    #[test]
    fn should_poll_returns_true_when_interval_exceeded() {
        let old = chrono::Utc::now() - chrono::Duration::seconds(130);
        assert!(should_poll(Some(old)));
    }

    #[test]
    fn should_poll_boundary_at_exactly_120_seconds() {
        let exactly = chrono::Utc::now() - chrono::Duration::seconds(120);
        assert!(should_poll(Some(exactly)));
    }

    // ===========================================================================
    // Phase 7: T701/T702 — Artifact verification and PR creation
    // ===========================================================================

    #[test]
    fn verify_task_artifacts_passed() {
        let result = verify_task_artifacts("cargo test", "all 42 tests passed", true);
        assert_eq!(result.status, TestStatus::Passed);
        assert_eq!(result.command, "cargo test");
        assert_eq!(result.output, Some("all 42 tests passed".to_string()));
        assert_eq!(result.attempt, 1);
    }

    #[test]
    fn verify_task_artifacts_failed() {
        let result = verify_task_artifacts("cargo test", "2 tests failed", false);
        assert_eq!(result.status, TestStatus::Failed);
        assert_eq!(result.command, "cargo test");
    }

    #[test]
    fn generate_pr_title_formats_correctly() {
        let title = generate_pr_title("Add login endpoint", 42);
        assert_eq!(title, "feat: Add login endpoint (#42)");
    }

    #[test]
    fn generate_pr_title_trims_whitespace() {
        let title = generate_pr_title("  Fix bug  ", 7);
        assert_eq!(title, "feat: Fix bug (#7)");
    }

    #[test]
    fn generate_pr_body_includes_task_details() {
        use gwt_core::agent::types::TaskId;
        let task = Task::new(
            TaskId("task-99".to_string()),
            "Login flow",
            "Implement OAuth login",
        );
        let body = generate_pr_body(&task, 42);
        assert!(body.contains("Implement OAuth login"));
        assert!(body.contains("Closes #42"));
        assert!(body.contains("task-99"));
        assert!(body.contains("Login flow"));
    }

    // ===========================================================================
    // Phase 7: T703/T704 — CI monitoring and auto-fix loop
    // ===========================================================================

    #[test]
    fn check_ci_status_with_pr_returns_pending() {
        let pr = Some(PullRequestRef {
            number: 10,
            url: "https://github.com/org/repo/pull/10".to_string(),
        });
        let status = check_ci_status(&pr);
        assert_eq!(status, Some(CiStatus::Pending));
    }

    #[test]
    fn check_ci_status_without_pr_returns_none() {
        let status = check_ci_status(&None);
        assert!(status.is_none());
    }

    #[test]
    fn should_retry_ci_fix_within_limit() {
        assert!(should_retry_ci_fix(0));
        assert!(should_retry_ci_fix(1));
        assert!(should_retry_ci_fix(2));
    }

    #[test]
    fn should_retry_ci_fix_at_limit_stops() {
        assert!(!should_retry_ci_fix(3));
        assert!(!should_retry_ci_fix(4));
        assert!(!should_retry_ci_fix(255));
    }

    #[test]
    fn format_ci_fix_prompt_includes_output_and_task() {
        use gwt_core::agent::types::TaskId;
        let task = Task::new(TaskId("t-1".to_string()), "Build API", "Build the REST API");
        let prompt = format_ci_fix_prompt("error: unused variable `x`", &task);
        assert!(prompt.contains("Build API"));
        assert!(prompt.contains("error: unused variable `x`"));
        assert!(prompt.contains("CI failed"));
    }

    // ===========================================================================
    // Phase 7: T705/T706 — Developer context sharing
    // ===========================================================================

    #[test]
    fn format_merge_command_produces_valid_command() {
        let cmd = format_merge_command("feature/login", "develop");
        assert_eq!(cmd, "git checkout develop && git merge feature/login");
    }

    #[test]
    fn detect_merge_conflict_finds_conflict_marker() {
        assert!(detect_merge_conflict(
            "CONFLICT (content): Merge conflict in src/main.rs"
        ));
    }

    #[test]
    fn detect_merge_conflict_finds_diff_markers() {
        assert!(detect_merge_conflict(
            "<<<<<<< HEAD\nsome code\n=======\nother code\n>>>>>>> branch"
        ));
    }

    #[test]
    fn detect_merge_conflict_returns_false_for_clean_merge() {
        assert!(!detect_merge_conflict(
            "Merge made by the 'ort' strategy.\n 1 file changed"
        ));
    }

    #[test]
    fn detect_merge_conflict_finds_lowercase_marker() {
        assert!(detect_merge_conflict(
            "Auto-merging file.rs\nmerge conflict detected"
        ));
    }

    // ===========================================================================
    // Phase 8: T801/T802 — Layer independence guarantee
    // ===========================================================================

    #[test]
    fn is_layer_healthy_operational_statuses() {
        assert!(is_layer_healthy("idle"));
        assert!(is_layer_healthy("thinking"));
        assert!(is_layer_healthy("orchestrating"));
        assert!(is_layer_healthy("running"));
        assert!(is_layer_healthy("waiting_approval"));
        assert!(is_layer_healthy("ready"));
    }

    #[test]
    fn is_layer_healthy_case_insensitive() {
        assert!(is_layer_healthy("IDLE"));
        assert!(is_layer_healthy("Thinking"));
        assert!(is_layer_healthy("RUNNING"));
    }

    #[test]
    fn is_layer_healthy_unhealthy_statuses() {
        assert!(!is_layer_healthy("crashed"));
        assert!(!is_layer_healthy("error"));
        assert!(!is_layer_healthy("disconnected"));
        assert!(!is_layer_healthy(""));
    }

    #[test]
    fn classify_failure_lead_errors() {
        assert_eq!(
            classify_failure("Lead AI connection timeout"),
            FailureScope::Lead
        );
        assert_eq!(
            classify_failure("AI settings are required"),
            FailureScope::Lead
        );
        assert_eq!(classify_failure("Invalid API key"), FailureScope::Lead);
    }

    #[test]
    fn classify_failure_coordinator_errors() {
        assert_eq!(
            classify_failure("Coordinator pane crashed"),
            FailureScope::Coordinator
        );
        assert_eq!(
            classify_failure("Terminal pane disconnected"),
            FailureScope::Coordinator
        );
        assert_eq!(
            classify_failure("Session expired for coordinator"),
            FailureScope::Coordinator
        );
    }

    #[test]
    fn classify_failure_developer_errors() {
        assert_eq!(
            classify_failure("Build failed with exit code 1"),
            FailureScope::Developer
        );
        assert_eq!(
            classify_failure("Test compilation error"),
            FailureScope::Developer
        );
    }

    // ===========================================================================
    // Phase 8: T803/T804 — Coordinator auto-restart
    // ===========================================================================

    #[test]
    fn should_restart_coordinator_within_limit() {
        assert!(should_restart_coordinator(0, 3));
        assert!(should_restart_coordinator(1, 3));
        assert!(should_restart_coordinator(2, 3));
    }

    #[test]
    fn should_restart_coordinator_at_limit_stops() {
        assert!(!should_restart_coordinator(3, 3));
        assert!(!should_restart_coordinator(4, 3));
    }

    #[test]
    fn should_restart_coordinator_custom_max() {
        assert!(should_restart_coordinator(4, 5));
        assert!(!should_restart_coordinator(5, 5));
    }

    #[test]
    fn coordinator_restart_delay_exponential_backoff() {
        assert_eq!(coordinator_restart_delay_ms(0), 1000); // 1s
        assert_eq!(coordinator_restart_delay_ms(1), 2000); // 2s
        assert_eq!(coordinator_restart_delay_ms(2), 4000); // 4s
    }

    #[test]
    fn coordinator_restart_delay_higher_counts() {
        assert_eq!(coordinator_restart_delay_ms(3), 8000); // 8s
        assert_eq!(coordinator_restart_delay_ms(4), 16000); // 16s
    }

    // ===========================================================================
    // Phase 5: T501/T502 — Coordinator launch
    // ===========================================================================

    #[test]
    fn launch_coordinator_creates_starting_state() {
        use gwt_core::agent::coordinator::CoordinatorStatus;
        use gwt_core::agent::issue::{IssueStatus, ProjectIssue};

        let issue = ProjectIssue {
            id: "issue-42".to_string(),
            github_issue_number: 42,
            github_issue_url: "https://github.com/org/repo/issues/42".to_string(),
            title: "Login feature".to_string(),
            status: IssueStatus::Planned,
            coordinator: None,
            tasks: Vec::new(),
        };

        let coord = launch_coordinator(&issue, "coord-pane-42");
        assert_eq!(coord.pane_id, "coord-pane-42");
        assert_eq!(coord.status, CoordinatorStatus::Starting);
        assert_eq!(coord.github_issue_number, 42);
        assert_eq!(coord.crash_count, 0);
        assert!(coord.pid.is_none());
    }

    #[test]
    fn launch_coordinator_preserves_issue_number() {
        use gwt_core::agent::issue::{IssueStatus, ProjectIssue};

        let issue = ProjectIssue {
            id: "issue-99".to_string(),
            github_issue_number: 99,
            github_issue_url: "https://github.com/org/repo/issues/99".to_string(),
            title: "Another feature".to_string(),
            status: IssueStatus::InProgress,
            coordinator: None,
            tasks: Vec::new(),
        };

        let coord = launch_coordinator(&issue, "coord-99");
        assert_eq!(coord.github_issue_number, 99);
    }

    #[test]
    fn build_coordinator_prompt_includes_issue_info() {
        let prompt = build_coordinator_prompt(42, "Login feature");
        assert!(prompt.contains("#42"));
        assert!(prompt.contains("Login feature"));
        assert!(prompt.contains("Coordinator"));
        assert!(prompt.contains("GWT_TASK_DONE"));
    }

    // ===========================================================================
    // Phase 5: T503/T504 — Task split and Developer assignment
    // ===========================================================================

    #[test]
    fn assign_developers_to_task_creates_entries() {
        use gwt_core::agent::developer::{AgentType, DeveloperStatus};
        use gwt_core::agent::types::TaskId;

        let mut task = Task::new(
            TaskId("task-1".to_string()),
            "Write tests",
            "Write unit tests for auth module",
        );
        assert!(task.developers.is_empty());

        assign_developers_to_task(&mut task, AgentType::Claude, 2);
        assert_eq!(task.developers.len(), 2);

        for (i, dev) in task.developers.iter().enumerate() {
            assert_eq!(dev.agent_type, AgentType::Claude);
            assert_eq!(dev.status, DeveloperStatus::Starting);
            assert!(dev.pid.is_none());
            assert!(dev.pane_id.contains(&format!("dev-task-1-{}", i)));
            assert!(dev.worktree.branch_name.starts_with("agent/"));
            assert!(dev.completed_at.is_none());
            assert!(dev.completion_source.is_none());
        }
    }

    #[test]
    fn assign_developers_to_task_multiple_types() {
        use gwt_core::agent::developer::AgentType;
        use gwt_core::agent::types::TaskId;

        let mut task = Task::new(TaskId("task-2".to_string()), "impl", "implement feature");

        assign_developers_to_task(&mut task, AgentType::Claude, 1);
        assign_developers_to_task(&mut task, AgentType::Codex, 1);

        assert_eq!(task.developers.len(), 2);
        assert_eq!(task.developers[0].agent_type, AgentType::Claude);
        assert_eq!(task.developers[1].agent_type, AgentType::Codex);
    }

    #[test]
    fn assign_developers_zero_count_adds_nothing() {
        use gwt_core::agent::developer::AgentType;
        use gwt_core::agent::types::TaskId;

        let mut task = Task::new(TaskId("task-3".to_string()), "nothing", "desc");
        assign_developers_to_task(&mut task, AgentType::Gemini, 0);
        assert!(task.developers.is_empty());
    }

    // ===========================================================================
    // Phase 5: T505/T506 — Worktree/branch auto-creation
    // ===========================================================================

    #[test]
    fn create_developer_worktree_no_conflict() {
        let wt = create_developer_worktree(Path::new("/repo"), "add login form", &[]);
        assert_eq!(wt.branch_name, "agent/add-login-form");
        assert_eq!(
            wt.path,
            PathBuf::from("/repo/.worktrees/agent-add-login-form")
        );
    }

    #[test]
    fn create_developer_worktree_with_conflict() {
        let existing = vec!["agent/add-login-form".to_string()];
        let wt = create_developer_worktree(Path::new("/repo"), "add login form", &existing);
        assert_eq!(wt.branch_name, "agent/add-login-form-2");
    }

    // ===========================================================================
    // Phase 5: T507/T508 — Developer launch and prompt sending
    // ===========================================================================

    #[test]
    fn auto_mode_flag_claude() {
        use gwt_core::agent::developer::AgentType;
        assert_eq!(
            auto_mode_flag(AgentType::Claude),
            "--dangerously-skip-permissions"
        );
    }

    #[test]
    fn auto_mode_flag_codex() {
        use gwt_core::agent::developer::AgentType;
        assert_eq!(auto_mode_flag(AgentType::Codex), "--full-auto");
    }

    #[test]
    fn auto_mode_flag_gemini() {
        use gwt_core::agent::developer::AgentType;
        assert_eq!(auto_mode_flag(AgentType::Gemini), "auto");
    }

    #[test]
    fn agent_command_name_for_all_types() {
        use gwt_core::agent::developer::AgentType;
        assert_eq!(agent_command_name(AgentType::Claude), "claude");
        assert_eq!(agent_command_name(AgentType::Codex), "codex");
        assert_eq!(agent_command_name(AgentType::Gemini), "gemini");
    }

    #[test]
    fn build_developer_prompt_includes_task_info() {
        let prompt = build_developer_prompt("Write unit tests", "Test the auth module");
        assert!(prompt.contains("Write unit tests"));
        assert!(prompt.contains("Test the auth module"));
        assert!(prompt.contains("GWT_TASK_DONE"));
    }

    // ===========================================================================
    // Phase 6: T601/T602 — Developer completion detection
    // ===========================================================================

    #[test]
    fn completion_pattern_is_expected() {
        assert_eq!(COMPLETION_PATTERN, "GWT_TASK_DONE");
    }

    #[test]
    fn detect_output_pattern_finds_marker() {
        assert!(detect_output_pattern(
            "some output\nGWT_TASK_DONE\nmore output"
        ));
    }

    #[test]
    fn detect_output_pattern_no_marker() {
        assert!(!detect_output_pattern("some output\nno marker here\n"));
    }

    #[test]
    fn detect_output_pattern_empty_string() {
        assert!(!detect_output_pattern(""));
    }

    #[test]
    fn detect_output_pattern_partial_match() {
        assert!(!detect_output_pattern("GWT_TASK_DON"));
    }

    #[test]
    fn check_pane_exit_nonexistent_pane() {
        let state = AppState::new();
        assert!(check_pane_exit(&state, "nonexistent").is_none());
    }

    #[test]
    fn detect_developer_completion_hook_stop_highest_priority() {
        let state = AppState::new();
        let result = detect_developer_completion(
            &state,
            "pane-1",
            true,
            "GWT_TASK_DONE", // pattern also present
        );
        assert_eq!(result, Some(CompletionDetection::HookStop));
    }

    #[test]
    fn detect_developer_completion_output_pattern_second_priority() {
        let state = AppState::new();
        let result = detect_developer_completion(
            &state,
            "nonexistent",
            false,
            "output line 1\nGWT_TASK_DONE\nline 3",
        );
        assert_eq!(result, Some(CompletionDetection::OutputPattern));
    }

    #[test]
    fn detect_developer_completion_none_when_running() {
        let state = AppState::new();
        let result = detect_developer_completion(&state, "nonexistent", false, "still running...");
        assert!(result.is_none());
    }

    // ===========================================================================
    // Phase 6: T603/T604 — Lead progress reporting
    // ===========================================================================

    #[test]
    fn format_progress_report_truncates_to_max_lines() {
        let scrollback = (1..=20)
            .map(|i| format!("Line {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        let report = format_progress_report(&scrollback, 10);
        let lines: Vec<&str> = report.lines().collect();
        assert_eq!(lines.len(), 10);
        assert!(report.contains("Line 11"));
        assert!(report.contains("Line 20"));
        assert!(!report.contains("Line 10\n"));
    }

    #[test]
    fn format_progress_report_short_output_passes_through() {
        let scrollback = "Line 1\nLine 2\nLine 3";
        let report = format_progress_report(scrollback, 10);
        assert_eq!(report.lines().count(), 3);
        assert!(report.contains("Line 1"));
        assert!(report.contains("Line 3"));
    }

    #[test]
    fn format_progress_report_skips_empty_lines() {
        let scrollback = "Line 1\n\n\nLine 2\n\nLine 3\n\n";
        let report = format_progress_report(scrollback, 10);
        assert_eq!(report.lines().count(), 3);
    }

    #[test]
    fn format_progress_report_empty_input() {
        let report = format_progress_report("", 10);
        assert!(report.is_empty());
    }

    #[test]
    fn format_progress_report_max_lines_zero() {
        let report = format_progress_report("Line 1\nLine 2", 0);
        assert!(report.is_empty());
    }

    // ===========================================================================
    // Phase 9: T901/T902 — Session persistence
    // ===========================================================================

    fn make_test_app_state_with_store() -> (AppState, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = gwt_core::agent::SessionStore::with_dir(dir.path().to_path_buf());
        let mut state = AppState::new();
        // Replace the session store with one backed by a temp dir
        state.session_store = store;
        (state, dir)
    }

    #[test]
    fn save_project_mode_state_persists_session() {
        let (state, _dir) = make_test_app_state_with_store();
        let session = ProjectModeSession::new(
            SessionId("pt-save-1".to_string()),
            PathBuf::from("/repo"),
            "main",
            AgentType::Claude,
        );

        let result = save_project_mode_state(&state, &session);
        assert!(result.is_ok());

        // Verify it was persisted by loading it back
        let loaded = state
            .session_store
            .load_project_mode(&SessionId("pt-save-1".to_string()))
            .unwrap();
        assert_eq!(loaded.id.0, "pt-save-1");
        assert_eq!(
            loaded.status,
            gwt_core::agent::session::SessionStatus::Active
        );
    }

    #[test]
    fn save_project_mode_state_preserves_lead_conversation() {
        let (state, _dir) = make_test_app_state_with_store();
        let mut session = ProjectModeSession::new(
            SessionId("pt-save-2".to_string()),
            PathBuf::from("/repo"),
            "main",
            AgentType::Claude,
        );

        session.lead.conversation.push(LeadMessage::new(
            MessageRole::User,
            MessageKind::Message,
            "implement auth",
        ));
        session.lead.llm_call_count = 5;
        session.lead.estimated_tokens = 2000;

        save_project_mode_state(&state, &session).unwrap();

        let loaded = state
            .session_store
            .load_project_mode(&SessionId("pt-save-2".to_string()))
            .unwrap();
        assert_eq!(loaded.lead.conversation.len(), 1);
        assert_eq!(loaded.lead.conversation[0].content, "implement auth");
        assert_eq!(loaded.lead.llm_call_count, 5);
        assert_eq!(loaded.lead.estimated_tokens, 2000);
    }

    // ===========================================================================
    // Phase 9: T903/T904 — Session restore
    // ===========================================================================

    #[test]
    fn restore_project_mode_session_roundtrip() {
        let (state, _dir) = make_test_app_state_with_store();
        let mut session = ProjectModeSession::new(
            SessionId("pt-restore-1".to_string()),
            PathBuf::from("/repo"),
            "main",
            AgentType::Claude,
        );
        session.lead.status = LeadStatus::WaitingApproval;

        session.lead.conversation.push(LeadMessage::new(
            MessageRole::User,
            MessageKind::Message,
            "build a login page",
        ));
        session.lead.conversation.push(LeadMessage::new(
            MessageRole::Assistant,
            MessageKind::Thought,
            "analyzing requirements",
        ));
        session.lead.llm_call_count = 3;
        session.lead.estimated_tokens = 1500;

        state.session_store.save_project_mode(&session).unwrap();

        let (restored_session, restored_mode) =
            restore_project_mode_session(&state, "pt-restore-1").unwrap();

        // Verify session data
        assert_eq!(restored_session.id.0, "pt-restore-1");
        assert_eq!(restored_session.lead.conversation.len(), 2);
        assert_eq!(
            restored_session.lead.conversation[0].content,
            "build a login page"
        );

        // Verify mode reconstruction
        assert_eq!(
            restored_mode.project_mode_session_id,
            Some("pt-restore-1".to_string())
        );
        assert_eq!(
            restored_mode.lead_status,
            Some("waiting_approval".to_string())
        );
        assert_eq!(restored_mode.llm_call_count, 3);
        assert_eq!(restored_mode.estimated_tokens, 1500);
        assert_eq!(restored_mode.messages.len(), 2);
        assert_eq!(restored_mode.messages[0].role, "user");
        assert_eq!(restored_mode.messages[0].kind, "message");
        assert_eq!(restored_mode.messages[0].content, "build a login page");
        assert_eq!(restored_mode.messages[1].role, "assistant");
        assert_eq!(restored_mode.messages[1].kind, "thought");
        assert!(restored_mode.ai_ready);
    }

    #[test]
    fn restore_project_mode_session_not_found() {
        let (state, _dir) = make_test_app_state_with_store();

        let result = restore_project_mode_session(&state, "nonexistent");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn list_project_mode_sessions_returns_summaries() {
        let (state, _dir) = make_test_app_state_with_store();

        let s1 = ProjectModeSession::new(
            SessionId("pt-list-a".to_string()),
            PathBuf::from("/repo"),
            "main",
            AgentType::Claude,
        );
        let s2 = ProjectModeSession::new(
            SessionId("pt-list-b".to_string()),
            PathBuf::from("/repo2"),
            "develop",
            AgentType::Codex,
        );
        state.session_store.save_project_mode(&s1).unwrap();
        state.session_store.save_project_mode(&s2).unwrap();

        let summaries = list_project_mode_sessions(&state).unwrap();
        assert_eq!(summaries.len(), 2);

        let ids: Vec<&str> = summaries.iter().map(|s| s.session_id.as_str()).collect();
        assert!(ids.contains(&"pt-list-a"));
        assert!(ids.contains(&"pt-list-b"));

        for summary in &summaries {
            assert_eq!(summary.status, "active");
            assert!(summary.updated_at.is_some());
        }
    }

    #[test]
    fn list_project_mode_sessions_empty_store() {
        let (state, _dir) = make_test_app_state_with_store();

        let summaries = list_project_mode_sessions(&state).unwrap();
        assert!(summaries.is_empty());
    }

    // ===========================================================================
    // Phase 9: T905/T906 — Session force stop
    // ===========================================================================

    #[test]
    fn force_stop_project_mode_pauses_session() {
        let (state, _dir) = make_test_app_state_with_store();
        let mut session = ProjectModeSession::new(
            SessionId("pt-stop-1".to_string()),
            PathBuf::from("/repo"),
            "main",
            AgentType::Claude,
        );
        session.lead.status = LeadStatus::Thinking;
        state.session_store.save_project_mode(&session).unwrap();

        let msg = force_stop_project_mode(&state, "pt-stop-1").unwrap();
        assert!(msg.contains("paused"));

        // Verify session status changed
        let loaded = state
            .session_store
            .load_project_mode(&SessionId("pt-stop-1".to_string()))
            .unwrap();
        assert_eq!(
            loaded.status,
            gwt_core::agent::session::SessionStatus::Paused
        );
        assert_eq!(loaded.lead.status, LeadStatus::Idle);
    }

    #[test]
    fn force_stop_project_mode_not_found() {
        let (state, _dir) = make_test_app_state_with_store();

        let result = force_stop_project_mode(&state, "nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn force_stop_project_mode_saves_and_reloads() {
        let (state, _dir) = make_test_app_state_with_store();
        let mut session = ProjectModeSession::new(
            SessionId("pt-stop-2".to_string()),
            PathBuf::from("/repo"),
            "main",
            AgentType::Claude,
        );
        session.lead.status = LeadStatus::Orchestrating;
        session.lead.conversation.push(LeadMessage::new(
            MessageRole::User,
            MessageKind::Message,
            "build feature",
        ));
        state.session_store.save_project_mode(&session).unwrap();

        force_stop_project_mode(&state, "pt-stop-2").unwrap();

        // Verify conversation is preserved after stop
        let loaded = state
            .session_store
            .load_project_mode(&SessionId("pt-stop-2".to_string()))
            .unwrap();
        assert_eq!(loaded.lead.conversation.len(), 1);
        assert_eq!(loaded.lead.conversation[0].content, "build feature");
        assert_eq!(
            loaded.status,
            gwt_core::agent::session::SessionStatus::Paused
        );
    }
}
