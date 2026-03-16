#![allow(dead_code)]
//! Built-in tool definitions for Assistant Mode LLM tool-call dispatch.

use serde_json::json;
use std::io::Read;
use std::path::Path;
use std::process::{Child, Stdio};
use std::time::Duration;

use crate::state::AppState;
use crate::tool_helpers::{
    execute_shared_tool, get_optional_u64_any, get_required_string_any, normalize_args,
    shared_tool_definitions,
};
use gwt_core::ai::{ToolCall, ToolDefinition, ToolFunction};
use gwt_core::process::command as process_command;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

// Tool name constants (assistant-specific)
pub const TOOL_READ_FILE: &str = "read_file";
pub const TOOL_GREP_FILE: &str = "grep_file";
pub const TOOL_LIST_DIRECTORY: &str = "list_directory";
pub const TOOL_GIT_LOG: &str = "git_log";
pub const TOOL_GIT_DIFF: &str = "git_diff";
pub const TOOL_GIT_STATUS: &str = "git_status";
pub const TOOL_RUN_COMMAND: &str = "run_command";

pub fn assistant_tool_definitions() -> Vec<ToolDefinition> {
    let mut tools = assistant_specific_tool_definitions();
    tools.extend(shared_tool_definitions());
    tools
}

fn assistant_specific_tool_definitions() -> Vec<ToolDefinition> {
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

    // Try shared tools first
    if let Some(result) = execute_shared_tool(call, &args, state, project_path) {
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
            #[cfg(target_os = "windows")]
            {
                let search_path = if full_path.is_dir() {
                    format!(r"{}\*", full_path.display())
                } else {
                    full_path.to_string_lossy().to_string()
                };
                run_command_in_dir(
                    project_path,
                    "findstr",
                    &["/N", "/R", "/S", pattern, &search_path],
                )
            }
            #[cfg(not(target_os = "windows"))]
            {
                run_command_in_dir(
                    project_path,
                    "grep",
                    &["-rn", pattern, &full_path.to_string_lossy()],
                )
            }
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
            let command = get_required_string_any(&args, &["command"])?;
            run_shell_command(project_path, command)
        }
        _ => Err(format!("Unknown tool: {}", call.name)),
    }
}

// ── helpers (assistant-specific) ─────────────────────────────────────

fn run_command_in_dir(dir: &str, program: &str, args: &[&str]) -> Result<String, String> {
    let child = process_command(program)
        .args(args)
        .current_dir(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to run {}: {}", program, e))?;
    run_child_with_timeout(child, program)
}

fn run_shell_command(dir: &str, shell_command: &str) -> Result<String, String> {
    #[cfg(target_os = "windows")]
    let (shell, flag) = ("cmd", "/C");
    #[cfg(not(target_os = "windows"))]
    let (shell, flag) = ("sh", "-c");

    let child = process_command(shell)
        .arg(flag)
        .arg(shell_command)
        .current_dir(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to run command: {}", e))?;
    run_child_with_timeout(child, shell)
}

fn run_child_with_timeout(mut child: Child, label: &str) -> Result<String, String> {
    let stdout_handle = child.stdout.take().map(|mut stdout| {
        std::thread::spawn(move || {
            let mut buf = String::new();
            let _ = stdout.read_to_string(&mut buf);
            buf
        })
    });
    let stderr_handle = child.stderr.take().map(|mut stderr| {
        std::thread::spawn(move || {
            let mut buf = String::new();
            let _ = stderr.read_to_string(&mut buf);
            buf
        })
    });

    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let stdout = stdout_handle
                    .map(|handle| handle.join().unwrap_or_default())
                    .unwrap_or_default();
                let stderr = stderr_handle
                    .map(|handle| handle.join().unwrap_or_default())
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
                    let _ = child.wait();
                    let _ = stdout_handle.map(|handle| handle.join());
                    let _ = stderr_handle.map(|handle| handle.join());
                    return Err(format!(
                        "{} timed out after {} seconds",
                        label,
                        COMMAND_TIMEOUT.as_secs(),
                    ));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_handle.map(|handle| handle.join());
                let _ = stderr_handle.map(|handle| handle.join());
                return Err(format!("Failed to wait for command: {}", e));
            }
        }
    }
}
