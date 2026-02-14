use gwt_core::ai::{AIClient, AIResponse, ChatMessage, ToolCall};
use gwt_core::config::ProfilesConfig;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

use crate::agent_tools::{builtin_tool_definitions, execute_tool_call};
use crate::state::AppState;

const MAX_TOOL_CALL_LOOPS: usize = 3;
const REQUIRED_SPEC_ARTIFACTS: [&str; 4] = ["spec.md", "plan.md", "tasks.md", "tdd.md"];
const SYSTEM_PROMPT: &str = "You are the master agent for gwt. Use ReAct and tool calls to send instructions to agent panes and capture output when needed. Keep instructions concise and in English.\n\nReAct format:\nThought: <short reasoning>\nAction: <tool name + short params summary>\nObservation: <tool result>\n\nRules:\n- Use tool calls for actions.\n- Do not fabricate observations; observations come from tool results.\n- Keep Thought to 2-4 lines.\n- When delegating to sub-agents, include a clear task and request a short completion summary.";
const SPEC_TEMPLATE: &str = include_str!("../../../.specify/templates/spec-template.md");
const PLAN_TEMPLATE: &str = include_str!("../../../.specify/templates/plan-template.md");
const TASKS_TEMPLATE: &str = include_str!("../../../.specify/templates/tasks-template.md");

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

#[derive(Debug, Clone)]
struct SpecKitPreparation {
    spec_id: String,
    spec_dir: PathBuf,
    created_files: Vec<String>,
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

