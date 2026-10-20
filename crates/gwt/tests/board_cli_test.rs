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
    let session_id = save_fixture_session(home.path(), project.path());
    Fixture {
        home,
        project,
        session_id,
    }
}

fn save_fixture_session(home: &Path, project: &Path) -> String {
    let mut session = Session::new(project, "work/board-cli-test", AgentId::Codex);
    session.project_state_root = Some(project.to_path_buf());
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
        .save(&home.join(".gwt").join("sessions"))
        .expect("save session");
    session_id
}

fn run_board(fixture: &Fixture, json: &str) -> Value {
    run_board_at(
        fixture.home.path(),
        fixture.project.path(),
        &fixture.session_id,
        json,
    )
}

fn run_board_at(home: &Path, project: &Path, session_id: &str, json: &str) -> Value {
    let mut child = hidden_command(env!("CARGO_BIN_EXE_gwtd"))
        .current_dir(project)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("GWT_SESSION_ID", session_id)
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

#[test]
fn local_board_succeeds_when_discussion_worker_and_origin_recording_fail() {
    use gwt_core::{
        coordination::{AuthorKind, BoardEntry, BoardEntryKind},
        test_support::{env_lock, ScopedEnvVar},
    };
    let _lock = env_lock().lock().unwrap();
    let fixture = fixture();
    let _home = ScopedEnvVar::set("HOME", fixture.home.path());
    let _profile = ScopedEnvVar::set("USERPROFILE", fixture.home.path());
    let unavailable = fixture.home.path().join("unavailable-worker");
    std::fs::write(&unavailable, "not an executable").unwrap();
    let _bin = ScopedEnvVar::set("GWT_BIN_PATH", &unavailable);
    assert!(hidden_command(&unavailable).spawn().is_err());
    let work = fixture.project.path().join(".gwt/work");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(
        work.join("board.toml"),
        "provider = 'local'\n[github_discussion]\nowner = 'example'\nrepo = 'project'\ncategory = 'General'\n",
    )
    .unwrap();
    let coordination = gwt_core::coordination::coordination_dir(fixture.project.path());
    std::fs::create_dir_all(&coordination).unwrap();
    std::fs::write(
        coordination.join("discussion-topic-origins"),
        "blocked directory",
    )
    .unwrap();
    let entry = BoardEntry::new(
        AuthorKind::Agent,
        "agent",
        BoardEntryKind::Decision,
        "local milestone survives",
        None,
        None,
        vec![],
        vec![],
    );
    gwt::board_provider::post_entry(fixture.project.path(), entry.clone()).unwrap();
    assert_eq!(
        gwt::board_provider::load_snapshot(fixture.project.path())
            .unwrap()
            .board
            .entries,
        [entry]
    );
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

/// The same canonical seed is exercised hermetically in CI and, only with an
/// explicit browser-check marker, exported for real headed T461 acceptance.
#[test]
fn recovery_center_live_fixture_has_three_states_and_one_real_board_entry() {
    let fixture = fixture();
    seed_recovery_center(
        fixture.home.path(),
        fixture.project.path(),
        &fixture.session_id,
    );

    if let Some(home) = std::env::var_os("GWT_T461_FIXTURE_HOME") {
        let home = std::fs::canonicalize(home).expect("isolated browser-check HOME");
        assert!(home.starts_with(std::fs::canonicalize(std::env::temp_dir()).unwrap()));
        let marker: Value = serde_json::from_slice(
            &std::fs::read(home.join(".gwt/t461-browser-check.json"))
                .expect("explicit isolation marker"),
        )
        .unwrap();
        let project = std::fs::canonicalize(marker["project_root"].as_str().unwrap()).unwrap();
        assert_eq!(project.parent(), Some(home.as_path()));
        assert!(
            !home.join(".gwt/sessions").exists(),
            "export requires a fresh Session ledger"
        );
        for path in [&home, &home.join(".gwt"), &home.join(".gwt/projects")] {
            assert!(!std::fs::symlink_metadata(path)
                .unwrap()
                .file_type()
                .is_symlink());
        }
        let session_id = save_fixture_session(&home, &project);
        let public = seed_recovery_center(&home, &project, &session_id);
        std::fs::write(
            home.join("t461-public.json"),
            serde_json::to_vec_pretty(&public).unwrap(),
        )
        .unwrap();
    }
}

fn seed_recovery_center(home: &Path, project: &Path, session_id: &str) -> Value {
    use gwt_core::{
        coordination::{AuthorKind, BoardEntry, BoardEntryKind, BoardWorktreeForm},
        recovery::{
            RecoveryConflictKind, RecoveryIntent, RecoveryProvider, RecoveryState, RecoveryStore,
        },
        test_support::ScopedGwtHome,
    };
    let _home = ScopedGwtHome::set(home);
    let store = RecoveryStore::for_repo(project, session_id).unwrap();
    for (name, form) in [
        ("pending", BoardWorktreeForm::Ephemeral),
        ("conflicted", BoardWorktreeForm::Unknown),
    ] {
        let recovery_id = format!("t461-private-{name}");
        let mut entry = BoardEntry::new(
            AuthorKind::Agent,
            "codex",
            BoardEntryKind::Status,
            format!("T461 {name} delivery"),
            None,
            None,
            vec![],
            vec![],
        );
        entry.id = format!("t461-unpublished-{name}");
        entry.origin_session_id = Some(session_id.to_string());
        entry.origin_worktree_form = Some(form);
        entry.origin_recovery_id = Some(recovery_id.clone());
        let record = store
            .prepare(
                &format!("prepare-{name}"),
                RecoveryIntent::new(&recovery_id, RecoveryProvider::Local, form, entry).unwrap(),
            )
            .unwrap()
            .record;
        if name == "conflicted" {
            store
                .mark_conflicted(
                    &recovery_id,
                    record.revision,
                    "conflict",
                    RecoveryConflictKind::PayloadMismatch,
                )
                .unwrap();
        }
    }
    // This invokes the real exact Local append and receipt validation, rather
    // than manufacturing an acknowledgement or a Board article in a fixture.
    let title = "T461 acknowledged Board article";
    let body = "T461 exact Local delivery";
    let response = run_board_at(
        home,
        project,
        session_id,
        &serde_json::json!({
            "schema_version": 1, "operation": "board.post", "params": {
                "kind": "status", "title": title, "body": body,
                "intent_id": "t461-acknowledged", "broadcast": true,
            }
        })
        .to_string(),
    );
    assert_ok(&response, "exact Local delivery");
    let report = recovery_report(&response);
    assert_eq!(report["state"], "acknowledged");
    assert_eq!(report["worktree_form"], "branch-backed");
    let records = store.list_read_only().unwrap();
    assert_eq!(records.len(), 3);
    for state in [
        RecoveryState::Pending,
        RecoveryState::Conflicted,
        RecoveryState::Acknowledged,
    ] {
        assert_eq!(
            records
                .iter()
                .filter(|record| record.state == state)
                .count(),
            1
        );
    }
    let acknowledged = records
        .iter()
        .find(|record| record.state == RecoveryState::Acknowledged)
        .unwrap();
    let entries = gwt::board_provider::load_snapshot(project)
        .unwrap()
        .board
        .entries;
    assert_eq!(
        entries.len(),
        1,
        "unpublished states must not fabricate Board entries"
    );
    assert_eq!(entries[0].id, report["entry_id"].as_str().unwrap());
    assert_eq!(entries[0], acknowledged.intent.entry);
    assert_eq!(entries[0].title.as_deref(), Some(title));
    assert_eq!(entries[0].body, body);
    serde_json::json!({"entry_id": entries[0].id, "title": title, "body": body})
}
