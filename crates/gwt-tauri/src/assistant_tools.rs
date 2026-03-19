#![allow(dead_code)]
//! Built-in tool definitions for Assistant Mode LLM tool-call dispatch.

use std::{path::Path, time::Duration};

use gwt_core::{
    ai::{ToolCall, ToolDefinition, ToolFunction},
    process::command as process_command,
};
use serde_json::json;

use crate::{
    state::AppState,
    tool_helpers::{
        execute_shared_tool_with_mode, get_optional_string_any, get_optional_u64_any,
        get_required_string_any, normalize_args, shared_tool_definitions_for_mode, ToolAccessMode,
    },
};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

// Tool name constants (assistant-specific)
pub const TOOL_READ_FILE: &str = "read_file";
pub const TOOL_GREP_FILE: &str = "grep_file";
pub const TOOL_LIST_DIRECTORY: &str = "list_directory";
pub const TOOL_GIT_LOG: &str = "git_log";
pub const TOOL_GIT_DIFF: &str = "git_diff";
pub const TOOL_GIT_STATUS: &str = "git_status";
pub const TOOL_RUN_COMMAND: &str = "run_command";
pub const TOOL_LIST_PANES: &str = "list_panes";
pub const TOOL_LIST_ISSUES: &str = "list_issues";
pub const TOOL_LIST_PULL_REQUESTS: &str = "list_pull_requests";
pub const TOOL_LIST_CONSULTATIONS: &str = "list_consultations";
pub const TOOL_READ_CONSULTATION: &str = "read_consultation";
pub const TOOL_RESPOND_TO_CONSULTATION: &str = "respond_to_consultation";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssistantToolMode {
    FullAccess,
    ReadOnly,
}

pub fn assistant_tool_definitions(mode: AssistantToolMode) -> Vec<ToolDefinition> {
    let access_mode = shared_tool_access_mode(mode);
    let mut tools = assistant_specific_tool_definitions(access_mode);
    tools.extend(shared_tool_definitions_for_mode(access_mode));
    tools.retain(|tool| assistant_tool_allowed(&tool.function.name, mode));
    tools
}

fn shared_tool_access_mode(mode: AssistantToolMode) -> ToolAccessMode {
    match mode {
        AssistantToolMode::FullAccess => ToolAccessMode::Full,
        AssistantToolMode::ReadOnly => ToolAccessMode::ReadOnly,
    }
}

