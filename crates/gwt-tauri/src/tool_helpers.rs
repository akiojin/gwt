//! Shared tool helpers, definitions, and dispatch for Assistant Mode and Project Mode.
//!
//! Both `assistant_tools` and `agent_tools` use these shared utilities to avoid
//! code duplication across the two tool-call dispatch modules.

use serde_json::{json, Value};

use crate::commands::issue_spec::{
    get_spec_issue_detail_cmd, upsert_spec_issue_cmd, SpecIssueSectionsData,
};
use crate::commands::terminal::{capture_scrollback_tail_from_state, send_keys_to_pane_from_state};
use crate::state::AppState;
use gwt_core::ai::{ToolCall, ToolDefinition, ToolFunction};

// ── Tool name constants (shared tools) ──────────────────────────────

pub const TOOL_SEND_KEYS_TO_PANE: &str = "send_keys_to_pane";
pub const TOOL_CAPTURE_SCROLLBACK_TAIL: &str = "capture_scrollback_tail";
pub const TOOL_GET_SPEC_ISSUE: &str = "get_spec_issue";
pub const TOOL_UPSERT_SPEC_ISSUE: &str = "upsert_spec_issue";

// ── Argument helpers ────────────────────────────────────────────────

/// Normalize tool arguments: if the value is a JSON string, parse it as an object.
pub fn normalize_args(value: &Value) -> Result<Value, String> {
    if let Some(text) = value.as_str() {
        serde_json::from_str(text).map_err(|e| format!("Invalid tool arguments: {e}"))
    } else {
        Ok(value.clone())
    }
}

/// Get a required string argument, trying multiple key names (e.g. snake_case and camelCase).
pub fn get_required_string_any<'a>(value: &'a Value, keys: &[&str]) -> Result<&'a str, String> {
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

/// Get an optional string argument, trying multiple key names.
pub fn get_optional_string_any<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
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

/// Get a required object argument, trying multiple key names.
pub fn get_required_object_any<'a>(value: &'a Value, keys: &[&str]) -> Result<&'a Value, String> {
    for key in keys {
        if let Some(found) = value.get(*key).filter(|v| v.is_object()) {
            return Ok(found);
        }
    }
    Err(format!("Missing required argument: {}", keys.join(" or ")))
}

/// Get a required u64 argument, trying multiple key names.
pub fn get_required_u64_any(value: &Value, keys: &[&str]) -> Result<u64, String> {
    for key in keys {
        if let Some(found) = value.get(*key).and_then(|v| v.as_u64()) {
            return Ok(found);
        }
    }
    Err(format!("Missing required argument: {}", keys.join(" or ")))
}

/// Get an optional u64 argument, trying multiple key names.
pub fn get_optional_u64_any(value: &Value, keys: &[&str]) -> Option<u64> {
    for key in keys {
        if let Some(v) = value.get(*key).and_then(|v| v.as_u64()) {
            return Some(v);
        }
    }
    None
}

// ── SpecIssueSectionsPatch ──────────────────────────────────────────

/// Partial patch for spec issue sections. `None` means "do not change".
#[derive(Debug, Default, Clone)]
pub struct SpecIssueSectionsPatch {
    pub spec: Option<String>,
    pub plan: Option<String>,
    pub tasks: Option<String>,
    pub tdd: Option<String>,
    pub research: Option<String>,
    pub data_model: Option<String>,
    pub quickstart: Option<String>,
    pub contracts: Option<String>,
    pub checklists: Option<String>,
}

