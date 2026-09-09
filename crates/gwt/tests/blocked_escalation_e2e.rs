//! End-to-end coverage for Issue #3655: a blocked agent reaches the PM.
//!
//! Everything here drives the real `gwtd` binary through the stdin JSON
//! envelope — the surface an agent actually uses. That matters more than usual
//! for this Issue, because the defect it fixes was never a wrong function: the
//! escalation logic that mattered simply was not on the path a stuck agent
//! travels. An in-crate unit test would have passed throughout the incident.
//!
//! **No pane is involved anywhere in this file (AC-9).** Every assertion is
//! reachable from files alone, because `pane.read` fails under GUI event-loop
//! saturation (#3629) and was exactly the channel the PM was forced to fall
//! back on.
//!
//! `HOME` / `USERPROFILE` point at an isolated temp home so the Board provider
//! resolves to the filesystem backend regardless of the developer machine's
//! `~/.gwt/config.toml`.

use std::{io::Write, path::Path, process::Stdio};

use gwt_agent::{AgentId, Session};
use gwt_core::{process::hidden_command, test_support::ScopedGwtHome};
use serde_json::Value;
use tempfile::TempDir;

/// The four-section body the escalation contract requires (AC-1).
const ESCALATION_BODY: &str = "事象: execution.reopen が immutable で拒否された\n\
     原因: Completed ECR はこの window では reopen できない\n\
     依頼: fresh launch を手配してほしい\n\
     再開条件: #2338 に紐づいた新しい pane が起動されること";

struct Fixture {
    home: TempDir,
    project: TempDir,
    session_id: String,
    /// The child `gwtd` writes under `HOME=fixture.home`, so the test process
    /// has to look there too. `ScopedGwtHome` is thread-local, which is what
    /// keeps these tests from writing into the developer's real `~/.gwt` while
    /// the rest of the suite runs in parallel around them.
    _scoped_home: ScopedGwtHome,
}

fn git_init_with_origin(path: &Path) {
    assert!(hidden_command("git")
        .arg("init")
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
            "https://github.com/example/gwt-blocked-escalation.git",
        ])
        .status()
        .expect("git remote add")
        .success());
}

fn fixture() -> Fixture {
    let home = tempfile::tempdir().expect("home tempdir");
    let project = tempfile::tempdir().expect("project tempdir");
    git_init_with_origin(project.path());
    let mut session = Session::new(project.path(), "work/issue-2338", AgentId::ClaudeCode);
    session.linked_issue_number = Some(2338);
    let session_id = session.id.clone();
    session
        .save(&home.path().join(".gwt").join("sessions"))
        .expect("save session");
    let _scoped_home = ScopedGwtHome::set(home.path());
    Fixture {
        home,
        project,
        session_id,
        _scoped_home,
    }
}

