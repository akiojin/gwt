use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Barrier, Mutex,
    },
};

use chrono::{DateTime, Utc};
use gwt_agent::{AgentId, ExecutionBindingIdentity, Session, SessionExecutionBinding};
use gwt_core::{
    coordination::{
        BoardAudienceScope, BoardEntry, BoardEntryKind, BoardExactAppendConflict,
        BoardExactAppendConflictKind, BoardExactAppendDisposition, BoardExactAppendError,
        BoardExactAppendReceipt, BoardExactAppendResult, BoardHistoryPage, BoardProvider,
        BoardRecoveryCapability, BoardWorktreeForm, CoordinationSnapshot, LocalProvider,
    },
    recovery::{RecoveryProvider, RecoveryState, RecoveryStore},
    test_support::ScopedGwtHome,
    Result,
};

use crate::recovery_delivery::{
    classify_worktree_origin, deliver_with, resolve_delivery_authority, RecoveryDeliveryAuthority,
    RecoveryDeliveryInput, RecoveryDeliveryRoute, RecoveryDeliveryState,
};

#[derive(Clone)]
enum ExactBehavior {
    Local,
    ResponseLossOnce(Arc<AtomicBool>),
    ReceiptEntryMismatch,
    ReceiptDigestMismatch,
    RefreshFailure,
    Conflict(BoardExactAppendConflictKind),
    Barrier(Arc<Barrier>),
}

type PendingProbe = Arc<dyn Fn(&BoardEntry) + Send + Sync>;

#[derive(Clone)]
struct ExactSpyProvider {
    capability: BoardRecoveryCapability,
    exact_calls: Arc<AtomicUsize>,
    normal_calls: Arc<AtomicUsize>,
    behavior: ExactBehavior,
    pending_probe: Option<PendingProbe>,
}

impl ExactSpyProvider {
    fn exact(behavior: ExactBehavior) -> Self {
        Self {
            capability: BoardRecoveryCapability::ExactAcknowledgement,
            exact_calls: Arc::new(AtomicUsize::new(0)),
            normal_calls: Arc::new(AtomicUsize::new(0)),
            behavior,
            pending_probe: None,
        }
    }

    fn unsupported() -> Self {
        Self {
            capability: BoardRecoveryCapability::Unsupported,
            ..Self::exact(ExactBehavior::Local)
        }
    }

    fn with_pending_probe(mut self, probe: impl Fn(&BoardEntry) + Send + Sync + 'static) -> Self {
        self.pending_probe = Some(Arc::new(probe));
        self
    }
}

impl BoardProvider for ExactSpyProvider {
    fn post_entry(&self, worktree_root: &Path, entry: BoardEntry) -> Result<CoordinationSnapshot> {
        self.normal_calls.fetch_add(1, Ordering::SeqCst);
        LocalProvider.post_entry(worktree_root, entry)
    }

    fn recovery_capability(&self) -> BoardRecoveryCapability {
        self.capability
    }

