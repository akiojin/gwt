use serde_json::{json, Value};

use crate::commands::issue_spec::{
    delete_spec_issue_artifact_comment_cmd, get_spec_issue_detail_cmd,
    list_spec_issue_artifact_comments_cmd, sync_spec_issue_project_cmd,
    upsert_spec_issue_artifact_comment_cmd, upsert_spec_issue_cmd, SpecIssueSectionsData,
};
use crate::commands::terminal::{
    capture_scrollback_tail_from_state, send_keys_broadcast_from_state,
    send_keys_to_pane_from_state,
};
use crate::state::AppState;
use gwt_core::ai::{ToolCall, ToolDefinition, ToolFunction};
use gwt_core::config::Settings;

pub const TOOL_SEND_KEYS_TO_PANE: &str = "send_keys_to_pane";
pub const TOOL_SEND_KEYS_BROADCAST: &str = "send_keys_broadcast";
pub const TOOL_CAPTURE_SCROLLBACK_TAIL: &str = "capture_scrollback_tail";
pub const TOOL_UPSERT_SPEC_ISSUE: &str = "upsert_spec_issue";
pub const TOOL_GET_SPEC_ISSUE: &str = "get_spec_issue";
pub const TOOL_APPEND_SPEC_CONTRACT_COMMENT: &str = "append_spec_contract_comment";
pub const TOOL_UPSERT_SPEC_ARTIFACT: &str = "upsert_spec_issue_artifact";
pub const TOOL_LIST_SPEC_ARTIFACTS: &str = "list_spec_issue_artifacts";
pub const TOOL_DELETE_SPEC_ARTIFACT: &str = "delete_spec_issue_artifact";
pub const TOOL_SYNC_SPEC_PROJECT: &str = "sync_spec_issue_project";

pub fn builtin_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            tool_type: "function".to_string(),
            function: ToolFunction {
                name: TOOL_SEND_KEYS_TO_PANE.to_string(),
                description: "Send text input to a specific agent pane.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "pane_id": { "type": "string" },
                        "text": { "type": "string" }
                    },
                    "required": ["pane_id", "text"]
                }),
            },
        },
        ToolDefinition {
            tool_type: "function".to_string(),
            function: ToolFunction {
                name: TOOL_SEND_KEYS_BROADCAST.to_string(),
                description: "Broadcast text input to all running agent panes.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "text": { "type": "string" }
                    },
                    "required": ["text"]
                }),
            },
        },
        ToolDefinition {
            tool_type: "function".to_string(),
            function: ToolFunction {
                name: TOOL_CAPTURE_SCROLLBACK_TAIL.to_string(),
                description: "Capture the scrollback tail for a pane as plain text.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "pane_id": { "type": "string" },
                        "max_bytes": { "type": "integer", "minimum": 0 }
                    },
                    "required": ["pane_id"]
                }),
            },
        },
        ToolDefinition {
            tool_type: "function".to_string(),
            function: ToolFunction {
                name: TOOL_UPSERT_SPEC_ISSUE.to_string(),
                description: "Create or update an issue-first spec artifact bundle for a SPEC ID."
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "spec_id": { "type": "string" },
                        "title": { "type": "string" },
                        "expected_etag": { "type": "string" },
                        "sections": {
                            "type": "object",
                            "properties": {
                                "spec": { "type": "string" },
                                "plan": { "type": "string" },
                                "tasks": { "type": "string" },
                                "tdd": { "type": "string" },
                                "research": { "type": "string" },
                                "data_model": { "type": "string" },
                                "quickstart": { "type": "string" },
                                "contracts": { "type": "string" },
                                "checklists": { "type": "string" }
                            }
                        }
                    },
                    "required": ["spec_id", "title", "sections"]
                }),
            },
        },
        ToolDefinition {
            tool_type: "function".to_string(),
            function: ToolFunction {
                name: TOOL_GET_SPEC_ISSUE.to_string(),
                description: "Get issue-first spec details for an issue number.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "issue_number": { "type": "integer", "minimum": 1 }
                    },
                    "required": ["issue_number"]
                }),
            },
        },
        ToolDefinition {
            tool_type: "function".to_string(),
            function: ToolFunction {
                name: TOOL_APPEND_SPEC_CONTRACT_COMMENT.to_string(),
                description:
                    "Append a contract payload as an issue comment using contract:<name> prefix."
                        .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "issue_number": { "type": "integer", "minimum": 1 },
                        "contract_name": { "type": "string" },
                        "content": { "type": "string" }
                    },
                    "required": ["issue_number", "contract_name", "content"]
                }),
            },
        },
        ToolDefinition {
            tool_type: "function".to_string(),
            function: ToolFunction {
                name: TOOL_UPSERT_SPEC_ARTIFACT.to_string(),
                description: "Create or update a spec artifact comment for contracts/checklists."
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "issue_number": { "type": "integer", "minimum": 1 },
                        "kind": { "type": "string", "enum": ["contract", "checklist"] },
                        "artifact_name": { "type": "string" },
                        "content": { "type": "string" },
                        "expected_etag": { "type": "string" }
                    },
                    "required": ["issue_number", "kind", "artifact_name", "content"]
                }),
            },
        },
        ToolDefinition {
            tool_type: "function".to_string(),
            function: ToolFunction {
                name: TOOL_LIST_SPEC_ARTIFACTS.to_string(),
                description: "List spec artifact comments (contracts/checklists) for an issue."
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "issue_number": { "type": "integer", "minimum": 1 },
                        "kind": { "type": "string", "enum": ["contract", "checklist"] }
                    },
                    "required": ["issue_number"]
                }),
            },
        },
        ToolDefinition {
            tool_type: "function".to_string(),
            function: ToolFunction {
                name: TOOL_DELETE_SPEC_ARTIFACT.to_string(),
                description:
                    "Delete a spec artifact comment for contracts/checklists from an issue."
                        .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "issue_number": { "type": "integer", "minimum": 1 },
                        "kind": { "type": "string", "enum": ["contract", "checklist"] },
                        "artifact_name": { "type": "string" },
                        "expected_etag": { "type": "string" }
                    },
                    "required": ["issue_number", "kind", "artifact_name"]
                }),
            },
        },
        ToolDefinition {
            tool_type: "function".to_string(),
            function: ToolFunction {
                name: TOOL_SYNC_SPEC_PROJECT.to_string(),
                description:
                    "Sync an issue-first spec issue to GitHub Project V2 and update status."
                        .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "issue_number": { "type": "integer", "minimum": 1 },
                        "phase": {
                            "type": "string",
                            "enum": ["draft", "ready", "planned", "ready-for-dev", "in-progress", "done", "blocked"]
                        },
                        "project_id": { "type": "string" }
                    },
                    "required": ["issue_number", "phase"]
                }),
            },
        },
    ]
}

