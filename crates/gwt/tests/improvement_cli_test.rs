use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use serde_json::{json, Value};
use tempfile::TempDir;

struct Fixture {
    home: TempDir,
    project: TempDir,
}

fn fixture() -> Fixture {
    Fixture {
        home: tempfile::tempdir().expect("home"),
        project: tempfile::tempdir().expect("project"),
    }
}

fn run_gwtd_json(fixture: &Fixture, payload: Value) -> Value {
    let output = run_gwtd_json_raw(fixture, payload);
    assert!(
        output.status.success(),
        "gwtd should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|err| {
        panic!(
            "parse gwtd response: {err}; stdout={}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

fn run_gwtd_json_raw(fixture: &Fixture, payload: Value) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_gwtd"))
        .current_dir(fixture.project.path())
        .env("HOME", fixture.home.path())
        .env("USERPROFILE", fixture.home.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run gwtd");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(payload.to_string().as_bytes())
        .expect("write JSON");
    child.wait_with_output().expect("wait gwtd")
}

fn operation_output(response: &Value) -> Value {
    assert_eq!(
        response.get("ok").and_then(Value::as_bool),
        Some(true),
        "operation should succeed: {response}"
    );
    let output = response
        .get("output")
        .and_then(Value::as_str)
        .expect("output string");
    serde_json::from_str(output.trim())
        .unwrap_or_else(|err| panic!("operation output must be JSON: {err}; output={output}"))
}

fn capture_payload(dedupe_key: &str, summary: &str) -> Value {
    json!({
        "schema_version": 1,
        "operation": "improvement.capture",
        "params": {
            "source": "agent-failure",
            "target_artifact": "skill",
            "classification": "gwt-caused",
            "confidence": "high",
            "summary": summary,
            "details": "Codex followed stale instructions from /Users/alice/private-repo/AGENTS.md with token ghp_1234567890abcdef.",
            "evidence_digest": "Stop hook allowed completion without skill update evidence.",
            "dedupe_key": dedupe_key,
            "local_evidence": [
                {
                    "kind": "transcript",
                    "path": "/Users/alice/private-repo/.gwt/transcript.jsonl"
                }
            ]
        }
    })
}

fn candidate_store(project: &Path) -> PathBuf {
    project
        .join(".gwt")
        .join("improvements")
        .join("candidates.json")
}

#[test]
fn improvement_capture_sanitizes_and_persists_pending_candidate() {
    let fixture = fixture();
    let response = run_gwtd_json(
        &fixture,
        capture_payload(
            "skill:gwt-discussion:stale-instruction",
            "Skill update missing for /Users/alice/private-repo failure",
        ),
    );
    let body = operation_output(&response);
    assert_eq!(body["state"], "pending");
    assert_eq!(body["occurrences"], 1);
    assert!(body["id"].as_str().unwrap_or_default().starts_with("impr-"));

    let stored =
        fs::read_to_string(candidate_store(fixture.project.path())).expect("candidate store");
    assert!(
        !stored.contains("/Users/alice"),
        "public candidate store fields must not contain absolute private paths: {stored}"
    );
    assert!(
        !stored.contains("ghp_1234567890abcdef"),
        "candidate store must redact token-like secrets: {stored}"
    );
    assert!(
        stored.contains("[redacted-path]"),
        "redacted path marker should be visible: {stored}"
    );
}

#[test]
fn improvement_capture_rejects_invalid_enum_value() {
    let fixture = fixture();
    let output = run_gwtd_json_raw(
        &fixture,
        json!({
            "schema_version": 1,
            "operation": "improvement.capture",
            "params": {
                "source": "agent-failure",
                "target_artifact": "skill",
                "classification": "bad",
                "confidence": "high",
                "summary": "bad enum"
            }
        }),
    );
    assert!(
        !output.status.success(),
        "invalid enum should fail, stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("invalid value for classification: bad"),
        "unexpected stderr: {stderr}"
    );
    assert!(
        !candidate_store(fixture.project.path()).exists(),
        "invalid capture should not create a candidate store"
    );
}

#[test]
fn improvement_capture_dedupes_and_list_returns_single_updated_candidate() {
    let fixture = fixture();
    let first = operation_output(&run_gwtd_json(
        &fixture,
        capture_payload("coordination:title-summary-drift", "Title summary drift"),
    ));
    let second = operation_output(&run_gwtd_json(
        &fixture,
        capture_payload(
            "coordination:title-summary-drift",
            "Title summary drift again",
        ),
    ));
    assert_eq!(
        first["id"], second["id"],
        "dedupe should reuse candidate id"
    );
    assert_eq!(second["occurrences"], 2);

    let list = operation_output(&run_gwtd_json(
        &fixture,
        json!({
            "schema_version": 1,
            "operation": "improvement.list",
            "params": {"state": "pending"}
        }),
    ));
    let candidates = list["candidates"].as_array().expect("candidates");
    assert_eq!(candidates.len(), 1);
    assert_eq!(
        candidates[0]["dedupe_key"],
        "coordination:title-summary-drift"
    );
    assert_eq!(candidates[0]["occurrences"], 2);
}

#[test]
fn improvement_dismiss_and_link_issue_update_lifecycle() {
    let fixture = fixture();
    let captured = operation_output(&run_gwtd_json(
        &fixture,
        capture_payload(
            "verification:user-skip-regression",
            "Verification skip regression",
        ),
    ));
    let id = captured["id"].as_str().expect("id");

    let linked = operation_output(&run_gwtd_json(
        &fixture,
        json!({
            "schema_version": 1,
            "operation": "improvement.link_issue",
            "params": {
                "id": id,
                "number": 3164,
                "url": "https://github.com/akiojin/gwt/issues/3164",
                "repository": "akiojin/gwt"
            }
        }),
    ));
    assert_eq!(linked["state"], "linked");
    assert_eq!(linked["linked_issue"]["number"], 3164);

    let dismissed = operation_output(&run_gwtd_json(
        &fixture,
        json!({
            "schema_version": 1,
            "operation": "improvement.dismiss",
            "params": {
                "id": id,
                "reason": "covered by existing SPEC"
            }
        }),
    ));
    assert_eq!(dismissed["state"], "dismissed");
}
