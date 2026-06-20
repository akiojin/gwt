//! Integration coverage for the workspace gwtd JSON operations
//! (`workspace.candidates` / `workspace.create`). SPEC-2359 Workspace /
//! Start Work.
//!
//! Audit gap (#3143): only `workspace.update` had an end-to-end test
//! (`gwtd_cli_test.rs`); candidates / create had none. `workspace.create`
//! resolves the agent from the projection, so the agent is registered first
//! through `workspace.update` (the same upsert path SessionStart uses). All
//! ops run the real `gwtd` binary through the stdin JSON envelope with an
//! isolated HOME.

use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

use gwt_core::{
    paths::project_scope_hash,
    workspace_projection::{load_workspace_projection_from_path, WorkspaceProjection},
};
use serde_json::Value;
use tempfile::TempDir;

const SESSION: &str = "ws-cli-session";

fn git_init_with_origin(path: &Path) {
    assert!(Command::new("git")
        .arg("init")
        .arg(path)
        .status()
        .expect("git init")
        .success());
    assert!(Command::new("git")
        .arg("-C")
        .arg(path)
        .args([
            "remote",
            "add",
            "origin",
            "https://github.com/example/gwt-workspace-cli.git",
        ])
        .status()
        .expect("git remote add")
        .success());
}

struct Fixture {
    home: TempDir,
    project: TempDir,
}

fn fixture() -> Fixture {
    let home = tempfile::tempdir().expect("home tempdir");
    let project = tempfile::tempdir().expect("project tempdir");
    git_init_with_origin(project.path());
    Fixture { home, project }
}

fn run_ws(fixture: &Fixture, json: &str) -> Value {
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

/// Run an op without asserting success — for exercising error/guard paths.
fn run_ws_raw(fixture: &Fixture, json: &str) -> std::process::Output {
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
    child.wait_with_output().expect("wait gwtd")
}

fn load_projection(fixture: &Fixture) -> WorkspaceProjection {
    let path = fixture
        .home
        .path()
        .join(".gwt/projects")
        .join(project_scope_hash(fixture.project.path()).as_str())
        .join("project-state/current.json");
    load_workspace_projection_from_path(&path)
        .expect("load workspace projection")
        .expect("workspace projection should exist under isolated home")
}

/// Register the agent in the projection (the precondition `workspace.create`
/// requires), mirroring the SessionStart upsert path.
fn register_agent(fixture: &Fixture) {
    assert_ok(
        &run_ws(
            fixture,
            &format!(
                r#"{{"schema_version":1,"operation":"workspace.update","params":{{"agent_session":"{SESSION}","purpose":"workspace cli coverage","current_focus":"registering"}}}}"#
            ),
        ),
        "workspace.update (register agent)",
    );
}

#[test]
fn workspace_candidates_reports_without_error() {
    let fixture = fixture();
    let candidates = run_ws(
        &fixture,
        &format!(
            r#"{{"schema_version":1,"operation":"workspace.candidates","params":{{"agent_session":"{SESSION}"}}}}"#
        ),
    );
    assert_ok(&candidates, "workspace.candidates");
    let rendered = candidates
        .get("output")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(
        rendered.contains("candidate"),
        "workspace.candidates output should describe candidates (incl. the `none` case), got: {rendered}"
    );
}

#[test]
fn workspace_create_rejects_duplicate_similar_workspace() {
    // `register_agent` registers the agent with the purpose "workspace cli
    // coverage", which synthesizes an incomplete Work item. `workspace.create`
    // then guards against duplicating a similar Workspace and surfaces an
    // actionable error (SPEC-2359: prefer joining the existing Work over
    // minting a near-duplicate).
    let fixture = fixture();
    register_agent(&fixture);

    let output = run_ws_raw(
        &fixture,
        &format!(
            r#"{{"schema_version":1,"operation":"workspace.create","params":{{"agent_session":"{SESSION}","purpose":"workspace cli coverage"}}}}"#
        ),
    );
    assert!(
        !output.status.success(),
        "workspace.create must reject a near-duplicate Workspace; stdout={}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("similar Workspace exists"),
        "the guard must explain the near-duplicate; stderr={stderr}"
    );

    // The agent and its original Work item remain intact after the rejected create.
    let projection = load_projection(&fixture);
    assert!(
        projection
            .agents
            .iter()
            .any(|agent| agent.session_id == SESSION),
        "the registered agent must remain in the projection after a rejected create"
    );
}

#[test]
fn workspace_update_then_focus_change_persists() {
    let fixture = fixture();
    register_agent(&fixture);

    assert_ok(
        &run_ws(
            &fixture,
            &format!(
                r#"{{"schema_version":1,"operation":"workspace.update","params":{{"agent_session":"{SESSION}","current_focus":"focus after register"}}}}"#
            ),
        ),
        "workspace.update (focus change)",
    );

    let projection = load_projection(&fixture);
    let agent = projection
        .agents
        .iter()
        .find(|agent| agent.session_id == SESSION)
        .expect("registered agent must exist");
    assert_eq!(
        agent.current_focus.as_deref(),
        Some("focus after register"),
        "current_focus must persist across workspace.update calls"
    );
}
