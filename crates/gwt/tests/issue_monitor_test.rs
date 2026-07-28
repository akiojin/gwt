use gwt::issue_monitor::{
    github_auth_setup_message, is_auto_improve_candidate, is_legacy_git_launch_failure_for_project,
    issue_monitor_launch_prompt, load_issue_monitor_prefs, mutate_issue_monitor_prefs_recovering,
    save_issue_monitor_prefs, scan_issue_monitor_candidates,
    scan_issue_monitor_candidates_with_provenance, AutonomousIssueRecord, AutonomousPhase,
    IssueMonitorCandidateSource, IssueMonitorConfig, IssueMonitorFailedIssue, IssueMonitorIssue,
    IssueMonitorIssueState, IssueMonitorPrefs, IssueMonitorState, MonitorInboxState,
    LEGACY_GIT_LAUNCH_FAILURE_MIGRATION_VERSION,
};
use gwt::issue_monitor_worker::{
    scan_loaded_issue_monitor_candidates, LoadedIssueMonitorCandidates,
};
use gwt::LinkedIssueKind;
use gwt_github::issue_auto_claim::{render_claim_comment, ClaimComment, ClaimStatus};
use gwt_github::{
    CommentId, CommentSnapshot, FakeIssueClient, IssueNumber, IssueSnapshot, IssueState, UpdatedAt,
};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn issue(number: u64, labels: &[&str]) -> IssueMonitorIssue {
    IssueMonitorIssue {
        number,
        title: format!("Issue {number}"),
        labels: labels.iter().map(|label| (*label).to_string()).collect(),
        state: IssueMonitorIssueState::Open,
        body: Some(format!("Body {number}")),
        url: Some(format!("https://github.com/example/repo/issues/{number}")),
    }
}

fn github_issue_number(number: u64, comments: Vec<CommentSnapshot>) -> IssueSnapshot {
    IssueSnapshot {
        number: IssueNumber(number),
        title: format!("Improve monitor {number}"),
        body: String::new(),
        labels: vec!["auto-improve".to_string()],
        state: IssueState::Open,
        updated_at: UpdatedAt::new("t1"),
        comments,
    }
}

fn github_issue(comments: Vec<CommentSnapshot>) -> IssueSnapshot {
    github_issue_number(42, comments)
}

fn claim_comment(owner: &str) -> CommentSnapshot {
    let claim = ClaimComment {
        comment_id: Some(CommentId(9)),
        claim_id: "claim-other".to_string(),
        owner: owner.to_string(),
        issue_number: 42,
        status: ClaimStatus::Active,
        heartbeat_at: "2026-06-23T10:00:00Z".to_string(),
        expires_at: "2026-06-23T10:30:00Z".to_string(),
        launched_work_id: Some("work/issue-42".to_string()),
    };
    CommentSnapshot {
        id: CommentId(9),
        body: render_claim_comment(&claim),
        updated_at: UpdatedAt::new("t1"),
    }
}

fn legacy_3272_failure(project_root: &Path) -> String {
    format!(
        "Current branch is unavailable: Git error: Not a git repository: {}",
        project_root.display()
    )
}

fn init_resolvable_git_repo() -> tempfile::TempDir {
    let repo = tempfile::tempdir().expect("tempdir");
    let init = gwt_core::process::hidden_command("git")
        .args(["init", "-b", "develop"])
        .arg(repo.path())
        .output()
        .expect("git init starts");
    assert!(
        init.status.success(),
        "git init failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );
    let commit = gwt_core::process::hidden_command("git")
        .args([
            "-c",
            "user.name=gwt test",
            "-c",
            "user.email=gwt-test@example.invalid",
            "commit",
            "--allow-empty",
            "-m",
            "initial",
        ])
        .current_dir(repo.path())
        .output()
        .expect("git commit starts");
    assert!(
        commit.status.success(),
        "git commit failed: {}",
        String::from_utf8_lossy(&commit.stderr)
    );
    repo
}

fn legacy_failed_prefs(project_root: &Path, issue_number: u64) -> IssueMonitorPrefs {
    IssueMonitorPrefs {
        enabled: true,
        legacy_git_launch_failure_migration_version: 0,
        failed_issues: vec![IssueMonitorFailedIssue {
            issue_number,
            message: legacy_3272_failure(project_root),
            window_id: None,
        }],
        ..IssueMonitorPrefs::default()
    }
}

#[test]
fn monitor_config_defaults_to_disabled_and_accepts_all_open_issues() {
    let config = IssueMonitorConfig::default();

    assert!(!config.enabled);
    assert!(is_auto_improve_candidate(&issue(1, &["bug"]), &config));
    assert!(is_auto_improve_candidate(
        &issue(2, &["auto-improve"]),
        &config
    ));

    let mut closed = issue(3, &["auto-improve"]);
    closed.state = IssueMonitorIssueState::Closed;
    assert!(!is_auto_improve_candidate(&closed, &config));
}

#[test]
fn monitor_maps_gwt_spec_label_to_spec_launch_kind() {
    let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
        enabled: true,
        ..IssueMonitorConfig::default()
    });
    monitor.set_gui_connected(true);
    monitor.record_claimed(issue(3165, &["gwt-spec"]), "claim-spec");

    let launch = monitor
        .next_launch_request("2026-07-02T00:00:00Z")
        .expect("spec launch request");

    assert_eq!(launch.issue_number, 3165);
    assert_eq!(launch.linked_issue_kind, LinkedIssueKind::Spec);
    assert_eq!(launch.branch_name, "work/issue-3165");
    assert_eq!(
        issue_monitor_launch_prompt(launch.linked_issue_kind, launch.issue_number),
        "$gwt-execute #3165"
    );
    assert_eq!(
        monitor
            .inbox_item(3165)
            .and_then(|item| item.launch_plan.as_ref())
            .map(|plan| plan.prompt.as_str()),
        Some("$gwt-execute #3165")
    );
}

#[test]
fn claimed_issue_waits_in_queue_until_gui_is_connected() {
    let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
        enabled: true,
        ..IssueMonitorConfig::default()
    });

    monitor.record_claimed(issue(42, &["auto-improve"]), "claim-a");

    assert_eq!(monitor.queue_len(), 1);
    assert!(monitor
        .next_launch_request("2026-07-02T00:00:00Z")
        .is_none());
    assert_eq!(
        monitor.inbox_item(42).expect("inbox item").state,
        MonitorInboxState::Queued
    );
}