pub fn execute_tool_call(
    state: &AppState,
    window_label: &str,
    call: &ToolCall,
) -> Result<String, String> {
    let args = normalize_args(&call.arguments)?;
    match call.name.as_str() {
        TOOL_SEND_KEYS_TO_PANE => {
            let pane_id = get_required_string_any(&args, &["pane_id", "paneId"])?;
            let text = get_required_string_any(&args, &["text"])?;
            send_keys_to_pane_from_state(state, pane_id, text)?;
            Ok("ok".to_string())
        }
        TOOL_SEND_KEYS_BROADCAST => {
            let text = get_required_string_any(&args, &["text"])?;
            let sent = send_keys_broadcast_from_state(state, text)?;
            Ok(sent.to_string())
        }
        TOOL_CAPTURE_SCROLLBACK_TAIL => {
            let pane_id = get_required_string_any(&args, &["pane_id", "paneId"])?;
            let max_bytes =
                get_optional_u64_any(&args, &["max_bytes", "maxBytes"]).map(|v| v as usize);
            match max_bytes {
                Some(limit) => capture_scrollback_tail_from_state(state, pane_id, limit),
                None => capture_scrollback_tail_from_state(state, pane_id, 0),
            }
        }
        TOOL_UPSERT_SPEC_ISSUE => {
            let project_path = get_project_path_for_window(state, window_label)?;
            let spec_id = get_required_string_any(&args, &["spec_id", "specId"])?;
            let title = get_required_string_any(&args, &["title"])?;
            let sections = get_required_object_any(&args, &["sections"])?;
            let expected_etag = get_optional_string_any(&args, &["expected_etag", "expectedEtag"])
                .map(str::to_string);
            let detail = upsert_spec_issue_cmd(
                project_path,
                spec_id.to_string(),
                title.to_string(),
                parse_sections_data(sections),
                expected_etag,
            )?;
            serde_json::to_string(&detail).map_err(|e| format!("Failed to serialize result: {e}"))
        }
        TOOL_GET_SPEC_ISSUE => {
            let project_path = get_project_path_for_window(state, window_label)?;
            let issue_number = get_required_u64_any(&args, &["issue_number", "issueNumber"])?;
            let detail = get_spec_issue_detail_cmd(project_path, issue_number)?;
            serde_json::to_string(&detail).map_err(|e| format!("Failed to serialize result: {e}"))
        }
        TOOL_APPEND_SPEC_CONTRACT_COMMENT => {
            let project_path = get_project_path_for_window(state, window_label)?;
            let issue_number = get_required_u64_any(&args, &["issue_number", "issueNumber"])?;
            let contract_name = get_required_string_any(&args, &["contract_name", "contractName"])?;
            let content = get_required_string_any(&args, &["content"])?;
            let detail = upsert_spec_issue_artifact_comment_cmd(
                project_path,
                issue_number,
                "contract".to_string(),
                contract_name.to_string(),
                content.to_string(),
                None,
            )?;
            serde_json::to_string(&detail).map_err(|e| format!("Failed to serialize result: {e}"))
        }
        TOOL_UPSERT_SPEC_ARTIFACT => {
            let project_path = get_project_path_for_window(state, window_label)?;
            let issue_number = get_required_u64_any(&args, &["issue_number", "issueNumber"])?;
            let kind = get_required_string_any(&args, &["kind"])?;
            let artifact_name = get_required_string_any(&args, &["artifact_name", "artifactName"])?;
            let content = get_required_string_any(&args, &["content"])?;
            let expected_etag = get_optional_string_any(&args, &["expected_etag", "expectedEtag"])
                .map(str::to_string);
            let detail = upsert_spec_issue_artifact_comment_cmd(
                project_path,
                issue_number,
                kind.to_string(),
                artifact_name.to_string(),
                content.to_string(),
                expected_etag,
            )?;
            serde_json::to_string(&detail).map_err(|e| format!("Failed to serialize result: {e}"))
        }
        TOOL_LIST_SPEC_ARTIFACTS => {
            let project_path = get_project_path_for_window(state, window_label)?;
            let issue_number = get_required_u64_any(&args, &["issue_number", "issueNumber"])?;
            let kind = get_optional_string_any(&args, &["kind"]).map(str::to_string);
            let comments = list_spec_issue_artifact_comments_cmd(project_path, issue_number, kind)?;
            serde_json::to_string(&comments).map_err(|e| format!("Failed to serialize result: {e}"))
        }
        TOOL_DELETE_SPEC_ARTIFACT => {
            let project_path = get_project_path_for_window(state, window_label)?;
            let issue_number = get_required_u64_any(&args, &["issue_number", "issueNumber"])?;
            let kind = get_required_string_any(&args, &["kind"])?;
            let artifact_name = get_required_string_any(&args, &["artifact_name", "artifactName"])?;
            let expected_etag = get_optional_string_any(&args, &["expected_etag", "expectedEtag"])
                .map(str::to_string);
            let deleted = delete_spec_issue_artifact_comment_cmd(
                project_path,
                issue_number,
                kind.to_string(),
                artifact_name.to_string(),
                expected_etag,
            )?;
            Ok(json!({ "deleted": deleted }).to_string())
        }
        TOOL_SYNC_SPEC_PROJECT => {
            let project_path = get_project_path_for_window(state, window_label)?;
            let issue_number = get_required_u64_any(&args, &["issue_number", "issueNumber"])?;
            let phase = get_required_string_any(&args, &["phase"])?;
            let project_id = match get_optional_string_any(&args, &["project_id", "projectId"]) {
                Some(v) if !v.trim().is_empty() => v.to_string(),
                _ => {
                    let settings =
                        Settings::load(std::path::Path::new(&project_path)).unwrap_or_default();
                    settings.agent.github_project_id.unwrap_or_default()
                }
            };
            let result = sync_spec_issue_project_cmd(
                project_path,
                issue_number,
                project_id,
                phase.to_string(),
            )?;
            serde_json::to_string(&result).map_err(|e| format!("Failed to serialize result: {e}"))
        }
        _ => Err(format!("Unknown tool: {}", call.name)),
    }
}

