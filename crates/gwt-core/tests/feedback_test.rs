use std::{fs, path::Path};

use chrono::{TimeDelta, Utc};
use gwt_core::feedback::auto_report::{
    AutoReason, KnownOperation, OwnedAutoPhase, OwnedAutoReason, OwnedCancelResult,
    OwnedCleanupObservation, OwnedExitObservation, SafeAutoReport,
};
use gwt_core::feedback::{
    requires_approval, FeedbackContext, FeedbackInput, FeedbackKind, FeedbackState, FeedbackStore,
};
use gwt_core::test_support::ScopedEnvVar;

#[test]
fn owned_attempt_registration_is_fresh_only_and_rejects_stale_writers() {
    let dir = tempfile::tempdir().unwrap();
    let store = FeedbackStore::new(dir.path().join("feedback"));
    let report = SafeAutoReport::current(KnownOperation::Search);
    let (legacy, _) = store.submit_auto(report.clone(), Utc::now()).unwrap();
    let legacy_path = store.path(&legacy.snapshot.id()).unwrap();
    let legacy_bytes = fs::read(&legacy_path).unwrap();
    let (repeated, fresh) = store.submit_owned_auto(report, Utc::now()).unwrap();
    assert!(!fresh);
    assert!(repeated.owned.is_none());
    assert_eq!(fs::read(legacy_path).unwrap(), legacy_bytes);
    assert!(store
        .begin_owned_attempt(&legacy.snapshot.id(), uuid::Uuid::new_v4(), || Ok(()))
        .is_err());
    let (record, fresh) = store
        .submit_owned_auto(
            SafeAutoReport::current(KnownOperation::VerifyRun),
            Utc::now(),
        )
        .unwrap();
    assert!(fresh);
    assert!(record.owned.as_ref().unwrap().attempt.is_none());
    let id = record.snapshot.id();
    let attempt = uuid::Uuid::new_v4();
    let _owner = store.try_owner_lock(&id).unwrap();
    let sender = store.try_sender_lock(&id).unwrap();
    assert!(store.begin_owned_attempt(&id, attempt, || Ok(())).is_err());
    assert_eq!(store.read_auto(&id).unwrap(), record);
    drop(sender);
    assert!(store
        .begin_owned_attempt(&id, attempt, || Err(std::io::Error::other(
            "not authorized"
        )))
        .is_err());
    assert_eq!(store.read_auto(&id).unwrap(), record);
    let registered = store.begin_owned_attempt(&id, attempt, || Ok(())).unwrap();
    assert_eq!(registered.state, FeedbackState::Sending);
    let metadata = registered.owned.as_ref().unwrap().attempt.as_ref().unwrap();
    assert_eq!(metadata.phase, OwnedAutoPhase::PreDispatch);
    assert_eq!(metadata.exit, OwnedExitObservation::Unknown);
    assert_eq!(metadata.cleanup, OwnedCleanupObservation::Unknown);
    assert!(store.begin_owned_attempt(&id, attempt, || Ok(())).is_err());
    assert!(store.save_auto(&registered).is_err());
    let mut stale = record;
    stale.owned = None;
    assert!(store.save_auto(&stale).is_err());
    assert_eq!(store.read_auto(&id).unwrap(), registered);
    assert_eq!(store.reconcile_owned_auto(&id).unwrap(), registered);
    drop(_owner);
    let orphan = store.reconcile_owned_auto(&id).unwrap();
    assert_eq!(orphan.state, FeedbackState::Uncertain);
    assert_eq!(orphan.error, Some(AutoReason::SenderInterrupted));
    let metadata = orphan.owned.unwrap().attempt.unwrap();
    assert_eq!(metadata.phase, OwnedAutoPhase::PreDispatch);
    assert_eq!(metadata.exit, OwnedExitObservation::Unknown);
    assert_eq!(metadata.cleanup, OwnedCleanupObservation::Unknown);
    assert_eq!(metadata.reason, Some(OwnedAutoReason::OwnerUnavailable));
    assert_eq!(
        store
            .finish_owned_attempt(&id, attempt, Err(AutoReason::AuthUnavailable))
            .unwrap()
            .state,
        FeedbackState::Uncertain
    );
    store
        .cancel_owned_attempt(&id, attempt, OwnedAutoReason::Cancelled)
        .unwrap();
    assert_eq!(
        store.read_auto(&id).unwrap().state,
        FeedbackState::Uncertain
    );
    assert!(store.retry_owned_auto(&id, || Ok(())).is_err());
    let (failed, _) = store
        .submit_owned_auto(
            SafeAutoReport::current(KnownOperation::VerifyPlan),
            Utc::now(),
        )
        .unwrap();
    let failed_id = failed.snapshot.id();
    let failed_attempt = uuid::Uuid::new_v4();
    let owner = store.try_owner_lock(&failed_id).unwrap();
    store
        .begin_owned_attempt(&failed_id, failed_attempt, || Ok(()))
        .unwrap();
    store
        .cancel_owned_attempt(&failed_id, failed_attempt, OwnedAutoReason::WorkDeadline)
        .unwrap();
    drop(owner);
    let orphan = store.reconcile_owned_auto(&failed_id).unwrap();
    assert_eq!(orphan.state, FeedbackState::Uncertain);
    assert_eq!(
        orphan.owned.unwrap().attempt.unwrap().reason,
        Some(OwnedAutoReason::WorkDeadline)
    );
}

