use super::*;
use serde_json::{json, Value};

fn lines(values: &[Value]) -> Vec<u8> {
    values
        .iter()
        .map(|v| format!("{v}\n"))
        .collect::<String>()
        .into_bytes()
}
fn claude_message(id: &str, role: &str, content: Value) -> Value {
    json!({"uuid":id,"sessionId":"conversation","cwd":"/work/project","type":role,
        "message":{"id":"shared-provider-message","role":role,"content":content}})
}
fn codex_meta() -> Value {
    json!({"type":"session_meta","payload":{"id":"conversation","cwd":"/work/project"}})
}
fn parse(agent: AgentId, records: &[Value]) -> PmConversationSnapshot {
    parse_native(
        &agent,
        &lines(records),
        "conversation",
        Path::new("/work/project"),
    )
}

#[test]
fn claude_keeps_text_and_ignores_tools_meta_and_silent_cycles() {
    let mut meta = claude_message("meta", "user", json!("hidden reminder"));
    meta["isMeta"] = json!(true);
    let mut compact = claude_message("summary", "user", json!("hidden summary"));
    compact["isCompactSummary"] = json!(true);
    let mut notification = claude_message("notification", "user", json!("background finished"));
    notification["origin"] = json!({"kind":"task-notification"});
    let answer = claude_message(
        "answer",
        "assistant",
        json!([{"type":"text","text":"Answer"}]),
    );
    let result = parse(
        AgentId::ClaudeCode,
        &[
            claude_message("human", "user", json!("Explain the text '[gwt] hello'")),
            meta,
            compact,
            notification,
            claude_message(
                "thinking",
                "assistant",
                json!([{"type":"thinking","thinking":"private"}]),
            ),
            claude_message(
                "tool",
                "assistant",
                json!([{"type":"tool_use","name":"Bash"}]),
            ),
            claude_message(
                "result",
                "user",
                json!([{"type":"tool_result","content":"secret tool output"}]),
            ),
            answer.clone(),
            answer,
        ],
    );
    assert_eq!(result.availability, PmConversationAvailability::Ready);
    assert_eq!(
        result
            .messages
            .iter()
            .map(|m| m.text.as_str())
            .collect::<Vec<_>>(),
        ["Explain the text '[gwt] hello'", "Answer"]
    );
}

#[test]
fn codex_uses_response_items_only_and_deduplicates_item_ids() {
    let answer = json!({"type":"response_item","payload":{"type":"message","id":"a","role":"assistant","phase":"final_answer","content":[{"type":"output_text","text":"Answer"}]}});
    let result = parse(
        AgentId::Codex,
        &[
            codex_meta(),
            json!({"type":"response_item","payload":{"type":"message","id":"u","role":"user","content":[{"type":"input_text","text":"Question"}]}}),
            json!({"type":"response_item","payload":{"type":"function_call_output","output":"secret"}}),
            json!({"type":"response_item","payload":{"type":"reasoning","summary":"private"}}),
            json!({"type":"response_item","payload":{"type":"message","role":"developer","content":[{"type":"input_text","text":"instructions"}]}}),
            answer.clone(),
            answer,
            json!({"type":"event_msg","payload":{"type":"task_complete","last_agent_message":"Answer"}}),
        ],
    );
    assert_eq!(
        result
            .messages
            .iter()
            .map(|m| m.text.as_str())
            .collect::<Vec<_>>(),
        ["Question", "Answer"]
    );
}

#[test]
fn metadata_identity_is_required_before_text_is_disclosed() {
    let mut wrong = codex_meta();
    wrong["payload"]["id"] = json!("foreign");
    let mut foreign = claude_message("u", "user", json!("private"));
    foreign["cwd"] = json!("/other/project");
    for snapshot in [
        parse(AgentId::Codex, &[wrong]),
        parse(AgentId::ClaudeCode, &[foreign]),
        parse(
            AgentId::Codex,
            &[
                json!({"type":"response_item","payload":{"type":"message","id":"u","role":"user","content":[{"type":"input_text","text":"private"}]}}),
            ],
        ),
    ] {
        assert_eq!(
            snapshot.availability,
            PmConversationAvailability::Unavailable
        );
        assert!(snapshot.messages.is_empty());
    }
}

#[test]
fn missing_conversation_and_unsupported_provider_have_explicit_states() {
    let mut session = Session::new(Path::new("/work/project"), "work/test", AgentId::ClaudeCode);
    assert_eq!(
        read_for_session(&session).availability,
        PmConversationAvailability::Waiting
    );
    session.agent_id = AgentId::OpenCode;
    assert_eq!(
        read_for_session(&session).availability,
        PmConversationAvailability::Unsupported
    );
}

