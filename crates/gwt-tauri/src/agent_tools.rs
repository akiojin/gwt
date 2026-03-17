//! Built-in tool definitions for Project Mode LLM tool-call dispatch.
//!
//! # Terminal tools
//!
//! - `send_keys_to_pane` — Send text input to a specific agent pane.
//! - `send_keys_broadcast` — Broadcast text input to all running agent panes.
//! - `capture_scrollback_tail` — Capture the scrollback tail for a pane as plain text.
//!
//! # Issue-first spec tools
//!
//! - `upsert_spec_issue` — Create or update an issue-first spec artifact bundle for an issue.
//!   Merges section patches (spec, plan, tasks, tdd, research, data_model, quickstart,
//!   contracts, checklists) with existing data via optimistic-concurrency `expected_etag`.
//! - `get_spec_issue` — Get issue-first spec details for a given issue number.
//! - `append_spec_contract_comment` — Append a contract payload as an issue comment
//!   using the `contract:<name>` prefix.
//! - `upsert_spec_issue_artifact` — Create or update a spec artifact comment
//!   (contracts/checklists) with optional `expected_etag` for concurrency control.
//! - `list_spec_issue_artifacts` — List spec artifact comments (contracts/checklists)
//!   for an issue, optionally filtered by kind.
//! - `delete_spec_issue_artifact` — Delete a spec artifact comment for
//!   contracts/checklists from an issue.
//! - `sync_spec_issue_project` — Sync an issue-first spec issue to GitHub Project V2
//!   and update its phase status (draft/ready/planned/ready-for-dev/in-progress/done/blocked).

use serde_json::json;

use crate::commands::issue_spec::{
    delete_spec_issue_artifact_comment_cmd, list_spec_issue_artifact_comments_cmd,
    sync_spec_issue_project_cmd, upsert_spec_issue_artifact_comment_cmd,
};
use crate::commands::terminal::send_keys_broadcast_from_state;
use crate::state::AppState;
use crate::tool_helpers::{
    self, execute_shared_tool, get_optional_string_any, get_required_string_any,
    get_required_u64_any, normalize_args, shared_tool_definitions,
};
use gwt_core::ai::{ToolCall, ToolDefinition, ToolFunction};

// Agent-specific tool name constants
pub const TOOL_SEND_KEYS_BROADCAST: &str = "send_keys_broadcast";
pub const TOOL_APPEND_SPEC_CONTRACT_COMMENT: &str = "append_spec_contract_comment";
pub const TOOL_UPSERT_SPEC_ARTIFACT: &str = "upsert_spec_issue_artifact";
pub const TOOL_LIST_SPEC_ARTIFACTS: &str = "list_spec_issue_artifacts";
pub const TOOL_DELETE_SPEC_ARTIFACT: &str = "delete_spec_issue_artifact";
pub const TOOL_SYNC_SPEC_PROJECT: &str = "sync_spec_issue_project";

pub fn builtin_tool_definitions() -> Vec<ToolDefinition> {
    let mut tools = shared_tool_definitions();
    tools.extend(agent_specific_tool_definitions());
    tools
}

fn agent_specific_tool_definitions() -> Vec<ToolDefinition> {
    vec![
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

    // For shared tools that need project_path, resolve it here
    let project_path_lazy = || get_project_path_for_window(state, window_label);

    // Try shared tools first (they need project_path)
    if matches!(
        call.name.as_str(),
        tool_helpers::TOOL_SEND_KEYS_TO_PANE
            | tool_helpers::TOOL_CAPTURE_SCROLLBACK_TAIL
            | tool_helpers::TOOL_GET_SPEC_ISSUE
            | tool_helpers::TOOL_UPSERT_SPEC_ISSUE
    ) {
        // send_keys_to_pane and capture_scrollback_tail don't need project_path,
        // but get_spec_issue and upsert_spec_issue do.
        let project_path = match call.name.as_str() {
            tool_helpers::TOOL_GET_SPEC_ISSUE | tool_helpers::TOOL_UPSERT_SPEC_ISSUE => {
                project_path_lazy()?
            }
            _ => String::new(),
        };
        if let Some(result) = execute_shared_tool(call, &args, state, &project_path) {
            return result;
        }
    }

    // Agent-specific tools
    match call.name.as_str() {
        TOOL_SEND_KEYS_BROADCAST => {
            let text = get_required_string_any(&args, &["text"])?;
            let sent = send_keys_broadcast_from_state(state, text)?;
            Ok(sent.to_string())
        }
        TOOL_APPEND_SPEC_CONTRACT_COMMENT => {
            let project_path = project_path_lazy()?;
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
            )
            .map_err(|e| e.message)?;
            serde_json::to_string(&detail).map_err(|e| format!("Failed to serialize result: {e}"))
        }
        TOOL_UPSERT_SPEC_ARTIFACT => {
            let project_path = project_path_lazy()?;
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
            )
            .map_err(|e| e.message)?;
            serde_json::to_string(&detail).map_err(|e| format!("Failed to serialize result: {e}"))
        }
        TOOL_LIST_SPEC_ARTIFACTS => {
            let project_path = project_path_lazy()?;
            let issue_number = get_required_u64_any(&args, &["issue_number", "issueNumber"])?;
            let kind = get_optional_string_any(&args, &["kind"]).map(str::to_string);
            let comments = list_spec_issue_artifact_comments_cmd(project_path, issue_number, kind)
                .map_err(|e| e.message)?;
            serde_json::to_string(&comments).map_err(|e| format!("Failed to serialize result: {e}"))
        }
        TOOL_DELETE_SPEC_ARTIFACT => {
            let project_path = project_path_lazy()?;
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
            )
            .map_err(|e| e.message)?;
            Ok(json!({ "deleted": deleted }).to_string())
        }
        TOOL_SYNC_SPEC_PROJECT => {
            let project_path = project_path_lazy()?;
            let issue_number = get_required_u64_any(&args, &["issue_number", "issueNumber"])?;
            let phase = get_required_string_any(&args, &["phase"])?;
            let project_id = get_optional_string_any(&args, &["project_id", "projectId"])
                .map(str::to_string)
                .unwrap_or_default();
            let result = sync_spec_issue_project_cmd(
                project_path,
                issue_number,
                project_id,
                phase.to_string(),
            )
            .map_err(|e| e.message)?;
            serde_json::to_string(&result).map_err(|e| format!("Failed to serialize result: {e}"))
        }
        _ => Err(format!("Unknown tool: {}", call.name)),
    }
}

fn get_project_path_for_window(state: &AppState, window_label: &str) -> Result<String, String> {
    state
        .project_for_window(window_label)
        .ok_or_else(|| "Open a project before using Project Mode.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::TestEnvGuard;
    use crate::commands::ENV_LOCK;
    use crate::tool_helpers::{
        TOOL_CAPTURE_SCROLLBACK_TAIL, TOOL_GET_SPEC_ISSUE, TOOL_SEND_KEYS_TO_PANE,
        TOOL_UPSERT_SPEC_ISSUE,
    };
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
            terminal_shell: None,
            interactive: false,
            windows_force_utf8: false,
            project_root: std::env::temp_dir(),
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
