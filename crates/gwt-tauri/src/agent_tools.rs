use serde_json::{json, Value};

use crate::commands::terminal::{
    capture_scrollback_tail_from_state, send_keys_broadcast_from_state,
    send_keys_to_pane_from_state,
};
use crate::state::AppState;
use gwt_core::ai::{ToolCall, ToolDefinition, ToolFunction};

pub const TOOL_SEND_KEYS_TO_PANE: &str = "send_keys_to_pane";
pub const TOOL_SEND_KEYS_BROADCAST: &str = "send_keys_broadcast";
pub const TOOL_CAPTURE_SCROLLBACK_TAIL: &str = "capture_scrollback_tail";

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
    ]
}

pub fn execute_tool_call(state: &AppState, call: &ToolCall) -> Result<String, String> {
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

fn get_optional_u64_any(value: &Value, keys: &[&str]) -> Option<u64> {
    for key in keys {
        if let Some(v) = value.get(*key).and_then(|v| v.as_u64()) {
            return Some(v);
        }
    }
    None
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
        let result = execute_tool_call(&state, &call).expect("tool call");
        assert!(result.contains("hello"));
    }
}