    fn post_recovery_entry_exact(
        &self,
        worktree_root: &Path,
        entry: BoardEntry,
    ) -> BoardExactAppendResult<BoardExactAppendReceipt> {
        self.exact_calls.fetch_add(1, Ordering::SeqCst);
        if let Some(probe) = self.pending_probe.as_ref() {
            probe(&entry);
        }
        match &self.behavior {
            ExactBehavior::Local => LocalProvider.post_recovery_entry_exact(worktree_root, entry),
            ExactBehavior::ResponseLossOnce(lost) => {
                let receipt = LocalProvider.post_recovery_entry_exact(worktree_root, entry)?;
                if !lost.swap(true, Ordering::SeqCst) {
                    Err(BoardExactAppendError::Storage(gwt_core::GwtError::Other(
                        "simulated response loss".to_string(),
                    )))
                } else {
                    Ok(receipt)
                }
            }
            ExactBehavior::ReceiptEntryMismatch => Ok(BoardExactAppendReceipt {
                entry_id: "foreign-entry".to_string(),
                payload_digest: gwt_core::coordination::board_entry_payload_digest(&entry)
                    .expect("payload digest"),
                disposition: BoardExactAppendDisposition::Appended,
                refresh_error: None,
            }),
            ExactBehavior::ReceiptDigestMismatch => Ok(BoardExactAppendReceipt {
                entry_id: entry.id,
                payload_digest: "sha256-v1:foreign".to_string(),
                disposition: BoardExactAppendDisposition::Appended,
                refresh_error: None,
            }),
            ExactBehavior::RefreshFailure => Ok(BoardExactAppendReceipt {
                entry_id: entry.id.clone(),
                payload_digest: gwt_core::coordination::board_entry_payload_digest(&entry)
                    .expect("payload digest"),
                disposition: BoardExactAppendDisposition::Appended,
                refresh_error: Some(
                    "/Users/private/token=super-secret snapshot refresh failed".to_string(),
                ),
            }),
            ExactBehavior::Conflict(kind) => {
                Err(BoardExactAppendError::Conflict(BoardExactAppendConflict {
                    entry_id: entry.id.clone(),
                    kind: *kind,
                    attempted_payload_digest: gwt_core::coordination::board_entry_payload_digest(
                        &entry,
                    )
                    .expect("payload digest"),
                    existing_payload_digests: vec!["sha256-v1:existing".to_string()],
                }))
            }
            ExactBehavior::Barrier(barrier) => {
                barrier.wait();
                LocalProvider.post_recovery_entry_exact(worktree_root, entry)
            }
        }
    }

    fn load_snapshot(&self, worktree_root: &Path) -> Result<CoordinationSnapshot> {
        LocalProvider.load_snapshot(worktree_root)
    }

    fn load_snapshot_for_scope(
        &self,
        worktree_root: &Path,
        scope: &BoardAudienceScope,
    ) -> Result<CoordinationSnapshot> {
        LocalProvider.load_snapshot_for_scope(worktree_root, scope)
    }

    fn load_entries_since(
        &self,
        worktree_root: &Path,
        since: DateTime<Utc>,
    ) -> Result<Vec<BoardEntry>> {
        LocalProvider.load_entries_since(worktree_root, since)
    }

    fn load_entries_since_for_scope(
        &self,
        worktree_root: &Path,
        since: DateTime<Utc>,
        scope: &BoardAudienceScope,
    ) -> Result<Vec<BoardEntry>> {
        LocalProvider.load_entries_since_for_scope(worktree_root, since, scope)
    }

    fn has_recent_post_by(
        &self,
        worktree_root: &Path,
        author: &str,
        kind: &BoardEntryKind,
        within: chrono::Duration,
    ) -> Result<bool> {
        LocalProvider.has_recent_post_by(worktree_root, author, kind, within)
    }

    fn board_entry_exists(&self, worktree_root: &Path, entry_id: &str) -> Result<bool> {
        LocalProvider.board_entry_exists(worktree_root, entry_id)
    }

    fn load_entries_before(
        &self,
        worktree_root: &Path,
        before_entry_id: Option<&str>,
        limit: usize,
    ) -> Result<BoardHistoryPage> {
        LocalProvider.load_entries_before(worktree_root, before_entry_id, limit)
    }

    fn load_entries_before_for_scope(
        &self,
        worktree_root: &Path,
        before_entry_id: Option<&str>,
        limit: usize,
        scope: &BoardAudienceScope,
    ) -> Result<BoardHistoryPage> {
        LocalProvider.load_entries_before_for_scope(worktree_root, before_entry_id, limit, scope)
    }
}

fn input(body: &str) -> RecoveryDeliveryInput {
    RecoveryDeliveryInput {
        kind: BoardEntryKind::Status,
        body: body.to_string(),
        title: None,
        title_summary: None,
        parent: None,
        topics: Vec::new(),
        owners: Vec::new(),
        targets: Vec::new(),
        mentions: Vec::new(),
        workspace_audience: Vec::new(),
        broadcast: true,
    }
}