fn normalize_args(value: &Value) -> Result<Value, String> {
    if let Some(text) = value.as_str() {
        serde_json::from_str(text).map_err(|e| format!("Invalid tool arguments: {e}"))
    } else {
        Ok(value.clone())
    }
}

fn get_required_string_any<'a>(value: &'a Value, keys: &[&str]) -> Result<&'a str, String> {
    for key in keys {
        if let Some(found) = value
            .get(*key)
            .and_then(|v| v.as_str())
            .filter(|v| !v.trim().is_empty())
        {
            return Ok(found);
        }
    }
    Err(format!("Missing required argument: {}", keys.join(" or ")))
}

fn get_optional_string_any<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    for key in keys {
        if let Some(found) = value
            .get(*key)
            .and_then(|v| v.as_str())
            .filter(|v| !v.trim().is_empty())
        {
            return Some(found);
        }
    }
    None
}

fn get_required_object_any<'a>(value: &'a Value, keys: &[&str]) -> Result<&'a Value, String> {
    for key in keys {
        if let Some(found) = value.get(*key).filter(|v| v.is_object()) {
            return Ok(found);
        }
    }
    Err(format!("Missing required argument: {}", keys.join(" or ")))
}

fn get_required_u64_any(value: &Value, keys: &[&str]) -> Result<u64, String> {
    for key in keys {
        if let Some(found) = value.get(*key).and_then(|v| v.as_u64()) {
            return Ok(found);
        }
    }
    Err(format!("Missing required argument: {}", keys.join(" or ")))
}