#[test]
fn owned_cancel_blocks_dispatch_and_retry_requires_exit_and_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let store = FeedbackStore::new(dir.path().join("feedback"));
    let (record, _) = store
        .submit_owned_auto(SafeAutoReport::current(KnownOperation::Search), Utc::now())
        .unwrap();
    let id = record.snapshot.id();
    let attempt = uuid::Uuid::new_v4();
    let owner = store.try_owner_lock(&id).unwrap();
    store.begin_owned_attempt(&id, attempt, || Ok(())).unwrap();
    assert_eq!(
        store
            .cancel_owned_attempt(&id, attempt, OwnedAutoReason::WorkDeadline)
            .unwrap(),
        OwnedCancelResult::Accepted
    );
    assert_eq!(
        store
            .cancel_owned_attempt(&id, attempt, OwnedAutoReason::WorkDeadline)
            .unwrap(),
        OwnedCancelResult::TooLateOrUnknown
    );
    assert!(store.check_owned_attempt(&id, attempt, || Ok(())).is_err());
    assert!(store.commit_owned_attempt(&id, attempt, || Ok(())).is_err());
    store
        .finish_owned_attempt(&id, attempt, Err(AutoReason::AuthUnavailable))
        .unwrap();
    assert!(store.retry_owned_auto(&id, || Ok(())).is_err());
    drop(owner);
    store
        .observe_owned_exit(
            &id,
            attempt,
            OwnedExitObservation::Confirmed,
            OwnedCleanupObservation::Unknown,
            None,
        )
        .unwrap();
    assert!(store.retry_owned_auto(&id, || Ok(())).is_err());
    store
        .observe_owned_exit(
            &id,
            attempt,
            OwnedExitObservation::Confirmed,
            OwnedCleanupObservation::Confirmed,
            None,
        )
        .unwrap();
    assert_eq!(
        store.reconcile_owned_auto(&id).unwrap().state,
        FeedbackState::Failed
    );
    let retried = store.retry_owned_auto(&id, || Ok(())).unwrap();
    assert_eq!(retried.snapshot, record.snapshot);
    assert_eq!(retried.state, FeedbackState::Queued);
    assert!(retried.owned.unwrap().attempt.is_none());
    let _owner = store.try_owner_lock(&id).unwrap();
    store
        .begin_owned_attempt(&id, uuid::Uuid::new_v4(), || Ok(()))
        .unwrap();
    assert!(store.finish_owned_attempt(&id, attempt, Ok(7)).is_err());
    assert!(store
        .observe_owned_exit(
            &id,
            attempt,
            OwnedExitObservation::Confirmed,
            OwnedCleanupObservation::Confirmed,
            None
        )
        .is_err());
}