fn authority() -> RecoveryDeliveryAuthority {
    RecoveryDeliveryAuthority::new(
        "project-1",
        "session-1",
        "Codex",
        "codex",
        Some("work/issue-1921".to_string()),
        BoardWorktreeForm::BranchBacked,
    )
    .expect("authority")
}

fn deliver(
    repo: &Path,
    store_root: &Path,
    provider: ExactSpyProvider,
    intent_id: &str,
    body: &str,
) -> crate::recovery_delivery::RecoveryDeliveryReport {
    deliver_with(
        repo,
        intent_id,
        input(body),
        |_| RecoveryDeliveryRoute::new(RecoveryProvider::Local, Box::new(provider)),
        |_| Ok(authority()),
        |_, authority| RecoveryStore::new(store_root, authority.authority().clone()),
    )
}

#[test]
fn unsupported_preflight_runs_before_authority_store_or_provider_effects() {
    let temp = tempfile::tempdir().expect("tempdir");
    let provider = ExactSpyProvider::unsupported();
    let exact_calls = provider.exact_calls.clone();
    let normal_calls = provider.normal_calls.clone();
    let authority_calls = Arc::new(AtomicUsize::new(0));
    let store_calls = Arc::new(AtomicUsize::new(0));
    let unexpected_store = temp.path().join("store");
    let report = deliver_with(
        temp.path(),
        "intent-unsupported",
        input("must not persist"),
        |_| RecoveryDeliveryRoute::new(RecoveryProvider::Slack, Box::new(provider)),
        {
            let calls = authority_calls.clone();
            move |_| {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(authority())
            }
        },
        {
            let calls = store_calls.clone();
            let unexpected_store = unexpected_store.clone();
            move |_, authority| {
                calls.fetch_add(1, Ordering::SeqCst);
                RecoveryStore::new(unexpected_store, authority.authority().clone())
            }
        },
    );

    assert_eq!(report.state, RecoveryDeliveryState::Refused);
    assert_eq!(report.code, "unsupported_provider");
    assert_eq!(authority_calls.load(Ordering::SeqCst), 0);
    assert_eq!(store_calls.load(Ordering::SeqCst), 0);
    assert_eq!(exact_calls.load(Ordering::SeqCst), 0);
    assert_eq!(normal_calls.load(Ordering::SeqCst), 0);
    assert!(!unexpected_store.exists());
}

#[test]
fn unsafe_payload_is_refused_before_store_or_provider_effects() {
    let temp = tempfile::tempdir().expect("tempdir");
    let provider = ExactSpyProvider::exact(ExactBehavior::Local);
    let exact_calls = provider.exact_calls.clone();
    let store_calls = Arc::new(AtomicUsize::new(0));
    let unexpected_store = temp.path().join("store");
    let report = deliver_with(
        temp.path(),
        "intent-unsafe",
        input("hidden reasoning must never be persisted"),
        |_| RecoveryDeliveryRoute::new(RecoveryProvider::Local, Box::new(provider)),
        |_| Ok(authority()),
        {
            let calls = store_calls.clone();
            let unexpected_store = unexpected_store.clone();
            move |_, authority| {
                calls.fetch_add(1, Ordering::SeqCst);
                RecoveryStore::new(unexpected_store, authority.authority().clone())
            }
        },
    );

    assert_eq!(report.state, RecoveryDeliveryState::Refused);
    assert_eq!(report.code, "unsafe_or_invalid_payload");
    assert_eq!(store_calls.load(Ordering::SeqCst), 0);
    assert_eq!(exact_calls.load(Ordering::SeqCst), 0);
    assert!(!unexpected_store.exists());
}

