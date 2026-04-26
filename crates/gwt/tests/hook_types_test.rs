//! T-010 (SPEC #1942) — HookKind::from_name coverage.
//!
//! Red phase for CORE-CLI hook shared types. Every documented hook name in
//! `specs/data-model.md` must map to its enum variant, and every unknown
//! string must map to `None`.

use gwt::cli::hook::{HookEvent, HookKind, HookOutput};

#[test]
fn hook_kind_from_name_covers_every_documented_hook() {
    assert_eq!(
        HookKind::from_name("runtime-state"),
        Some(HookKind::RuntimeState)
    );
    assert_eq!(
        HookKind::from_name("block-bash-policy"),
        Some(HookKind::BlockBashPolicy)
    );
    assert_eq!(
        HookKind::from_name("workflow-policy"),
        Some(HookKind::WorkflowPolicy)
    );
    assert_eq!(HookKind::from_name("event"), Some(HookKind::Event));
    assert_eq!(
        HookKind::from_name("coordination-event"),
        Some(HookKind::CoordinationEvent)
    );
    assert_eq!(HookKind::from_name("forward"), Some(HookKind::Forward));
}

#[test]
fn hook_event_command_returns_none_when_tool_input_missing() {
    let ev: HookEvent = serde_json::from_str(r#"{"tool_name":"Bash"}"#).unwrap();
    assert_eq!(ev.command(), None);
}

#[test]
fn hook_event_command_returns_none_when_command_field_missing() {
    let ev: HookEvent =
        serde_json::from_str(r#"{"tool_name":"Bash","tool_input":{"other":"value"}}"#).unwrap();
    assert_eq!(ev.command(), None);
}

#[test]
fn hook_event_command_returns_command_string_when_present() {
    let ev: HookEvent = serde_json::from_str(
        r#"{"tool_name":"Bash","tool_input":{"command":"git rebase -i origin/main"}}"#,
    )
    .unwrap();
    assert_eq!(ev.command(), Some("git rebase -i origin/main"));
}

#[test]
fn hook_event_command_returns_none_when_command_field_is_not_a_string() {
    let ev: HookEvent =
        serde_json::from_str(r#"{"tool_name":"Bash","tool_input":{"command":123}}"#).unwrap();
    assert_eq!(ev.command(), None);
}

#[test]
fn pre_tool_use_permission_serializes_as_hook_specific_output() {
    // Claude Code PreToolUse contract: the hook must emit
    // `hookSpecificOutput.permissionDecisionReason` so the reason text is
    // actually surfaced to the LLM/user. The legacy `decision`/`reason`/
    // `stopReason` top-level fields are intentionally dropped because
    // `stopReason` is ignored on PreToolUse and only `reason` was visible.
    let decision = HookOutput::pre_tool_use_permission("forbidden command", "policy violation");
    let mut buf = Vec::new();
    decision.serialize_to(&mut buf).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&buf).unwrap();

    assert!(
        json.get("decision").is_none(),
        "legacy top-level `decision` field must not be emitted, got: {json}"
    );
    assert!(
        json.get("reason").is_none(),
        "legacy top-level `reason` field must not be emitted"
    );
    assert!(
        json.get("stopReason").is_none(),
        "legacy top-level `stopReason` field must not be emitted (Stop-hook only)"
    );
    assert!(
        json.get("stop_reason").is_none(),
        "snake_case field must never leak into the wire format"
    );

    let hook_output = json
        .get("hookSpecificOutput")
        .expect("hookSpecificOutput must be the top-level payload");
    assert_eq!(hook_output["hookEventName"], "PreToolUse");
    assert_eq!(hook_output["permissionDecision"], "deny");

    let reason = hook_output["permissionDecisionReason"]
        .as_str()
        .expect("permissionDecisionReason must be a string");
    assert!(
        reason.contains("forbidden command"),
        "summary must be part of the visible reason, got: {reason}"
    );
    assert!(
        reason.contains("policy violation"),
        "detail must be part of the visible reason, got: {reason}"
    );
}

#[test]
fn pre_tool_use_permission_accessors_expose_summary_and_detail() {
    let decision = HookOutput::pre_tool_use_permission("forbidden command", "policy violation");
    assert_eq!(decision.summary(), "forbidden command");
    assert_eq!(decision.detail(), "policy violation");
    assert!(decision
        .permission_decision_reason()
        .contains("forbidden command"));
    assert!(decision
        .permission_decision_reason()
        .contains("policy violation"));
}

#[test]
fn hook_kind_from_name_rejects_unknown_names() {
    assert_eq!(HookKind::from_name(""), None);
    assert_eq!(HookKind::from_name("RuntimeState"), None); // case-sensitive
    assert_eq!(HookKind::from_name("block_git_branch_ops"), None); // snake_case not accepted
    assert_eq!(HookKind::from_name("block-git"), None);
    assert_eq!(HookKind::from_name("unknown-hook"), None);
}
