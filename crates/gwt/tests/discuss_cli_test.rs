//! Integration coverage for the discuss exit-CLI gwtd JSON operations
//! (`discuss.park` / `discuss.reject` / `discuss.clear_next_question`).
//! SPEC-1935 FR-014p (gwt-discussion Stop-block contract).
//!
//! Audit gap (#3143): the discuss state transitions had no end-to-end test.
//! These drive the real `gwtd` binary through the stdin JSON envelope against a
//! legacy repo-local `.gwt/work/discussions.md` fixture and assert that the
//! `[active]` → `[parked]` / `[rejected]` rewrite and the Next Question clear
//! land in the machine-local home work-notes file (SPEC-3214 FR-007) after the
//! one-time import, leaving the git-tracked repo-local source untouched.

use std::{fs, io::Write, path::Path, process::Stdio};

use gwt_core::process::hidden_command;
use serde_json::Value;
use tempfile::TempDir;

const ACTIVE_DISCUSSION: &str = "\
## Discussion TODO
Status: active
Origin Session: session-current

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

fn repo_local_discussions_path(fixture: &Fixture) -> std::path::PathBuf {
    fixture.project.path().join(".gwt/work/discussions.md")
}

/// SPEC-3214 (FR-007): mutations land in the home work-notes file
/// (`<home>/.gwt/projects/<repo-hash>/work-notes/discussions.md`). The repo
/// hash is computed by gwtd from its cwd, so discover the file by walking.
fn discussions_path(fixture: &Fixture) -> std::path::PathBuf {
    let projects = fixture.home.path().join(".gwt").join("projects");
    let mut found = Vec::new();
    if let Ok(entries) = fs::read_dir(&projects) {
        for entry in entries.filter_map(Result::ok) {
            let candidate = entry.path().join("work-notes").join("discussions.md");
            if candidate.is_file() {
                found.push(candidate);
            }
        }
    }
    assert!(
        found.len() == 1,
        "expected exactly one home work-notes discussions file, got {found:?}"
    );
    found.pop().expect("home discussions file")
}

fn read_discussions(fixture: &Fixture) -> String {
    fs::read_to_string(discussions_path(fixture)).expect("read discussions.md")
}

