use std::io::Write;

use serde::Serialize;

use super::HookError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntentBoundaryEvent {
    SessionStart,
    UserPromptSubmit,
    Stop,
}

impl IntentBoundaryEvent {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "SessionStart" => Some(Self::SessionStart),
            "UserPromptSubmit" => Some(Self::UserPromptSubmit),
            "Stop" => Some(Self::Stop),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SessionStart => "SessionStart",
            Self::UserPromptSubmit => "UserPromptSubmit",
            Self::Stop => "Stop",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookOutput {
    PreToolUsePermission {
        summary: String,
        detail: String,
        deny_reason: String,
    },
    HookSpecificAdditionalContext {
        event: IntentBoundaryEvent,
        text: String,
    },
    SystemMessage(String),
    Silent,
}

#[derive(Debug, Serialize)]
struct PreToolUsePermissionPayload<'a> {
    #[serde(rename = "hookSpecificOutput")]
    hook_specific_output: PreToolUsePermissionContent<'a>,
}

#[derive(Debug, Serialize)]
struct PreToolUsePermissionContent<'a> {
    #[serde(rename = "hookEventName")]
    hook_event_name: &'static str,
    #[serde(rename = "permissionDecision")]
    permission_decision: &'static str,
    #[serde(rename = "permissionDecisionReason")]
    permission_decision_reason: &'a str,
}

#[derive(Debug, Serialize)]
struct AdditionalContextPayload<'a> {
    #[serde(rename = "hookSpecificOutput")]
    hook_specific_output: AdditionalContextContent<'a>,
}

#[derive(Debug, Serialize)]
struct AdditionalContextContent<'a> {
    #[serde(rename = "hookEventName")]
    hook_event_name: &'static str,
    #[serde(rename = "additionalContext")]
    additional_context: &'a str,
}

#[derive(Debug, Serialize)]
struct SystemMessagePayload<'a> {
    #[serde(rename = "systemMessage")]
    system_message: &'a str,
}

impl HookOutput {
    pub fn pre_tool_use_permission(summary: impl Into<String>, detail: impl Into<String>) -> Self {
        let summary = summary.into();
        let detail = detail.into();
        let deny_reason = match (summary.is_empty(), detail.is_empty()) {
            (true, _) => detail.clone(),
            (_, true) => summary.clone(),
            _ => format!("{summary}\n\n{detail}"),
        };
        Self::PreToolUsePermission {
            summary,
            detail,
            deny_reason,
        }
    }

    pub fn hook_specific_additional_context(
        event: IntentBoundaryEvent,
        text: impl Into<String>,
    ) -> Self {
        Self::HookSpecificAdditionalContext {
            event,
            text: text.into(),
        }
    }

    pub fn system_message(text: impl Into<String>) -> Self {
        Self::SystemMessage(text.into())
    }

    pub fn summary(&self) -> &str {
        match self {
            Self::PreToolUsePermission { summary, .. } => summary,
            _ => panic!("summary() is available only for PreToolUsePermission"),
        }
    }

    pub fn detail(&self) -> &str {
        match self {
            Self::PreToolUsePermission { detail, .. } => detail,
            _ => panic!("detail() is available only for PreToolUsePermission"),
        }
    }

    pub fn permission_decision_reason(&self) -> &str {
        match self {
            Self::PreToolUsePermission { deny_reason, .. } => deny_reason,
            _ => panic!("permission_decision_reason() is available only for PreToolUsePermission"),
        }
    }

    pub fn exit_code(&self) -> i32 {
        match self {
            Self::PreToolUsePermission { .. } => 2,
            Self::HookSpecificAdditionalContext { .. } | Self::SystemMessage(_) | Self::Silent => 0,
        }
    }