    let prep = match prepare_spec_kit_artifacts_for_window(state, window_label, trimmed) {
        Ok(p) => p,
        Err(e) => {
            working.last_error = Some(e);
            working.is_waiting = false;
            save_agent_mode_state(state, window_label, &working);
            return working;
        }
    };
    if !prep.created_files.is_empty() {
        let files = prep.created_files.join(", ");
        let note = format!(
            "Prepared {} for {} at {}.",
            files,
            prep.spec_id,
            prep.spec_dir.display()
        );
        push_message(&mut working, "assistant", "observation", &note);
        save_agent_mode_state(state, window_label, &working);
    }

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
        let result = execute_tool_call(state, call).unwrap_or_else(|e| format!("error: {e}"));
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

fn prepare_spec_kit_artifacts_for_window(
    state: &AppState,
    window_label: &str,
    user_input: &str,
) -> Result<SpecKitPreparation, String> {
    let Some(project_path) = state.project_for_window(window_label) else {
        return Err("Open a project before using Agent Mode.".to_string());
    };
    prepare_spec_kit_artifacts(Path::new(&project_path), user_input)
}

fn prepare_spec_kit_artifacts(
    project_root: &Path,
    user_input: &str,
) -> Result<SpecKitPreparation, String> {
    let specs_root = project_root.join("specs");
    fs::create_dir_all(&specs_root)
        .map_err(|e| format!("Failed to create specs directory: {e}"))?;

    let (spec_id, spec_dir) = resolve_target_spec_dir(&specs_root, user_input)?;
    fs::create_dir_all(&spec_dir).map_err(|e| format!("Failed to create {}: {e}", spec_id))?;

    let mut created_files = Vec::new();

    ensure_spec_md(&spec_dir, &spec_id, user_input, &mut created_files)?;
    ensure_plan_md(&spec_dir, &spec_id, user_input, &mut created_files)?;
    ensure_tasks_md(&spec_dir, user_input, &mut created_files)?;
    ensure_tdd_md(&spec_dir, &spec_id, &mut created_files)?;

    let missing = missing_required_artifacts(&spec_dir);
    if !missing.is_empty() {
        return Err(format!(
            "Spec Kit artifacts are incomplete for {}: {}",
            spec_id,
            missing.join(", ")
        ));
    }

    Ok(SpecKitPreparation {
        spec_id,
        spec_dir,
        created_files,
    })
}

fn resolve_target_spec_dir(
    specs_root: &Path,
    user_input: &str,
) -> Result<(String, PathBuf), String> {
    if let Some(spec_id) = extract_spec_id(user_input) {
        return Ok((spec_id.clone(), specs_root.join(spec_id)));
    }

    if let Some(existing) = latest_spec_dir(specs_root)? {
        let spec_id = existing
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .ok_or_else(|| "Invalid spec directory name.".to_string())?;
        return Ok((spec_id, existing));
    }

    let spec_id = generate_spec_id();
    Ok((spec_id.clone(), specs_root.join(spec_id)))
}

fn latest_spec_dir(specs_root: &Path) -> Result<Option<PathBuf>, String> {
    let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
    let entries = fs::read_dir(specs_root).map_err(|e| format!("Failed to list specs: {e}"))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("Failed to read specs entry: {e}"))?;
        if !entry
            .file_type()
            .map_err(|e| format!("Failed to read file type: {e}"))?
            .is_dir()
        {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("SPEC-") {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        match &newest {
            Some((current, _)) if &modified <= current => {}
            _ => newest = Some((modified, entry.path())),
        }
    }
    Ok(newest.map(|(_, path)| path))
}

fn ensure_spec_md(
    spec_dir: &Path,
    spec_id: &str,
    user_input: &str,
    created_files: &mut Vec<String>,
) -> Result<(), String> {
    let path = spec_dir.join("spec.md");
    if artifact_is_file(&path) {
        return Ok(());
    }
    if path.exists() {
        return Err(format!(
            "Failed to write {}: path exists but is not a file",
            path.display()
        ));
    }
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let feature = feature_name_from_input(user_input);
    let input = collapse_whitespace(user_input);
    let content = render_template(
        SPEC_TEMPLATE,
        &[
            ("[FEATURE_NAME]", feature),
            ("[SPEC_ID]", spec_id.to_string()),
            ("[DATE]", date.clone()),
            ("[UPDATED_DATE]", date),
            ("[INPUT]", input),
        ],
    );
    fs::write(&path, content).map_err(|e| format!("Failed to write {}: {e}", path.display()))?;
    created_files.push("spec.md".to_string());
    Ok(())
}

fn ensure_plan_md(
    spec_dir: &Path,
    spec_id: &str,
    user_input: &str,
    created_files: &mut Vec<String>,
) -> Result<(), String> {
    let path = spec_dir.join("plan.md");
    if artifact_is_file(&path) {
        return Ok(());
    }
    if path.exists() {
        return Err(format!(
            "Failed to write {}: path exists but is not a file",
            path.display()
        ));
    }
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let feature = feature_name_from_input(user_input);
    let content = render_template(
        PLAN_TEMPLATE,
        &[
            ("[FEATURE_NAME]", feature),
            ("[SPEC_ID]", spec_id.to_string()),
            ("[DATE]", date),
        ],
    );
    fs::write(&path, content).map_err(|e| format!("Failed to write {}: {e}", path.display()))?;
    created_files.push("plan.md".to_string());
    Ok(())
}

fn ensure_tasks_md(
    spec_dir: &Path,
    user_input: &str,
    created_files: &mut Vec<String>,
) -> Result<(), String> {
    let path = spec_dir.join("tasks.md");
    if artifact_is_file(&path) {
        return Ok(());
    }
    if path.exists() {
        return Err(format!(
            "Failed to write {}: path exists but is not a file",
            path.display()
        ));
    }
    let feature = feature_name_from_input(user_input);
    let content = render_template(TASKS_TEMPLATE, &[("[FEATURE_NAME]", feature)]);
    fs::write(&path, content).map_err(|e| format!("Failed to write {}: {e}", path.display()))?;
    created_files.push("tasks.md".to_string());
    Ok(())
}

fn ensure_tdd_md(
    spec_dir: &Path,
    spec_id: &str,
    created_files: &mut Vec<String>,
) -> Result<(), String> {
    let path = spec_dir.join("tdd.md");
    if artifact_is_file(&path) {
        return Ok(());
    }
    if path.exists() {
        return Err(format!(
            "Failed to write {}: path exists but is not a file",
            path.display()
        ));
    }
    let tasks_path = spec_dir.join("tasks.md");
    let tasks = fs::read_to_string(&tasks_path)
        .map_err(|e| format!("Failed to read {}: {e}", tasks_path.display()))?;
    let content = generate_tdd_markdown(spec_id, &tasks);
    fs::write(&path, content).map_err(|e| format!("Failed to write {}: {e}", path.display()))?;
    created_files.push("tdd.md".to_string());
    Ok(())
}

fn generate_tdd_markdown(spec_id: &str, tasks_content: &str) -> String {
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let task_lines: Vec<String> = tasks_content
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("- [ ]") || line.starts_with("- [x]"))
        .map(|line| {
            line.trim_start_matches("- [ ]")
                .trim_start_matches("- [x]")
                .trim()
                .to_string()
        })
        .filter(|line| !line.is_empty())
        .take(30)
        .collect();

    let mut out = String::new();
    out.push_str(&format!("# TDD テスト仕様: {}\n\n", spec_id));
    out.push_str(&format!("**作成日**: {}\n\n", date));
    out.push_str("## テスト戦略\n\n");
    out.push_str("- まず失敗するテストを作成し、その後で実装を行う。\n");
    out.push_str("- 各タスクは最小単位でテスト可能な形に分解する。\n");
    out.push_str("- 回帰防止のため、修正時は関連テストを追加する。\n\n");
    out.push_str("## タスク対応テスト\n\n");
    if task_lines.is_empty() {
        out.push_str("- tasks.md から自動抽出できるタスクが見つからなかったため、手動でテストケースを追記する。\n");
    } else {
        for (idx, task) in task_lines.iter().enumerate() {
            out.push_str(&format!("{}. `{}`\n", idx + 1, task));
        }
    }
    out.push_str("\n## テスト実行コマンド\n\n");
    out.push_str("```bash\ncargo test\ncd gwt-gui && pnpm test\n```\n");
    out
}