#[test]
fn monitor_runs_one_active_launch_and_keeps_remaining_items_queued() {
    let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
        enabled: true,
        ..IssueMonitorConfig::default()
    });
    monitor.set_gui_connected(true);
    monitor.record_claimed(issue(42, &["auto-improve"]), "claim-a");
    monitor.record_claimed(issue(43, &["auto-improve"]), "claim-b");

    let first = monitor
        .next_launch_request("2026-07-02T00:00:00Z")
        .expect("first launch");
    assert_eq!(first.issue_number, 42);
    assert_eq!(first.branch_name, "work/issue-42");
    assert_eq!(
        issue_monitor_launch_prompt(first.linked_issue_kind, first.issue_number),
        "$gwt-execute #42"
    );
    assert_eq!(monitor.queue_len(), 1);
    assert!(monitor
        .next_launch_request("2026-07-02T00:00:00Z")
        .is_none());

    monitor.complete_active_launch(42, "tab::agent-42");
    assert_eq!(monitor.active_count(), 1);
    assert_eq!(
        monitor.inbox_item(42).expect("inbox item").state,
        MonitorInboxState::Launched
    );
    assert!(
        monitor
            .next_launch_request("2026-07-02T00:00:00Z")
            .is_none(),
        "launched work still consumes the configured active capacity"
    );

    monitor.set_max_active_agents(2);
    let second = monitor
        .next_launch_request("2026-07-02T00:00:00Z")
        .expect("second launch");
    assert_eq!(second.issue_number, 43);
    assert_eq!(second.branch_name, "work/issue-43");
}

#[test]
fn monitor_allows_active_launches_up_to_configured_max() {
    let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
        enabled: true,
        max_active: 2,
        ..IssueMonitorConfig::default()
    });
    monitor.set_gui_connected(true);
    monitor.record_claimed(issue(42, &["bug"]), "claim-a");
    monitor.record_claimed(issue(43, &["enhancement"]), "claim-b");
    monitor.record_claimed(issue(44, &["question"]), "claim-c");

    let first = monitor
        .next_launch_request("2026-07-02T00:00:00Z")
        .expect("first launch");
    let second = monitor
        .next_launch_request("2026-07-02T00:00:00Z")
        .expect("second launch");

    assert_eq!(first.issue_number, 42);
    assert_eq!(second.issue_number, 43);
    assert_eq!(monitor.active_count(), 2);
    assert_eq!(monitor.queue_len(), 1);
    assert!(monitor
        .next_launch_request("2026-07-02T00:00:00Z")
        .is_none());
}

#[test]
fn monitor_reorders_queued_issues_without_preempting_active_launches() {
    let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
        enabled: true,
        ..IssueMonitorConfig::default()
    });
    monitor.set_gui_connected(true);
    monitor.record_claimed(issue(42, &["bug"]), "claim-a");
    monitor.record_claimed(issue(43, &["enhancement"]), "claim-b");
    monitor.record_claimed(issue(44, &["question"]), "claim-c");
    let active = monitor
        .next_launch_request("2026-07-02T00:00:00Z")
        .expect("active launch");
    assert_eq!(active.issue_number, 42);

    monitor.reorder_queued_issues(&[44, 43, 42]);

    let next = monitor.next_launch_request("2026-07-02T00:00:00Z");
    assert!(next.is_none(), "active launch should not be preempted");
    monitor.complete_active_launch(42, "tab::agent-42");
    assert!(
        monitor
            .next_launch_request("2026-07-02T00:00:00Z")
            .is_none(),
        "launched work should not free the active slot until capacity changes or the work stops"
    );
    monitor.set_max_active_agents(2);
    assert_eq!(
        monitor
            .next_launch_request("2026-07-02T00:00:00Z")
            .expect("next launch")
            .issue_number,
        44
    );
}

#[test]
fn monitor_reorders_visible_inbox_to_match_queue_priority() {
    let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
        enabled: true,
        ..IssueMonitorConfig::default()
    });
    monitor.record_candidate(issue(42, &["bug"]));
    monitor.record_candidate(issue(43, &["enhancement"]));
    monitor.record_candidate(issue(44, &["question"]));

    monitor.reorder_queued_issues(&[44, 42, 43]);

    let visible_numbers: Vec<u64> = monitor.inbox.iter().map(|item| item.issue.number).collect();
    assert_eq!(visible_numbers, vec![44, 42, 43]);
}

#[test]
fn monitor_prefs_persist_priority_and_max_active_agents() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("issue_monitor.json");
    let prefs = IssueMonitorPrefs {
        enabled: true,
        max_active_agents: 3,
        priority_order: vec![44, 42, 43],
        launch_profile: None,
        ..IssueMonitorPrefs::default()
    };

    save_issue_monitor_prefs(&path, &prefs).expect("save prefs");
    let loaded = load_issue_monitor_prefs(&path).expect("load prefs");

    assert_eq!(loaded, prefs);
}

#[test]
fn schema_data_errors_are_not_recovered_or_overwritten() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("issue_monitor.json");
    let mut schema_invalid =
        serde_json::to_value(IssueMonitorPrefs::default()).expect("serialize prefs");
    schema_invalid["enabled"] = serde_json::Value::String("future-schema-value".to_string());
    let original = serde_json::to_vec_pretty(&schema_invalid).expect("encode invalid schema");
    fs::write(&path, &original).expect("seed schema-invalid prefs");
    let mut mutation_ran = false;

    let result =
        mutate_issue_monitor_prefs_recovering(&path, &IssueMonitorPrefs::default(), |_| {
            mutation_ran = true
        });

    assert!(result.is_err(), "schema data errors must fail closed");
    assert!(!mutation_ran, "a rejected snapshot must not be mutated");
    assert_eq!(fs::read(&path).expect("read original prefs"), original);
    assert!(
        fs::read_dir(dir.path())
            .expect("read prefs directory")
            .filter_map(Result::ok)
            .all(|entry| !entry
                .file_name()
                .to_string_lossy()
                .starts_with("issue_monitor.json.corrupt-")),
        "schema data errors are not torn JSON and must not be quarantined"
    );
}

#[test]
fn claimed_active_launch_stays_launching_when_scan_refreshes_claim() {
    let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
        enabled: true,
        ..IssueMonitorConfig::default()
    });
    monitor.set_gui_connected(true);
    monitor.record_claimed(issue(42, &["auto-improve"]), "claim-a");
    let first = monitor
        .next_launch_request("2026-07-02T00:00:00Z")
        .expect("launch request");
    assert_eq!(first.issue_number, 42);

    monitor.record_claimed(issue(42, &["auto-improve"]), "claim-a-refresh");

    assert_eq!(monitor.queue_len(), 0);
    assert_eq!(
        monitor.inbox_item(42).expect("inbox item").state,
        MonitorInboxState::Launching
    );
}