fn assistant_specific_tool_definitions(access_mode: ToolAccessMode) -> Vec<ToolDefinition> {
    let mut tools = vec![
        ToolDefinition {
            tool_type: "function".to_string(),
            function: ToolFunction {
                name: TOOL_READ_FILE.to_string(),
                description: "Read the contents of a file.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "File path relative to project root" }
                    },
                    "required": ["path"]
                }),
            },
        },
        ToolDefinition {
            tool_type: "function".to_string(),
            function: ToolFunction {
                name: TOOL_GREP_FILE.to_string(),
                description: "Search for a pattern in a file or directory using grep.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "pattern": { "type": "string", "description": "Search pattern (regex)" },
                        "path": { "type": "string", "description": "File or directory path relative to project root" }
                    },
                    "required": ["pattern", "path"]
                }),
            },
        },
        ToolDefinition {
            tool_type: "function".to_string(),
            function: ToolFunction {
                name: TOOL_LIST_DIRECTORY.to_string(),
                description: "List files and directories in a path.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Directory path relative to project root" }
                    },
                    "required": ["path"]
                }),
            },
        },
        ToolDefinition {
            tool_type: "function".to_string(),
            function: ToolFunction {
                name: TOOL_GIT_LOG.to_string(),
                description: "Show recent git commits.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "count": { "type": "integer", "description": "Number of commits to show", "minimum": 1, "maximum": 50 }
                    },
                    "required": ["count"]
                }),
            },
        },
        ToolDefinition {
            tool_type: "function".to_string(),
            function: ToolFunction {
                name: TOOL_GIT_DIFF.to_string(),
                description: "Show git diff of current changes.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            },
        },
        ToolDefinition {
            tool_type: "function".to_string(),
            function: ToolFunction {
                name: TOOL_GIT_STATUS.to_string(),
                description: "Show git status.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            },
        },
        ToolDefinition {
            tool_type: "function".to_string(),
            function: ToolFunction {
                name: TOOL_LIST_PANES.to_string(),
                description: "List panes for the current project.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            },
        },
        ToolDefinition {
            tool_type: "function".to_string(),
            function: ToolFunction {
                name: TOOL_LIST_PANES.to_string(),
                description: "List panes for the current project.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            },
        },
        ToolDefinition {
            tool_type: "function".to_string(),
            function: ToolFunction {
                name: TOOL_LIST_ISSUES.to_string(),
                description: "List GitHub issues for the current project in read-only mode."
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "state": { "type": "string", "description": "Issue state (open, closed, all)" },
                        "limit": { "type": "integer", "description": "Maximum number of issues to return", "minimum": 1, "maximum": 20 }
                    },
                    "required": []
                }),
            },
        },
        ToolDefinition {
            tool_type: "function".to_string(),
            function: ToolFunction {
                name: TOOL_LIST_PULL_REQUESTS.to_string(),
                description: "List GitHub pull requests for the current project in read-only mode."
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "state": { "type": "string", "description": "PR state (open, closed, merged, all)" },
                        "limit": { "type": "integer", "description": "Maximum number of PRs to return", "minimum": 1, "maximum": 20 }
                    },
                    "required": []
                }),
            },
        },
    ];

    // Consultation tools (read-only: list and read; write: respond)
    tools.push(ToolDefinition {
        tool_type: "function".to_string(),
        function: ToolFunction {
            name: TOOL_LIST_CONSULTATIONS.to_string(),
            description: "List pending consultation requests from coding agents.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
    });
    tools.push(ToolDefinition {
        tool_type: "function".to_string(),
        function: ToolFunction {
            name: TOOL_READ_CONSULTATION.to_string(),
            description: "Read a specific consultation request from a coding agent.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pane_id": { "type": "string", "description": "Pane ID of the requesting agent" },
                    "timestamp": { "type": "string", "description": "Timestamp of the consultation request" }
                },
                "required": ["pane_id", "timestamp"]
            }),
        },
    });

    if access_mode.allows_write() {
        tools.push(ToolDefinition {
            tool_type: "function".to_string(),
            function: ToolFunction {
                name: TOOL_RUN_COMMAND.to_string(),
                description: "Run a shell command with a 30-second timeout.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "command": { "type": "string", "description": "Shell command to execute" }
                    },
                    "required": ["command"]
                }),
            },
        });
        tools.push(ToolDefinition {
            tool_type: "function".to_string(),
            function: ToolFunction {
                name: TOOL_RESPOND_TO_CONSULTATION.to_string(),
                description: "Respond to a consultation request from a coding agent.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "pane_id": { "type": "string", "description": "Pane ID of the requesting agent" },
                        "timestamp": { "type": "string", "description": "Timestamp of the consultation request" },
                        "response": { "type": "string", "description": "Response to the consultation" }
                    },
                    "required": ["pane_id", "timestamp", "response"]
                }),
            },
        });
    }

    tools
}

pub fn assistant_tool_allowed(name: &str, mode: AssistantToolMode) -> bool {
    match mode {
        AssistantToolMode::FullAccess => true,
        AssistantToolMode::ReadOnly => !matches!(
            name,
            TOOL_RUN_COMMAND
                | TOOL_RESPOND_TO_CONSULTATION
                | crate::tool_helpers::TOOL_SEND_KEYS_TO_PANE
                | crate::tool_helpers::TOOL_UPSERT_SPEC_ISSUE
                | crate::tool_helpers::TOOL_UPSERT_SPEC_ARTIFACT
                | crate::tool_helpers::TOOL_CLOSE_SPEC_ISSUE
        ),
    }
}