#[test]
fn owned_commit_is_durable_and_remote_success_survives_unknown_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let storage = dir.path().join("feedback");
    let store = FeedbackStore::new(storage.clone());
    let (record, _) = store
        .submit_owned_auto(SafeAutoReport::current(KnownOperation::Search), Utc::now())
        .unwrap();
    let id = record.snapshot.id();
    let attempt = uuid::Uuid::new_v4();
    let _owner = store.try_owner_lock(&id).unwrap();
    let receipt = store.path(&id).unwrap();
    let backup = dir.path().join("receipt-backup.json");
    let block_save = || {
        // Keep the live owner/sender/dispatch lock files in their directory.
        // A failed setup must not masquerade as a publication failure.
        fs::rename(&receipt, &backup).expect("back up the closed receipt");
        fs::create_dir(&receipt).expect("obstruct the receipt publication target");
        Ok(())
    };
    assert!(store.begin_owned_attempt(&id, attempt, block_save).is_err());
    fs::remove_dir(&receipt).unwrap();
    fs::rename(&backup, &receipt).unwrap();
    assert_eq!(store.read_auto(&id).unwrap(), record);
    store.begin_owned_attempt(&id, attempt, || Ok(())).unwrap();
    store.check_owned_attempt(&id, attempt, || Ok(())).unwrap();
    let pre_dispatch = store.read_auto(&id).unwrap();
    assert!(store
        .commit_owned_attempt(&id, attempt, block_save)
        .is_err());
    fs::remove_dir(&receipt).unwrap();
    fs::rename(&backup, &receipt).unwrap();
    assert_eq!(store.read_auto(&id).unwrap(), pre_dispatch);
    store.commit_owned_attempt(&id, attempt, || Ok(())).unwrap();
    assert_eq!(
        store
            .cancel_owned_attempt(&id, attempt, OwnedAutoReason::Cancelled)
            .unwrap(),
        OwnedCancelResult::TooLateOrUnknown
    );
    let uncertain = store
        .finish_owned_attempt(&id, attempt, Err(AutoReason::PreSubmitFailed))
        .unwrap();
    assert_eq!(uncertain.state, FeedbackState::Uncertain);
    assert!(store.retry_owned_auto(&id, || Ok(())).is_err());
    let sent = store.finish_owned_attempt(&id, attempt, Ok(42)).unwrap();
    assert_eq!(sent.state, FeedbackState::Sent);
    assert_eq!(
        sent.owned.unwrap().attempt.unwrap().cleanup,
        OwnedCleanupObservation::Unknown
    );
    assert_eq!(
        store
            .observe_owned_exit(
                &id,
                attempt,
                OwnedExitObservation::Unknown,
                OwnedCleanupObservation::Unknown,
                Some(OwnedAutoReason::CleanupUnobserved)
            )
            .unwrap()
            .state,
        FeedbackState::Sent
    );
    let observed = store
        .observe_owned_exit(
            &id,
            attempt,
            OwnedExitObservation::Confirmed,
            OwnedCleanupObservation::Confirmed,
            None,
        )
        .unwrap();
    assert_eq!(observed.state, FeedbackState::Sent);
    assert_eq!(observed.issue_number, Some(42));
    assert_eq!(
        store
            .finish_owned_attempt(&id, attempt, Err(AutoReason::SenderInterrupted))
            .unwrap(),
        observed
    );
    let (unfinished, _) = store
        .submit_owned_auto(
            SafeAutoReport::current(KnownOperation::HookHealth),
            Utc::now(),
        )
        .unwrap();
    let unfinished_id = unfinished.snapshot.id();
    let _owner = store.try_owner_lock(&unfinished_id).unwrap();
    let unfinished_attempt = uuid::Uuid::new_v4();
    store
        .begin_owned_attempt(&unfinished_id, unfinished_attempt, || Ok(()))
        .unwrap();
    let fenced = store
        .observe_owned_exit(
            &unfinished_id,
            unfinished_attempt,
            OwnedExitObservation::Unknown,
            OwnedCleanupObservation::Unknown,
            Some(OwnedAutoReason::ExitDeadline),
        )
        .unwrap();
    assert_eq!(fenced.state, FeedbackState::Uncertain);
    assert_eq!(fenced.error, Some(AutoReason::SenderInterrupted));
    let metadata = fenced.owned.unwrap().attempt.unwrap();
    assert_eq!(metadata.phase, OwnedAutoPhase::PreDispatch);
    assert_eq!(metadata.exit, OwnedExitObservation::Unknown);
    assert_eq!(metadata.cleanup, OwnedCleanupObservation::Unknown);
}

