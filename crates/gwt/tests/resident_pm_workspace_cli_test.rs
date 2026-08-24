//! End-to-end coverage for the resident PM's Work updates from its own
//! detached worktree (Issue #3477 / SPEC-3431 FR-032, FR-042).
//!
//! The unit suite in `agent_project_state` pins the authority resolver. This
//! file pins the shape the PM actually runs in: the real `gwtd` binary, an
//! isolated HOME, the canonical `<gwt projects dir>/<hash>/pm/worktree`
//! checked out at a detached HEAD, a `pm.json` registration, and the
//! unassigned agent row the GUI writes at PM launch. It covers the two moments
//! AC-4 names — right after launch and after a crash resume — plus the
//! fail-closed refusal when no registration names the caller.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Stdio,
};

use chrono::Utc;
use gwt_agent::{
    AgentId, Session, GWT_HOOK_FORWARD_TOKEN_ENV, GWT_HOOK_FORWARD_URL_ENV, GWT_SESSION_ID_ENV,
    GWT_SESSION_RUNTIME_PATH_ENV,
};
use gwt_core::process::hidden_command;
use gwt_core::{
    paths::project_scope_hash,
    workspace_projection::{
        load_workspace_projection_from_path, load_workspace_work_items_from_path,
        save_workspace_projection_to_path, WorkspaceAgentAffiliationStatus, WorkspaceAgentSummary,
        WorkspaceProjection, WorkspaceStatusCategory,
    },
};
use serde_json::Value;
use tempfile::TempDir;

/// The literal the PM launch fallback writes into the Session ledger. The PM
/// carries no branch, so `spawn_agent_window` stores this while the worktree
/// itself stays detached — the exact contradiction Issue #3477 resolves.
const PM_LEDGER_BRANCH: &str = "work";

struct PmFixture {
    home: TempDir,
    project: TempDir,
    pm_worktree: PathBuf,
    session_id: String,
}

impl PmFixture {
    fn project_state_dir(&self) -> PathBuf {
        self.home
            .path()
            .join(".gwt/projects")
            .join(project_scope_hash(self.project.path()).as_str())
            .join("project-state")
    }

    fn pm_prefs_path(&self) -> PathBuf {
        self.project_state_dir().join("pm.json")
    }
}

fn git(args: &[&str], cwd: &Path) {
    let status = hidden_command("git")
        .args(args)
        .current_dir(cwd)
        .status()
        .unwrap_or_else(|error| panic!("git {args:?}: {error}"));
    assert!(status.success(), "git {args:?} failed in {}", cwd.display());
}

fn fixture() -> PmFixture {
    let home = tempfile::tempdir().expect("home tempdir");
    let project = tempfile::tempdir().expect("project tempdir");
    let project_path = project.path().to_path_buf();
    git(&["init", "-q", "-b", "develop"], &project_path);
    git(&["config", "user.email", "test@example.com"], &project_path);
    git(&["config", "user.name", "Test User"], &project_path);
    git(
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/example/gwt-resident-pm.git",
        ],
        &project_path,
    );
    git(
        &["commit", "-q", "--allow-empty", "-m", "initial"],
        &project_path,
    );

    // T-016: the PM's dedicated detached worktree, at the canonical path
    // `is_pm_worktree` anchors on.
    let pm_worktree = home
        .path()
        .join(".gwt/projects")
        .join(project_scope_hash(&project_path).as_str())
        .join("pm/worktree");
    fs::create_dir_all(pm_worktree.parent().expect("pm dir")).expect("create pm dir");
    git(
        &[
            "worktree",
            "add",
            "--detach",
            pm_worktree.to_str().expect("pm worktree path"),
            "HEAD",
        ],
        &project_path,
    );
    assert!(
        !hidden_command("git")
            .args(["symbolic-ref", "--quiet", "--short", "HEAD"])
            .current_dir(&pm_worktree)
            .status()
            .expect("symbolic-ref")
            .success(),
        "the PM worktree fixture must have a detached HEAD"
    );

    let mut session = Session::new(&pm_worktree, PM_LEDGER_BRANCH, AgentId::Codex);
    session.id = "resident-pm-cli-session".to_string();
    session.project_state_root = Some(project_path.clone());
    session.status = gwt_agent::AgentStatus::Running;
    session
        .save(&home.path().join(".gwt/sessions"))
        .expect("save PM Session ledger");
    let session_id = session.id.clone();

    let fixture = PmFixture {
        home,
        project,
        pm_worktree,
        session_id,
    };
    write_pm_registration(&fixture, &fixture.session_id);
    seed_unassigned_pm_agent(&fixture);
    fixture
}