fn run(fixture: &Fixture, json: &str) -> Value {
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
    serde_json::from_slice(&output.stdout).unwrap_or_else(|err| {
        panic!(
            "parse gwtd JSON response: {err}; stdout={}; stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn output_text(value: &Value) -> String {
    value
        .get("output")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn needs_human(fixture: &Fixture) -> Vec<u64> {
    let status = run(
        fixture,
        r#"{"schema_version":1,"operation":"issue.monitor.status","params":{}}"#,
    );
    let snapshot: Value = serde_json::from_str(output_text(&status).trim())
        .unwrap_or_else(|err| panic!("parse issue.monitor.status payload: {err}; got {status}"));
    snapshot["needs_human"]
        .as_array()
        .map(|values| values.iter().filter_map(Value::as_u64).collect())
        .unwrap_or_default()
}

fn open_escalation_ids(fixture: &Fixture) -> Vec<String> {
    gwt_core::coordination::load_open_escalations(fixture.project.path())
        .expect("read the escalation index")
        .into_iter()
        .map(|escalation| escalation.entry_id)
        .collect()
}

/// AC-1 + AC-4: an agent that says it is blocked becomes findable in the one
/// field a PM reads, without anybody reading its pane.
#[test]
fn a_blocked_post_reaches_needs_human_and_leaves_when_resolved() {
    let fixture = fixture();
    assert!(needs_human(&fixture).is_empty(), "clean baseline");

    let posted = run(
        &fixture,
        &serde_json::json!({
            "schema_version": 1,
            "operation": "board.post",
            "params": {"kind": "blocked", "owners": ["2338"], "body": ESCALATION_BODY},
        })
        .to_string(),
    );
    assert_eq!(posted["ok"], Value::Bool(true), "{posted}");

    assert_eq!(
        needs_human(&fixture),
        vec![2338],
        "AC-4: an open unblock request must surface as needs_human"
    );

    let entry_id = open_escalation_ids(&fixture).remove(0);
    let resolved = run(
        &fixture,
        &serde_json::json!({
            "schema_version": 1,
            "operation": "board.post",
            "params": {
                "kind": "decision",
                "owners": ["2338"],
                "resolves": entry_id,
                "body": "現在の状態: fresh launch を手配したので unblock 済みです。",
            },
        })
        .to_string(),
    );
    assert_eq!(resolved["ok"], Value::Bool(true), "{resolved}");
    assert!(
        needs_human(&fixture).is_empty(),
        "AC-4: resolving the escalation must clear the row"
    );
}

/// AC-1: an escalation nobody can act on is refused at the posting surface.
#[test]
fn an_escalation_without_the_four_required_sections_is_refused() {
    let fixture = fixture();
    let posted = run(
        &fixture,
        r#"{"schema_version":1,"operation":"board.post","params":{"kind":"blocked","owners":["2338"],"body":"進められません"}}"#,
    );

    assert_eq!(posted["ok"], Value::Bool(false), "{posted}");
    let message = posted["error"].as_str().unwrap_or_default();
    for expected in ["事象", "原因", "依頼", "再開条件"] {
        assert!(message.contains(expected), "{message}");
    }
    assert!(
        needs_human(&fixture).is_empty(),
        "a refused post must not open an escalation"
    );
}

/// AC-2: a governance refusal escalates on its own, with no agent volition.
///
/// `execution.reopen` from a directory that is not the recovery scope is a
/// real refusal that needs no fixture — the same class of refusal that left
/// #2338 stalled for two PM cycles.
#[test]
fn a_refused_operation_escalates_itself_without_the_agent_asking() {
    let fixture = fixture();
    let refused = run(
        &fixture,
        r#"{"schema_version":1,"operation":"execution.reopen","params":{"reason":"blocker resolved"}}"#,
    );
    assert_eq!(
        refused["ok"],
        Value::Bool(false),
        "the fixture must actually be refused, or this test proves nothing: {refused}"
    );

    assert_eq!(
        needs_human(&fixture),
        vec![2338],
        "AC-2: the refusal alone must raise the escalation, on the session's own Issue"
    );
    let body = gwt_core::coordination::load_open_escalations(fixture.project.path())
        .expect("escalations")
        .remove(0)
        .body;
    assert!(
        body.contains("execution.reopen"),
        "the PM must be told which operation refused: {body}"
    );
}

/// AC-1: declaring the block through `execution.blocked` is the same moment as
/// posting one, so it must produce the same escalation.
///
/// The agent in the #2338 incident had already worked out that it needed a
/// fresh launch. Requiring it to *also* remember a second operation is what put
/// that conclusion nowhere the PM could see it.
#[test]
fn declaring_execution_blocked_raises_the_escalation_too() {
    let fixture = fixture();
    let declared = run(
        &fixture,
        r#"{"schema_version":1,"operation":"execution.blocked","params":{"reason":"Completed ECR のためこの window では残 AC を実装できない","missing_verification":"cargo test -p gwt --bin gwt"}}"#,
    );

    assert_eq!(
        needs_human(&fixture),
        vec![2338],
        "AC-1: `execution.blocked` must escalate on its own; got {declared}"
    );
    let body = gwt_core::coordination::load_open_escalations(fixture.project.path())
        .expect("escalations")
        .remove(0)
        .body;
    assert!(
        body.contains("execution.blocked"),
        "the escalation must name how the block was declared: {body}"
    );
    assert!(
        body.contains("Completed ECR のためこの window では残 AC を実装できない"),
        "the agent's own reason is the whole point of the escalation: {body}"
    );
    assert!(
        body.contains("cargo test -p gwt --bin gwt"),
        "the verification it could not run must travel with the ask: {body}"
    );
}

/// AC-2 again: an ordinary failure stays quiet, or the signal is worthless.
#[test]
fn an_ordinary_operation_failure_does_not_escalate() {
    let fixture = fixture();
    run(
        &fixture,
        r#"{"schema_version":1,"operation":"issue.view","params":{"number":99999999}}"#,
    );

    assert!(
        needs_human(&fixture).is_empty(),
        "a failure the agent can fix by calling differently is not a PM escalation"
    );
}

/// AC-3: the Stop-gate summary must neither replace nor bury the escalation.
///
/// This is the exact production sequence: the agent posts that it cannot
/// proceed, then the Stop gate fires and used to append "ready for the next
/// instruction" — a `status` post, which moved the Work back to Active and
/// read, to the PM, as an agent waiting for its next task.
#[test]
fn the_stop_gate_summary_does_not_overwrite_an_open_escalation() {
    let fixture = fixture();
    run(
        &fixture,
        &serde_json::json!({
            "schema_version": 1,
            "operation": "board.post",
            "params": {"kind": "blocked", "owners": ["2338"], "body": ESCALATION_BODY},
        })
        .to_string(),
    );

    let mut child = hidden_command(env!("CARGO_BIN_EXE_gwtd"))
        .args(["hook", "event", "Stop"])
        .current_dir(fixture.project.path())
        .env("HOME", fixture.home.path())
        .env("USERPROFILE", fixture.home.path())
        .env("GWT_SESSION_ID", &fixture.session_id)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run the Stop hook");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(b"{}")
        .expect("write stdin");
    child.wait_with_output().expect("wait for the Stop hook");

    let snapshot = gwt_core::coordination::load_snapshot(fixture.project.path()).expect("board");
    assert!(
        !snapshot
            .board
            .entries
            .iter()
            .any(|entry| entry.body.contains("ready for the next instruction")),
        "AC-3: the routine notice must not stand on top of an open escalation: {:?}",
        snapshot
            .board
            .entries
            .iter()
            .map(|entry| entry.body.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        needs_human(&fixture),
        vec![2338],
        "AC-3: and the escalation must still be standing afterwards"
    );
}

/// AC-4 + AC-9: the escalation stays answerable long after the post has
/// scrolled out of the Board's hot projection — the condition under which the
/// PM went blind in production, on a Board carrying thousands of entries.
#[test]
fn the_escalation_outlives_the_board_projection_window() {
    let fixture = fixture();
    run(
        &fixture,
        &serde_json::json!({
            "schema_version": 1,
            "operation": "board.post",
            "params": {"kind": "blocked", "owners": ["2338"], "body": ESCALATION_BODY},
        })
        .to_string(),
    );

    // Push the escalation out of the hot projection with ordinary chatter.
    let entry = gwt_core::coordination::BoardEntry::new(
        gwt_core::coordination::AuthorKind::Agent,
        "Codex",
        gwt_core::coordination::BoardEntryKind::Status,
        "noise",
        None,
        None,
        vec![],
        vec![],
    );
    for _ in 0..gwt_core::coordination::HOT_PROJECTION_ENTRY_LIMIT {
        let mut noise = entry.clone();
        noise.id = uuid::Uuid::new_v4().to_string();
        noise.created_at = chrono::Utc::now();
        noise.updated_at = noise.created_at;
        gwt_core::coordination::post_entry(fixture.project.path(), noise).expect("post noise");
    }

    let snapshot = gwt_core::coordination::load_snapshot(fixture.project.path()).expect("board");
    assert!(
        !snapshot
            .board
            .entries
            .iter()
            .any(|entry| entry.kind == gwt_core::coordination::BoardEntryKind::Blocked),
        "the escalation must have scrolled out for this test to mean anything"
    );
    assert_eq!(
        needs_human(&fixture),
        vec![2338],
        "AC-9: the PM-facing answer must not depend on the Board timeline window"
    );

    // Issue #3690: the same overflowed handle must still close through
    // params.resolves — the surface the PM actually uses.
    let entry_id = open_escalation_ids(&fixture).remove(0);
    let resolved = run(
        &fixture,
        &serde_json::json!({
            "schema_version": 1,
            "operation": "board.post",
            "params": {
                "kind": "decision",
                "owners": ["2338"],
                "resolves": entry_id,
                "body": "現在の状態: fresh launch を手配したので unblock 済みです。",
            },
        })
        .to_string(),
    );
    assert_eq!(resolved["ok"], Value::Bool(true), "{resolved}");
    let output = output_text(&resolved);
    assert!(
        output.contains(&format!("board escalations resolved: {entry_id}")),
        "overflowed ids must close, not be reported as unknown: {output}"
    );
    assert!(
        !output.contains("not found"),
        "an overflowed durable id is not missing: {output}"
    );
    assert!(
        needs_human(&fixture).is_empty(),
        "closing the overflowed escalation must clear needs_human"
    );
    let store = gwt_core::coordination::load_escalation_store(fixture.project.path())
        .expect("escalation index");
    let closed = store
        .escalations
        .iter()
        .find(|escalation| escalation.entry_id == entry_id)
        .expect("the overflowed row remains in the index");
    assert!(closed.resolved_at.is_some());
    assert!(closed.resolved_by_entry_id.is_some());
}