#[test]
fn owned_retry_accepts_confirmed_no_sender_and_initial_withheld_receipts() {
    let dir = tempfile::tempdir().unwrap();
    let store = FeedbackStore::new(dir.path().join("feedback"));
    let (record, _) = store
        .submit_owned_auto_withheld(
            SafeAutoReport::current(KnownOperation::Search),
            Utc::now(),
            AutoReason::PolicyUnavailable,
        )
        .unwrap();
    let id = record.snapshot.id();
    assert_eq!(store.reconcile_owned_auto(&id).unwrap(), record);
    let queued = store.retry_owned_auto(&id, || Ok(())).unwrap();
    assert_eq!(queued.state, FeedbackState::Queued);
    let owner = store.try_owner_lock(&id).unwrap();
    let attempt = uuid::Uuid::new_v4();
    store.begin_owned_attempt(&id, attempt, || Ok(())).unwrap();
    store
        .observe_owned_exit(
            &id,
            attempt,
            OwnedExitObservation::NotStarted,
            OwnedCleanupObservation::Confirmed,
            Some(OwnedAutoReason::WorkerUnavailable),
        )
        .unwrap();
    assert!(store.check_owned_attempt(&id, attempt, || Ok(())).is_err());
    assert!(store.commit_owned_attempt(&id, attempt, || Ok(())).is_err());
    store
        .finish_owned_attempt(&id, attempt, Err(AutoReason::WorkerUnavailable))
        .unwrap();
    drop(owner);
    let pending = store.retry_owned_auto(&id, || Ok(())).unwrap();
    assert_eq!(pending.state, FeedbackState::Queued);
    let orphan = store.reconcile_owned_auto(&id).unwrap();
    assert_eq!(orphan.state, FeedbackState::Uncertain);
    assert!(orphan.owned.unwrap().attempt.is_none());
    assert!(store
        .begin_owned_attempt(&id, uuid::Uuid::new_v4(), || Ok(()))
        .is_err());
}

#[test]
fn owned_receipt_schema_is_versioned_and_preserves_legacy_serialization() {
    use gwt_core::feedback::auto_report::{
        AutoFeedbackRecord, AutoReason, KnownOperation, SafeAutoReport, SafeAutoSnapshot,
    };
    // Frozen pre-owned reader: unknown local ownership must never be ignored.
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    #[allow(dead_code)]
    struct LegacyAutoRecord {
        mode: String,
        snapshot: SafeAutoSnapshot,
        created_at: chrono::DateTime<Utc>,
        state: FeedbackState,
        issue_number: Option<u64>,
        error: Option<AutoReason>,
    }
    let dir = tempfile::tempdir().unwrap();
    let store = FeedbackStore::new(dir.path().join("feedback"));
    let (legacy, _) = store
        .submit_auto(SafeAutoReport::current(KnownOperation::Search), Utc::now())
        .unwrap();
    let path = store.path(&legacy.snapshot.id()).unwrap();
    let bytes = fs::read(&path).unwrap();
    let decoded: AutoFeedbackRecord = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(serde_json::to_vec_pretty(&decoded).unwrap(), bytes);
    assert!(serde_json::from_slice::<LegacyAutoRecord>(&bytes).is_ok());
    let mut owned: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    owned["owned"] = serde_json::json!({"version":"v1","attempt":null});
    fs::write(&path, serde_json::to_vec(&owned).unwrap()).unwrap();
    let pending = store.read_auto(&legacy.snapshot.id()).unwrap();
    assert_eq!(pending.snapshot, legacy.snapshot);
    assert!(serde_json::from_value::<LegacyAutoRecord>(owned.clone()).is_err());
    assert!(store.read(&legacy.snapshot.id()).is_err());
    for invalid in [
        serde_json::Value::Null,
        serde_json::json!({"version":"v2","attempt":null}),
        serde_json::json!({"version":"v1","attempt":null,"token":"forbidden"}),
    ] {
        owned["owned"] = invalid;
        fs::write(&path, serde_json::to_vec(&owned).unwrap()).unwrap();
        assert!(store.read_entry(&legacy.snapshot.id()).is_err());
    }
}

