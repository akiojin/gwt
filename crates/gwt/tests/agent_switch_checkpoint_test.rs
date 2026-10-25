//! AC-33: a checkpoint must be clean and present on the live upstream.
use gwt_core::process::{resolved_command, ProcessPlanRequest};
use std::path::Path;

use gwt::issue_monitor::verify_switch_git_checkpoint;

fn git(root: &Path, args: &[&str]) -> String {
    let output = resolved_command(ProcessPlanRequest::new("git"))
        .expect("resolve git")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

#[test]
fn switch_checkpoint_requires_clean_saved_head_on_the_live_remote() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let remote = temp.path().join("remote.git");
    std::fs::create_dir(&source).unwrap();
    git(temp.path(), &["init", "--bare", remote.to_str().unwrap()]);
    git(&source, &["init", "-b", "main"]);
    git(&source, &["config", "user.name", "Switch fixture"]);
    git(&source, &["config", "user.email", "switch@example.invalid"]);
    std::fs::write(source.join("saved.txt"), "checkpoint\n").unwrap();
    git(&source, &["add", "saved.txt"]);
    git(&source, &["commit", "-m", "fixture: saved checkpoint"]);
    git(
        &source,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    git(&source, &["push", "-u", "origin", "main"]);
    let first = git(&source, &["rev-parse", "HEAD"]);
    verify_switch_git_checkpoint(&source, &first).unwrap();

    std::fs::write(source.join("saved.txt"), "unsaved\n").unwrap();
    assert!(verify_switch_git_checkpoint(&source, &first)
        .unwrap_err()
        .to_string()
        .contains("clean"));
    git(&source, &["restore", "saved.txt"]);
    std::fs::write(source.join("new.txt"), "untracked\n").unwrap();
    assert!(verify_switch_git_checkpoint(&source, &first)
        .unwrap_err()
        .to_string()
        .contains("clean"));
    git(&source, &["add", "new.txt"]);
    git(&source, &["commit", "-m", "fixture: next checkpoint"]);
    let second = git(&source, &["rev-parse", "HEAD"]);
    assert!(verify_switch_git_checkpoint(&source, &second)
        .unwrap_err()
        .to_string()
        .contains("upstream"));
    git(&source, &["push"]);
    verify_switch_git_checkpoint(&source, &second).unwrap();
    assert!(verify_switch_git_checkpoint(&source, &first)
        .unwrap_err()
        .to_string()
        .contains("HEAD"));

    // A cached tracking ref is insufficient after the upstream changes.
    git(&remote, &["update-ref", "refs/heads/main", &first]);
    assert!(verify_switch_git_checkpoint(&source, &second)
        .unwrap_err()
        .to_string()
        .contains("upstream"));
}

#[test]
#[cfg(unix)]
fn switch_cwd_observation_counts_the_provider_in_a_worktree_subdirectory() {
    use std::{
        io::{BufRead, BufReader},
        process::Stdio,
    };
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("nested");
    std::fs::create_dir(&directory).unwrap();
    struct OwnedChild(std::process::Child);
    impl Drop for OwnedChild {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut child = OwnedChild(
        resolved_command(ProcessPlanRequest::new("sh"))
            .expect("resolve shell")
            .args(["-c", "printf 'ready\\n'; read line"])
            .current_dir(directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut ready = String::new();
    BufReader::new(child.0.stdout.take().unwrap())
        .read_line(&mut ready)
        .unwrap();
    assert_eq!(ready, "ready\n");
    let observation = gwt::issue_monitor::observe_agent_switch_cwd(temp.path());
    assert!(observation.matching_pids.contains(&child.0.id()));
    assert!(
        !observation.is_clear(),
        "an idle provider still owns its cwd"
    );
}

#[test]
fn switch_cwd_unknown_or_incomplete_observation_never_proves_clear() {
    use gwt::issue_monitor::AgentSwitchCwdObservation;
    let mut observation = AgentSwitchCwdObservation {
        matching_pids: Vec::new(),
        unknown_pids: Vec::new(),
        complete: true,
    };
    assert!(observation.is_clear());
    observation.unknown_pids.push(42);
    assert!(!observation.is_clear());
    observation.unknown_pids.clear();
    observation.complete = false;
    assert!(!observation.is_clear());
    assert!(
        !gwt::issue_monitor::observe_agent_switch_cwd(Path::new("/missing-switch-worktree"))
            .is_clear()
    );
}

#[test]
fn switch_continuation_requires_the_source_work_and_correlated_board_handoff() {
    use chrono::Utc;
    use gwt_core::{
        coordination::{post_entry, AuthorKind, BoardEntry, BoardEntryKind},
        test_support::ScopedGwtHome,
        workspace_projection::{
            record_workspace_work_event, save_workspace_projection, WorkEvent, WorkEventKind,
            WorkspaceAgentAffiliationStatus, WorkspaceAgentSummary, WorkspaceExecutionContainerRef,
            WorkspaceProjection, WorkspaceStatusCategory,
        },
    };
    let temp = tempfile::tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path().join("home"));
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    git(&source, &["init", "-b", "main"]);
    git(&source, &["config", "user.name", "Switch fixture"]);
    git(&source, &["config", "user.email", "switch@example.invalid"]);
    git(
        &source,
        &[
            "remote",
            "add",
            "origin",
            "https://example.invalid/switch.git",
        ],
    );
    git(
        &source,
        &["commit", "--allow-empty", "-m", "fixture: continuation"],
    );
    let source = dunce::canonicalize(source).unwrap();
    let mut session = gwt_agent::Session::new(&source, "main", gwt_agent::AgentId::Codex);
    session.project_state_root = Some(source.clone());
    session.linked_issue_number = Some(4777);
    session.save(&gwt_core::paths::gwt_sessions_dir()).unwrap();
    let now = Utc::now();
    let mut projection = WorkspaceProjection::default_for_project(&source);
    projection.agents.push(WorkspaceAgentSummary {
        session_id: session.id.clone(),
        window_id: None,
        agent_id: "codex".into(),
        display_name: "Codex".into(),
        status_category: WorkspaceStatusCategory::Active,
        current_focus: None,
        title_summary: None,
        worktree_path: Some(source.clone()),
        branch: Some("main".into()),
        last_board_entry_id: None,
        last_board_entry_kind: None,
        coordination_scope: None,
        affiliation_status: WorkspaceAgentAffiliationStatus::Assigned,
        workspace_id: Some("source-work".into()),
        updated_at: now,
    });
    save_workspace_projection(&source, &projection).unwrap();
    let operation = "00000000-0000-4000-8000-000000000033";
    let mut board = BoardEntry::new(
        AuthorKind::Agent,
        "Codex",
        BoardEntryKind::Handoff,
        format!("Saved continuation for {operation}"),
        None,
        None,
        Vec::new(),
        vec!["4777".into()],
    )
    .with_origin_session_id(&session.id);
    let mut event = WorkEvent::new(WorkEventKind::Start, "source-work", now);
    event.title = Some("Switch source".into());
    event.owner = Some("Issue #4777".into());
    event.agent_session_id = Some(session.id.clone());
    event.agent_id = Some("codex".into());
    event.execution_container = Some(WorkspaceExecutionContainerRef {
        branch: Some("main".into()),
        worktree_path: Some(source.clone()),
        pr_number: None,
        pr_url: None,
        pr_state: None,
    });
    record_workspace_work_event(&source, event.clone()).unwrap();
    assert!(gwt::issue_monitor::verify_switch_continuation(
        &source,
        &session.id,
        operation,
        "source-work",
        &board.id
    )
    .is_err());
    post_entry(&source, board.clone()).unwrap();
    event.id = uuid::Uuid::new_v4().to_string();
    event.kind = WorkEventKind::Update;
    event.summary = Some(format!("Saved checkpoint {operation}"));
    event.progress_summary = Some("Saved current implementation".into());
    event.board_entry_id = Some(board.id.clone());
    record_workspace_work_event(&source, event.clone()).unwrap();
    assert!(
        gwt::issue_monitor::verify_switch_continuation(
            &source,
            &session.id,
            operation,
            "source-work",
            &board.id
        )
        .is_err(),
        "next action is required"
    );
    event.id = uuid::Uuid::new_v4().to_string();
    event.next_action = Some("Resume the remaining acceptance scenario".into());
    record_workspace_work_event(&source, event.clone()).unwrap();
    assert!(
        gwt::issue_monitor::verify_switch_continuation(
            &source,
            &session.id,
            operation,
            "source-work",
            &board.id
        )
        .is_err(),
        "an Active source Work has not been saved as resumable Paused work"
    );
    event.id = uuid::Uuid::new_v4().to_string();
    event.kind = WorkEventKind::Pause;
    event.status_category = Some(WorkspaceStatusCategory::Idle);
    record_workspace_work_event(&source, event.clone()).unwrap();
    gwt::issue_monitor::verify_switch_continuation(
        &source,
        &session.id,
        operation,
        "source-work",
        &board.id,
    )
    .unwrap();
    assert!(gwt::issue_monitor::verify_switch_continuation(
        &source,
        &session.id,
        "00000000-0000-4000-8000-000000000034",
        "source-work",
        &board.id
    )
    .is_err());
    assert!(gwt::issue_monitor::verify_switch_continuation(
        &source,
        &session.id,
        operation,
        "foreign-work",
        &board.id
    )
    .is_err());
    board.id = uuid::Uuid::new_v4().to_string();
    board.origin_session_id = Some("foreign-session".into());
    post_entry(&source, board.clone()).unwrap();
    event.id = uuid::Uuid::new_v4().to_string();
    event.board_entry_id = Some(board.id.clone());
    record_workspace_work_event(&source, event.clone()).unwrap();
    assert!(gwt::issue_monitor::verify_switch_continuation(
        &source,
        &session.id,
        operation,
        "source-work",
        &board.id
    )
    .is_err());
    board.id = uuid::Uuid::new_v4().to_string();
    board.origin_session_id = Some(session.id.clone());
    board.related_owners = vec!["4778".into()];
    post_entry(&source, board.clone()).unwrap();
    event.id = uuid::Uuid::new_v4().to_string();
    event.board_entry_id = Some(board.id.clone());
    record_workspace_work_event(&source, event).unwrap();
    assert!(gwt::issue_monitor::verify_switch_continuation(
        &source,
        &session.id,
        operation,
        "source-work",
        &board.id
    )
    .is_err());
}