#[test]
fn known_internal_prompts_are_hidden_without_erasing_human_mentions() {
    let wake = format!("[gwt] Scheduled supervision tick: reconcile now — read a fresh `issue.monitor.status` snapshot and inventory open PRs with `pr.list` (stale / SUPERSEDED / owner-Issue-closed rows: digest escalations, never auto-close). {} {} {}", crate::pm_registry::PM_STEERING_WAKE_CLAUSE, crate::pm_registry::PM_GWTD_EXECUTION_WAKE_CLAUSE, crate::pm_registry::PM_CYCLE_REPORTING_CLAUSE);
    let delivery = crate::pm_registry::protected_pm_delivery_prompt(
        "00000000-0000-4000-8000-000000000001",
        "injected",
    )
    .unwrap();
    let result = parse(
        AgentId::ClaudeCode,
        &[
            claude_message("bootstrap", "user", json!("$gwt-pm")),
            claude_message("wake", "user", json!(wake)),
            claude_message("delivery", "user", json!(delivery)),
            claude_message("human", "user", json!("[gwt] please explain this prefix")),
        ],
    );
    assert_eq!(
        result
            .messages
            .iter()
            .map(|m| m.text.as_str())
            .collect::<Vec<_>>(),
        ["[gwt] please explain this prefix"]
    );
}

#[test]
fn missing_store_and_partial_append_do_not_expose_unverified_text() {
    let directory = tempfile::tempdir().unwrap();
    let missing = read_path(
        &AgentId::Codex,
        &directory.path().join("missing.jsonl"),
        "conversation",
        Path::new("/work/project"),
    );
    assert_eq!(missing.availability, PmConversationAvailability::Waiting);
    let mut bytes = lines(&[codex_meta()]);
    bytes.extend_from_slice(b"{\"type\":\"response_item\"");
    let result = parse_native(
        &AgentId::Codex,
        &bytes,
        "conversation",
        Path::new("/work/project"),
    );
    assert_eq!(result.availability, PmConversationAvailability::Ready);
    assert!(result.messages.is_empty());
}

#[test]
fn large_tool_only_cycle_keeps_the_last_visible_exchange() {
    let result = parse(
        AgentId::Codex,
        &[
            codex_meta(),
            json!({"type":"response_item","payload":{"type":"message","id":"u","role":"user","content":[{"type":"input_text","text":"Question"}]}}),
            json!({"type":"response_item","payload":{"type":"message","id":"a","role":"assistant","content":[{"type":"output_text","text":"Answer"}]}}),
            json!({"type":"response_item","payload":{"type":"function_call_output","output":"x".repeat(MAX_TEXT_BYTES + 100)}}),
        ],
    );
    assert_eq!(result.availability, PmConversationAvailability::Ready);
    assert_eq!(
        result
            .messages
            .iter()
            .map(|m| m.text.as_str())
            .collect::<Vec<_>>(),
        ["Question", "Answer"]
    );
}

#[test]
fn stop_feedback_metadata_cannot_render_or_invalidate_verified_chat() {
    let feedback =
        "Stop hook feedback:\nResident PM loop: run one cycle before stopping. Internal contract.";
    let result = parse(
        AgentId::ClaudeCode,
        &[
            claude_message("u", "user", json!("Please explain the Stop hook.")),
            // Native Stop feedback is meta. Its omitted identity must not poison
            // an otherwise verified conversation, nor count as proof of identity.
            json!({"type":"user","uuid":"hook","isMeta":true,"message":{"role":"user","content":feedback}}),
            claude_message(
                "a",
                "assistant",
                json!([{"type":"text","text":"The phrase Resident PM loop: run one cycle before stopping. comes from a hook."}]),
            ),
        ],
    );
    assert_eq!(result.availability, PmConversationAvailability::Ready);
    assert_eq!(result.messages.len(), 2);
    assert!(result
        .messages
        .iter()
        .all(|message| !message.text.starts_with("Stop hook feedback:")));
    let only_meta = parse(
        AgentId::ClaudeCode,
        &[json!({"type":"user","isMeta":true,"message":{"role":"user","content":feedback}})],
    );
    assert_eq!(
        only_meta.availability,
        PmConversationAvailability::Unavailable
    );
    let codex = parse(
        AgentId::Codex,
        &[
            codex_meta(),
            json!({"type":"response_item","payload":{"type":"message","id":"d","role":"developer","content":[{"type":"input_text","text":feedback}]}}),
        ],
    );
    assert!(codex.messages.is_empty());
}