fn write_pm_registration(fixture: &PmFixture, session_id: &str) {
    let path = fixture.pm_prefs_path();
    fs::create_dir_all(path.parent().expect("pm prefs dir")).expect("create pm prefs dir");
    let prefs = serde_json::json!({
        "registration": {
            "session_id": session_id,
            "agent_id": "codex",
            "worktree_path": fixture.pm_worktree,
            "consecutive_crashes": 0,
        },
        "settings": {"auto_start": true, "loop_interval_secs": 60},
    });
    fs::write(
        &path,
        serde_json::to_vec_pretty(&prefs).expect("encode pm prefs"),
    )
    .expect("write pm prefs");
}

/// What PM launch leaves behind: the agent is visible but unassigned, and the
/// project has no Work for it yet (`save_start_work_workspace_projection`
/// takes the ownerless path).
fn seed_unassigned_pm_agent(fixture: &PmFixture) {
    let now = Utc::now();
    let mut projection = WorkspaceProjection::default_for_project(fixture.project.path());
    projection.agents = vec![WorkspaceAgentSummary {
        session_id: fixture.session_id.clone(),
        window_id: None,
        agent_id: "codex".to_string(),
        display_name: "Codex".to_string(),
        status_category: WorkspaceStatusCategory::Active,
        current_focus: None,
        title_summary: None,
        worktree_path: Some(fixture.pm_worktree.clone()),
        branch: Some(PM_LEDGER_BRANCH.to_string()),
        last_board_entry_id: None,
        last_board_entry_kind: None,
        coordination_scope: None,
        affiliation_status: WorkspaceAgentAffiliationStatus::Unassigned,
        workspace_id: None,
        updated_at: now,
    }];
    let path = fixture.project_state_dir().join("current.json");
    fs::create_dir_all(path.parent().expect("project state dir")).expect("create project state");
    save_workspace_projection_to_path(&path, &projection).expect("seed PM agent projection");
}

fn gwtd(fixture: &PmFixture, json: &str) -> std::process::Output {
    let mut child = hidden_command(env!("CARGO_BIN_EXE_gwtd"))
        // The PM always runs with its detached worktree as cwd.
        .current_dir(&fixture.pm_worktree)
        .env("HOME", fixture.home.path())
        .env("USERPROFILE", fixture.home.path())
        .env(GWT_SESSION_ID_ENV, &fixture.session_id)
        .env_remove(GWT_HOOK_FORWARD_URL_ENV)
        .env_remove(GWT_HOOK_FORWARD_TOKEN_ENV)
        .env_remove(GWT_SESSION_RUNTIME_PATH_ENV)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn gwtd");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(json.as_bytes())
        .expect("write envelope");
    child.wait_with_output().expect("wait gwtd")
}