#[test]
fn failed_launch_marks_inbox_failed_and_clears_active_launch() {
    let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
        enabled: true,
        ..IssueMonitorConfig::default()
    });
    monitor.set_gui_connected(true);
    monitor.record_claimed(issue(42, &["auto-improve"]), "claim-a");
    let first = monitor
        .next_launch_request("2026-07-02T00:00:00Z")
        .expect("launch request");
    assert_eq!(first.issue_number, 42);

    monitor.record_launch_auth_required("2026-06-23T10:02:00Z");
    monitor.record_launch_failed(42, "binary missing");

    assert_eq!(monitor.active_count(), 0);
    assert_eq!(monitor.queue_len(), 0);
    assert_eq!(
        monitor.inbox_item(42).expect("inbox item").state,
        MonitorInboxState::LaunchFailed
    );
    assert_eq!(
        monitor
            .inbox_item(42)
            .expect("inbox item")
            .error_message
            .as_deref(),
        Some("binary missing")
    );
    assert_eq!(
        monitor.status_view().last_error.as_deref(),
        Some("issue #42: binary missing")
    );
    assert_eq!(monitor.status_view().state, "error");
}

#[test]
fn agent_runtime_failure_marks_launched_issue_failed_and_persists_error() {
    let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
        enabled: true,
        max_active: 2,
        ..IssueMonitorConfig::default()
    });
    monitor.set_gui_connected(true);
    monitor.record_claimed(issue(42, &["auto-improve"]), "claim-a");
    monitor.record_claimed(issue(43, &["bug"]), "claim-b");
    let first = monitor
        .next_launch_request("2026-07-02T00:00:00Z")
        .expect("launch request");
    assert_eq!(first.issue_number, 42);
    monitor.complete_active_launch(42, "tab::agent-42");

    let failed_issue =
        monitor.record_agent_window_failed("tab::agent-42", "Stop-block hit an error");

    assert_eq!(failed_issue, Some(42));
    assert_eq!(monitor.active_count(), 0);
    assert_eq!(monitor.queue_len(), 1);
    let item = monitor.inbox_item(42).expect("failed inbox item");
    assert_eq!(item.state, MonitorInboxState::AgentFailed);
    assert_eq!(item.launched_window_id, None);
    assert_eq!(
        item.error_message.as_deref(),
        Some("Stop-block hit an error")
    );
    assert_eq!(monitor.status_view().state, "error");
    assert_eq!(
        monitor.status_view().last_error.as_deref(),
        Some("issue #42: Stop-block hit an error")
    );
    assert_eq!(
        monitor.prefs().failed_issues,
        vec![IssueMonitorFailedIssue {
            issue_number: 42,
            message: "Stop-block hit an error".to_string(),
            // #3165 error-window lifecycle: the failed agent window id is
            // retained so an explicit Launch Now can close the stale window.
            window_id: Some("tab::agent-42".to_string()),
        }]
    );

    let mut restored =
        IssueMonitorState::with_prefs(IssueMonitorConfig::default(), monitor.prefs());
    scan_issue_monitor_candidates(
        &mut restored,
        &[issue(42, &["bug"])],
        "2026-06-23T10:03:00Z",
    );
    let restored_item = restored.inbox_item(42).expect("restored failed item");
    assert_eq!(restored_item.state, MonitorInboxState::AgentFailed);
    assert_eq!(
        restored_item.error_message.as_deref(),
        Some("Stop-block hit an error")
    );
    assert_eq!(restored.queue_len(), 0);
    assert_eq!(restored.status_view().state, "error");
}

#[test]
fn agent_runtime_failure_matches_raw_and_combined_window_ids() {
    let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
        enabled: true,
        max_active: 2,
        ..IssueMonitorConfig::default()
    });
    monitor.set_gui_connected(true);
    monitor.record_claimed(issue(42, &["auto-improve"]), "claim-a");
    monitor
        .next_launch_request("2026-07-02T00:00:00Z")
        .expect("launch request");
    monitor.complete_active_launch(42, "tab-1::agent-42");

    let failed_issue = monitor.record_agent_window_failed("agent-42", "Stop-block hit an error");

    assert_eq!(failed_issue, Some(42));
    let item = monitor.inbox_item(42).expect("failed inbox item");
    assert_eq!(item.state, MonitorInboxState::AgentFailed);
    assert_eq!(
        item.error_message.as_deref(),
        Some("Stop-block hit an error")
    );
}

#[test]
fn max_active_increase_allows_more_queued_launches_without_rescan() {
    let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
        enabled: true,
        max_active: 1,
        ..IssueMonitorConfig::default()
    });
    monitor.set_gui_connected(true);
    monitor.record_claimed(issue(42, &["auto-improve"]), "claim-a");
    monitor.record_claimed(issue(43, &["auto-improve"]), "claim-b");
    monitor.record_claimed(issue(44, &["auto-improve"]), "claim-c");

    let first = monitor
        .next_launch_request("2026-07-02T00:00:00Z")
        .expect("first launch");
    assert_eq!(first.issue_number, 42);
    assert!(monitor
        .next_launch_request("2026-07-02T00:00:00Z")
        .is_none());

    monitor.set_max_active_agents(3);

    let second = monitor
        .next_launch_request("2026-07-02T00:00:00Z")
        .expect("second launch after max increase");
    let third = monitor
        .next_launch_request("2026-07-02T00:00:00Z")
        .expect("third launch after max increase");
    assert_eq!(second.issue_number, 43);
    assert_eq!(third.issue_number, 44);
    assert_eq!(monitor.active_count(), 3);
    assert_eq!(monitor.queue_len(), 0);
}

#[test]
fn claim_capacity_cap_prevents_multiple_preconfigured_launches() {
    let client = FakeIssueClient::new();
    client.seed(github_issue_number(42, vec![]));
    client.seed(github_issue_number(43, vec![]));
    client.seed(github_issue_number(44, vec![]));
    let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
        enabled: true,
        max_active: 3,
        ..IssueMonitorConfig::default()
    });
    monitor.set_gui_connected(true);
    scan_issue_monitor_candidates(
        &mut monitor,
        &[
            issue(42, &["bug"]),
            issue(43, &["enhancement"]),
            issue(44, &["question"]),
        ],
        "2026-06-23T10:01:00Z",
    );

    let launches = monitor.claim_next_launch_requests_with_active_cap(
        &client,
        "host-a/session-a",
        "2026-06-23T10:01:00Z",
        1,
    );
    let repeated = monitor.claim_next_launch_requests_with_active_cap(
        &client,
        "host-a/session-a",
        "2026-06-23T10:02:00Z",
        1,
    );

    assert_eq!(launches.len(), 1);
    assert_eq!(launches[0].issue_number, 42);
    assert!(repeated.is_empty());
    assert_eq!(monitor.active_count(), 1);
    assert_eq!(monitor.queue_len(), 2);
}