fn get_optional_u64_any(value: &Value, keys: &[&str]) -> Option<u64> {
    for key in keys {
        if let Some(v) = value.get(*key).and_then(|v| v.as_u64()) {
            return Some(v);
        }
    }
    None
}

fn parse_sections_data(value: &Value) -> SpecIssueSectionsData {
    let read = |keys: &[&str]| -> String {
        for key in keys {
            if let Some(found) = value.get(*key).and_then(|v| v.as_str()) {
                return found.to_string();
            }
        }
        String::new()
    };
    SpecIssueSectionsData {
        spec: read(&["spec"]),
        plan: read(&["plan"]),
        tasks: read(&["tasks"]),
        tdd: read(&["tdd"]),
        research: read(&["research"]),
        data_model: read(&["data_model", "dataModel"]),
        quickstart: read(&["quickstart"]),
        contracts: read(&["contracts"]),
        checklists: read(&["checklists"]),
    }
}

fn get_project_path_for_window(state: &AppState, window_label: &str) -> Result<String, String> {
    state
        .project_for_window(window_label)
        .ok_or_else(|| "Open a project before using Agent Mode.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::TestEnvGuard;
    use crate::commands::ENV_LOCK;
    use gwt_core::terminal::pane::{PaneConfig, TerminalPane};
    use gwt_core::terminal::AgentColor;

    #[test]
    fn builtin_tool_definitions_has_expected_names() {
        let names: Vec<String> = builtin_tool_definitions()
            .into_iter()
            .map(|t| t.function.name)
            .collect();
        assert!(names.contains(&TOOL_SEND_KEYS_TO_PANE.to_string()));
        assert!(names.contains(&TOOL_SEND_KEYS_BROADCAST.to_string()));
        assert!(names.contains(&TOOL_CAPTURE_SCROLLBACK_TAIL.to_string()));
        assert!(names.contains(&TOOL_UPSERT_SPEC_ISSUE.to_string()));
        assert!(names.contains(&TOOL_GET_SPEC_ISSUE.to_string()));
        assert!(names.contains(&TOOL_APPEND_SPEC_CONTRACT_COMMENT.to_string()));
        assert!(names.contains(&TOOL_UPSERT_SPEC_ARTIFACT.to_string()));
        assert!(names.contains(&TOOL_LIST_SPEC_ARTIFACTS.to_string()));
        assert!(names.contains(&TOOL_DELETE_SPEC_ARTIFACT.to_string()));
        assert!(names.contains(&TOOL_SYNC_SPEC_PROJECT.to_string()));
    }

    #[test]
    fn execute_tool_call_captures_scrollback() {
        let _lock = ENV_LOCK.lock().unwrap();
        let home = tempfile::TempDir::new().unwrap();
        let _env = TestEnvGuard::new(home.path());

        let state = AppState::new();
        let pane_id = "pane-tool-test";
        let pane = TerminalPane::new(PaneConfig {
            pane_id: pane_id.to_string(),
            command: "/bin/cat".to_string(),
            args: vec![],
            working_dir: std::env::temp_dir(),
            branch_name: "test-branch".to_string(),
            agent_name: "test-agent".to_string(),
            agent_color: AgentColor::Green,
            rows: 24,
            cols: 80,
            env_vars: Default::default(),
        })
        .expect("failed to create test pane");

        {
            let mut mgr = state.pane_manager.lock().unwrap();
            mgr.add_pane(pane).expect("failed to add test pane");
            let pane = mgr.pane_mut_by_id(pane_id).expect("missing test pane");
            pane.process_bytes(b"hello\n").expect("write scrollback");
        }

        let call = ToolCall {
            name: TOOL_CAPTURE_SCROLLBACK_TAIL.to_string(),
            arguments: json!({ "pane_id": pane_id }),
            call_id: None,
        };
        let result = execute_tool_call(&state, "main", &call).expect("tool call");
        assert!(result.contains("hello"));
    }
}
