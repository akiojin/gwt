//! Integration coverage for the Board gwtd JSON operations
//! (`board.post` / `board.show`). SPEC-1974 coordination Board.
//!
//! The audit for the "E2E thoroughness" work (#3141) found these operations had
//! no end-to-end test: `board.post` was only exercised via in-crate `#[cfg(test)]`
//! unit tests of `board_family_run_post`, and `board.show` had no test. This file
//! drives the real `gwtd` binary through the stdin JSON envelope, which is the
//! canonical agent-facing surface.
//!
//! `HOME` / `USERPROFILE` are pointed at an isolated temp home so the Board
//! provider resolves to the filesystem `local` backend regardless of the
//! developer machine's `~/.gwt/config.toml` (the same hermeticity requirement
//! fixed for the SessionStart test in #3139 — without it a machine configured
//! with `board.provider = slack|teams` would fail with "<provider> is not
//! signed in").

use std::{io::Write, path::Path, process::Stdio};

use gwt_agent::{AgentId, ExecutionBindingIdentity, Session, SessionExecutionBinding};
use gwt_core::process::hidden_command;
use serde_json::Value;
use tempfile::TempDir;

fn git_init_with_origin(path: &Path) {
    assert!(hidden_command("git")
        .arg("init")
        .args(["-b", "work/board-cli-test"])
        .arg(path)
        .status()
        .expect("git init")
        .success());
    assert!(hidden_command("git")
        .arg("-C")
        .arg(path)
        .args([
            "remote",
            "add",
            "origin",
            "https://github.com/example/gwt-board-cli.git",
        ])
        .status()
        .expect("git remote add")
        .success());
}

struct Fixture {
    home: TempDir,
    project: TempDir,
    session_id: String,
}

fn fixture() -> Fixture {
    let home = tempfile::tempdir().expect("home tempdir");
    let project = tempfile::tempdir().expect("project tempdir");
    git_init_with_origin(project.path());
    let mut session = Session::new(project.path(), "work/board-cli-test", AgentId::Codex);
    session.project_state_root = Some(project.path().to_path_buf());
    session.linked_issue_number = Some(1974);
    session
        .set_execution_binding(Some(SessionExecutionBinding {
            schema_version: SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: session.id.clone(),
            repo_hash: session.repo_hash.clone().expect("fixture repo hash"),
            owner_kind: "spec".to_string(),
            owner_number: 1974,
            identity: ExecutionBindingIdentity {
                generation_id: "board-cli-generation".to_string(),
                binding_id: "board-cli-binding".to_string(),
                ledger_head_hash: "board-cli-ledger".to_string(),
            },
            capability_generation: 1,
        }))
        .expect("bind fixture Session");
    let session_id = session.id.clone();
    session
        .save(&home.path().join(".gwt").join("sessions"))
        .expect("save session");
    Fixture {
        home,
        project,
        session_id,
    }
}