#[test]
fn claim_capacity_zero_keeps_queue_visible_without_claiming_or_launching() {
    let client = FakeIssueClient::new();
    client.seed(github_issue_number(42, vec![]));
    let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
        enabled: true,
        max_active: 5,
        ..IssueMonitorConfig::default()
    });
    monitor.set_gui_connected(true);
    scan_issue_monitor_candidates(&mut monitor, &[issue(42, &["bug"])], "2026-06-23T10:01:00Z");

    let launches = monitor.claim_next_launch_requests_with_active_cap(
        &client,
        "host-a/session-a",
        "2026-06-23T10:01:00Z",
        0,
    );

    assert!(launches.is_empty());
    assert_eq!(monitor.active_count(), 0);
    assert_eq!(monitor.queue_len(), 1);
    assert_eq!(
        monitor.inbox_item(42).expect("inbox item").state,
        MonitorInboxState::Queued
    );
    assert!(
        client.comments(IssueNumber(42)).is_empty(),
        "capacity 0 must not create a GitHub claim"
    );
}

#[test]
fn blocked_claim_is_visible_in_inbox_without_queueing_launch() {
    let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
        enabled: true,
        ..IssueMonitorConfig::default()
    });

    monitor.record_blocked_by_claim(
        issue(42, &["auto-improve"]),
        "other-host/session",
        "2026-06-23T10:30:00Z",
    );

    let item = monitor.inbox_item(42).expect("inbox item");
    assert_eq!(item.state, MonitorInboxState::BlockedByClaim);
    assert_eq!(item.blocked_by_owner.as_deref(), Some("other-host/session"));
    assert_eq!(
        item.claim_expires_at.as_deref(),
        Some("2026-06-23T10:30:00Z")
    );
    assert_eq!(monitor.queue_len(), 0);
}

#[test]
fn scan_candidates_queues_all_open_issues_without_claiming_at_scan_time() {
    let client = FakeIssueClient::new();
    client.seed(github_issue(vec![]));
    let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
        enabled: true,
        ..IssueMonitorConfig::default()
    });

    let summary = scan_issue_monitor_candidates(
        &mut monitor,
        &[issue(42, &["auto-improve"]), issue(43, &["bug"])],
        "2026-06-23T10:00:00Z",
    );

    assert_eq!(summary.claimed, 0);
    assert_eq!(summary.scanned, 2);
    assert_eq!(summary.skipped, 0);
    assert!(client.comments(IssueNumber(42)).is_empty());
    assert_eq!(monitor.queue_len(), 2);
    assert_eq!(
        monitor.inbox_item(42).expect("inbox item").state,
        MonitorInboxState::Queued
    );
}

#[test]
fn scan_candidates_ignores_claims_until_launch_time() {
    let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
        enabled: true,
        ..IssueMonitorConfig::default()
    });

    let summary = scan_issue_monitor_candidates(
        &mut monitor,
        &[issue(42, &["auto-improve"])],
        "2026-06-23T10:01:00Z",
    );

    assert_eq!(summary.blocked, 0);
    assert_eq!(monitor.queue_len(), 1);
    assert_eq!(
        monitor.inbox_item(42).expect("inbox item").state,
        MonitorInboxState::Queued
    );
}

#[test]
fn scan_error_keeps_visible_queue_candidates() {
    let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
        enabled: true,
        ..IssueMonitorConfig::default()
    });
    scan_issue_monitor_candidates(&mut monitor, &[issue(42, &["bug"])], "2026-06-23T10:01:00Z");

    monitor.record_scan_error(
        "2026-06-23T10:02:00Z",
        "GitHub auth failed: test auth failure",
    );

    let status = monitor.status_view();
    assert_eq!(status.queue_len, 1);
    assert_eq!(status.total_candidates, 1);
    assert_eq!(
        status.last_error.as_deref(),
        Some("GitHub auth failed: test auth failure")
    );
    assert_eq!(
        monitor.inbox_item(42).expect("inbox item").state,
        MonitorInboxState::Queued
    );
}

#[test]
fn launch_auth_required_keeps_visible_queue_with_blocking_error() {
    let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
        enabled: true,
        ..IssueMonitorConfig::default()
    });
    scan_issue_monitor_candidates(&mut monitor, &[issue(42, &["bug"])], "2026-06-23T10:01:00Z");

    monitor.record_launch_auth_required("2026-06-23T10:02:00Z");

    let status = monitor.status_view();
    assert_eq!(status.state, "error");
    assert_eq!(status.queue_len, 1);
    assert_eq!(status.total_candidates, 1);
    assert_eq!(
        status.last_error.as_deref(),
        Some(github_auth_setup_message())
    );
    let message = status.last_error.as_deref().expect("auth help message");
    assert!(message.contains("gh auth login --hostname github.com"));
    assert!(message.contains("gh auth setup-git"));
    assert!(message.contains("git ls-remote origin HEAD"));
    assert_eq!(
        monitor.inbox_item(42).expect("inbox item").state,
        MonitorInboxState::Queued
    );
}

#[test]
fn monitor_claims_queue_head_just_in_time_and_skips_blocked_claims() {
    let client = FakeIssueClient::new();
    client.seed(github_issue_number(
        42,
        vec![claim_comment("host-b/session-b")],
    ));
    client.seed(github_issue_number(43, vec![]));
    let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
        enabled: true,
        ..IssueMonitorConfig::default()
    });
    monitor.set_gui_connected(true);
    scan_issue_monitor_candidates(
        &mut monitor,
        &[issue(42, &["bug"]), issue(43, &["enhancement"])],
        "2026-06-23T10:01:00Z",
    );
    assert!(client.comments(IssueNumber(43)).is_empty());

    let launches =
        monitor.claim_next_launch_requests(&client, "host-a/session-a", "2026-06-23T10:01:00Z");

    assert_eq!(launches.len(), 1);
    assert_eq!(launches[0].issue_number, 43);
    assert_eq!(
        monitor.inbox_item(42).expect("blocked item").state,
        MonitorInboxState::BlockedByClaim
    );
    assert_eq!(
        monitor.inbox_item(43).expect("launched item").state,
        MonitorInboxState::Launching
    );
    assert_eq!(client.comments(IssueNumber(43)).len(), 1);
}