#[test]
fn owned_receipt_requires_consistent_exit_and_cleanup_observations() {
    use gwt_core::feedback::auto_report::{KnownOperation, SafeAutoReport};
    let dir = tempfile::tempdir().unwrap();
    let store = FeedbackStore::new(dir.path().join("feedback"));
    let (record, _) = store
        .submit_auto(SafeAutoReport::current(KnownOperation::Search), Utc::now())
        .unwrap();
    let path = store.path(&record.snapshot.id()).unwrap();
    let mut value = serde_json::to_value(&record).unwrap();
    value["state"] = serde_json::json!("sending");
    value["owned"] = serde_json::json!({
        "version":"v1", "attempt":{
            "id":uuid::Uuid::new_v4().to_string(), "origin":"inherited_cli_environment",
            "phase":"pre_dispatch", "exit":"unknown", "cleanup":"unknown", "reason":null
        }
    });
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(store.read_auto(&record.snapshot.id()).is_ok());
    for (field, invalid) in [
        ("id", serde_json::json!("invalid-id")),
        ("origin", serde_json::json!("arbitrary_environment")),
        ("cleanup", serde_json::json!("confirmed")),
    ] {
        let mut corrupted = value.clone();
        corrupted["owned"]["attempt"][field] = invalid;
        fs::write(&path, serde_json::to_vec(&corrupted).unwrap()).unwrap();
        assert!(store.read_entry(&record.snapshot.id()).is_err());
    }
    value["owned"]["attempt"]["phase"] = serde_json::json!("dispatch_committed");
    value["owned"]["attempt"]["exit"] = serde_json::json!("not_started");
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(store.read_entry(&record.snapshot.id()).is_err());
}

#[test]
fn auto_receipts_contain_only_safe_snapshots_and_dedupe_uncertain_indefinitely() {
    use gwt_core::feedback::auto_report::{AutoReason, KnownOperation, SafeAutoReport};
    let dir = tempfile::tempdir().unwrap();
    let store = FeedbackStore::new(dir.path().join("feedback"));
    let report = SafeAutoReport::current(KnownOperation::WorkspaceUpdate);
    let now = Utc::now();
    let (mut record, fresh) = store.submit_auto(report.clone(), now).unwrap();
    assert!(fresh);
    assert_eq!(record.state, FeedbackState::Queued);
    let original_snapshot = record.snapshot.clone();
    record.state = FeedbackState::Uncertain;
    record.error = Some(AutoReason::RemoteOutcomeUnknown);
    store.save_auto(&record).unwrap();
    let (repeated, fresh) = store.submit_auto(report, now + TimeDelta::days(2)).unwrap();
    assert!(!fresh);
    assert_eq!(repeated, record);
    assert_eq!(repeated.snapshot, original_snapshot);
    let (unavailable, fresh) = store
        .submit_auto_withheld(
            original_snapshot.report().clone(),
            now,
            AutoReason::PolicyUnavailable,
        )
        .unwrap();
    assert!(!fresh);
    assert_eq!(
        unavailable, record,
        "unavailable intake cannot overwrite an uncertain receipt"
    );
    assert!(
        store.read(&record.snapshot.id()).is_err(),
        "Auto is never a legacy raw record"
    );
    let bytes = fs::read(store.path(&record.snapshot.id()).unwrap()).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["mode"], "auto");
    assert!(value.get("input").is_none());
    assert!(value.get("context").is_none());
    let mut corrupted = value;
    corrupted["snapshot"]["report"]["operation"] = serde_json::json!("plain private content");
    fs::write(
        store.path(&record.snapshot.id()).unwrap(),
        serde_json::to_vec(&corrupted).unwrap(),
    )
    .unwrap();
    assert_eq!(
        store
            .read_auto(&record.snapshot.id())
            .unwrap_err()
            .to_string(),
        "invalid feedback receipt"
    );
}

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
    assert!(!saved.render_issue().contains("source-project"));
    assert!(!saved.render_issue().contains("error-123"));
}