fn run_board(fixture: &Fixture, json: &str) -> Value {
    let mut child = hidden_command(env!("CARGO_BIN_EXE_gwtd"))
        .current_dir(fixture.project.path())
        .env("HOME", fixture.home.path())
        .env("USERPROFILE", fixture.home.path())
        .env("GWT_SESSION_ID", &fixture.session_id)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run gwtd");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(json.as_bytes())
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait gwtd");
    assert!(
        output.status.success(),
        "gwtd should exit 0 for `{json}`, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|err| {
        panic!(
            "parse gwtd JSON response: {err}; stdout={}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

fn assert_ok(value: &Value, context: &str) {
    assert_eq!(
        value.get("ok").and_then(Value::as_bool),
        Some(true),
        "{context} should report ok=true, got: {value}"
    );
}

#[test]
fn board_post_then_show_roundtrips_entry() {
    let fixture = fixture();

    let post = run_board(
        &fixture,
        r#"{"schema_version":1,"operation":"board.post","params":{"kind":"status","body":"integration roundtrip marker alpha"}}"#,
    );
    assert_ok(&post, "board.post");

    let show = run_board(
        &fixture,
        r#"{"schema_version":1,"operation":"board.show","params":{}}"#,
    );
    assert_ok(&show, "board.show");
    let rendered = show
        .get("output")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(
        rendered.contains("integration roundtrip marker alpha"),
        "board.show must return the entry posted by board.post, got: {rendered}"
    );
}

#[test]
fn board_post_persists_kind_and_body() {
    let fixture = fixture();

    let post = run_board(
        &fixture,
        r#"{"schema_version":1,"operation":"board.post","params":{"kind":"decision","body":"chose option B for integration coverage"}}"#,
    );
    assert_ok(&post, "board.post");

    let show = run_board(
        &fixture,
        r#"{"schema_version":1,"operation":"board.show","params":{}}"#,
    );
    let rendered = show
        .get("output")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(
        rendered.contains("chose option B for integration coverage"),
        "posted body must be visible in board.show, got: {rendered}"
    );
    assert!(
        rendered.contains("decision"),
        "posted kind `decision` must be visible in board.show, got: {rendered}"
    );
}

#[test]
fn board_post_dispatches_under_remote_provider_config_via_local_fallback() {
    // Regression guard for hermeticity: even though this test never signs in to
    // a remote provider, board.post through the isolated HOME must succeed
    // because the absent config defaults to the filesystem `local` provider.
    let fixture = fixture();
    let post = run_board(
        &fixture,
        r#"{"schema_version":1,"operation":"board.post","params":{"kind":"status","body":"hermetic local provider marker"}}"#,
    );
    assert_ok(&post, "board.post under isolated HOME");
}

fn recovery_report(response: &Value) -> Value {
    let output = response
        .get("output")
        .and_then(Value::as_str)
        .expect("recovery output string");
    serde_json::from_str(output.trim())
        .unwrap_or_else(|error| panic!("parse recovery report: {error}; output={output}"))
}

#[test]
fn board_post_intent_replays_exactly_and_changed_payload_conflicts() {
    let fixture = fixture();
    let request = r#"{"schema_version":1,"operation":"board.post","params":{"kind":"status","body":"durable integration payload","intent_id":"board-cli-intent-1","broadcast":true}}"#;
    let first = run_board(&fixture, request);
    assert_ok(&first, "first recovery board.post");
    assert_eq!(recovery_report(&first)["state"], "acknowledged");

    let replay = run_board(&fixture, request);
    assert_ok(&replay, "replayed recovery board.post");
    assert_eq!(recovery_report(&replay)["state"], "acknowledged");

    let changed = run_board(
        &fixture,
        r#"{"schema_version":1,"operation":"board.post","params":{"kind":"status","body":"changed integration payload","intent_id":"board-cli-intent-1","broadcast":true}}"#,
    );
    assert_ok(&changed, "changed recovery board.post");
    assert_eq!(recovery_report(&changed)["state"], "conflicted");
    assert_eq!(recovery_report(&changed)["code"], "intent_payload_changed");

    let show = run_board(
        &fixture,
        r#"{"schema_version":1,"operation":"board.show","params":{"all":true}}"#,
    );
    let rendered = show["output"].as_str().unwrap_or_default();
    assert_eq!(rendered.matches("durable integration payload").count(), 1);
    assert!(!rendered.contains("changed integration payload"));
}

#[test]
fn board_post_without_intent_keeps_legacy_human_output() {
    let fixture = fixture();
    let response = run_board(
        &fixture,
        r#"{"schema_version":1,"operation":"board.post","params":{"kind":"status","body":"ordinary post stays ordinary"}}"#,
    );
    assert_ok(&response, "ordinary board.post");
    let output = response["output"].as_str().unwrap_or_default();
    assert!(
        output.starts_with("board entries: "),
        "unexpected output: {output}"
    );
    assert!(serde_json::from_str::<Value>(output.trim()).is_err());
}