#[test]
fn prepare_storage_outcome_unknown_stays_pending_without_provider_effect() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store_root = temp.path().join("recovery");
    let provider = ExactSpyProvider::exact(ExactBehavior::Local);
    let exact_calls = provider.exact_calls.clone();
    let report = deliver_with(
        temp.path(),
        "intent-prepare-storage",
        input("prepare storage outcome unknown"),
        |_| RecoveryDeliveryRoute::new(RecoveryProvider::Local, Box::new(provider)),
        |_| Ok(authority()),
        |_, authority| {
            let store = RecoveryStore::new(&store_root, authority.authority().clone())?;
            std::fs::remove_dir_all(&store_root)
                .map_err(|_| gwt_core::recovery::RecoveryError::Storage)?;
            std::fs::write(&store_root, b"block recovery directory")
                .map_err(|_| gwt_core::recovery::RecoveryError::Storage)?;
            Ok(store)
        },
    );

    assert_eq!(report.state, RecoveryDeliveryState::Pending);
    assert_eq!(report.code, "prepare_outcome_unknown");
    assert!(report.retryable);
    assert_eq!(exact_calls.load(Ordering::SeqCst), 0);
    assert!(!gwt_core::coordination::coordination_events_path(temp.path()).exists());
}

#[test]
fn prepare_is_durable_before_local_append_and_first_delivery_acknowledges() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store_root = temp.path().join("recovery");
    let observed_pending = Arc::new(AtomicBool::new(false));
    let provider = ExactSpyProvider::exact(ExactBehavior::Local).with_pending_probe({
        let store_root = store_root.clone();
        let observed_pending = observed_pending.clone();
        move |entry| {
            let recovery_id = entry
                .origin_recovery_id
                .as_deref()
                .expect("recovery id before append");
            let store = RecoveryStore::new(&store_root, authority().authority().clone())
                .expect("reopen store from append spy");
            let record = store
                .get(recovery_id)
                .expect("read pending record")
                .expect("pending record exists");
            observed_pending.store(record.state == RecoveryState::Pending, Ordering::SeqCst);
        }
    });

    let report = deliver(
        temp.path(),
        &store_root,
        provider,
        "intent-first",
        "durable before effect",
    );

    assert_eq!(report.state, RecoveryDeliveryState::Acknowledged);
    assert!(observed_pending.load(Ordering::SeqCst));
    assert_eq!(
        LocalProvider
            .load_snapshot(temp.path())
            .expect("Board snapshot")
            .board
            .entries
            .len(),
        1
    );
}

#[test]
fn acknowledged_terminal_retry_has_zero_extra_provider_or_board_effect() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store_root = temp.path().join("recovery");
    let provider = ExactSpyProvider::exact(ExactBehavior::Local);
    let calls = provider.exact_calls.clone();

    let first = deliver(
        temp.path(),
        &store_root,
        provider.clone(),
        "intent-retry",
        "same payload",
    );
    let retry = deliver(
        temp.path(),
        &store_root,
        provider,
        "intent-retry",
        "same payload",
    );

    assert_eq!(first.state, RecoveryDeliveryState::Acknowledged);
    assert_eq!(retry.state, RecoveryDeliveryState::Acknowledged);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        LocalProvider
            .load_snapshot(temp.path())
            .expect("Board snapshot")
            .board
            .entries
            .len(),
        1
    );
}

#[test]
fn append_response_loss_stays_pending_then_exact_replay_acknowledges() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store_root = temp.path().join("recovery");
    let provider = ExactSpyProvider::exact(ExactBehavior::ResponseLossOnce(Arc::new(
        AtomicBool::new(false),
    )));
    let calls = provider.exact_calls.clone();

    let first = deliver(
        temp.path(),
        &store_root,
        provider.clone(),
        "intent-loss",
        "response loss",
    );
    let retry = deliver(
        temp.path(),
        &store_root,
        provider,
        "intent-loss",
        "response loss",
    );

    assert_eq!(first.state, RecoveryDeliveryState::Pending);
    assert!(first.retryable);
    assert_eq!(retry.state, RecoveryDeliveryState::Acknowledged);
    assert_eq!(retry.disposition.as_deref(), Some("replayed"));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        LocalProvider
            .load_snapshot(temp.path())
            .expect("Board snapshot")
            .board
            .entries
            .len(),
        1
    );
}