#[test]
fn public_snapshot_masks_private_content_without_changing_the_local_report() {
    let dir = tempfile::tempdir().unwrap();
    let store = FeedbackStore::new(dir.path().join("feedback"));
    let mut report = input();
    report.title = "Failure in src/private.rs".into();
    report.operation = "open ./private/config".into();
    report.refusal = concat!(
        "before\r\ngithub_pat_synthetic_0123456789ABCDEF\r\n",
        "sk-proj-synthetic_0123456789-ABCDEF\r\n",
        "PRIVATE_SETTING=\"private value; with spaces\"\r\n",
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
        "github_pat_synthetic_0123456789ABCDEF",
        "sk-proj-synthetic_0123456789-ABCDEF",
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
    assert!(public
        .input
        .refusal
        .starts_with("before\r\n***redacted***\r\n***redacted***\r\n"));
    assert!(!public.render_issue().contains("project_root"));
    assert!(!public.render_issue().contains("project_hash"));
    assert_eq!(public.marker(), record.marker());
    assert_eq!(
        public.input.refusal.matches("\r\n").count(),
        report.refusal.matches("\r\n").count()
    );
    assert!(public.input.body.is_empty());
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
fn public_report_contains_only_gwt_diagnostics_and_no_source_identifiers() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("private-product");
    repo(
        &source,
        "https://github.com/private-team/private-product.git",
    );
    let store = FeedbackStore::new(dir.path().join("feedback"));
    let mut report = input();
    report.title = "workspace.update refuses private-product".into();
    report.refusal = format!("workspace.update: authority_mismatch (http_status=409) at {} commit a123456789abcdef0123456789abcdef012345678", source.display());
    report.reproduction = "In private-product branch feature/secret-ticket, issue #4781, HEAD a123456789abcdef0123456789abcdef012345678, run `workspace.update`; exit_code=64".into();
    report.workaround = Some("private-product: run `execution.status`".into());
    report.body = "Private monorepo uses a confidential framework".into();
    let original = report.clone();
    let (record, _) = store.submit(report, context(&source), Utc::now()).unwrap();
    let public = record.public_snapshot();
    let rendered = format!("{}\n{}", public.input.title, public.render_issue());
    for forbidden in [
        "project_hash",
        "project_root",
        "active_agents",
        "source-project",
        "private-product",
        "private-team",
        "feature/secret-ticket",
        "4781",
        "a123456789abcdef0123456789abcdef012345678",
        "Private monorepo",
        "error-123",
    ] {
        assert!(
            !rendered.contains(forbidden),
            "outbound report exposed {forbidden}"
        );
    }
    assert!(rendered.contains("http_status=409"));
    assert!(rendered.contains("authority_mismatch"));
    assert!(rendered.contains("workspace.update"));
    assert!(rendered.contains("execution.status"));
    assert!(rendered.contains("exit_code=64"));
    assert!(rendered.contains("gwtd_version"));
    assert_eq!(store.read(&record.id).unwrap().input, original);
}

#[test]
fn approval_explains_removed_categories_and_unreadable_mask_ratio() {
    let dir = tempfile::tempdir().unwrap();
    let store = FeedbackStore::new(dir.path().join("feedback"));
    let mut report = input();
    report.title = "failure".into();
    report.operation = "workspace.update".into();
    report.reproduction = "unconfirmed".into();
    report.refusal = "/private/source.rs ".repeat(100);
    let (record, _) = store
        .submit(report, context(dir.path()), Utc::now())
        .unwrap();
    let summary = record.public_snapshot().privacy_summary();
    assert!(summary.contains("file paths"));
    assert!(summary.contains("Privacy masks:"));
    assert!(summary.contains("may be unreadable"));
    assert!(summary.contains("Unknown project-specific names may remain"));
    assert!(!summary.contains("/private/source.rs"));
}

#[test]
fn masking_preserves_gwt_diagnostic_literals_and_large_numeric_values() {
    let dir = tempfile::tempdir().unwrap();
    let store = FeedbackStore::new(dir.path().join("feedback"));
    let mut report = input();
    report.refusal = "`workspace.update` rejected: timeout_ns=1000000000\n(http_status=409), code=authority_mismatch; exit_code=64\n`execution_id=366a102c-4096-4185-aa5b-f07683fed026`\n`state=active`\n```text\ngwtd: operation=workspace.update failed\nhttp_status=409\ncode=authority_mismatch\n```\n".into();
    let (record, _) = store
        .submit(report.clone(), context(dir.path()), Utc::now())
        .unwrap();
    assert_eq!(record.public_snapshot().input.refusal, report.refusal);
}

#[test]
fn masking_removes_source_blocks_and_mixed_case_environment_values() {
    let dir = tempfile::tempdir().unwrap();
    let store = FeedbackStore::new(dir.path().join("feedback"));
    let mut report = input();
    report.refusal = "before\n```python\ncustomer_discount = 0.137\n```\n`customer_discount = 0.137`\nprivate_setting=confidential-value\nPrivateSetting=second-secret\nhttp_status=409 private_setting=third-secret\nPRIVATE_PIN=123456\nafter".into();
    let (record, _) = store
        .submit(report, context(dir.path()), Utc::now())
        .unwrap();
    let public = record.public_snapshot();
    for hidden in [
        "customer_discount",
        "0.137",
        "confidential-value",
        "second-secret",
        "third-secret",
        "123456",
    ] {
        assert!(!public.render_issue().contains(hidden));
    }
    assert!(public.input.refusal.starts_with("before\n"));
    assert!(public.input.refusal.ends_with("\nafter"));
    assert!(public.input.refusal.contains("http_status=409"));
}

#[test]
fn masking_handles_punctuation_in_repository_names_and_other_branch_references() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join(".private-product");
    repo(
        &root,
        "https://github.com/private-team/.private-product.git",
    );
    let store = FeedbackStore::new(dir.path().join("feedback"));
    let mut report = input();
    report.reproduction =
        "In .private-product on branch confidential-release, call `execution.status`".into();
    let (record, _) = store.submit(report, context(&root), Utc::now()).unwrap();
    let text = record.public_snapshot().render_issue();
    assert!(!text.contains(".private-product"));
    assert!(!text.contains("confidential-release"));
    assert!(text.contains("execution.status"));
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
    // Text-identical reports must not lose new version/OS/evidence identity.
    let mut without_evidence_id = original.clone();
    without_evidence_id.error_record_id = None;
    for (report, environment) in [
        (
            original.clone(),
            FeedbackContext {
                gwt_version: "2.0.0".into(),
                ..context(dir.path())
            },
        ),
        (
            original.clone(),
            FeedbackContext {
                gwtd_version: "2.0.0".into(),
                ..context(dir.path())
            },
        ),
        (
            original.clone(),
            FeedbackContext {
                os: "different-os".into(),
                ..context(dir.path())
            },
        ),
        (without_evidence_id, context(dir.path())),
    ] {
        let (additional, fresh) = store
            .submit(report.clone(), environment.clone(), now)
            .unwrap();
        assert!(
            fresh,
            "new context/evidence must not reuse an older receipt"
        );
        assert_ne!(additional.id, first.id);
        assert_eq!(additional.context, environment);
        assert_eq!(additional.input, report);
        let (repeat, fresh) = store
            .submit(report, environment, now + TimeDelta::minutes(15))
            .unwrap();
        assert!(
            !fresh,
            "complete duplicates still share one receipt within one hour"
        );
        assert_eq!(repeat.id, additional.id);
    }
    assert_eq!(store.read(&first.id).unwrap().input, original);
}
