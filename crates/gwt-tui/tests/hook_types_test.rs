//! T-010 (SPEC #1942) — HookKind::from_name coverage.
//!
//! Red phase for CORE-CLI hook shared types. Every documented hook name in
//! `specs/data-model.md` must map to its enum variant, and every unknown
//! string must map to `None`.

use gwt_tui::cli::hook::{BlockDecision, HookEvent, HookKind};

#[test]
fn hook_kind_from_name_covers_every_documented_hook() {
    assert_eq!(
        HookKind::from_name("runtime-state"),
        Some(HookKind::RuntimeState)
    );
    assert_eq!(
        HookKind::from_name("block-git-branch-ops"),
        Some(HookKind::BlockGitBranchOps)
    );
    assert_eq!(
        HookKind::from_name("block-cd-command"),
        Some(HookKind::BlockCdCommand)
    );
    assert_eq!(
        HookKind::from_name("block-file-ops"),
        Some(HookKind::BlockFileOps)
    );
    assert_eq!(
        HookKind::from_name("block-git-dir-override"),
        Some(HookKind::BlockGitDirOverride)
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
fn block_decision_new_serializes_with_camelcase_stop_reason() {
    let decision = BlockDecision::new("forbidden command", "policy violation");
    let json = serde_json::to_value(&decision).unwrap();
    assert_eq!(json["decision"], "block");
    assert_eq!(json["reason"], "forbidden command");
    assert_eq!(json["stopReason"], "policy violation");
    assert!(
        json.get("stop_reason").is_none(),
        "must not expose snake_case field"
    );
}

#[test]
fn hook_kind_from_name_rejects_unknown_names() {
    assert_eq!(HookKind::from_name(""), None);
    assert_eq!(HookKind::from_name("RuntimeState"), None); // case-sensitive
    assert_eq!(HookKind::from_name("block_git_branch_ops"), None); // snake_case not accepted
    assert_eq!(HookKind::from_name("block-git"), None);
    assert_eq!(HookKind::from_name("unknown-hook"), None);
}