#[test]
fn same_intent_changed_payload_conflicts_without_second_provider_effect() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store_root = temp.path().join("recovery");
    let provider = ExactSpyProvider::exact(ExactBehavior::Local);
    let calls = provider.exact_calls.clone();
    assert_eq!(
        deliver(
            temp.path(),
            &store_root,
            provider.clone(),
            "intent-mutated",
            "payload A",
        )
        .state,
        RecoveryDeliveryState::Acknowledged
    );

    let changed = deliver(
        temp.path(),
        &store_root,
        provider,
        "intent-mutated",
        "payload B",
    );
    assert_eq!(changed.state, RecoveryDeliveryState::Conflicted);
    assert_eq!(changed.code, "intent_payload_changed");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        LocalProvider
            .load_snapshot(temp.path())
            .expect("Board snapshot")
            .board
            .entries
            .len(),
        1
    );
}

#[test]
fn receipt_identity_or_digest_mismatch_becomes_durable_conflict() {
    for (suffix, behavior) in [
        ("entry", ExactBehavior::ReceiptEntryMismatch),
        ("digest", ExactBehavior::ReceiptDigestMismatch),
    ] {
        let temp = tempfile::tempdir().expect("tempdir");
        let store_root = temp.path().join("recovery");
        let report = deliver(
            temp.path(),
            &store_root,
            ExactSpyProvider::exact(behavior),
            &format!("intent-mismatch-{suffix}"),
            "receipt mismatch",
        );
        assert_eq!(report.state, RecoveryDeliveryState::Conflicted);
        assert_eq!(report.code, "receipt_mismatch");

        let store = RecoveryStore::new(&store_root, authority().authority().clone())
            .expect("reopen recovery store");
        let records = store.list().expect("list records");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].state, RecoveryState::Conflicted);
    }
}

#[test]
fn provider_payload_and_duplicate_conflicts_map_to_durable_conflict() {
    for (kind, code) in [
        (
            BoardExactAppendConflictKind::PayloadMismatch,
            "provider_payload_conflict",
        ),
        (
            BoardExactAppendConflictKind::DuplicateIdentity,
            "provider_duplicate_identity",
        ),
    ] {
        let temp = tempfile::tempdir().expect("tempdir");
        let store_root = temp.path().join("recovery");
        let report = deliver(
            temp.path(),
            &store_root,
            ExactSpyProvider::exact(ExactBehavior::Conflict(kind)),
            &format!("intent-provider-{code}"),
            "provider conflict",
        );
        assert_eq!(report.state, RecoveryDeliveryState::Conflicted);
        assert_eq!(report.code, code);
        let store = RecoveryStore::new(&store_root, authority().authority().clone())
            .expect("reopen recovery store");
        assert_eq!(
            store.list().expect("records")[0].state,
            RecoveryState::Conflicted
        );
    }
}

#[test]
fn refresh_error_acknowledges_with_only_public_safe_boolean() {
    let temp = tempfile::tempdir().expect("tempdir");
    let report = deliver(
        temp.path(),
        &temp.path().join("recovery"),
        ExactSpyProvider::exact(ExactBehavior::RefreshFailure),
        "intent-refresh",
        "refresh failed after commit",
    );
    assert_eq!(report.state, RecoveryDeliveryState::Acknowledged);
    assert_eq!(report.snapshot_refresh_pending, Some(true));
    let rendered = serde_json::to_string(&report).expect("serialize public report");
    for private in [
        "/Users/private",
        "super-secret",
        "recovery_id",
        "intent_id",
        "session_id",
        "project_id",
        "payload_digest",
        "provider_receipt",
    ] {
        assert!(!rendered.contains(private), "leaked {private}: {rendered}");
    }
}

