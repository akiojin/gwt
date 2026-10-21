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
fn public_snapshot_masks_private_content_without_changing_the_local_report() {
    let dir = tempfile::tempdir().unwrap();
    let store = FeedbackStore::new(dir.path().join("feedback"));
    let mut report = input();
    report.title = "Failure in src/private.rs".into();
    report.operation = "open ./private/config".into();
    report.refusal = concat!(
        "before\r\nPRIVATE_SETTING=\"private value; with spaces\"\r\n",
        "{\"PRIVATE_JSON\": \"json secret\"}\r\n",
        "C:\\Private\\source.rs \\\\server\\private\\source.rs ~/private/file\r\n",
        "```rust\r\nfn confidential_code() {}\r\n"
    )
    .into();
    report.body = "before `inline_secret()` after\n\n    indented_secret()\n\nend".into();
    report.workaround = Some("PRIVATE_OPTION='other secret'".into());
    report.labels = vec!["private/label".into()];
    report.error_record_id = Some("SECRET_ID=hidden-id".into());
    let (record, _) = store
        .submit(report.clone(), context(dir.path()), Utc::now())
        .unwrap();
    let public = record.public_snapshot();
    let rendered = format!("{}\n{}", public.input.title, public.render_issue());
    for hidden in [
        "private.rs",
        "./private",
        "private value",
        "json secret",
        "source.rs",
        "server",
        "~/private",
        "confidential_code",
        "inline_secret",
        "indented_secret",
        "other secret",
        "private/label",
        "hidden-id",
    ] {
        assert!(
            !rendered.contains(hidden),
            "public snapshot exposed {hidden}"
        );
    }
    assert_ne!(public.context.project_root, record.context.project_root);
    assert_eq!(public.context.project_hash, record.context.project_hash);
    assert_eq!(public.marker(), record.marker());
    assert_eq!(
        public.input.refusal.matches("\r\n").count(),
        report.refusal.matches("\r\n").count()
    );
    assert!(public
        .input
        .body
        .starts_with("before [REDACTED CODE] after\n\n"));
    assert!(public.input.body.ends_with("\n\nend"));
    assert_eq!(store.read(&record.id).unwrap().input, report);
}

#[test]
fn public_snapshot_keeps_safe_refusal_bytes_without_truncation() {
    let dir = tempfile::tempdir().unwrap();
    let store = FeedbackStore::new(dir.path().join("feedback"));
    let mut report = input();
    report.refusal = format!(
        "  before\r\nERROR: access denied\r\n{}\tend\r\nafter  ",
        "拒否".repeat(1800)
    );
    let (record, _) = store
        .submit(report.clone(), context(dir.path()), Utc::now())
        .unwrap();
    assert_eq!(
        record.public_snapshot().input.refusal.as_bytes(),
        report.refusal.as_bytes()
    );
}

#[test]
fn public_snapshot_masks_complete_bearer_and_multiline_assignment_values() {
    let dir = tempfile::tempdir().unwrap();
    let store = FeedbackStore::new(dir.path().join("feedback"));
    let mut report = input();
    report.refusal = "before\r\nBearer abc+bearer-secret\r\nPRIVATE_SETTING = \"first-part\\\nmiddle\r\nmultiline-secret\"suffix-secret\r\nUNQUOTED=first unquoted-secret\r\nCONTINUED_LF=first\\\nlf-continuation-secret\r\nCONTINUED_CRLF=first\\\r\ncrlf-continuation-secret\r\nafter  ".into();
    let (record, _) = store
        .submit(report.clone(), context(dir.path()), Utc::now())
        .unwrap();
    let public = record.public_snapshot().input.refusal;
    let leaked: Vec<_> = [
        "bearer-secret",
        "multiline-secret",
        "suffix-secret",
        "unquoted-secret",
        "lf-continuation-secret",
        "crlf-continuation-secret",
    ]
    .into_iter()
    .filter(|secret| public.contains(secret))
    .collect();
    assert!(leaked.is_empty(), "public snapshot exposed {leaked:?}");
    assert!(public.starts_with("before\r\n"));
    assert!(public.ends_with("\r\nafter  "));
    assert_eq!(
        public
            .chars()
            .filter(|c| matches!(c, '\r' | '\n'))
            .collect::<String>(),
        report
            .refusal
            .chars()
            .filter(|c| matches!(c, '\r' | '\n'))
            .collect::<String>()
    );
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
