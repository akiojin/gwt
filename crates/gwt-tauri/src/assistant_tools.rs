#![allow(dead_code)]
//! Built-in tool definitions for Assistant Mode LLM tool-call dispatch.

use serde_json::{json, Value};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use crate::commands::issue_spec::{
    get_spec_issue_detail_cmd, upsert_spec_issue_cmd, SpecIssueSectionsData,
};
use crate::commands::terminal::{
    capture_scrollback_tail_from_state, send_keys_to_pane_from_state,
};
use crate::state::AppState;
use gwt_core::ai::{ToolCall, ToolDefinition, ToolFunction};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

// Tool name constants
pub const TOOL_READ_FILE: &str = "read_file";
pub const TOOL_GREP_FILE: &str = "grep_file";
pub const TOOL_LIST_DIRECTORY: &str = "list_directory";
pub const TOOL_GIT_LOG: &str = "git_log";
pub const TOOL_GIT_DIFF: &str = "git_diff";
pub const TOOL_GIT_STATUS: &str = "git_status";
pub const TOOL_RUN_COMMAND: &str = "run_command";
pub const TOOL_GET_SPEC_ISSUE: &str = "get_spec_issue";
pub const TOOL_UPSERT_SPEC_ISSUE: &str = "upsert_spec_issue";
pub const TOOL_SEND_KEYS_TO_PANE: &str = "send_keys_to_pane";
pub const TOOL_CAPTURE_SCROLLBACK_TAIL: &str = "capture_scrollback_tail";

pub fn assistant_tool_definitions() -> Vec<ToolDefinition> {
    vec![
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
                name: TOOL_UPSERT_SPEC_ISSUE.to_string(),
                description: "Create or update an issue-first spec artifact bundle for an issue."
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "issue_number": { "type": "integer", "minimum": 1 },
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
                    "required": ["title", "sections"]
                }),
            },
        },
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
    ]
}

/// Execute a single assistant tool call and return the result as a string.
pub fn execute_assistant_tool(
    call: &ToolCall,
    state: &AppState,
    _window_label: &str,
    project_path: &str,
) -> Result<String, String> {
    let args = normalize_args(&call.arguments)?;
    match call.name.as_str() {
        TOOL_READ_FILE => {
            let rel_path = get_required_string(&args, "path")?;
            let full_path = Path::new(project_path).join(rel_path);
            std::fs::read_to_string(&full_path)
                .map_err(|e| format!("Failed to read file {}: {}", full_path.display(), e))
        }
        TOOL_GREP_FILE => {
            let pattern = get_required_string(&args, "pattern")?;
            let rel_path = get_required_string(&args, "path")?;
            let full_path = Path::new(project_path).join(rel_path);
            run_command_in_dir(
                project_path,
                "grep",
                &["-rn", pattern, &full_path.to_string_lossy()],
            )
        }
        TOOL_LIST_DIRECTORY => {
            let rel_path = get_required_string(&args, "path")?;
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
            let count = get_optional_u64(&args, "count").unwrap_or(10);
            run_command_in_dir(
                project_path,
                "git",
                &[
                    "log",
                    "--oneline",
                    &format!("-{}", count.min(50)),
                ],
            )
        }
        TOOL_GIT_DIFF => run_command_in_dir(project_path, "git", &["diff"]),
        TOOL_GIT_STATUS => run_command_in_dir(project_path, "git", &["status", "--short"]),
        TOOL_RUN_COMMAND => {
            let command = get_required_string(&args, "command")?;
            run_shell_command(project_path, command)
        }
        TOOL_GET_SPEC_ISSUE => {
            let issue_number = get_required_u64(&args, "issue_number")?;
            let detail = get_spec_issue_detail_cmd(project_path.to_string(), issue_number)
                .map_err(|e| e.message)?;
            serde_json::to_string(&detail).map_err(|e| format!("Failed to serialize: {e}"))
        }
        TOOL_UPSERT_SPEC_ISSUE => {
            let issue_number = get_optional_u64(&args, "issue_number");
            let title = get_required_string(&args, "title")?;
            let sections_val = args.get("sections").ok_or("Missing sections argument")?;
            let sections = parse_sections(sections_val);
            let expected_etag = args
                .get("expected_etag")
                .and_then(|v| v.as_str())
                .map(str::to_string);

            let existing = match issue_number {
                Some(num) => Some(
                    get_spec_issue_detail_cmd(project_path.to_string(), num)
                        .map_err(|e| e.message)?,
                ),
                None => None,
            };
            let merged = match existing {
                Some(detail) => merge_sections(detail.sections, sections),
                None => sections,
            };
            let detail = upsert_spec_issue_cmd(
                project_path.to_string(),
                issue_number,
                title.to_string(),
                merged,
                expected_etag,
            )
            .map_err(|e| e.message)?;
            serde_json::to_string(&detail).map_err(|e| format!("Failed to serialize: {e}"))
        }
        TOOL_SEND_KEYS_TO_PANE => {
            let pane_id = get_required_string(&args, "pane_id")?;
            let text = get_required_string(&args, "text")?;
            send_keys_to_pane_from_state(state, pane_id, text, None)?;
            Ok("ok".to_string())
        }
        TOOL_CAPTURE_SCROLLBACK_TAIL => {
            let pane_id = get_required_string(&args, "pane_id")?;
            let max_bytes = get_optional_u64(&args, "max_bytes").map(|v| v as usize);
            match max_bytes {
                Some(limit) => capture_scrollback_tail_from_state(state, pane_id, limit, None),
                None => capture_scrollback_tail_from_state(state, pane_id, 0, None),
            }
        }
        _ => Err(format!("Unknown tool: {}", call.name)),
    }
}