#[test]
fn concurrent_delivery_converges_to_acknowledged_and_one_board_event() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = Arc::new(temp.path().to_path_buf());
    let store_root = Arc::new(temp.path().join("recovery"));
    let provider = ExactSpyProvider::exact(ExactBehavior::Barrier(Arc::new(Barrier::new(2))));
    let calls = provider.exact_calls.clone();
    let reports = Arc::new(Mutex::new(Vec::new()));
    let mut handles = Vec::new();
    for _ in 0..2 {
        let repo = repo.clone();
        let store_root = store_root.clone();
        let provider = provider.clone();
        let reports = reports.clone();
        handles.push(std::thread::spawn(move || {
            let report = deliver(
                repo.as_path(),
                store_root.as_path(),
                provider,
                "intent-concurrent",
                "concurrent delivery",
            );
            reports.lock().expect("reports lock").push(report);
        }));
    }
    for handle in handles {
        handle.join().expect("delivery thread");
    }
    assert!(reports
        .lock()
        .expect("reports")
        .iter()
        .all(|report| report.state == RecoveryDeliveryState::Acknowledged));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        LocalProvider
            .load_snapshot(repo.as_path())
            .expect("Board snapshot")
            .board
            .entries
            .len(),
        1
    );
}

fn git_repo(path: &Path, origin: &str) {
    assert!(gwt_core::process::hidden_command("git")
        .args(["init", "-b", "work/authority"])
        .arg(path)
        .status()
        .expect("git init")
        .success());
    assert!(gwt_core::process::hidden_command("git")
        .arg("-C")
        .arg(path)
        .args(["remote", "add", "origin", origin])
        .status()
        .expect("git remote")
        .success());
}

fn durable_session(worktree: &Path, id: &str) -> Session {
    let mut session = Session::new(worktree, "work/authority", AgentId::Codex);
    session.id = id.to_string();
    session.project_state_root = Some(worktree.to_path_buf());
    session.linked_issue_number = Some(1921);
    let binding = SessionExecutionBinding {
        schema_version: SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
        session_id: session.id.clone(),
        repo_hash: session.repo_hash.clone().expect("repo hash"),
        owner_kind: "spec".to_string(),
        owner_number: 1921,
        identity: ExecutionBindingIdentity {
            generation_id: "generation-1".to_string(),
            binding_id: "binding-1".to_string(),
            ledger_head_hash: "ledger-1".to_string(),
        },
        capability_generation: 1,
    };
    session
        .set_execution_binding(Some(binding))
        .expect("bind durable session");
    session
}