#[test]
fn pre_3314_prefs_missing_marker_round_trip_without_losing_existing_state() {
    let project_root = Path::new("/tmp/gwt-pre-3314-project");
    let fixture = include_str!("fixtures/issue_monitor_prefs_pre_3314_migration.json")
        .replace("__PROJECT_ROOT__", &project_root.display().to_string());

    let prefs: IssueMonitorPrefs =
        serde_json::from_str(&fixture).expect("pre-3314 prefs deserialize");

    assert_eq!(prefs.legacy_git_launch_failure_migration_version, 0);
    assert!(prefs.enabled);
    assert_eq!(prefs.max_active_agents, 3);
    assert_eq!(prefs.priority_order, vec![43, 42, 99]);
    assert_eq!(prefs.launch_profile.as_ref().unwrap().agent_id, "codex");
    assert_eq!(prefs.launched_issues[0].issue_number, 70);
    assert_eq!(prefs.launching_issues[0].issue_number, 71);
    assert_eq!(prefs.failed_issues.len(), 2);
    assert_eq!(prefs.merged_issues, vec![77]);
    assert!(prefs.autonomous_mode);
    assert_eq!(prefs.autonomous_tuning.max_attempts, 7);
    assert_eq!(prefs.autonomous_records[0].issue_number, 99);
    assert_eq!(prefs.autonomous_records[0].attempts, 2);

    let serialized = serde_json::to_string(&prefs).expect("serialize migrated schema");
    let round_trip: IssueMonitorPrefs =
        serde_json::from_str(&serialized).expect("round trip deserialize");
    assert_eq!(round_trip, prefs);

    let state = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), prefs.clone());
    let state_prefs = state.prefs();
    assert_eq!(state_prefs.legacy_git_launch_failure_migration_version, 0);
    assert_eq!(state_prefs.priority_order, prefs.priority_order);
    assert_eq!(state_prefs.launch_profile, prefs.launch_profile);
    assert_eq!(state_prefs.launched_issues, prefs.launched_issues);
    assert_eq!(state_prefs.launching_issues, prefs.launching_issues);
    assert_eq!(state_prefs.merged_issues, prefs.merged_issues);
    assert_eq!(state_prefs.autonomous_mode, prefs.autonomous_mode);
    assert_eq!(state_prefs.autonomous_tuning, prefs.autonomous_tuning);
    assert_eq!(state_prefs.autonomous_records, prefs.autonomous_records);
}

#[test]
fn fresh_issue_monitor_state_starts_at_current_legacy_failure_migration_version() {
    assert_eq!(
        IssueMonitorPrefs::default().legacy_git_launch_failure_migration_version,
        LEGACY_GIT_LAUNCH_FAILURE_MIGRATION_VERSION
    );
    assert_eq!(
        IssueMonitorState::new(IssueMonitorConfig::default())
            .prefs()
            .legacy_git_launch_failure_migration_version,
        LEGACY_GIT_LAUNCH_FAILURE_MIGRATION_VERSION
    );
}

#[test]
fn legacy_3272_failure_matcher_requires_exact_normalized_project_path() {
    let project_root =
        Path::new(r"Microsoft.PowerShell.Core\FileSystem::\\?\E:\gwt\work\issue-3314");
    let exact = concat!(
        "Current branch is unavailable: Git error: Not a git repository: ",
        r"E:\gwt\work\issue-3314"
    );
    assert!(is_legacy_git_launch_failure_for_project(
        exact,
        project_root
    ));

    for rejected in [
        concat!(
            "Current branch is unavailable: Git error: Not a git repository: ",
            r"E:\gwt\work\issue-3314\child"
        ),
        concat!(
            "prefix Current branch is unavailable: Git error: Not a git repository: ",
            r"E:\gwt\work\issue-3314"
        ),
        concat!(
            "Current branch is unavailable: Git error: Not a git repository (or any parent): ",
            r"E:\gwt\work\issue-3314"
        ),
        concat!(
            "Current branch is unavailable: Git error: Not a git repository: ",
            r"E:\gwt\work\issue-331"
        ),
    ] {
        assert!(
            !is_legacy_git_launch_failure_for_project(rejected, project_root),
            "must reject approximate failure: {rejected}"
        );
    }
}

#[test]
fn live_scan_with_resolvable_git_migrates_exact_failure_and_requeues_open_issue() {
    let repo = init_resolvable_git_repo();
    let mut monitor = IssueMonitorState::with_prefs(
        IssueMonitorConfig::default(),
        legacy_failed_prefs(repo.path(), 42),
    );
    scan_issue_monitor_candidates(&mut monitor, &[issue(42, &["bug"])], "2026-07-21T00:00:00Z");
    assert_eq!(
        monitor.inbox_item(42).map(|item| item.state),
        Some(MonitorInboxState::AgentFailed)
    );

    scan_issue_monitor_candidates_with_provenance(
        &mut monitor,
        &[issue(42, &["bug"])],
        IssueMonitorCandidateSource::Live,
        repo.path(),
        "2026-07-21T00:01:00Z",
    );

    assert_eq!(
        monitor.prefs().legacy_git_launch_failure_migration_version,
        LEGACY_GIT_LAUNCH_FAILURE_MIGRATION_VERSION
    );
    assert!(monitor.prefs().failed_issues.is_empty());
    assert_eq!(monitor.status_view().last_error, None);
    assert_eq!(monitor.queue_len(), 1);
    let item = monitor.inbox_item(42).expect("normal candidate rebuilt");
    assert_eq!(item.state, MonitorInboxState::Queued);
    assert_eq!(item.error_message, None);
}

#[test]
fn live_migration_removes_absent_or_closed_failed_rows_without_queueing() {
    let repo = init_resolvable_git_repo();
    for fresh in [None, Some(IssueMonitorIssueState::Closed)] {
        let mut monitor = IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            legacy_failed_prefs(repo.path(), 42),
        );
        scan_issue_monitor_candidates(&mut monitor, &[issue(42, &["bug"])], "2026-07-21T00:00:00Z");
        let mut candidates = Vec::new();
        if let Some(state) = fresh {
            let mut candidate = issue(42, &["bug"]);
            candidate.state = state;
            candidates.push(candidate);
        }

        scan_issue_monitor_candidates_with_provenance(
            &mut monitor,
            &candidates,
            IssueMonitorCandidateSource::Live,
            repo.path(),
            "2026-07-21T00:01:00Z",
        );

        assert!(monitor.prefs().failed_issues.is_empty());
        assert!(monitor.inbox_item(42).is_none());
        assert_eq!(monitor.queue_len(), 0);
    }
}

