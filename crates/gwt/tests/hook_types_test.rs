//! T-010 (SPEC #1942) — HookKind::from_name coverage.
//!
//! Red phase for CORE-CLI hook shared types. Every documented hook name in
//! `specs/data-model.md` must map to its enum variant, and every unknown
//! string must map to `None`.

use gwt::cli::hook::{HookEvent, HookKind, HookOutput};

#[test]
fn issue_3713_plugin_active_skill_plugin_advisory_identity_survives_both_spellings() {
    for payload in [
        r#"{"active_skill":"external:ship","active_plugin":"external@market"}"#,
        r#"{"activeSkill":"external:ship","activePlugin":"external@market"}"#,
        r#"{"active_skill":"external:ship","activeSkill":"other","active_plugin":"external@market","activePlugin":"other"}"#,
    ] {
        let event: HookEvent = serde_json::from_str(payload).unwrap();
        let observed = format!("{event:?}");
        assert!(
            observed.contains("external:ship"),
            "lost skill provenance: {observed}"
        );
        assert!(
            observed.contains("external@market"),
            "lost plugin provenance: {observed}"
        );
    }
    let legacy: HookEvent = serde_json::from_str("{}").unwrap();
    assert!(legacy.tool_name.is_none());
}

#[test]
fn issue_3713_plugin_provider_adapters_preserve_advisory_provenance() {
    for (provider, event_name) in [
        ("opencode", "tool.execute.before"),
        ("openclaw", "before_tool_call"),
        ("hermes", "pre_tool_call"),
    ] {
        let normalized = gwt::cli::hook::provider_event::normalize_provider_payload(provider, event_name,
            r#"{"toolName":"Skill","activeSkill":"external:ship","activePlugin":"external@market"}"#).unwrap();
        let event: HookEvent = serde_json::from_value(normalized.payload).unwrap();
        assert_eq!(event.active_skill.as_deref(), Some("external:ship"));
        assert_eq!(event.active_plugin.as_deref(), Some("external@market"));
    }
    let malformed: HookEvent =
        serde_json::from_str(r#"{"activeSkill":7,"activePlugin":{}}"#).unwrap();
    assert!(malformed.active_skill.is_none() && malformed.active_plugin.is_none());
}

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

/// Real captures from macOS arm64. The fixture path records the exact provider
/// version. The Grok Build 1.0.41 captures are from the Issue #4771 comment
/// (5884553029), recorded while investigating Issue #4768.
#[test]
fn issue_4771_real_hook_payloads_pass_the_parser_entry() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hook_payloads");
    for (provider, version, events) in [
        (
            "claude",
            "2.1.332",
            &[
                "SessionStart",
                "UserPromptSubmit",
                "PreToolUse",
                "PostToolUse",
                "Stop",
            ][..],
        ),
        (
            "codex",
            "0.172.0",
            &[
                "SessionStart",
                "UserPromptSubmit",
                "PreToolUse",
                "PostToolUse",
                "Stop",
            ][..],
        ),
        (
            "grok-build",
            "1.0.41",
            &["SessionStart", "PreToolUse", "PostToolUse"][..],
        ),
    ] {
        for event_name in events {
            let path = root
                .join(provider)
                .join(version)
                .join(format!("{event_name}.json"));
            let label = format!("{provider}/{version}/{event_name}");
            let raw = std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{label}: {err}"));
            let parsed = HookEvent::read_from_str(&raw)
                .unwrap_or_else(|err| panic!("{label}: {err}"))
                .unwrap_or_else(|| panic!("{label}: empty payload"));
            let value: serde_json::Value = serde_json::from_str(&raw).unwrap();

            assert_eq!(
                value["hook_event_name"].as_str(),
                Some(*event_name),
                "{label}"
            );
            assert_eq!(parsed.cwd.as_deref(), value["cwd"].as_str(), "{label}");
            assert_fixture_scrubbed(&label, &value);
        }
    }
}

fn assert_fixture_scrubbed(label: &str, value: &serde_json::Value) {
    match value {
        serde_json::Value::String(value) => {
            assert!(
                ![
                    "/Users/",
                    "/home/",
                    "/private/",
                    "/tmp/",
                    "%2FUsers",
                    ":\\",
                    "Bearer ",
                    "sk-",
                    "xai-",
                    "ghp_",
                    "github_pat_"
                ]
                .iter()
                .any(|marker| value.contains(marker)),
                "{label}: fixture contains a path or credential marker"
            );
            assert!(
                !value.starts_with('/') || value.starts_with("/redacted/"),
                "{label}: absolute path outside /redacted/"
            );
        }
        serde_json::Value::Array(items) => {
            for item in items {
                assert_fixture_scrubbed(label, item);
            }
        }
        serde_json::Value::Object(fields) => {
            for (key, value) in fields {
                if matches!(
                    key.as_str(),
                    "session_id"
                        | "sessionId"
                        | "prompt_id"
                        | "promptId"
                        | "turn_id"
                        | "tool_use_id"
                        | "toolUseId"
                ) {
                    assert!(
                        value
                            .as_str()
                            .is_some_and(|id| id.starts_with("00000000-0000-0000-0000-")),
                        "{label}: unredacted {key}"
                    );
                }
                assert_fixture_scrubbed(label, value);
            }
        }
        _ => {}
    }
}

#[test]
fn issue_4771_invalid_hook_payloads_fail_closed() {
    for (name, raw, expected) in [
        (
            "wrong type",
            include_str!("fixtures/hook_payload_rejections/wrong_type.json"),
            "invalid type",
        ),
        (
            "duplicate key",
            include_str!("fixtures/hook_payload_rejections/duplicate_key.json"),
            "duplicate field `session_id`",
        ),
        (
            "duplicate camelCase key",
            include_str!("fixtures/hook_payload_rejections/duplicate_camel_case_key.json"),
            "duplicate field `sessionId`",
        ),
    ] {
        let error = HookEvent::read_from_str(raw).expect_err(name).to_string();
        assert!(error.contains(expected), "{name}: {error}");
    }
}
