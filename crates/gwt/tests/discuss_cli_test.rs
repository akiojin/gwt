//! Integration coverage for the discuss exit-CLI gwtd JSON operations
//! (`discuss.park` / `discuss.reject` / `discuss.clear_next_question`).
//! SPEC-1935 FR-014p (gwt-discussion Stop-block contract).
//!
//! Audit gap (#3143): the discuss state transitions had no end-to-end test.
//! These drive the real `gwtd` binary through the stdin JSON envelope against a
//! canonical `.gwt/work/discussions.md` fixture and assert the on-disk
//! `[active]` → `[parked]` / `[rejected]` rewrite and the Next Question clear.

use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

use serde_json::Value;
use tempfile::TempDir;

const ACTIVE_DISCUSSION: &str = "\
## Discussion TODO

### Proposal A - Integration coverage proposal [active]
- Summary: exercising the discuss gwtd operations end-to-end
- Next Question: What should the next coverage step be?
- Evidence Gate: complete
";

struct Fixture {
    home: TempDir,
    project: TempDir,
}

fn fixture() -> Fixture {
    let home = tempfile::tempdir().expect("home tempdir");
    let project = tempfile::tempdir().expect("project tempdir");
    let discussions = project.path().join(".gwt/work/discussions.md");
    fs::create_dir_all(discussions.parent().expect("parent")).expect("create .gwt/work");
    fs::write(&discussions, ACTIVE_DISCUSSION).expect("write discussions.md");
    Fixture { home, project }
}

fn discussions_path(fixture: &Fixture) -> std::path::PathBuf {
    fixture.project.path().join(".gwt/work/discussions.md")
}

fn read_discussions(fixture: &Fixture) -> String {
    fs::read_to_string(discussions_path(fixture)).expect("read discussions.md")
}

fn run_discuss(fixture: &Fixture, json: &str) -> Value {
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

fn header_status_line(content: &str) -> &str {
    content
        .lines()
        .find(|line| line.trim_start().starts_with("### Proposal "))
        .unwrap_or("")
}

fn assert_marker(line: &str, marker: &str, path: &Path) {
    assert!(
        line.contains(marker),
        "proposal header in {} should contain {marker}, got: {line}",
        path.display()
    );
}

#[test]
fn discuss_park_marks_proposal_parked() {
    let fixture = fixture();
    let response = run_discuss(
        &fixture,
        r#"{"schema_version":1,"operation":"discuss.park","params":{"proposal":"Proposal A"}}"#,
    );
    assert_ok(&response, "discuss.park");

    let content = read_discussions(&fixture);
    let header = header_status_line(&content);
    assert_marker(header, "[parked]", &discussions_path(&fixture));
    assert!(
        !header.contains("[active]"),
        "parked proposal must no longer be [active], got: {header}"
    );
}

#[test]
fn discuss_reject_marks_proposal_rejected() {
    let fixture = fixture();
    let response = run_discuss(
        &fixture,
        r#"{"schema_version":1,"operation":"discuss.reject","params":{"proposal":"Proposal A"}}"#,
    );
    assert_ok(&response, "discuss.reject");

    let content = read_discussions(&fixture);
    let header = header_status_line(&content);
    assert_marker(header, "[rejected]", &discussions_path(&fixture));
    assert!(
        !header.contains("[active]"),
        "rejected proposal must no longer be [active], got: {header}"
    );
}

#[test]
fn discuss_clear_next_question_empties_the_field() {
    let fixture = fixture();
    let response = run_discuss(
        &fixture,
        r#"{"schema_version":1,"operation":"discuss.clear_next_question","params":{"proposal":"Proposal A"}}"#,
    );
    assert_ok(&response, "discuss.clear_next_question");

    let content = read_discussions(&fixture);
    assert!(
        !content.contains("What should the next coverage step be?"),
        "the Next Question value must be cleared, got: {content}"
    );
    // The proposal stays active; only the Next Question line is emptied.
    assert!(
        header_status_line(&content).contains("[active]"),
        "clear_next_question must not change the proposal status, got: {content}"
    );
}