#[test]
fn cache_or_failed_resolver_does_not_mutate_legacy_migration_state() {
    let repo = init_resolvable_git_repo();
    let candidate = issue(42, &["bug"]);
    let mut cached = IssueMonitorState::with_prefs(
        IssueMonitorConfig::default(),
        legacy_failed_prefs(repo.path(), 42),
    );
    scan_issue_monitor_candidates_with_provenance(
        &mut cached,
        std::slice::from_ref(&candidate),
        IssueMonitorCandidateSource::Cache,
        repo.path(),
        "2026-07-21T00:00:00Z",
    );
    assert_eq!(
        cached.prefs().legacy_git_launch_failure_migration_version,
        0
    );
    assert_eq!(cached.prefs().failed_issues.len(), 1);
    assert_eq!(
        cached.inbox_item(42).map(|item| item.state),
        Some(MonitorInboxState::AgentFailed)
    );

    let non_repo = tempfile::tempdir().expect("non-repo tempdir");
    let mut unresolved = IssueMonitorState::with_prefs(
        IssueMonitorConfig::default(),
        legacy_failed_prefs(non_repo.path(), 42),
    );
    scan_issue_monitor_candidates_with_provenance(
        &mut unresolved,
        &[candidate],
        IssueMonitorCandidateSource::Live,
        non_repo.path(),
        "2026-07-21T00:00:00Z",
    );
    assert_eq!(
        unresolved
            .prefs()
            .legacy_git_launch_failure_migration_version,
        0
    );
    assert_eq!(unresolved.prefs().failed_issues.len(), 1);
}

#[test]
fn migration_preserves_windows_needs_human_and_all_unrelated_prefs() {
    let repo = init_resolvable_git_repo();
    let target_message = legacy_3272_failure(repo.path());
    let mut prefs = legacy_failed_prefs(repo.path(), 42);
    prefs.max_active_agents = 4;
    prefs.priority_order = vec![99, 45, 44, 43, 42];
    prefs
        .launching_issues
        .push(gwt::IssueMonitorLaunchingIssue {
            issue_number: 99,
            claimed_at: Some("2026-07-21T00:00:00Z".to_string()),
        });
    prefs.merged_issues.push(88);
    prefs.autonomous_mode = true;
    prefs.autonomous_tuning.max_attempts = 9;
    prefs.failed_issues.extend([
        IssueMonitorFailedIssue {
            issue_number: 43,
            message: target_message.clone(),
            window_id: Some("tab::agent-43".to_string()),
        },
        IssueMonitorFailedIssue {
            issue_number: 44,
            message: "unrelated failure".to_string(),
            window_id: None,
        },
        IssueMonitorFailedIssue {
            issue_number: 45,
            message: target_message,
            window_id: None,
        },
    ]);
    prefs.autonomous_records.push(AutonomousIssueRecord {
        issue_number: 45,
        phase: AutonomousPhase::NeedsHuman,
        active_launch_id: None,
        attempts: 6,
        acceptance_snapshot: None,
        retry_not_before: None,
        last_heartbeat: Some("2026-07-20T00:00:00Z".to_string()),
        pr_number: None,
        reviewed_sha: None,
        review_passed: None,
    });
    let mut monitor = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), prefs);
    scan_issue_monitor_candidates(
        &mut monitor,
        &[
            issue(42, &["bug"]),
            issue(43, &["bug"]),
            issue(44, &["bug"]),
            issue(45, &["auto-merge"]),
        ],
        "2026-07-21T00:00:00Z",
    );
    monitor
        .inbox
        .iter_mut()
        .find(|item| item.issue.number == 45)
        .expect("needs-human row")
        .state = MonitorInboxState::NeedsHuman;

    scan_issue_monitor_candidates_with_provenance(
        &mut monitor,
        &[
            issue(42, &["bug"]),
            issue(43, &["bug"]),
            issue(44, &["bug"]),
            issue(45, &["auto-merge"]),
        ],
        IssueMonitorCandidateSource::Live,
        repo.path(),
        "2026-07-21T00:01:00Z",
    );

    let after = monitor.prefs();
    assert_eq!(after.max_active_agents, 4);
    assert_eq!(after.priority_order, vec![99, 45, 44, 43, 42]);
    assert_eq!(after.launching_issues[0].issue_number, 99);
    assert_eq!(after.merged_issues, vec![88]);
    assert!(after.autonomous_mode);
    assert_eq!(after.autonomous_tuning.max_attempts, 9);
    assert_eq!(after.autonomous_records[0].attempts, 6);
    assert!(after
        .failed_issues
        .iter()
        .all(|failed| failed.issue_number != 42));
    assert!(after.failed_issues.iter().any(|failed| {
        failed.issue_number == 43 && failed.window_id.as_deref() == Some("tab::agent-43")
    }));
    assert!(after
        .failed_issues
        .iter()
        .any(|failed| failed.issue_number == 44 && failed.message == "unrelated failure"));
    assert!(after
        .failed_issues
        .iter()
        .any(|failed| failed.issue_number == 45));
    assert_eq!(
        monitor.inbox_item(45).map(|item| item.state),
        Some(MonitorInboxState::NeedsHuman)
    );
}

#[test]
fn migration_rederives_banner_from_unrelated_failure() {
    let repo = init_resolvable_git_repo();
    let mut prefs = legacy_failed_prefs(repo.path(), 42);
    prefs.failed_issues.push(IssueMonitorFailedIssue {
        issue_number: 99,
        message: "unrelated failure".to_string(),
        window_id: None,
    });
    let mut monitor = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), prefs);
    scan_issue_monitor_candidates(
        &mut monitor,
        &[issue(42, &["bug"]), issue(99, &["bug"])],
        "2026-07-21T00:00:00Z",
    );

    scan_issue_monitor_candidates_with_provenance(
        &mut monitor,
        &[issue(42, &["bug"]), issue(99, &["bug"])],
        IssueMonitorCandidateSource::Live,
        repo.path(),
        "2026-07-21T00:01:00Z",
    );

    assert_eq!(
        monitor.status_view().last_error.as_deref(),
        Some("issue #99: unrelated failure")
    );
}