/// Parse a JSON object into a `SpecIssueSectionsPatch`.
pub fn parse_sections_patch(value: &Value) -> SpecIssueSectionsPatch {
    let read = |keys: &[&str]| -> Option<String> {
        for key in keys {
            if let Some(found) = value.get(*key).and_then(|v| v.as_str()) {
                return Some(found.to_string());
            }
        }
        None
    };
    SpecIssueSectionsPatch {
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

/// Create a full `SpecIssueSectionsData` from a patch (missing fields default to empty).
pub fn sections_from_patch(patch: SpecIssueSectionsPatch) -> SpecIssueSectionsData {
    merge_sections_data(
        SpecIssueSectionsData {
            spec: String::new(),
            plan: String::new(),
            tasks: String::new(),
            tdd: String::new(),
            research: String::new(),
            data_model: String::new(),
            quickstart: String::new(),
            contracts: String::new(),
            checklists: String::new(),
        },
        patch,
    )
}

/// Merge a patch into existing sections data. Only non-None patch fields overwrite the base.
pub fn merge_sections_data(
    mut base: SpecIssueSectionsData,
    patch: SpecIssueSectionsPatch,
) -> SpecIssueSectionsData {
    if let Some(value) = patch.spec {
        base.spec = value;
    }
    if let Some(value) = patch.plan {
        base.plan = value;
    }
    if let Some(value) = patch.tasks {
        base.tasks = value;
    }
    if let Some(value) = patch.tdd {
        base.tdd = value;
    }
    if let Some(value) = patch.research {
        base.research = value;
    }
    if let Some(value) = patch.data_model {
        base.data_model = value;
    }
    if let Some(value) = patch.quickstart {
        base.quickstart = value;
    }
    if let Some(value) = patch.contracts {
        base.contracts = value;
    }
    if let Some(value) = patch.checklists {
        base.checklists = value;
    }
    base
}

// ── Shared tool definitions ─────────────────────────────────────────

/// Returns tool definitions shared between Assistant Mode and Project Mode:
/// `send_keys_to_pane`, `capture_scrollback_tail`, `get_spec_issue`, `upsert_spec_issue`.
pub fn shared_tool_definitions() -> Vec<ToolDefinition> {
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
    ]
}

/// Execute a shared tool call. Returns `Some(result)` if the tool name matches a shared tool,
/// `None` if the tool is not a shared tool (caller should handle it).
pub fn execute_shared_tool(
    call: &ToolCall,
    args: &Value,
    state: &AppState,
    project_path: &str,
) -> Option<Result<String, String>> {
    match call.name.as_str() {
        TOOL_SEND_KEYS_TO_PANE => {
            let result = (|| {
                let pane_id = get_required_string_any(args, &["pane_id", "paneId"])?;
                let text = get_required_string_any(args, &["text"])?;
                send_keys_to_pane_from_state(state, pane_id, text, None)?;
                Ok("ok".to_string())
            })();
            Some(result)
        }
        TOOL_CAPTURE_SCROLLBACK_TAIL => {
            let result = (|| {
                let pane_id = get_required_string_any(args, &["pane_id", "paneId"])?;
                let max_bytes =
                    get_optional_u64_any(args, &["max_bytes", "maxBytes"]).map(|v| v as usize);
                match max_bytes {
                    Some(limit) => capture_scrollback_tail_from_state(state, pane_id, limit, None),
                    None => capture_scrollback_tail_from_state(state, pane_id, 0, None),
                }
            })();
            Some(result)
        }
        TOOL_GET_SPEC_ISSUE => {
            let result = (|| {
                let issue_number = get_required_u64_any(args, &["issue_number", "issueNumber"])?;
                let detail = get_spec_issue_detail_cmd(project_path.to_string(), issue_number)
                    .map_err(|e| e.message)?;
                serde_json::to_string(&detail)
                    .map_err(|e| format!("Failed to serialize result: {e}"))
            })();
            Some(result)
        }
        TOOL_UPSERT_SPEC_ISSUE => {
            let result = (|| {
                let issue_number = get_optional_u64_any(args, &["issue_number", "issueNumber"]);
                let title = get_required_string_any(args, &["title"])?;
                let sections = get_required_object_any(args, &["sections"])?;
                let expected_etag =
                    get_optional_string_any(args, &["expected_etag", "expectedEtag"])
                        .map(str::to_string);
                let patch = parse_sections_patch(sections);
                let existing = match issue_number {
                    Some(number) => Some(
                        get_spec_issue_detail_cmd(project_path.to_string(), number)
                            .map_err(|e| e.message)?,
                    ),
                    None => None,
                };
                let merged_sections = match existing {
                    Some(detail) => merge_sections_data(detail.sections, patch),
                    None => sections_from_patch(patch),
                };
                let detail = upsert_spec_issue_cmd(
                    project_path.to_string(),
                    issue_number,
                    title.to_string(),
                    merged_sections,
                    expected_etag,
                )
                .map_err(|e| e.message)?;
                serde_json::to_string(&detail)
                    .map_err(|e| format!("Failed to serialize result: {e}"))
            })();
            Some(result)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_args_parses_json_string() {
        let val = Value::String(r#"{"key": "value"}"#.to_string());
        let result = normalize_args(&val).unwrap();
        assert_eq!(result["key"], "value");
    }

    #[test]
    fn normalize_args_passes_object_through() {
        let val = json!({"key": "value"});
        let result = normalize_args(&val).unwrap();
        assert_eq!(result["key"], "value");
    }

    #[test]
    fn get_required_string_any_finds_first_match() {
        let val = json!({"paneId": "p1"});
        assert_eq!(
            get_required_string_any(&val, &["pane_id", "paneId"]).unwrap(),
            "p1"
        );
    }

    #[test]
    fn get_required_string_any_errors_on_missing() {
        let val = json!({});
        assert!(get_required_string_any(&val, &["pane_id"]).is_err());
    }

    #[test]
    fn parse_sections_patch_keeps_missing_fields_none() {
        let patch = parse_sections_patch(&json!({ "tasks": "new tasks" }));
        assert_eq!(patch.tasks.as_deref(), Some("new tasks"));
        assert!(patch.spec.is_none());
        assert!(patch.plan.is_none());
    }

    #[test]
    fn merge_sections_data_preserves_omitted_fields() {
        let base = SpecIssueSectionsData {
            spec: "spec".to_string(),
            plan: "plan".to_string(),
            tasks: "tasks".to_string(),
            tdd: "tdd".to_string(),
            research: "research".to_string(),
            data_model: "data-model".to_string(),
            quickstart: "quickstart".to_string(),
            contracts: "contracts".to_string(),
            checklists: "checklists".to_string(),
        };
        let patch = parse_sections_patch(&json!({ "tasks": "updated tasks" }));
        let merged = merge_sections_data(base, patch);
        assert_eq!(merged.spec, "spec");
        assert_eq!(merged.plan, "plan");
        assert_eq!(merged.tasks, "updated tasks");
        assert_eq!(merged.tdd, "tdd");
    }

    #[test]
    fn shared_tool_definitions_count() {
        assert_eq!(shared_tool_definitions().len(), 4);
    }
}