#[test]
fn canonical_pm_runtime_cwd_is_allowed_without_accepting_other_directories() {
    let home = tempfile::tempdir().unwrap();
    let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
    let worktree = crate::pm_registry::pm_worktree_path_for_repo_path(Path::new("/fixture"));
    let runtime = crate::pm_registry::pm_runtime_dir_for_pm_worktree(&worktree).unwrap();
    let foreign_worktree =
        crate::pm_registry::pm_worktree_path_for_repo_path(Path::new("/foreign"));
    let foreign_runtime =
        crate::pm_registry::pm_runtime_dir_for_pm_worktree(&foreign_worktree).unwrap();
    for agent in [AgentId::ClaudeCode, AgentId::Codex] {
        for (cwd, expected) in [
            (runtime.clone(), PmConversationAvailability::Ready),
            (
                worktree.parent().unwrap().join("scratch"),
                PmConversationAvailability::Unavailable,
            ),
            (
                foreign_runtime.clone(),
                PmConversationAvailability::Unavailable,
            ),
        ] {
            let records = if agent == AgentId::ClaudeCode {
                vec![
                    json!({"type":"user","uuid":"u","sessionId":"conversation","cwd":cwd,
                    "message":{"role":"user","content":"Question"}}),
                ]
            } else {
                vec![
                    json!({"type":"session_meta","payload":{"id":"conversation","cwd":cwd}}),
                    json!({"type":"response_item","payload":{"type":"message","id":"u","role":"user","content":[{"type":"input_text","text":"Question"}]}}),
                ]
            };
            let result = parse_native(&agent, &lines(&records), "conversation", &worktree);
            assert_eq!(result.availability, expected, "{agent:?}");
            assert_eq!(
                result.messages.len(),
                usize::from(expected == PmConversationAvailability::Ready)
            );
        }
    }
    assert!(!same_path(
        Path::new("/untrusted/pm/runtime"),
        Path::new("/untrusted/pm/worktree")
    ));
}

#[test]
fn reader_only_parses_appended_complete_records() {
    use std::io::Write;
    let _env = gwt_core::test_support::env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = tempfile::tempdir().unwrap();
    let _home = gwt_core::test_support::ScopedEnvVar::set("CLAUDE_CONFIG_DIR", home.path());
    let dir = home.path().join("projects/project");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("conversation.jsonl");
    let first = lines(&[claude_message("u", "user", json!("Question"))]);
    std::fs::write(&path, &first).unwrap();
    let mut session = Session::new(Path::new("/work/project"), "pm", AgentId::ClaudeCode);
    session.agent_session_id = Some("conversation".to_owned());
    let mut reader = PmConversationReader::default();
    assert_eq!(reader.read_for_session(&session).messages.len(), 1);
    assert_eq!(reader.processed_bytes, first.len() as u64);
    assert_eq!(reader.read_for_session(&session).messages.len(), 1);
    assert_eq!(
        reader.processed_bytes,
        first.len() as u64,
        "unchanged source must not be reparsed"
    );
    let answer = lines(&[claude_message(
        "a",
        "assistant",
        json!([{"type":"text","text":"Answer"}]),
    )]);
    let split = answer.len() / 2;
    let mut output = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    output.write_all(&answer[..split]).unwrap();
    assert_eq!(reader.read_for_session(&session).messages.len(), 1);
    assert_eq!(
        reader.processed_bytes,
        first.len() as u64,
        "a partial record is not committed"
    );
    output.write_all(&answer[split..]).unwrap();
    assert_eq!(reader.read_for_session(&session).messages.len(), 2);
    assert_eq!(
        reader.processed_bytes,
        (first.len() + answer.len()) as u64,
        "only new complete record bytes are parsed"
    );
}

#[test]
fn reader_resets_for_truncation_replacement_and_same_length_rewrite() {
    let _env = gwt_core::test_support::env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = tempfile::tempdir().unwrap();
    let _home = gwt_core::test_support::ScopedEnvVar::set("CLAUDE_CONFIG_DIR", home.path());
    let dir = home.path().join("projects/project");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("conversation.jsonl");
    let content = |text: &str| lines(&[claude_message("u", "user", json!(text))]);
    std::fs::write(&path, content("Long original message")).unwrap();
    let mut session = Session::new(Path::new("/work/project"), "pm", AgentId::ClaudeCode);
    session.agent_session_id = Some("conversation".to_owned());
    let mut reader = PmConversationReader::default();
    assert_eq!(
        reader.read_for_session(&session).messages[0].text,
        "Long original message"
    );
    std::fs::write(&path, content("A")).unwrap();
    assert_eq!(reader.read_for_session(&session).messages[0].text, "A");
    let replacement = dir.join("replacement");
    std::fs::write(&replacement, content("B")).unwrap();
    std::fs::rename(&replacement, &path).unwrap();
    assert_eq!(reader.read_for_session(&session).messages[0].text, "B");
    let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
    std::fs::write(&path, content("C")).unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(modified + std::time::Duration::from_secs(1))
        .unwrap();
    assert_eq!(reader.read_for_session(&session).messages[0].text, "C");
}

