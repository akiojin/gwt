use std::{fs, path::Path};

use chrono::{TimeDelta, Utc};
use gwt_core::feedback::{
    requires_approval, FeedbackContext, FeedbackInput, FeedbackKind, FeedbackState, FeedbackStore,
};
use gwt_core::test_support::ScopedEnvVar;

fn repo(path: &Path, origin: &str) {
    fs::create_dir_all(path.join(".git")).unwrap();
    fs::write(
        path.join(".git/config"),
        format!("[remote \"origin\"]\nurl = {origin}\n"),
    )
    .unwrap();
}

fn input() -> FeedbackInput {
    FeedbackInput {
        title: "workspace update refuses the current session".into(),
        kind: FeedbackKind::Bug,
        operation: "workspace.update".into(),
        refusal: format!("  original\r\n{}\n```\n終端  ", "拒否".repeat(1800)),
        reproduction: "未確定".into(),
        workaround: None,
        body: "Affects the normal workflow.".into(),
        error_record_id: Some("error-123".into()),
        labels: vec![],
    }
}

fn context(path: &Path) -> FeedbackContext {
    FeedbackContext {
        project_root: path.to_path_buf(),
        project_hash: "source-project".into(),
        gwt_version: "1.0.0".into(),
        gwtd_version: "1.0.0".into(),
        os: "test-os".into(),
        active_agents: 3,
    }
}

#[test]
fn approval_uses_local_repository_identity_despite_environment_overrides() {
    let _guard = gwt_core::test_support::env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let global = dir.path().join("global.gitconfig");
    fs::write(
        &global,
        "[url \"https://github.com/akiojin/gwt\"]\ninsteadOf = https://github.com/acme/app.git\n",
    )
    .unwrap();
    let _global = ScopedEnvVar::set("GIT_CONFIG_GLOBAL", global.to_str().unwrap());
    let _override = ScopedEnvVar::set("GWT_GITHUB_REPO", "akiojin/gwt");
    let _remote = ScopedEnvVar::set("GWT_REMOTE", "upstream");
    repo(dir.path(), "https://github.com/acme/app.git");
    assert!(requires_approval(dir.path()));
    repo(dir.path(), "git@github.com:akiojin/gwt.git");
    assert!(!requires_approval(dir.path()));
    assert!(requires_approval(&dir.path().join("unknown")));
}

#[test]
fn linked_worktree_uses_its_raw_common_origin_for_approval() {
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("main");
    repo(&main, "https://github.com/akiojin/gwt.git");
    let git_dir = main.join(".git/worktrees/linked");
    fs::create_dir_all(&git_dir).unwrap();
    fs::write(git_dir.join("commondir"), "../..\n").unwrap();
    let linked = dir.path().join("linked");
    fs::create_dir_all(&linked).unwrap();
    fs::write(
        linked.join(".git"),
        format!("gitdir: {}\n", git_dir.display()),
    )
    .unwrap();
    assert!(!requires_approval(&linked));
}

#[test]
fn a_github_looking_path_never_grants_the_upstream_exemption() {
    let dir = tempfile::tempdir().unwrap();
    repo(
        dir.path(),
        "https://evil.example/@github.com/akiojin/gwt.git",
    );
    assert!(requires_approval(dir.path()));
}

#[test]
fn failed_report_and_issue_body_preserve_the_entire_refusal() {
    let dir = tempfile::tempdir().unwrap();
    let store = FeedbackStore::new(dir.path().join("feedback"));
    let report = input();
    let (mut record, _) = store
        .submit(report.clone(), context(dir.path()), Utc::now())
        .unwrap();
    record.state = FeedbackState::Failed;
    record.error = Some("akiojin/gwt: permission denied".into());
    store.save(&record).unwrap();
    let saved = store.read(&record.id).unwrap();
    assert_eq!(saved.input, report);
    assert_eq!(saved.state, FeedbackState::Failed);
    assert!(saved.render_issue().contains(&report.refusal));
    assert!(saved.render_issue().contains("source-project"));
    assert!(saved.render_issue().contains("error-123"));
}

#[test]
fn repeat_intake_reuses_one_receipt_within_an_hour_and_retains_old_reports() {
    let dir = tempfile::tempdir().unwrap();
    let store = FeedbackStore::new(dir.path().join("feedback"));
    let now = Utc::now();
    let (first, new) = store.submit(input(), context(dir.path()), now).unwrap();
    assert!(new);
    let (repeat, new) = store
        .submit(input(), context(dir.path()), now + TimeDelta::minutes(15))
        .unwrap();
    assert!(!new);
    assert_eq!(first.id, repeat.id);
    let (later, new) = store
        .submit(input(), context(dir.path()), now + TimeDelta::hours(2))
        .unwrap();
    assert!(new);
    assert_ne!(first.id, later.id);
    assert_eq!(store.read(&first.id).unwrap().input, first.input);
    assert!(store.read("../escape").is_err());
}

#[test]
fn repeated_problem_with_new_evidence_retains_both_reports() {
    let dir = tempfile::tempdir().unwrap();
    let store = FeedbackStore::new(dir.path().join("feedback"));
    let now = Utc::now();
    let original = input();
    let (first, _) = store
        .submit(original.clone(), context(dir.path()), now)
        .unwrap();
    let mut updated = original.clone();
    updated.reproduction = "Confirmed after relaunch".into();
    updated.workaround = Some("retry from the assigned checkout".into());
    updated.body.push_str("\nNew diagnostic evidence");
    let (second, fresh) = store
        .submit(updated.clone(), context(dir.path()), now)
        .unwrap();
    assert!(fresh);
    assert_ne!(first.id, second.id);
    assert_eq!(store.read(&first.id).unwrap().input, original);
    assert_eq!(store.read(&second.id).unwrap().input, updated);
}