    pub fn serialize_to<W: Write + ?Sized>(&self, writer: &mut W) -> Result<(), HookError> {
        match self {
            Self::PreToolUsePermission { deny_reason, .. } => {
                let payload = PreToolUsePermissionPayload {
                    hook_specific_output: PreToolUsePermissionContent {
                        hook_event_name: "PreToolUse",
                        permission_decision: "deny",
                        permission_decision_reason: deny_reason,
                    },
                };
                serde_json::to_writer(&mut *writer, &payload)?;
                writer.write_all(b"\n")?;
            }
            Self::HookSpecificAdditionalContext { event, text } => {
                if matches!(event, IntentBoundaryEvent::Stop) {
                    debug_assert!(
                        !matches!(event, IntentBoundaryEvent::Stop),
                        "Stop must not serialize as additionalContext; use systemMessage"
                    );
                    let payload = SystemMessagePayload {
                        system_message: text,
                    };
                    serde_json::to_writer(&mut *writer, &payload)?;
                    writer.write_all(b"\n")?;
                    return Ok(());
                }
                let payload = AdditionalContextPayload {
                    hook_specific_output: AdditionalContextContent {
                        hook_event_name: event.as_str(),
                        additional_context: text,
                    },
                };
                serde_json::to_writer(&mut *writer, &payload)?;
                writer.write_all(b"\n")?;
            }
            Self::SystemMessage(text) => {
                let payload = SystemMessagePayload {
                    system_message: text,
                };
                serde_json::to_writer(&mut *writer, &payload)?;
                writer.write_all(b"\n")?;
            }
            Self::Silent => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::panic::catch_unwind;

    use serde_json::Value;

    use super::{HookOutput, IntentBoundaryEvent};

    fn serialize(output: &HookOutput) -> String {
        let mut buf = Vec::new();
        output
            .serialize_to(&mut buf)
            .expect("serialize hook output");
        String::from_utf8(buf).expect("utf8 hook output")
    }

    #[test]
    fn pre_tool_use_permission_serializes_as_permission_decision_envelope() {
        let text = serialize(&HookOutput::pre_tool_use_permission(
            "forbidden command",
            "policy violation",
        ));
        let json: Value = serde_json::from_str(text.trim()).expect("json");
        assert_eq!(
            json,
            serde_json::json!({
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "deny",
                    "permissionDecisionReason": "forbidden command\n\npolicy violation"
                }
            })
        );
    }

    #[test]
    fn session_start_and_user_prompt_submit_serialize_as_additional_context() {
        for event in [
            IntentBoundaryEvent::SessionStart,
            IntentBoundaryEvent::UserPromptSubmit,
        ] {
            let text = serialize(&HookOutput::hook_specific_additional_context(
                event,
                "board reminder",
            ));
            let json: Value = serde_json::from_str(text.trim()).expect("json");
            assert_eq!(
                json,
                serde_json::json!({
                    "hookSpecificOutput": {
                        "hookEventName": event.as_str(),
                        "additionalContext": "board reminder"
                    }
                })
            );
        }
    }

    #[test]
    fn system_message_serializes_without_hook_specific_output() {
        let text = serialize(&HookOutput::system_message("stop reminder"));
        let json: Value = serde_json::from_str(text.trim()).expect("json");
        assert_eq!(
            json,
            serde_json::json!({ "systemMessage": "stop reminder" })
        );
        assert!(
            json.get("hookSpecificOutput").is_none(),
            "systemMessage envelope must not contain hookSpecificOutput"
        );
    }

    #[test]
    fn silent_emits_no_stdout() {
        let text = serialize(&HookOutput::Silent);
        assert!(text.is_empty(), "silent output must not write stdout");
    }

    #[test]
    fn stop_additional_context_panics_in_debug_and_falls_back_in_release() {
        let output =
            HookOutput::hook_specific_additional_context(IntentBoundaryEvent::Stop, "stop text");

        if cfg!(debug_assertions) {
            let panic = catch_unwind(|| serialize(&output));
            assert!(
                panic.is_err(),
                "Stop additionalContext must panic in debug builds"
            );
        } else {
            let text = serialize(&output);
            let json: Value = serde_json::from_str(text.trim()).expect("json");
            assert_eq!(json, serde_json::json!({ "systemMessage": "stop text" }));
        }
    }
}
