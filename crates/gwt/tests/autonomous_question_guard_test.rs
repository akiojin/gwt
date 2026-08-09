//! Issue #3478 (SPEC #3200 FR-025): the PreToolUse interception half. A
//! monitor-launched autonomous session must never reach a waiting question UI:
//! the question tool is denied *before* it can wait and is converted into a
//! durable handoff in the Issue Monitor control plane.

use std::path::Path;

use gwt::autonomous_handoff::{AutonomousExecutionContext, AutonomousHandoffState};
use gwt::cli::hook::autonomous_question_guard::{
    evaluate_and_record, QuestionGuardInputs, QUESTION_HANDOFF_SUMMARY,
};
use gwt::cli::hook::{HookEvent, HookOutput};
use gwt::IssueMonitorPrefs;
use serde_json::json;

const NOW: &str = "2026-08-06T05:00:00Z";

fn context() -> AutonomousExecutionContext {
    AutonomousExecutionContext {
        issue_number: 3478,
        session_id: "session-abc".to_string(),
    }
}

fn event(tool_name: &str, tool_input: serde_json::Value) -> HookEvent {
    HookEvent {
        tool_name: Some(tool_name.to_string()),
        tool_input: Some(tool_input),
        transcript_path: None,
        cwd: None,
    }
}

fn inputs<'a>(prefs_path: &'a Path, provider: &'a str) -> QuestionGuardInputs<'a> {
    QuestionGuardInputs {
        context: context(),
        prefs_path,
        provider,
        now: NOW,
        handoff_id: "handoff-fixed",
    }
}

fn load(prefs_path: &Path) -> IssueMonitorPrefs {
    let raw = std::fs::read_to_string(prefs_path).expect("prefs written");
    serde_json::from_str(&raw).expect("prefs parse")
}

/// AC-3/AC-4: the Claude Code question tool is denied and converted, and the
/// deny text tells the agent the execution is over rather than paused.
#[test]
fn claude_question_is_denied_and_recorded_as_a_handoff() {
    let dir = tempfile::tempdir().expect("tempdir");
    let prefs_path = dir.path().join("issue-monitor.json");

    let output = evaluate_and_record(
        &event(
            "AskUserQuestion",
            json!({
                "questions": [{
                    "question": "Should I delete the legacy table?",
                    "options": [{"label": "Delete", "description": "irreversible"}]
                }]
            }),
        ),
        &inputs(&prefs_path, "claude-code"),
    );

    let HookOutput::PreToolUsePermission {
        summary, detail, ..
    } = output
    else {
        panic!("expected the question tool to be denied");
    };
    assert_eq!(summary, QUESTION_HANDOFF_SUMMARY);
    assert!(detail.contains("Issue #3478"));
    assert!(
        detail.contains("handoff-fixed"),
        "the deny text names the handoff so the agent can reference it"
    );

    let prefs = load(&prefs_path);
    assert_eq!(prefs.autonomous_handoffs.len(), 1);
    let handoff = &prefs.autonomous_handoffs[0];
    assert_eq!(handoff.issue_number, 3478);
    assert_eq!(handoff.session_id, "session-abc");
    assert_eq!(handoff.provider, "claude-code");
    assert_eq!(handoff.tool_name, "AskUserQuestion");
    assert_eq!(handoff.question, "Should I delete the legacy table?");
    assert_eq!(handoff.state, AutonomousHandoffState::Pending);
}

/// AC-3 (provider parity): the Codex question tool takes the identical path.
#[test]
fn codex_question_is_denied_and_recorded_as_a_handoff() {
    let dir = tempfile::tempdir().expect("tempdir");
    let prefs_path = dir.path().join("issue-monitor.json");

    let output = evaluate_and_record(
        &event(
            "request_user_input",
            json!({"question": "Which credential store?", "options": ["Keychain"]}),
        ),
        &inputs(&prefs_path, "codex"),
    );

    assert!(matches!(output, HookOutput::PreToolUsePermission { .. }));
    let prefs = load(&prefs_path);
    assert_eq!(prefs.autonomous_handoffs.len(), 1);
    assert_eq!(prefs.autonomous_handoffs[0].provider, "codex");
    assert_eq!(prefs.autonomous_handoffs[0].tool_name, "request_user_input");
}

/// A repeated hook invocation for the same intercepted call must not park the
/// same question twice.
#[test]
fn recording_the_same_handoff_twice_is_idempotent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let prefs_path = dir.path().join("issue-monitor.json");
    let question = event("AskUserQuestion", json!({"question": "Proceed?"}));

    evaluate_and_record(&question, &inputs(&prefs_path, "claude-code"));
    evaluate_and_record(&question, &inputs(&prefs_path, "claude-code"));

    assert_eq!(load(&prefs_path).autonomous_handoffs.len(), 1);
}

/// AC-6 non-regression: non-question tools in an autonomous session are
/// untouched — the guard adds no new denial surface.
#[test]
fn non_question_tools_are_not_intercepted() {
    let dir = tempfile::tempdir().expect("tempdir");
    let prefs_path = dir.path().join("issue-monitor.json");

    let output = evaluate_and_record(
        &event("Bash", json!({"command": "cargo test"})),
        &inputs(&prefs_path, "claude-code"),
    );

    assert_eq!(output, HookOutput::Silent);
    assert!(
        !prefs_path.exists(),
        "an unrelated tool must not touch the control plane"
    );
}

/// A question whose payload cannot be read still converts: losing the text may
/// never degrade into silently waiting for a human.
#[test]
fn unreadable_question_payload_still_converts() {
    let dir = tempfile::tempdir().expect("tempdir");
    let prefs_path = dir.path().join("issue-monitor.json");

    let output = evaluate_and_record(
        &HookEvent {
            tool_name: Some("AskUserQuestion".to_string()),
            tool_input: None,
            transcript_path: None,
            cwd: None,
        },
        &inputs(&prefs_path, "claude-code"),
    );

    assert!(matches!(output, HookOutput::PreToolUsePermission { .. }));
    assert_eq!(load(&prefs_path).autonomous_handoffs.len(), 1);
}