fn missing_required_artifacts(spec_dir: &Path) -> Vec<String> {
    REQUIRED_SPEC_ARTIFACTS
        .iter()
        .filter_map(|name| {
            let path = spec_dir.join(name);
            if artifact_is_file(&path) {
                None
            } else {
                Some((*name).to_string())
            }
        })
        .collect()
}

fn artifact_is_file(path: &Path) -> bool {
    fs::metadata(path).map(|m| m.is_file()).unwrap_or(false)
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

fn feature_name_from_input(user_input: &str) -> String {
    let collapsed = collapse_whitespace(user_input);
    let mut title: String = collapsed.chars().take(64).collect();
    if title.is_empty() {
        title = "Agent Mode Task".to_string();
    }
    title
}

fn collapse_whitespace(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn render_template(template: &str, replacements: &[(&str, String)]) -> String {
    let mut out = template.to_string();
    for (needle, value) in replacements {
        out = out.replace(needle, value);
    }
    out
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
    use std::fs;

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
    fn prepare_spec_kit_artifacts_for_window_requires_open_project() {
        let state = AppState::new();
        let err = prepare_spec_kit_artifacts_for_window(&state, "main", "implement auth");
        assert!(err.is_err());
        assert!(err
            .unwrap_err()
            .contains("Open a project before using Agent Mode."));
    }

    #[test]
    fn prepare_spec_kit_artifacts_creates_required_files() {
        let temp = tempfile::TempDir::new().unwrap();
        let prep = prepare_spec_kit_artifacts(temp.path(), "Implement authentication")
            .expect("should prepare spec artifacts");

        assert!(prep.spec_id.starts_with("SPEC-"));
        for name in REQUIRED_SPEC_ARTIFACTS {
            assert!(prep.spec_dir.join(name).exists(), "missing {}", name);
        }
        assert!(prep.created_files.iter().any(|f| f == "tdd.md"));
    }

    #[test]
    fn prepare_spec_kit_artifacts_uses_explicit_spec_id_and_generates_tdd() {
        let temp = tempfile::TempDir::new().unwrap();
        let spec_dir = temp.path().join("specs").join("SPEC-deadbeef");
        fs::create_dir_all(&spec_dir).unwrap();
        fs::write(spec_dir.join("spec.md"), "# spec\n").unwrap();
        fs::write(spec_dir.join("plan.md"), "# plan\n").unwrap();
        fs::write(
            spec_dir.join("tasks.md"),
            "# tasks\n\n- [ ] T001 [US1] [実装] sample\n",
        )
        .unwrap();

        let prep = prepare_spec_kit_artifacts(temp.path(), "continue SPEC-deadbeef")
            .expect("should use explicit spec id");
        assert_eq!(prep.spec_id, "SPEC-deadbeef");
        assert!(spec_dir.join("tdd.md").exists());

        let tdd = fs::read_to_string(spec_dir.join("tdd.md")).unwrap();
        assert!(tdd.contains("T001 [US1] [実装] sample"));
    }

    #[test]
    fn prepare_spec_kit_artifacts_rejects_non_file_artifact_paths() {
        let temp = tempfile::TempDir::new().unwrap();
        let spec_dir = temp.path().join("specs").join("SPEC-feedface");
        fs::create_dir_all(&spec_dir).unwrap();
        fs::create_dir_all(spec_dir.join("spec.md")).unwrap();

        let err = prepare_spec_kit_artifacts(temp.path(), "continue SPEC-feedface")
            .expect_err("should reject directories where artifact files are expected");
        assert!(err.contains("path exists but is not a file"));
    }

    #[test]
    fn missing_required_artifacts_requires_regular_files() {
        let temp = tempfile::TempDir::new().unwrap();
        let spec_dir = temp.path().join("specs").join("SPEC-cafebabe");
        fs::create_dir_all(&spec_dir).unwrap();
        fs::write(spec_dir.join("spec.md"), "# spec\n").unwrap();
        fs::write(spec_dir.join("plan.md"), "# plan\n").unwrap();
        fs::create_dir_all(spec_dir.join("tasks.md")).unwrap();
        fs::write(spec_dir.join("tdd.md"), "# tdd\n").unwrap();

        let missing = missing_required_artifacts(&spec_dir);
        assert_eq!(missing, vec!["tasks.md".to_string()]);
    }
}