fn invoke_discuss(fixture: &Fixture, json: &str) -> Value {
    let mut child = hidden_command(env!("CARGO_BIN_EXE_gwtd"))
        .current_dir(fixture.project.path())
        .env("HOME", fixture.home.path())
        .env("USERPROFILE", fixture.home.path())
        .env("GWT_SESSION_ID", "session-current")
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
            "parse gwtd JSON response: {err}; stdout={}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

fn run_discuss(fixture: &Fixture, json: &str) -> Value {
    let response = invoke_discuss(fixture, json);
    assert_ok(&response, json);
    response
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
    // The git-tracked repo-local source stays untouched (read fallback only).
    assert_eq!(
        fs::read_to_string(repo_local_discussions_path(&fixture)).expect("repo-local"),
        ACTIVE_DISCUSSION,
        "repo-local discussions.md must not receive mutations"
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

#[test]
fn discuss_status_refuses_same_label_from_two_sessions_without_importing() {
    for operation in ["discuss.park", "discuss.reject", "discuss.resolve"] {
        let fixture = fixture();
        let source = "## First\nStatus: active\nOrigin Session: session-other\n\n### Proposal A - Foreign proposal [active]\n\n## Second\nStatus: active\nOrigin Session: session-current\n\n### Proposal A - Own proposal [active]\n";
        fs::write(repo_local_discussions_path(&fixture), source).unwrap();
        let response = invoke_discuss(
            &fixture,
            &serde_json::json!({
                "schema_version": 1, "operation": operation,
                "params": {"proposal": "Proposal A"}
            })
            .to_string(),
        );
        assert_eq!(response["ok"], false, "{response}");
        let output = response.to_string();
        for candidate in [
            "ambiguous",
            "Foreign proposal",
            "Own proposal",
            "session-other",
            "session-current",
        ] {
            assert!(output.contains(candidate), "missing {candidate}: {output}");
        }
        assert_eq!(
            fs::read_to_string(repo_local_discussions_path(&fixture)).unwrap(),
            source
        );
        assert!(
            !fixture
                .home
                .path()
                .join(".gwt/projects")
                .read_dir()
                .unwrap()
                .any(|entry| {
                    entry
                        .unwrap()
                        .path()
                        .join("work-notes/discussions.md")
                        .exists()
                }),
            "refusal must not import the discussion document"
        );
    }
}

#[test]
fn discuss_status_refuses_foreign_origin_and_requires_explicit_target() {
    for operation in ["discuss.park", "discuss.reject", "discuss.resolve"] {
        let fixture = fixture();
        let source = "## Historical\nStatus: active\nOrigin Session: session-other\n\n### Proposal A - Foreign proposal [active]\n";
        fs::write(repo_local_discussions_path(&fixture), source).unwrap();
        let response = invoke_discuss(
            &fixture,
            &serde_json::json!({
                "schema_version": 1, "operation": operation,
                "params": {"proposal": "Proposal A"}
            })
            .to_string(),
        );
        assert_eq!(response["ok"], false, "{response}");
        let output = response.to_string();
        assert!(
            output.contains("session-other")
                && output.contains("title")
                && output.contains("origin_session"),
            "{output}"
        );
        assert_eq!(
            fs::read_to_string(repo_local_discussions_path(&fixture)).unwrap(),
            source
        );
    }
}

const COMPLETE_EVIDENCE: &str = "- Implementation Proof: discussion implementation inspected\n- SPEC/Issue Proof: Issue 4956 checked\n- Gap Check Proof: dependencies and migration checked\n- Official Docs Proof: not-applicable: local behavior\n- External Research Proof: not-applicable: local behavior\n- Exit Blockers: none\n- Depth Mode: normal\n- Question Ledger: scope and verification checked\n- Depth Gate: complete\n- Evidence Gate: complete\n";

#[test]
fn discuss_status_success_identifies_owned_proposal() {
    for operation in ["discuss.resolve", "discuss.park", "discuss.reject"] {
        let fixture = fixture();
        let source = format!("## Own\nStatus: active\nOrigin Session: session-current\n\n### Proposal A - Own proposal [active]\n{COMPLETE_EVIDENCE}");
        fs::write(repo_local_discussions_path(&fixture), source).unwrap();
        let response = run_discuss(
            &fixture,
            &serde_json::json!({
                "schema_version": 1, "operation": operation,
                "params": {"proposal": "Proposal A"}
            })
            .to_string(),
        );
        let output = response["output"].as_str().unwrap();
        assert!(
            output.contains("Own proposal") && output.contains("Origin Session: session-current"),
            "{output}"
        );
    }
}

#[test]
fn discuss_status_explicit_history_uses_exact_title_and_origin() {
    let fixture = fixture();
    let source = format!("## Own\nStatus: active\nOrigin Session: session-current\n\n### Proposal A - Own proposal [active]\n\n## Historical\nStatus: active\nOrigin Session: session-other\n\n### Proposal A - Historical proposal [active]\n{COMPLETE_EVIDENCE}");
    fs::write(repo_local_discussions_path(&fixture), source).unwrap();
    let response = run_discuss(
        &fixture,
        r#"{"schema_version":1,"operation":"discuss.resolve","params":{"proposal":"Proposal A","title":"Historical proposal","origin_session":"session-other"}}"#,
    );
    let content = read_discussions(&fixture);
    assert!(content.contains("Own proposal [active]"));
    assert!(content.contains("Historical proposal [chosen]"));
    assert!(response["output"]
        .as_str()
        .unwrap()
        .contains("Origin Session: session-other"));
}

#[test]
fn discuss_status_unattributed_history_requires_title() {
    let fixture = fixture();
    fs::write(
        repo_local_discussions_path(&fixture),
        ACTIVE_DISCUSSION.replace("Origin Session: session-current\n", ""),
    )
    .unwrap();
    let refused = invoke_discuss(
        &fixture,
        r#"{"schema_version":1,"operation":"discuss.park","params":{"proposal":"Proposal A"}}"#,
    );
    assert_eq!(refused["ok"], false, "{refused}");
    let response = run_discuss(
        &fixture,
        r#"{"schema_version":1,"operation":"discuss.park","params":{"proposal":"Proposal A","title":"Integration coverage proposal"}}"#,
    );
    assert!(response["output"]
        .as_str()
        .unwrap()
        .contains("Origin Session: unknown"));
}