#[test]
fn missing_foreign_and_wrong_branch_authority_stop_before_store_or_provider() {
    let home = tempfile::tempdir().expect("home");
    let _home = ScopedGwtHome::set(home.path());
    let repo = tempfile::tempdir().expect("repo");
    git_repo(
        repo.path(),
        "https://github.com/example/recovery-authority.git",
    );
    let foreign = tempfile::tempdir().expect("foreign worktree");
    git_repo(
        foreign.path(),
        "https://github.com/example/recovery-authority.git",
    );
    let foreign_project = tempfile::tempdir().expect("foreign project");
    git_repo(
        foreign_project.path(),
        "https://github.com/example/other-project.git",
    );
    let sessions = home.path().join(".gwt").join("sessions");

    let foreign_session = durable_session(foreign.path(), "session-foreign");
    foreign_session
        .save(&sessions)
        .expect("save foreign Session");
    let foreign_project_session = durable_session(foreign_project.path(), "session-project");
    foreign_project_session
        .save(&sessions)
        .expect("save foreign project Session");
    let mut wrong_branch = durable_session(repo.path(), "session-wrong-branch");
    wrong_branch.branch = "work/other".to_string();
    wrong_branch
        .save(&sessions)
        .expect("save wrong branch Session");
    let unexpected_store = home.path().join("unexpected-store");

    for session_id in [
        None,
        Some("session-foreign"),
        Some("session-project"),
        Some("session-wrong-branch"),
    ] {
        let provider = ExactSpyProvider::exact(ExactBehavior::Local);
        let exact_calls = provider.exact_calls.clone();
        let store_calls = Arc::new(AtomicUsize::new(0));
        let report = deliver_with(
            repo.path(),
            "intent-authority-refusal",
            input("must remain effect free"),
            |_| RecoveryDeliveryRoute::new(RecoveryProvider::Local, Box::new(provider)),
            |root| resolve_delivery_authority(root, session_id),
            {
                let calls = store_calls.clone();
                let unexpected_store = unexpected_store.clone();
                move |_, authority| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    RecoveryStore::new(unexpected_store, authority.authority().clone())
                }
            },
        );
        assert_eq!(report.state, RecoveryDeliveryState::Refused);
        assert_eq!(report.code, "authority_unavailable");
        assert_eq!(store_calls.load(Ordering::SeqCst), 0);
        assert_eq!(exact_calls.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn worktree_form_comes_from_attached_or_detached_head_not_path_prefix() {
    let home = tempfile::tempdir().expect("home");
    let _home = ScopedGwtHome::set(home.path());
    let sessions = home.path().join(".gwt").join("sessions");

    let branch_named_like_legacy_intake = tempfile::tempdir().expect("branch repo");
    let branch_path = branch_named_like_legacy_intake
        .path()
        .join(".intake-looks-ephemeral");
    std::fs::create_dir_all(&branch_path).expect("branch path");
    git_repo(
        &branch_path,
        "https://github.com/example/recovery-form-branch.git",
    );
    durable_session(&branch_path, "session-branch")
        .save(&sessions)
        .expect("save branch Session");
    let branch =
        resolve_delivery_authority(&branch_path, Some("session-branch")).expect("branch authority");
    assert_eq!(branch.worktree_form(), BoardWorktreeForm::BranchBacked);
    assert_eq!(branch.origin_branch(), Some("work/authority"));

    let detached = tempfile::tempdir().expect("detached repo");
    git_repo(
        detached.path(),
        "https://github.com/example/recovery-form-detached.git",
    );
    assert!(gwt_core::process::hidden_command("git")
        .arg("-C")
        .arg(detached.path())
        .args(["commit", "--allow-empty", "-m", "fixture"])
        .env("GIT_AUTHOR_NAME", "fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.com")
        .env("GIT_COMMITTER_NAME", "fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.com")
        .status()
        .expect("fixture commit")
        .success());
    assert!(gwt_core::process::hidden_command("git")
        .arg("-C")
        .arg(detached.path())
        .args(["checkout", "--detach", "HEAD"])
        .status()
        .expect("detach")
        .success());
    durable_session(detached.path(), "session-detached")
        .save(&sessions)
        .expect("save detached Session");
    let detached = resolve_delivery_authority(detached.path(), Some("session-detached"))
        .expect("detached authority");
    assert_eq!(detached.worktree_form(), BoardWorktreeForm::Ephemeral);
    assert_eq!(detached.origin_branch(), None);
}

#[test]
fn unreadable_head_is_unknown_instead_of_ephemeral() {
    let repo = tempfile::tempdir().expect("repo");
    git_repo(
        repo.path(),
        "https://github.com/example/recovery-form-unknown.git",
    );
    std::fs::write(
        repo.path().join(".git").join("HEAD"),
        b"not-a-symbolic-ref\n",
    )
    .expect("make HEAD structurally unreadable");

    let repository = gwt_git::Repository::discover(repo.path()).expect("discover repository");
    let (form, origin_branch) = classify_worktree_origin(&repository);
    assert_eq!(form, BoardWorktreeForm::Unknown);
    assert_eq!(origin_branch, None);
}