/// Execute a single assistant tool call and return the result as a string.
pub fn execute_assistant_tool(
    call: &ToolCall,
    state: &AppState,
    _window_label: &str,
    project_path: &str,
    mode: AssistantToolMode,
) -> Result<String, String> {
    let args = normalize_args(&call.arguments)?;
    let access_mode = shared_tool_access_mode(mode);

    if !assistant_tool_allowed(call.name.as_str(), mode) {
        return Err(format!(
            "Tool is not available in {:?} mode: {}",
            mode, call.name
        ));
    }

    // Try shared tools first
    if let Some(result) =
        execute_shared_tool_with_mode(call, &args, state, project_path, access_mode)
    {
        return result;
    }

    // Assistant-specific tools
    match call.name.as_str() {
        TOOL_READ_FILE => {
            let rel_path = get_required_string_any(&args, &["path"])?;
            let full_path = Path::new(project_path).join(rel_path);
            const MAX_FILE_SIZE: u64 = 64 * 1024; // 64KB
            let meta = std::fs::metadata(&full_path)
                .map_err(|e| format!("Failed to read file {}: {}", full_path.display(), e))?;
            if meta.len() > MAX_FILE_SIZE {
                return Err(format!(
                    "File too large ({} bytes, max {}). Use grep_file for targeted search.",
                    meta.len(),
                    MAX_FILE_SIZE
                ));
            }
            std::fs::read_to_string(&full_path)
                .map_err(|e| format!("Failed to read file {}: {}", full_path.display(), e))
        }
        TOOL_GREP_FILE => {
            let pattern = get_required_string_any(&args, &["pattern"])?;
            let rel_path = get_required_string_any(&args, &["path"])?;
            let full_path = Path::new(project_path).join(rel_path);
            run_command_in_dir(
                project_path,
                "grep",
                &["-rn", pattern, &full_path.to_string_lossy()],
            )
        }
        TOOL_LIST_DIRECTORY => {
            let rel_path = get_required_string_any(&args, &["path"])?;
            let full_path = Path::new(project_path).join(rel_path);
            let entries = std::fs::read_dir(&full_path)
                .map_err(|e| format!("Failed to list directory {}: {}", full_path.display(), e))?;
            let mut lines = Vec::new();
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                let file_type = entry.file_type().ok();
                let prefix = if file_type.map(|t| t.is_dir()).unwrap_or(false) {
                    "d "
                } else {
                    "f "
                };
                lines.push(format!("{}{}", prefix, name));
            }
            lines.sort();
            Ok(lines.join("\n"))
        }
        TOOL_GIT_LOG => {
            let count = get_optional_u64_any(&args, &["count"]).unwrap_or(10);
            run_command_in_dir(
                project_path,
                "git",
                &["log", "--oneline", &format!("-{}", count.min(50))],
            )
        }
        TOOL_GIT_DIFF => run_command_in_dir(project_path, "git", &["diff"]),
        TOOL_GIT_STATUS => run_command_in_dir(project_path, "git", &["status", "--short"]),
        TOOL_RUN_COMMAND => {
            if !access_mode.allows_write() {
                return Err(format!(
                    "Tool is not available in {:?} mode: {}",
                    mode, call.name
                ));
            }
            let command = get_required_string_any(&args, &["command"])?;
            run_shell_command(project_path, command)
        }
        TOOL_LIST_PANES => list_project_panes(state, project_path),
        TOOL_LIST_ISSUES => list_project_issues(project_path, &args),
        TOOL_LIST_PULL_REQUESTS => list_project_pull_requests(project_path, &args),
        TOOL_LIST_CONSULTATIONS => {
            let consultations =
                crate::consultation::list_pending_consultations(Path::new(project_path))?;
            serde_json::to_string(&consultations)
                .map_err(|e| format!("Failed to serialize consultations: {e}"))
        }
        TOOL_READ_CONSULTATION => {
            let pane_id = get_required_string_any(&args, &["pane_id", "paneId"])?;
            let timestamp = get_required_string_any(&args, &["timestamp"])?;
            let consultation = crate::consultation::read_consultation(
                Path::new(project_path),
                pane_id,
                timestamp,
            )?;
            serde_json::to_string(&consultation)
                .map_err(|e| format!("Failed to serialize consultation: {e}"))
        }
        TOOL_RESPOND_TO_CONSULTATION => {
            if !access_mode.allows_write() {
                return Err(format!(
                    "Tool is not available in {:?} mode: {}",
                    mode, call.name
                ));
            }
            let pane_id = get_required_string_any(&args, &["pane_id", "paneId"])?;
            let timestamp = get_required_string_any(&args, &["timestamp"])?;
            let response = get_required_string_any(&args, &["response"])?;
            crate::consultation::write_consultation_response(
                Path::new(project_path),
                pane_id,
                timestamp,
                response,
            )?;
            Ok(json!({ "status": "responded" }).to_string())
        }
        _ => Err(format!("Unknown tool: {}", call.name)),
    }
}