#[test]
fn legacy_failure_migration_is_one_shot_even_when_no_initial_target_exists() {
    let repo = init_resolvable_git_repo();
    let prefs = IssueMonitorPrefs {
        enabled: true,
        legacy_git_launch_failure_migration_version: 0,
        ..IssueMonitorPrefs::default()
    };
    let mut monitor = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), prefs);
    scan_issue_monitor_candidates_with_provenance(
        &mut monitor,
        &[issue(42, &["bug"])],
        IssueMonitorCandidateSource::Live,
        repo.path(),
        "2026-07-21T00:00:00Z",
    );
    assert_eq!(
        monitor.prefs().legacy_git_launch_failure_migration_version,
        LEGACY_GIT_LAUNCH_FAILURE_MIGRATION_VERSION
    );

    monitor.record_launch_failed(42, legacy_3272_failure(repo.path()));
    scan_issue_monitor_candidates_with_provenance(
        &mut monitor,
        &[issue(42, &["bug"])],
        IssueMonitorCandidateSource::Live,
        repo.path(),
        "2026-07-21T00:01:00Z",
    );
    let persisted = monitor.prefs();
    assert_eq!(persisted.failed_issues.len(), 1);
    assert_eq!(
        monitor.inbox_item(42).map(|item| item.state),
        Some(MonitorInboxState::LaunchFailed)
    );
    assert_eq!(monitor.queue_len(), 0);
}

#[test]
fn legacy_3272_recovery_respects_priority_capacity_and_idempotency() {
    let repo = init_resolvable_git_repo();
    let mut prefs = legacy_failed_prefs(repo.path(), 42);
    prefs.max_active_agents = 2;
    prefs.priority_order = vec![43, 42];
    let mut monitor = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), prefs);
    monitor.set_gui_connected(true);

    let loaded = LoadedIssueMonitorCandidates {
        issues: vec![issue(42, &["bug"]), issue(43, &["enhancement"])],
        source: IssueMonitorCandidateSource::Live,
    };
    scan_loaded_issue_monitor_candidates(
        &mut monitor,
        &loaded,
        repo.path(),
        "2026-07-21T00:00:00Z",
    );
    let client = FakeIssueClient::new();
    client.seed(github_issue_number(42, vec![]));
    client.seed(github_issue_number(43, vec![]));

    let first = monitor.claim_next_launch_requests_with_active_cap(
        &client,
        "host-a/session-a",
        "2026-07-21T00:01:00Z",
        1,
    );
    let repeated = monitor.claim_next_launch_requests_with_active_cap(
        &client,
        "host-a/session-a",
        "2026-07-21T00:02:00Z",
        1,
    );

    assert_eq!(first.len(), 1);
    assert_eq!(first[0].issue_number, 43, "existing priority path wins");
    assert!(repeated.is_empty(), "active cap prevents duplicate claim");
    assert_eq!(monitor.active_count(), 1);
    assert_eq!(monitor.queue_len(), 1);
    assert_eq!(client.comments(IssueNumber(43)).len(), 1);
    assert!(client.comments(IssueNumber(42)).is_empty());
}

#[test]
fn newer_disk_migration_is_adopted_but_equal_marker_keeps_fresh_failure() {
    let project_root = PathBuf::from("/tmp/gwt-adoption-project");
    let target = IssueMonitorFailedIssue {
        issue_number: 42,
        message: legacy_3272_failure(&project_root),
        window_id: None,
    };
    let unrelated = IssueMonitorFailedIssue {
        issue_number: 99,
        message: "unrelated failure".to_string(),
        window_id: Some("tab::agent-99".to_string()),
    };
    let mut outgoing = IssueMonitorPrefs {
        legacy_git_launch_failure_migration_version: 0,
        failed_issues: vec![target.clone()],
        ..IssueMonitorPrefs::default()
    };
    let disk = IssueMonitorPrefs {
        legacy_git_launch_failure_migration_version: LEGACY_GIT_LAUNCH_FAILURE_MIGRATION_VERSION,
        failed_issues: vec![unrelated.clone()],
        ..IssueMonitorPrefs::default()
    };
    assert!(outgoing.adopt_newer_legacy_git_launch_failure_migration(&disk));
    assert_eq!(
        outgoing.legacy_git_launch_failure_migration_version,
        LEGACY_GIT_LAUNCH_FAILURE_MIGRATION_VERSION
    );
    assert_eq!(outgoing.failed_issues, vec![unrelated.clone()]);

    outgoing.failed_issues.push(target.clone());
    let equal_disk = IssueMonitorPrefs {
        legacy_git_launch_failure_migration_version: LEGACY_GIT_LAUNCH_FAILURE_MIGRATION_VERSION,
        failed_issues: vec![unrelated.clone()],
        ..IssueMonitorPrefs::default()
    };
    assert!(!outgoing.adopt_newer_legacy_git_launch_failure_migration(&equal_disk));
    assert!(outgoing.failed_issues.contains(&target));

    let mut state = IssueMonitorState::with_prefs(
        IssueMonitorConfig::default(),
        IssueMonitorPrefs {
            enabled: true,
            legacy_git_launch_failure_migration_version: 0,
            failed_issues: vec![target.clone()],
            ..IssueMonitorPrefs::default()
        },
    );
    scan_issue_monitor_candidates(&mut state, &[issue(42, &["bug"])], "2026-07-21T00:00:00Z");
    assert!(state.adopt_newer_legacy_git_launch_failure_migration_from_prefs(&disk));
    assert!(state.inbox_item(42).is_none(), "stale failed row removed");
    assert_eq!(state.prefs().failed_issues, vec![unrelated]);

    state.record_launch_failed(42, target.message.clone());
    assert!(
        !state.adopt_newer_legacy_git_launch_failure_migration_from_prefs(&equal_disk),
        "equal marker cannot erase a failure recorded after migration"
    );
    assert!(state
        .prefs()
        .failed_issues
        .iter()
        .any(|failed| failed.issue_number == 42));
}

#[test]
fn newer_disk_failure_adoption_cancels_stale_pending_launch_and_reconciles_inbox() {
    let mut state = IssueMonitorState::with_prefs(
        IssueMonitorConfig::default(),
        IssueMonitorPrefs {
            enabled: true,
            max_active_agents: 2,
            legacy_git_launch_failure_migration_version: 0,
            ..IssueMonitorPrefs::default()
        },
    );
    state.set_gui_connected(true);
    scan_issue_monitor_candidates(&mut state, &[issue(99, &["bug"])], "2026-07-21T00:00:00Z");
    let client = FakeIssueClient::new();
    client.seed(github_issue_number(99, vec![]));
    let launches = state.claim_next_launch_requests_with_active_cap(
        &client,
        "host-a/session-a",
        "2026-07-21T00:00:10Z",
        1,
    );
    assert_eq!(launches.len(), 1);
    assert_eq!(state.active_count(), 1);

    let disk = IssueMonitorPrefs {
        legacy_git_launch_failure_migration_version: LEGACY_GIT_LAUNCH_FAILURE_MIGRATION_VERSION,
        failed_issues: vec![IssueMonitorFailedIssue {
            issue_number: 99,
            message: "unrelated failure retained on disk".to_string(),
            window_id: None,
        }],
        ..IssueMonitorPrefs::default()
    };

    assert!(state.adopt_newer_legacy_git_launch_failure_migration_from_prefs(&disk));
    assert!(state.take_pending_launch_requests().is_empty());
    assert_eq!(
        state.active_count(),
        0,
        "cancelled pending launch frees slot"
    );
    assert_eq!(state.queue_len(), 0);
    let item = state.inbox_item(99).expect("retained failure row");
    assert_eq!(item.state, MonitorInboxState::AgentFailed);
    assert_eq!(
        item.error_message.as_deref(),
        Some("unrelated failure retained on disk")
    );
}