// ── helpers ──────────────────────────────────────────────────────────

fn normalize_args(value: &Value) -> Result<Value, String> {
    if let Some(text) = value.as_str() {
        serde_json::from_str(text).map_err(|e| format!("Invalid tool arguments: {e}"))
    } else {
        Ok(value.clone())
    }
}

fn get_required_string<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| format!("Missing required argument: {}", key))
}

fn get_required_u64(value: &Value, key: &str) -> Result<u64, String> {
    value
        .get(key)
        .and_then(|v| v.as_u64())
        .ok_or_else(|| format!("Missing required argument: {}", key))
}

fn get_optional_u64(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(|v| v.as_u64())
}

fn run_command_in_dir(dir: &str, program: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(program)
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| format!("Failed to run {}: {}", program, e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if output.status.success() {
        Ok(stdout.to_string())
    } else {
        Ok(format!("Exit code: {}\nstdout:\n{}\nstderr:\n{}", output.status, stdout, stderr))
    }
}

fn run_shell_command(dir: &str, command: &str) -> Result<String, String> {
    #[cfg(target_os = "windows")]
    let (shell, flag) = ("cmd", "/C");
    #[cfg(not(target_os = "windows"))]
    let (shell, flag) = ("sh", "-c");

    let child = Command::new(shell)
        .arg(flag)
        .arg(command)
        .current_dir(dir)
        .output();

    match child {
        Ok(output) => {
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
        Err(e) => Err(format!("Failed to run command: {}", e)),
    }
}

fn parse_sections(value: &Value) -> SpecIssueSectionsData {
    let read = |key: &str| -> String {
        value
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    SpecIssueSectionsData {
        spec: read("spec"),
        plan: read("plan"),
        tasks: read("tasks"),
        tdd: read("tdd"),
        research: read("research"),
        data_model: read("data_model"),
        quickstart: read("quickstart"),
        contracts: read("contracts"),
        checklists: read("checklists"),
    }
}

fn merge_sections(
    mut base: SpecIssueSectionsData,
    patch: SpecIssueSectionsData,
) -> SpecIssueSectionsData {
    if !patch.spec.is_empty() {
        base.spec = patch.spec;
    }
    if !patch.plan.is_empty() {
        base.plan = patch.plan;
    }
    if !patch.tasks.is_empty() {
        base.tasks = patch.tasks;
    }
    if !patch.tdd.is_empty() {
        base.tdd = patch.tdd;
    }
    if !patch.research.is_empty() {
        base.research = patch.research;
    }
    if !patch.data_model.is_empty() {
        base.data_model = patch.data_model;
    }
    if !patch.quickstart.is_empty() {
        base.quickstart = patch.quickstart;
    }
    if !patch.contracts.is_empty() {
        base.contracts = patch.contracts;
    }
    if !patch.checklists.is_empty() {
        base.checklists = patch.checklists;
    }
    base
}