fn run_ok(fixture: &PmFixture, json: &str) -> Value {
    let output = gwtd(fixture, json);
    assert!(
        output.status.success(),
        "gwtd should exit 0 for `{json}`\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("parse gwtd response")
}

fn pm_agent(fixture: &PmFixture) -> WorkspaceAgentSummary {
    load_workspace_projection_from_path(&fixture.project_state_dir().join("current.json"))
        .expect("load projection")
        .expect("projection")
        .agents
        .into_iter()
        .find(|agent| agent.session_id == fixture.session_id)
        .expect("resident PM agent row")
}

fn pm_work(fixture: &PmFixture, work_id: &str) -> gwt_core::workspace_projection::WorkItem {
    load_workspace_work_items_from_path(&fixture.project_state_dir().join("works.json"))
        .expect("load WorkItems projection")
        .expect("WorkItems projection")
        .work_items
        .into_iter()
        .find(|item| item.id == work_id)
        .unwrap_or_else(|| panic!("resident PM Work {work_id}"))
}

fn created_work_id(response: &Value) -> String {
    response
        .get("output")
        .and_then(Value::as_str)
        .and_then(|out| out.trim().rsplit_once(' ').map(|(_, id)| id.to_string()))
        .expect("workspace.create must report the Work id")
}

const CREATE_ENVELOPE: &str = r#"{"schema_version":1,"operation":"workspace.create","params":{"agent_session":"resident-pm-cli-session","purpose":"常駐 PM 運用","current_focus":"Issue Monitor の照合"}}"#;

#[test]
fn resident_pm_updates_its_work_from_a_detached_worktree() {
    let fixture = fixture();

    let created = run_ok(&fixture, CREATE_ENVELOPE);
    let work_id = created_work_id(&created);

    // AC-1 / AC-4 (right after launch).
    run_ok(
        &fixture,
        r#"{"schema_version":1,"operation":"workspace.update","params":{"purpose":"常駐 PM 運用","current_focus":"needs_human の裁定","summary":"PM digest","progress_summary":"起動直後の照合を完了"}}"#,
    );

    let agent = pm_agent(&fixture);
    assert_eq!(agent.title_summary.as_deref(), Some("常駐 PM 運用"));
    assert_eq!(agent.current_focus.as_deref(), Some("needs_human の裁定"));
    assert_eq!(agent.workspace_id.as_deref(), Some(work_id.as_str()));

    let work = pm_work(&fixture, &work_id);
    assert_eq!(work.summary.as_deref(), Some("PM digest"));
    assert_eq!(
        work.progress_summary.as_deref(),
        Some("起動直後の照合を完了")
    );
    // FR-042: the resident PM Work stays a non-producing projection.
    assert_eq!(work.owner, None);

    // AC-4 (after a crash resume): the pane died, the registration still names
    // this Session, and the successor keeps writing to the same Work.
    run_ok(
        &fixture,
        r#"{"schema_version":1,"operation":"workspace.update","params":{"purpose":"常駐 PM 運用","progress_summary":"resume 後も継続して照合"}}"#,
    );
    let resumed = pm_work(&fixture, &work_id);
    assert_eq!(
        resumed.progress_summary.as_deref(),
        Some("resume 後も継続して照合")
    );
    assert_eq!(
        pm_agent(&fixture).title_summary.as_deref(),
        Some("常駐 PM 運用")
    );
}

#[test]
fn detached_worktree_update_is_refused_without_a_matching_pm_registration() {
    let fixture = fixture();
    run_ok(&fixture, CREATE_ENVELOPE);

    // AC-3: a registration naming a different Session leaves this caller with
    // no branchless authority at all.
    write_pm_registration(&fixture, "some-other-session");
    let output = gwtd(
        &fixture,
        r#"{"schema_version":1,"operation":"workspace.update","params":{"purpose":"常駐 PM 運用","current_focus":"登録なしでの更新"}}"#,
    );
    assert!(
        !output.status.success(),
        "a foreign PM registration must fail closed"
    );

    // AC-6: the refusal names the detached HEAD, the missing registration, and
    // both recovery routes.
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    for expected in [
        "detached HEAD",
        "resident PM",
        "no PM registration names this Session",
        "Recovery:",
    ] {
        assert!(
            stderr.contains(expected),
            "refusal must mention `{expected}`: {stderr}"
        );
    }
}
