//! Issue #3478 (SPEC #3200 FR-025): an autonomous agent's confirmation
//! question must be converted into a structured NeedsHuman handoff *before*
//! the provider's question UI starts waiting, so the Issue Monitor's active
//! slot is released immediately instead of after `stuck_timeout_secs`.

use gwt::autonomous_handoff::{
    autonomous_execution_context_from_env, classify_handoff_reason, extract_question,
    is_question_tool, AutonomousExecutionContext, AutonomousHandoffReason, AutonomousHandoffState,
    AutonomousQuestionHandoff,
};
use serde_json::json;

/// AC-3 (provider parity): both the Claude Code and the Codex question tool
/// are recognized, and unrelated tools are left alone.
#[test]
fn question_tools_are_recognized_across_providers() {
    for tool in [
        "AskUserQuestion",
        "AskUserQuestionTool",
        "ask_user_question",
        "request_user_input",
        "ask_user",
        "ask_followup_question",
    ] {
        assert!(
            is_question_tool(tool),
            "expected {tool} to be a question tool"
        );
    }

    for tool in ["Bash", "Edit", "Read", "Write", "TodoWrite", "board.post"] {
        assert!(
            !is_question_tool(tool),
            "expected {tool} NOT to be a question tool"
        );
    }
}

/// AC-3: the Claude Code `AskUserQuestion` payload keeps its question text and
/// its options so the human sees exactly what the agent would have asked.
#[test]
fn claude_question_payload_is_extracted_with_options() {
    let input = json!({
        "questions": [{
            "question": "Should the migration drop the legacy column?",
            "header": "Migration",
            "multiSelect": false,
            "options": [
                {"label": "Drop it", "description": "Irreversible data loss"},
                {"label": "Keep it", "description": "Leave the column in place"}
            ]
        }]
    });

    let extracted = extract_question(Some(&input));

    assert_eq!(
        extracted.question,
        "Should the migration drop the legacy column?"
    );
    assert_eq!(extracted.options.len(), 2);
    assert_eq!(extracted.options[0].label, "Drop it");
    assert_eq!(extracted.options[0].description, "Irreversible data loss");
    assert_eq!(extracted.options[1].label, "Keep it");
}

/// AC-3 (provider parity): the Codex `request_user_input` payload uses a flat
/// shape with plain-string options.
#[test]
fn codex_question_payload_is_extracted_with_options() {
    let input = json!({
        "question": "Which credential store should be used?",
        "options": ["Keychain", "Environment variable"]
    });

    let extracted = extract_question(Some(&input));

    assert_eq!(extracted.question, "Which credential store should be used?");
    assert_eq!(extracted.options.len(), 2);
    assert_eq!(extracted.options[0].label, "Keychain");
    assert_eq!(extracted.options[1].label, "Environment variable");
}

/// A question tool invoked with an unparseable / absent payload still produces
/// a handoff: losing the text must never degrade into silently waiting.
#[test]
fn missing_question_payload_still_yields_a_handoff_question() {
    let extracted = extract_question(None);
    assert!(!extracted.question.trim().is_empty());
    assert!(extracted.options.is_empty());
}

/// AC-3: the reason code is machine-readable. Classification is a label for
/// the human queue only — it never decides whether the handoff happens.
#[test]
fn handoff_reason_is_classified_from_the_question_text() {
    assert_eq!(
        classify_handoff_reason("Should I force-push and delete the remote branch?", &[]),
        AutonomousHandoffReason::IrreversibleAction
    );
    assert_eq!(
        classify_handoff_reason("Which API token should the client use?", &[]),
        AutonomousHandoffReason::SecurityCredential
    );
    assert_eq!(
        classify_handoff_reason(
            "The SPEC acceptance criteria contradict the current behavior — which wins?",
            &[]
        ),
        AutonomousHandoffReason::SpecConflict
    );
    assert_eq!(
        classify_handoff_reason("Please visually verify the new modal in the GUI", &[]),
        AutonomousHandoffReason::HumanVerification
    );
    assert_eq!(
        classify_handoff_reason("Which of these two variable names reads better?", &[]),
        AutonomousHandoffReason::Unclassified
    );
}

/// AC-1: the autonomous execution context is machine-readable and is absent
/// for a non-autonomous launch (no behavior change outside autonomous mode).
#[test]
fn autonomous_execution_context_is_read_from_the_injected_environment() {
    let context = autonomous_execution_context_from_env(|name| match name {
        "GWT_AUTONOMOUS_EXECUTION" => Some("1".to_string()),
        "GWT_AUTONOMOUS_ISSUE" => Some("3478".to_string()),
        "GWT_SESSION_ID" => Some("session-abc".to_string()),
        _ => None,
    });

    assert_eq!(
        context,
        Some(AutonomousExecutionContext {
            issue_number: 3478,
            session_id: "session-abc".to_string(),
        })
    );

    // Non-autonomous launch: the marker is absent, so nothing is intercepted.
    assert!(autonomous_execution_context_from_env(|name| match name {
        "GWT_SESSION_ID" => Some("session-abc".to_string()),
        _ => None,
    })
    .is_none());

    // Marker present but no owner issue: fail-closed, do not fabricate one.
    assert!(autonomous_execution_context_from_env(|name| match name {
        "GWT_AUTONOMOUS_EXECUTION" => Some("1".to_string()),
        "GWT_SESSION_ID" => Some("session-abc".to_string()),
        _ => None,
    })
    .is_none());
}

/// AC-3/AC-4: a handoff carries owner, session, question, options, rationale
/// and reason code, and starts in the `Pending` state the driver consumes.
#[test]
fn handoff_round_trips_through_serde_with_every_required_field() {
    let handoff = AutonomousQuestionHandoff::new(
        "handoff-1".to_string(),
        &AutonomousExecutionContext {
            issue_number: 3478,
            session_id: "session-abc".to_string(),
        },
        "claude-code",
        "AskUserQuestion",
        extract_question(Some(&json!({
            "questions": [{
                "question": "Delete the production bucket?",
                "options": [{"label": "Yes", "description": "irreversible"}]
            }]
        }))),
        "2026-08-06T05:00:00Z",
    );

    assert_eq!(handoff.issue_number, 3478);
    assert_eq!(handoff.session_id, "session-abc");
    assert_eq!(handoff.provider, "claude-code");
    assert_eq!(handoff.tool_name, "AskUserQuestion");
    assert_eq!(handoff.question, "Delete the production bucket?");
    assert_eq!(handoff.options.len(), 1);
    assert_eq!(
        handoff.reason_code,
        AutonomousHandoffReason::IrreversibleAction
    );
    assert!(!handoff.rationale.trim().is_empty());
    assert_eq!(handoff.state, AutonomousHandoffState::Pending);
    assert!(handoff.answer.is_none());

    let encoded = serde_json::to_string(&handoff).expect("serialize handoff");
    let decoded: AutonomousQuestionHandoff =
        serde_json::from_str(&encoded).expect("deserialize handoff");
    assert_eq!(decoded, handoff);
}