#[test]
fn newer_disk_failure_adoption_preserves_real_launched_window_across_roundtrip_and_scan() {
    let mut state = IssueMonitorState::with_prefs(
        IssueMonitorConfig::default(),
        IssueMonitorPrefs {
            enabled: true,
            legacy_git_launch_failure_migration_version: 0,
            launched_issues: vec![gwt::IssueMonitorLaunchedIssue {
                issue_number: 42,
                window_id: "tab::agent-42".to_string(),
            }],
            ..IssueMonitorPrefs::default()
        },
    );
    scan_issue_monitor_candidates(&mut state, &[issue(42, &["bug"])], "2026-07-21T00:00:00Z");
    assert_eq!(
        state.inbox_item(42).map(|item| item.state),
        Some(MonitorInboxState::Launched)
    );

    let disk = IssueMonitorPrefs {
        legacy_git_launch_failure_migration_version: LEGACY_GIT_LAUNCH_FAILURE_MIGRATION_VERSION,
        failed_issues: vec![IssueMonitorFailedIssue {
            issue_number: 42,
            message: "stale disk failure for a live launch".to_string(),
            window_id: Some("tab::stale-agent-42".to_string()),
        }],
        ..IssueMonitorPrefs::default()
    };

    assert!(state.adopt_newer_legacy_git_launch_failure_migration_from_prefs(&disk));
    assert_eq!(state.active_count(), 1);
    let launched = state.inbox_item(42).expect("live launched row");
    assert_eq!(launched.state, MonitorInboxState::Launched);
    assert_eq!(
        launched.launched_window_id.as_deref(),
        Some("tab::agent-42")
    );
    let persisted = state.prefs();
    assert!(persisted
        .failed_issues
        .iter()
        .all(|failed| failed.issue_number != 42));
    assert_eq!(persisted.launched_issues[0].window_id, "tab::agent-42");

    let mut restored = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), persisted);
    scan_issue_monitor_candidates(
        &mut restored,
        &[issue(42, &["bug"])],
        "2026-07-21T00:01:00Z",
    );
    assert_eq!(restored.active_count(), 1);
    let restored_item = restored.inbox_item(42).expect("restored launched row");
    assert_eq!(restored_item.state, MonitorInboxState::Launched);
    assert_eq!(
        restored_item.launched_window_id.as_deref(),
        Some("tab::agent-42")
    );
    assert!(restored
        .prefs()
        .failed_issues
        .iter()
        .all(|failed| failed.issue_number != 42));
}

#[test]
fn prefs_newer_disk_failure_adoption_preserves_real_launch_and_reconciles_unbound_launch() {
    let mut outgoing = IssueMonitorPrefs {
        enabled: true,
        max_active_agents: 3,
        legacy_git_launch_failure_migration_version: 0,
        launched_issues: vec![gwt::IssueMonitorLaunchedIssue {
            issue_number: 42,
            window_id: "tab::agent-42".to_string(),
        }],
        launching_issues: vec![gwt::IssueMonitorLaunchingIssue {
            issue_number: 43,
            claimed_at: Some("2026-07-21T00:00:00Z".to_string()),
        }],
        ..IssueMonitorPrefs::default()
    };
    let disk = IssueMonitorPrefs {
        legacy_git_launch_failure_migration_version: LEGACY_GIT_LAUNCH_FAILURE_MIGRATION_VERSION,
        failed_issues: vec![
            IssueMonitorFailedIssue {
                issue_number: 42,
                message: "stale failure for real launch".to_string(),
                window_id: Some("tab::stale-agent-42".to_string()),
            },
            IssueMonitorFailedIssue {
                issue_number: 43,
                message: "authoritative unbound launch failure".to_string(),
                window_id: None,
            },
            IssueMonitorFailedIssue {
                issue_number: 99,
                message: "unrelated authoritative failure".to_string(),
                window_id: Some("tab::agent-99".to_string()),
            },
        ],
        ..IssueMonitorPrefs::default()
    };

    assert!(outgoing.adopt_newer_legacy_git_launch_failure_migration(&disk));
    assert_eq!(outgoing.launched_issues[0].issue_number, 42);
    assert_eq!(outgoing.launched_issues[0].window_id, "tab::agent-42");
    assert!(outgoing
        .failed_issues
        .iter()
        .all(|failed| failed.issue_number != 42));
    assert!(outgoing
        .launching_issues
        .iter()
        .all(|launching| launching.issue_number != 43));
    assert_eq!(
        outgoing
            .failed_issues
            .iter()
            .map(|failed| failed.issue_number)
            .collect::<Vec<_>>(),
        vec![43, 99]
    );
    assert!(outgoing.launched_issues.iter().all(|launched| {
        outgoing
            .failed_issues
            .iter()
            .all(|failed| failed.issue_number != launched.issue_number)
    }));

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("issue-monitor.json");
    save_issue_monitor_prefs(&path, &outgoing).expect("save adopted prefs");
    let loaded = load_issue_monitor_prefs(&path).expect("reload adopted prefs");
    let mut state = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), loaded);
    scan_issue_monitor_candidates(
        &mut state,
        &[
            issue(42, &["bug"]),
            issue(43, &["bug"]),
            issue(99, &["bug"]),
        ],
        "2026-07-21T00:01:00Z",
    );

    assert_eq!(state.active_count(), 1);
    let launched = state.inbox_item(42).expect("restored real launch");
    assert_eq!(launched.state, MonitorInboxState::Launched);
    assert_eq!(
        launched.launched_window_id.as_deref(),
        Some("tab::agent-42")
    );
    assert_eq!(
        state.inbox_item(43).map(|item| item.state),
        Some(MonitorInboxState::AgentFailed)
    );
    assert!(state
        .prefs()
        .failed_issues
        .iter()
        .all(|failed| failed.issue_number != 42));
}