// ── helpers (assistant-specific) ─────────────────────────────────────

fn run_command_in_dir(dir: &str, program: &str, args: &[&str]) -> Result<String, String> {
    let output = process_command(program)
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| format!("Failed to run {}: {}", program, e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if output.status.success() {
        Ok(stdout.to_string())
    } else {
        Ok(format!(
            "Exit code: {}\nstdout:\n{}\nstderr:\n{}",
            output.status, stdout, stderr
        ))
    }
}

fn run_shell_command(dir: &str, shell_command: &str) -> Result<String, String> {
    #[cfg(target_os = "windows")]
    let (shell, flag) = ("cmd", "/C");
    #[cfg(not(target_os = "windows"))]
    let (shell, flag) = ("sh", "-c");

    let mut child = process_command(shell)
        .arg(flag)
        .arg(shell_command)
        .current_dir(dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to run command: {}", e))?;

    // Poll with timeout
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let stdout = child
                    .stdout
                    .take()
                    .map(|mut s| {
                        let mut buf = String::new();
                        use std::io::Read;
                        let _ = s.read_to_string(&mut buf);
                        buf
                    })
                    .unwrap_or_default();
                let stderr = child
                    .stderr
                    .take()
                    .map(|mut s| {
                        let mut buf = String::new();
                        use std::io::Read;
                        let _ = s.read_to_string(&mut buf);
                        buf
                    })
                    .unwrap_or_default();
                return if status.success() {
                    Ok(stdout)
                } else {
                    Ok(format!(
                        "Exit code: {}\nstdout:\n{}\nstderr:\n{}",
                        status, stdout, stderr
                    ))
                };
            }
            Ok(None) => {
                if start.elapsed() >= COMMAND_TIMEOUT {
                    let _ = child.kill();
                    return Err(format!(
                        "Command timed out after {} seconds",
                        COMMAND_TIMEOUT.as_secs()
                    ));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(format!("Failed to wait for command: {}", e)),
        }
    }
}

fn list_project_panes(state: &AppState, project_path: &str) -> Result<String, String> {
    let repo_path =
        crate::commands::project::resolve_repo_path_for_project_root(Path::new(project_path))
            .map_err(|e| format!("Failed to resolve repository path: {e}"))?;

    let manager = state
        .pane_manager
        .lock()
        .map_err(|e| format!("Failed to lock pane manager: {e}"))?;
    let panes = manager
        .panes()
        .iter()
        .filter(|pane| pane.project_root() == repo_path.as_path())
        .map(|pane| {
            json!({
                "paneId": pane.pane_id(),
                "agentName": pane.agent_name(),
                "branchName": pane.branch_name(),
                "status": format!("{:?}", pane.status()),
            })
        })
        .collect::<Vec<_>>();

    serde_json::to_string(&json!({ "panes": panes }))
        .map_err(|e| format!("Failed to serialize panes: {e}"))
}

fn list_project_issues(project_path: &str, args: &serde_json::Value) -> Result<String, String> {
    if !gwt_core::git::is_gh_cli_available() || !gwt_core::git::is_gh_cli_authenticated() {
        return Ok(json!({ "ghAvailable": false, "issues": [] }).to_string());
    }

    let repo_path =
        crate::commands::project::resolve_repo_path_for_project_root(Path::new(project_path))
            .map_err(|e| format!("Failed to resolve repository path: {e}"))?;
    let state = get_optional_string_any(args, &["state"]).unwrap_or("open");
    let limit = get_optional_u64_any(args, &["limit"]).unwrap_or(5).min(20) as u32;
    let result =
        gwt_core::git::fetch_issues_with_options(&repo_path, 1, limit, state, false, "all")
            .map_err(|e| format!("Failed to list issues: {e}"))?;

    let issues = result
        .issues
        .into_iter()
        .map(|issue| {
            json!({
                "number": issue.number,
                "title": issue.title,
                "state": issue.state,
                "updatedAt": issue.updated_at,
                "url": issue.html_url,
                "labels": issue.labels.into_iter().map(|label| label.name).collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();

    serde_json::to_string(&json!({
        "ghAvailable": true,
        "hasNextPage": result.has_next_page,
        "issues": issues,
    }))
    .map_err(|e| format!("Failed to serialize issues: {e}"))
}

fn list_project_pull_requests(
    project_path: &str,
    args: &serde_json::Value,
) -> Result<String, String> {
    if !gwt_core::git::is_gh_cli_available() || !gwt_core::git::is_gh_cli_authenticated() {
        return Ok(json!({ "ghAvailable": false, "items": [] }).to_string());
    }

    let repo_path =
        crate::commands::project::resolve_repo_path_for_project_root(Path::new(project_path))
            .map_err(|e| format!("Failed to resolve repository path: {e}"))?;
    let state = get_optional_string_any(args, &["state"]).unwrap_or("open");
    let limit = get_optional_u64_any(args, &["limit"]).unwrap_or(5).min(20) as u32;
    let items = gwt_core::git::gh_cli::fetch_pr_list(&repo_path, state, limit)
        .map_err(|e| format!("Failed to list pull requests: {e}"))?;

    serde_json::to_string(&json!({
        "ghAvailable": true,
        "items": items,
    }))
    .map_err(|e| format!("Failed to serialize pull requests: {e}"))
}

#[cfg(test)]
mod tests {
    use gwt_core::ai::ToolCall;

    use super::*;
    use crate::state::AppState;

    #[test]
    fn read_only_mode_excludes_mutating_tools() {
        let names: Vec<String> = assistant_tool_definitions(AssistantToolMode::ReadOnly)
            .into_iter()
            .map(|tool| tool.function.name)
            .collect();

        assert!(!names.contains(&TOOL_RUN_COMMAND.to_string()));
        assert!(!names.contains(&TOOL_RESPOND_TO_CONSULTATION.to_string()));
        assert!(!names.contains(&crate::tool_helpers::TOOL_SEND_KEYS_TO_PANE.to_string()));
        assert!(!names.contains(&crate::tool_helpers::TOOL_UPSERT_SPEC_ISSUE.to_string()));
        assert!(!names.contains(&crate::tool_helpers::TOOL_UPSERT_SPEC_ARTIFACT.to_string()));
        assert!(!names.contains(&crate::tool_helpers::TOOL_CLOSE_SPEC_ISSUE.to_string()));
        assert!(names.contains(&TOOL_LIST_PANES.to_string()));
        assert!(names.contains(&TOOL_LIST_ISSUES.to_string()));
        assert!(names.contains(&TOOL_LIST_PULL_REQUESTS.to_string()));
        assert!(names.contains(&TOOL_LIST_CONSULTATIONS.to_string()));
        assert!(names.contains(&TOOL_READ_CONSULTATION.to_string()));
        assert!(names.contains(&crate::tool_helpers::TOOL_SEARCH_SPEC_ISSUES.to_string()));
        assert!(names.contains(&crate::tool_helpers::TOOL_LIST_SPEC_ARTIFACTS.to_string()));
    }

    #[test]
    fn read_only_mode_rejects_mutating_shared_tools() {
        let call = ToolCall {
            name: crate::tool_helpers::TOOL_SEND_KEYS_TO_PANE.to_string(),
            arguments: json!({
                "pane_id": "pane-1",
                "text": "hello",
            }),
            call_id: None,
        };

        let state = AppState::new();
        let result =
            execute_assistant_tool(&call, &state, "main", ".", AssistantToolMode::ReadOnly);
        assert!(result.is_err());
    }
}