/// Coarse filesystem clocks (e.g. a 15.6 ms Windows tick) can give a
/// same-length replacement the exact length and timestamps of the file it
/// replaces; only the file identity tells the generations apart.
#[test]
fn reader_resets_for_replacement_with_identical_length_and_timestamps() {
    let _env = gwt_core::test_support::env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = tempfile::tempdir().unwrap();
    let _home = gwt_core::test_support::ScopedEnvVar::set("CLAUDE_CONFIG_DIR", home.path());
    let dir = home.path().join("projects/project");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("conversation.jsonl");
    let content = |text: &str| lines(&[claude_message("u", "user", json!(text))]);
    std::fs::write(&path, content("A")).unwrap();
    let mut session = Session::new(Path::new("/work/project"), "pm", AgentId::ClaudeCode);
    session.agent_session_id = Some("conversation".to_owned());
    let mut reader = PmConversationReader::default();
    assert_eq!(reader.read_for_session(&session).messages[0].text, "A");
    let original = std::fs::metadata(&path).unwrap();
    let replacement = dir.join("replacement");
    std::fs::write(&replacement, content("B")).unwrap();
    let times = std::fs::FileTimes::new().set_modified(original.modified().unwrap());
    #[cfg(windows)]
    let times = std::os::windows::fs::FileTimesExt::set_created(times, original.created().unwrap());
    std::fs::OpenOptions::new()
        .write(true)
        .open(&replacement)
        .unwrap()
        .set_times(times)
        .unwrap();
    std::fs::rename(&replacement, &path).unwrap();
    assert_eq!(reader.read_for_session(&session).messages[0].text, "B");
}

#[test]
fn reader_does_not_reuse_another_identity_home_or_source() {
    let _env = gwt_core::test_support::env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = tempfile::tempdir().unwrap();
    let _home = gwt_core::test_support::ScopedEnvVar::set("CLAUDE_CONFIG_DIR", home.path());
    let dir = home.path().join("projects/project");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("conversation.jsonl");
    std::fs::write(
        &path,
        lines(&[claude_message("u", "user", json!("Original"))]),
    )
    .unwrap();
    let mut session = Session::new(Path::new("/work/project"), "pm", AgentId::ClaudeCode);
    session.agent_session_id = Some("conversation".to_owned());
    let mut reader = PmConversationReader::default();
    assert_eq!(
        reader.read_for_session(&session).messages[0].text,
        "Original"
    );
    let moved = home.path().join("projects/moved");
    std::fs::create_dir_all(&moved).unwrap();
    std::fs::rename(&path, moved.join("conversation.jsonl")).unwrap();
    assert_eq!(
        reader.read_for_session(&session).messages[0].text,
        "Original"
    );
    session.worktree_path = "/foreign".into();
    assert_eq!(
        reader.read_for_session(&session).availability,
        PmConversationAvailability::Unavailable
    );
    session.worktree_path = "/work/project".into();
    assert_eq!(
        reader.read_for_session(&session).messages[0].text,
        "Original"
    );
    session.agent_session_id = Some("replacement".to_owned());
    assert_eq!(
        reader.read_for_session(&session).availability,
        PmConversationAvailability::Waiting
    );
    session.agent_session_id = Some("conversation".to_owned());
    assert_eq!(
        reader.read_for_session(&session).messages[0].text,
        "Original"
    );
    let other_home = tempfile::tempdir().unwrap();
    let _other_home =
        gwt_core::test_support::ScopedEnvVar::set("CLAUDE_CONFIG_DIR", other_home.path());
    let other_dir = other_home.path().join("projects/project");
    std::fs::create_dir_all(&other_dir).unwrap();
    std::fs::write(
        other_dir.join("conversation.jsonl"),
        lines(&[claude_message("u", "user", json!("Other home"))]),
    )
    .unwrap();
    assert_eq!(
        reader.read_for_session(&session).messages[0].text,
        "Other home"
    );
    let codex_dir = other_home.path().join("sessions/2026/10/03");
    std::fs::create_dir_all(&codex_dir).unwrap();
    std::fs::write(codex_dir.join("rollout-conversation.jsonl"), lines(&[
        codex_meta(),
        json!({"type":"response_item","payload":{"type":"message","id":"u","role":"user","content":[{"type":"input_text","text":"Other provider"}]}}),
    ])).unwrap();
    session.agent_id = AgentId::Codex;
    session.codex_auth_root = Some(gwt_agent::CodexAuthRoot {
        path: other_home.path().to_owned(),
        origin: gwt_agent::CodexAuthRootOrigin::CallerEnv,
    });
    assert_eq!(
        reader.read_for_session(&session).messages[0].text,
        "Other provider"
    );
}
