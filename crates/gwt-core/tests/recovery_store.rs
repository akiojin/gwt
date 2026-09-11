use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Barrier},
    thread,
};

use gwt_core::{
    coordination::{
        board_entry_payload_digest, AuthorKind, BoardEntry, BoardEntryKind, BoardWorktreeForm,
    },
    paths::project_scope_hash,
    recovery::{
        AcknowledgementMismatchKind, RecoveryAcknowledgement, RecoveryAuthority,
        RecoveryConflictKind, RecoveryError, RecoveryIntent, RecoveryProvider,
        RecoveryProviderReceipt, RecoveryState, RecoveryStore, RecoveryWriteDisposition,
    },
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tempfile::TempDir;

const RECOVERY_ID: &str = "recover/raw-id-must-not-be-a-path";

fn authority() -> RecoveryAuthority {
    RecoveryAuthority::new("project-1974", "session-1974").expect("valid authority")
}

fn board_entry(recovery_id: &str, body: &str) -> BoardEntry {
    let mut entry = BoardEntry::new(
        AuthorKind::Agent,
        "codex",
        BoardEntryKind::Status,
        body,
        None,
        None,
        vec!["spec-1921".to_string()],
        vec!["issue-1974".to_string()],
    );
    entry.id = "board-entry-1974".to_string();
    entry.origin_branch = Some("work/issue-1974".to_string());
    entry.origin_session_id = Some("session-1974".to_string());
    entry.origin_agent_id = Some("codex".to_string());
    entry.origin_worktree_form = Some(BoardWorktreeForm::BranchBacked);
    entry.origin_recovery_id = Some(recovery_id.to_string());
    entry
}

fn intent(recovery_id: &str, body: &str) -> RecoveryIntent {
    RecoveryIntent::new(
        recovery_id,
        RecoveryProvider::Local,
        BoardWorktreeForm::BranchBacked,
        board_entry(recovery_id, body),
    )
    .expect("valid recovery intent")
}

fn intent_for_session(recovery_id: &str, body: &str, session_id: &str) -> RecoveryIntent {
    let mut entry = board_entry(recovery_id, body);
    entry.origin_session_id = Some(session_id.to_string());
    entry.id = format!("board-entry-{session_id}");
    RecoveryIntent::new(
        recovery_id,
        RecoveryProvider::Local,
        BoardWorktreeForm::BranchBacked,
        entry,
    )
    .expect("valid session recovery intent")
}

fn open_store(temp: &TempDir) -> RecoveryStore {
    RecoveryStore::new(temp.path().join("recovery"), authority()).expect("recovery store")
}

#[test]
fn canonical_store_derives_project_and_current_session_authority() {
    let repo = TempDir::new().unwrap();
    let expected_project = project_scope_hash(repo.path());

    let store = RecoveryStore::for_repo(repo.path(), "current-session").unwrap();

    assert_eq!(store.authority().project_id, expected_project.as_str());
    assert_eq!(store.authority().session_id, "current-session");
}

#[test]
fn canonical_store_lists_only_the_current_session_partition() {
    let repo = TempDir::new().unwrap();
    let first = RecoveryStore::for_repo(repo.path(), "session-one").unwrap();
    let second = RecoveryStore::for_repo(repo.path(), "session-two").unwrap();
    let first_record = first
        .prepare(
            "prepare-one",
            intent_for_session("recovery-one", "first safe status", "session-one"),
        )
        .unwrap()
        .record;
    let second_record = second
        .prepare(
            "prepare-two",
            intent_for_session("recovery-two", "second safe status", "session-two"),
        )
        .unwrap()
        .record;

    assert_eq!(first.list().unwrap(), vec![first_record]);
    assert_eq!(second.list().unwrap(), vec![second_record]);
    assert_eq!(first.get("recovery-two").unwrap(), None);
    assert_eq!(second.get("recovery-one").unwrap(), None);
}

fn acknowledgement(record: &gwt_core::recovery::RecoveryRecord) -> RecoveryAcknowledgement {
    RecoveryAcknowledgement::for_record(
        record,
        RecoveryProviderReceipt::new("board-entry-1974").expect("public-safe receipt"),
    )
    .expect("pending record acknowledgement")
}

fn json_files_in(root: &Path, directory_name: &str) -> Vec<PathBuf> {
    let mut files = Vec::new();
    fn visit(path: &Path, directory_name: &str, files: &mut Vec<PathBuf>) {
        if !path.is_dir() {
            return;
        }
        for entry in fs::read_dir(path).expect("read recovery directory") {
            let path = entry.expect("recovery entry").path();
            if path.is_dir() {
                visit(&path, directory_name, files);
            } else if path
                .parent()
                .and_then(Path::file_name)
                .is_some_and(|name| name == directory_name)
                && path
                    .extension()
                    .is_some_and(|extension| extension == "json")
            {
                files.push(path);
            }
        }
    }
    visit(root, directory_name, &mut files);
    files.sort();
    files
}

fn revision_files(root: &Path) -> Vec<PathBuf> {
    json_files_in(root, "revisions")
}

fn head_files(root: &Path) -> Vec<PathBuf> {
    json_files_in(root, "heads")
}

fn only_revisions_dir(root: &Path) -> PathBuf {
    revision_files(root)
        .into_iter()
        .next()
        .expect("one committed revision")
        .parent()
        .expect("revision parent")
        .to_path_buf()
}

#[test]
fn pending_intent_survives_restart_before_board_append() {
    let temp = TempDir::new().unwrap();
    let first = open_store(&temp);
    let write = first
        .prepare("prepare-1974", intent(RECOVERY_ID, "safe status"))
        .unwrap();

    assert_eq!(write.disposition, RecoveryWriteDisposition::Committed);
    assert_eq!(write.record.revision, 1);
    assert_eq!(write.record.state, RecoveryState::Pending);
    drop(first);

    let reopened = open_store(&temp);
    assert_eq!(
        reopened.get(RECOVERY_ID).unwrap(),
        Some(write.record.clone())
    );
    assert_eq!(reopened.list().unwrap(), vec![write.record.clone()]);
    let replay = reopened
        .prepare("prepare-1974", intent(RECOVERY_ID, "safe status"))
        .unwrap();
    assert_eq!(replay.disposition, RecoveryWriteDisposition::Replayed);
    assert_eq!(replay.record, write.record);
    assert_eq!(revision_files(&temp.path().join("recovery")).len(), 1);
}

#[test]
fn recovery_restart_preserves_escalation_resolution_targets() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let mut entry = board_entry(RECOVERY_ID, "safe status");
    entry.resolves_entry_ids = vec!["blocked-entry-1974".to_string()];
    let intent = RecoveryIntent::new(
        RECOVERY_ID,
        RecoveryProvider::Local,
        BoardWorktreeForm::BranchBacked,
        entry,
    )
    .unwrap();
    let write = store.prepare("prepare-resolution", intent).unwrap();
    drop(store);
    assert_eq!(
        open_store(&temp).get(RECOVERY_ID).unwrap(),
        Some(write.record)
    );
}

#[test]
fn prepare_replays_same_operation_and_rejects_mutated_payload() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let first = store
        .prepare("prepare-1974", intent(RECOVERY_ID, "safe status"))
        .unwrap();
    let replay = store
        .prepare("prepare-1974", intent(RECOVERY_ID, "safe status"))
        .unwrap();

    assert_eq!(replay.disposition, RecoveryWriteDisposition::Replayed);
    assert_eq!(replay.record, first.record);
    assert_eq!(revision_files(&temp.path().join("recovery")).len(), 1);

    let error = store
        .prepare("prepare-1974", intent(RECOVERY_ID, "mutated status"))
        .unwrap_err();
    assert_eq!(error, RecoveryError::OperationConflict);
    assert_eq!(revision_files(&temp.path().join("recovery")).len(), 1);
}

#[test]
fn exact_matching_acknowledgement_commits_terminal_state() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let pending = store
        .prepare("prepare-1974", intent(RECOVERY_ID, "safe status"))
        .unwrap()
        .record;
    let ack = acknowledgement(&pending);

    let acknowledged = store
        .acknowledge(RECOVERY_ID, pending.revision, "ack-1974", ack.clone())
        .unwrap();

    assert_eq!(
        acknowledged.disposition,
        RecoveryWriteDisposition::Committed
    );
    assert_eq!(acknowledged.record.revision, 2);
    assert_eq!(acknowledged.record.state, RecoveryState::Acknowledged);
    assert_eq!(acknowledged.record.acknowledgement, Some(ack));
    let files = revision_files(&temp.path().join("recovery"));
    assert_eq!(files.len(), 2);
    assert_eq!(head_files(&temp.path().join("recovery")).len(), 2);
    let first: Value = serde_json::from_slice(&fs::read(&files[0]).unwrap()).unwrap();
    let second: Value = serde_json::from_slice(&fs::read(&files[1]).unwrap()).unwrap();
    assert!(first["previous_revision_digest"].is_null());
    assert_eq!(second["previous_revision_digest"], first["revision_digest"]);
    assert_eq!(second["record"]["intent"], first["record"]["intent"]);
}

#[test]
fn missing_or_renamed_terminal_revision_fails_closed() {
    for rename_terminal in [false, true] {
        let temp = TempDir::new().unwrap();
        let store = open_store(&temp);
        let pending = store
            .prepare("prepare-1974", intent(RECOVERY_ID, "safe status"))
            .unwrap()
            .record;
        store
            .acknowledge(
                RECOVERY_ID,
                pending.revision,
                "ack-1974",
                acknowledgement(&pending),
            )
            .unwrap();
        let files = revision_files(&temp.path().join("recovery"));
        assert_eq!(files.len(), 2);
        if rename_terminal {
            fs::rename(&files[1], files[1].with_extension("bak")).unwrap();
        } else {
            fs::remove_file(&files[1]).unwrap();
        }

        assert_eq!(
            store.get(RECOVERY_ID).unwrap_err(),
            RecoveryError::CorruptStore
        );
    }
}

#[test]
fn foreign_acknowledgement_fields_are_rejected_without_mutation() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let pending = store
        .prepare("prepare-1974", intent(RECOVERY_ID, "safe status"))
        .unwrap()
        .record;
    let baseline = acknowledgement(&pending);

    let mut cases = Vec::new();
    let mut foreign_recovery = baseline.clone();
    foreign_recovery.recovery_id = "foreign-recovery".to_string();
    cases.push((
        foreign_recovery,
        AcknowledgementMismatchKind::RecoveryIdentity,
    ));
    let mut foreign_entry = baseline.clone();
    foreign_entry.entry_id = "foreign-entry".to_string();
    cases.push((foreign_entry, AcknowledgementMismatchKind::EntryIdentity));
    let mut foreign_project = baseline.clone();
    foreign_project.authority.project_id = "foreign-project".to_string();
    cases.push((
        foreign_project,
        AcknowledgementMismatchKind::ProjectAuthority,
    ));
    let mut foreign_session = baseline.clone();
    foreign_session.authority.session_id = "foreign-session".to_string();
    cases.push((
        foreign_session,
        AcknowledgementMismatchKind::SessionAuthority,
    ));
    let mut foreign_provider = baseline.clone();
    foreign_provider.provider = RecoveryProvider::Slack;
    cases.push((foreign_provider, AcknowledgementMismatchKind::Provider));
    let mut foreign_payload = baseline.clone();
    foreign_payload.payload_digest = "v1:sha256:foreign".to_string();
    cases.push((foreign_payload, AcknowledgementMismatchKind::PayloadDigest));
    let mut foreign_receipt = baseline.clone();
    foreign_receipt.provider_receipt =
        RecoveryProviderReceipt::new("foreign-receipt").expect("public-safe receipt");
    cases.push((
        foreign_receipt,
        AcknowledgementMismatchKind::ProviderReceipt,
    ));

    for (index, (ack, expected_kind)) in cases.into_iter().enumerate() {
        let error = store
            .acknowledge(
                RECOVERY_ID,
                pending.revision,
                &format!("foreign-ack-{index}"),
                ack,
            )
            .unwrap_err();
        assert_eq!(error, RecoveryError::AcknowledgementMismatch(expected_kind));
        assert_eq!(revision_files(&temp.path().join("recovery")).len(), 1);
        assert_eq!(store.get(RECOVERY_ID).unwrap(), Some(pending.clone()));
    }
}

#[test]
fn foreign_store_authority_cannot_read_or_mutate_record() {
    let temp = TempDir::new().unwrap();
    let owner = open_store(&temp);
    let pending = owner
        .prepare("prepare-1974", intent(RECOVERY_ID, "safe status"))
        .unwrap()
        .record;
    let foreign_authorities = [
        RecoveryAuthority::new("project-1974", "foreign-session").unwrap(),
        RecoveryAuthority::new("foreign-project", "session-1974").unwrap(),
    ];
    for (index, foreign_authority) in foreign_authorities.into_iter().enumerate() {
        let foreign = RecoveryStore::new(temp.path().join("recovery"), foreign_authority).unwrap();
        assert_eq!(foreign.get(RECOVERY_ID).unwrap(), None);
        assert_eq!(
            foreign
                .acknowledge(
                    RECOVERY_ID,
                    pending.revision,
                    &format!("foreign-authority-ack-{index}"),
                    acknowledgement(&pending),
                )
                .unwrap_err(),
            RecoveryError::NotFound
        );
    }
    assert_eq!(revision_files(&temp.path().join("recovery")).len(), 1);
}

#[test]
fn prepare_rejects_foreign_session_origin_without_creating_a_revision() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let mut foreign_intent = intent(RECOVERY_ID, "safe status");
    foreign_intent.entry.origin_session_id = Some("foreign-session".to_string());
    foreign_intent.payload_digest = board_entry_payload_digest(&foreign_intent.entry).unwrap();

    assert_eq!(
        store
            .prepare("foreign-session-prepare", foreign_intent)
            .unwrap_err(),
        RecoveryError::AuthorityMismatch
    );
    assert!(revision_files(&temp.path().join("recovery")).is_empty());
}

#[test]
fn stale_revision_from_another_operation_is_rejected() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let pending = store
        .prepare("prepare-1974", intent(RECOVERY_ID, "safe status"))
        .unwrap()
        .record;
    store
        .acknowledge(
            RECOVERY_ID,
            pending.revision,
            "ack-1974",
            acknowledgement(&pending),
        )
        .unwrap();

    let error = store
        .mark_conflicted(
            RECOVERY_ID,
            pending.revision,
            "late-conflict",
            RecoveryConflictKind::StorageUncertain,
        )
        .unwrap_err();
    assert_eq!(
        error,
        RecoveryError::StaleRevision {
            expected: 1,
            actual: 2,
        }
    );
    assert_eq!(revision_files(&temp.path().join("recovery")).len(), 2);
}

#[test]
fn partial_temporary_revision_is_ignored_after_restart() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let pending = store
        .prepare("prepare-1974", intent(RECOVERY_ID, "safe status"))
        .unwrap()
        .record;
    let revisions = only_revisions_dir(&temp.path().join("recovery"));
    fs::write(
        revisions.join(".00000000000000000002.json.tmp-crash"),
        b"{not committed",
    )
    .unwrap();
    drop(store);

    let reopened = open_store(&temp);
    assert_eq!(reopened.get(RECOVERY_ID).unwrap(), Some(pending));
    assert_eq!(revision_files(&temp.path().join("recovery")).len(), 1);
}

#[test]
fn acknowledgement_response_loss_replays_committed_revision() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let pending = store
        .prepare("prepare-1974", intent(RECOVERY_ID, "safe status"))
        .unwrap()
        .record;
    let ack = acknowledgement(&pending);
    let committed = store
        .acknowledge(
            RECOVERY_ID,
            pending.revision,
            "ack-response-loss",
            ack.clone(),
        )
        .unwrap();
    drop(store);

    let reopened = open_store(&temp);
    let replay = reopened
        .acknowledge(RECOVERY_ID, pending.revision, "ack-response-loss", ack)
        .unwrap();
    assert_eq!(replay.disposition, RecoveryWriteDisposition::Replayed);
    assert_eq!(replay.record, committed.record);
    assert_eq!(revision_files(&temp.path().join("recovery")).len(), 2);

    let mut mutated_ack = acknowledgement(&pending);
    mutated_ack.provider_receipt =
        RecoveryProviderReceipt::new("different-receipt").expect("public-safe receipt");
    let error = reopened
        .acknowledge(
            RECOVERY_ID,
            pending.revision,
            "ack-response-loss",
            mutated_ack,
        )
        .unwrap_err();
    assert_eq!(error, RecoveryError::OperationConflict);
    assert_eq!(revision_files(&temp.path().join("recovery")).len(), 2);
}

#[test]
fn persistence_rejects_unsanitized_fields_and_uses_opaque_paths() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let debug = format!("{store:?}");
    assert!(!debug.contains(&temp.path().to_string_lossy().to_string()));
    assert!(!debug.contains("project-1974"));
    assert!(!debug.contains("session-1974"));
    let mut unsafe_entry = board_entry(RECOVERY_ID, "safe status");
    unsafe_entry.body_html =
        Some("<pre>/Users/private SECRET_TOKEN=credential hidden reasoning</pre>".to_string());
    let error = RecoveryIntent::new(
        RECOVERY_ID,
        RecoveryProvider::Local,
        BoardWorktreeForm::BranchBacked,
        unsafe_entry,
    )
    .unwrap_err();
    assert_eq!(error, RecoveryError::UnsanitizedEntry);
    assert!(revision_files(&temp.path().join("recovery")).is_empty());

    let mut bypassed_constructor = intent(RECOVERY_ID, "safe status");
    bypassed_constructor.entry.body_html =
        Some("<pre>/Users/private SECRET_TOKEN=credential hidden reasoning</pre>".to_string());
    let error = store
        .prepare("unsafe-derived-html", bypassed_constructor)
        .unwrap_err();
    assert_eq!(error, RecoveryError::UnsanitizedEntry);
    assert!(revision_files(&temp.path().join("recovery")).is_empty());

    store
        .prepare("prepare-1974", intent(RECOVERY_ID, "safe status"))
        .unwrap();
    let files = revision_files(&temp.path().join("recovery"));
    assert_eq!(files.len(), 1);
    assert!(!files[0].to_string_lossy().contains(RECOVERY_ID));
    let record_dir = files[0].parent().unwrap().parent().unwrap();
    assert_eq!(
        record_dir.file_name().unwrap().to_string_lossy(),
        hex::encode(Sha256::digest(RECOVERY_ID.as_bytes()))
    );
    let persisted = fs::read_to_string(&files[0]).unwrap();
    assert!(!persisted.contains("body_html"));
    assert!(!persisted.contains("/Users/private"));
    assert!(!persisted.contains("SECRET_TOKEN"));
    assert!(!persisted.contains("hidden reasoning"));
}

#[test]
fn public_recovery_debug_views_redact_payload_and_authority() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let write = store
        .prepare("prepare-1974", intent(RECOVERY_ID, "safe status"))
        .unwrap();
    let receipt = RecoveryProviderReceipt::new("board-entry-1974").unwrap();
    let ack = RecoveryAcknowledgement::for_record(&write.record, receipt.clone()).unwrap();
    let debug_views = [
        format!("{:?}", authority()),
        format!("{receipt:?}"),
        format!("{:?}", write.record.intent),
        format!("{ack:?}"),
        format!("{:?}", write.record),
        format!("{write:?}"),
        format!("{store:?}"),
    ];
    let payload_digest = write.record.intent.payload_digest.as_str();
    let secrets = [
        RECOVERY_ID,
        "project-1974",
        "session-1974",
        "board-entry-1974",
        "safe status",
        payload_digest,
    ];

    for debug in debug_views {
        for secret in secrets {
            assert!(!debug.contains(secret), "debug leaked {secret}: {debug}");
        }
    }
}

#[test]
fn persisted_board_payload_uses_an_exact_versioned_field_allowlist() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    store
        .prepare("prepare-1974", intent(RECOVERY_ID, "safe status"))
        .unwrap();
    let revision = revision_files(&temp.path().join("recovery"))
        .into_iter()
        .next()
        .unwrap();
    let json: Value = serde_json::from_slice(&fs::read(revision).unwrap()).unwrap();
    let actual = json["record"]["intent"]["entry"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    let expected = [
        "schema_version",
        "id",
        "author_kind",
        "author",
        "kind",
        "body",
        "title",
        "title_summary",
        "state",
        "parent_id",
        "created_at",
        "updated_at",
        "related_topics",
        "related_owners",
        "origin_branch",
        "origin_session_id",
        "origin_agent_id",
        "origin_worktree_form",
        "origin_recovery_id",
        "target_owners",
        "mentions",
        "audience",
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<BTreeSet<_>>();

    assert_eq!(actual, expected);
    assert!(json["record"]["intent"]["entry"].get("body_html").is_none());
}

#[test]
fn persistence_rejects_private_board_body_content_without_mutation() {
    let private_bodies = [
        "workspace is /Users/private/work/gwt",
        "GWT_PROJECT_ROOT=/Users/private/work/gwt",
        "ENV=raw-environment-value",
        "PORT=5432",
        "SECRET=super-secret",
        "KEY=provider-key",
        "Authorization: Bearer provider-secret",
        "<analysis>hidden reasoning must not be persisted</analysis>",
        "provider response body: {\"access_token\":\"provider-secret\"}",
        r#"provider payload: {"access_token":"xoxb-secret","cwd":"/Users/private/work"}"#,
        r#"provider payload: {\"access_token\":\"xoxb-secret\"}"#,
        r#"{"refresh_token":"refresh-secret"}"#,
        r#"{"api_key":"api-secret"}"#,
        r#"{"apikey":"api-secret"}"#,
        r#"{"accessToken":"access-secret"}"#,
        r#"{"client_secret":"client-secret"}"#,
        r#"{"password":"provider-password"}"#,
        r#"{"private_key":"private-key"}"#,
        r#"{"secret":"provider-secret"}"#,
        r#"{"authorization":"Bearer compact-secret"}"#,
        r#"{"token":"provider-token"}"#,
    ];

    for (index, body) in private_bodies.into_iter().enumerate() {
        let temp = TempDir::new().unwrap();
        let store = open_store(&temp);
        let mut unsafe_intent = intent(RECOVERY_ID, "safe status");
        unsafe_intent.entry.body = body.to_string();
        unsafe_intent.payload_digest = board_entry_payload_digest(&unsafe_intent.entry).unwrap();

        let error = store
            .prepare(&format!("private-body-{index}"), unsafe_intent)
            .unwrap_err();
        assert_eq!(error, RecoveryError::UnsanitizedEntry, "body: {body}");
        assert!(revision_files(&temp.path().join("recovery")).is_empty());
    }
}

#[test]
fn persistence_rejects_sensitive_key_after_a_long_escape_run() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let slashes = "\\".repeat(4_096);
    let body =
        format!("provider payload: {{{slashes}\"access_token{slashes}\":\"provider-secret\"}}");
    let mut unsafe_intent = intent(RECOVERY_ID, "safe status");
    unsafe_intent.entry.body = body;
    unsafe_intent.payload_digest = board_entry_payload_digest(&unsafe_intent.entry).unwrap();

    assert_eq!(
        store.prepare("long-escape-run", unsafe_intent).unwrap_err(),
        RecoveryError::UnsanitizedEntry
    );
    assert!(revision_files(&temp.path().join("recovery")).is_empty());
}

#[test]
fn missing_initial_head_is_repaired_without_republishing_a_revision() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let committed = store
        .prepare("prepare-response-loss", intent(RECOVERY_ID, "safe status"))
        .unwrap();
    let root = temp.path().join("recovery");
    let heads = head_files(&root);
    assert_eq!(heads.len(), 1);
    fs::remove_file(&heads[0]).unwrap();
    assert!(head_files(&root).is_empty());
    drop(store);

    let reopened = open_store(&temp);
    let replay = reopened
        .prepare("prepare-response-loss", intent(RECOVERY_ID, "safe status"))
        .unwrap();
    assert_eq!(replay.disposition, RecoveryWriteDisposition::Replayed);
    assert_eq!(replay.record, committed.record);
    assert_eq!(revision_files(&root).len(), 1);
    assert_eq!(head_files(&root).len(), 1);
}

#[test]
fn missing_tail_head_is_repaired_without_republishing_a_revision() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let pending = store
        .prepare("prepare-1974", intent(RECOVERY_ID, "safe status"))
        .unwrap()
        .record;
    let ack = acknowledgement(&pending);
    let committed = store
        .acknowledge(
            RECOVERY_ID,
            pending.revision,
            "ack-response-loss",
            ack.clone(),
        )
        .unwrap();
    let root = temp.path().join("recovery");
    let heads = head_files(&root);
    assert_eq!(heads.len(), 2);
    fs::remove_file(&heads[1]).unwrap();
    assert_eq!(head_files(&root).len(), 1);
    drop(store);

    let reopened = open_store(&temp);
    let replay = reopened
        .acknowledge(RECOVERY_ID, pending.revision, "ack-response-loss", ack)
        .unwrap();
    assert_eq!(replay.disposition, RecoveryWriteDisposition::Replayed);
    assert_eq!(replay.record, committed.record);
    assert_eq!(revision_files(&root).len(), 2);
    assert_eq!(head_files(&root).len(), 2);
}

#[test]
fn persistence_rejects_credentials_in_identifiers_and_receipts() {
    const CREDENTIAL: &str = "ghp_abcdef0123456789ABCDEF";

    assert_eq!(
        RecoveryAuthority::new("project-1974", CREDENTIAL).unwrap_err(),
        RecoveryError::InvalidAuthority
    );
    assert_eq!(
        RecoveryProviderReceipt::new(CREDENTIAL).unwrap_err(),
        RecoveryError::InvalidProviderReceipt
    );

    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    assert_eq!(
        store
            .prepare(CREDENTIAL, intent(RECOVERY_ID, "safe status"))
            .unwrap_err(),
        RecoveryError::InvalidOperationId
    );
    assert!(revision_files(&temp.path().join("recovery")).is_empty());
}

#[test]
fn unsupported_remote_providers_cannot_enter_the_store() {
    for provider in [RecoveryProvider::Slack, RecoveryProvider::Teams] {
        let error = RecoveryIntent::new(
            RECOVERY_ID,
            provider,
            BoardWorktreeForm::BranchBacked,
            board_entry(RECOVERY_ID, "safe status"),
        )
        .unwrap_err();
        assert_eq!(error, RecoveryError::UnsupportedProvider);

        let temp = TempDir::new().unwrap();
        let store = open_store(&temp);
        let mut bypassed_constructor = intent(RECOVERY_ID, "safe status");
        bypassed_constructor.provider = provider;
        assert_eq!(
            store
                .prepare("remote-prepare", bypassed_constructor)
                .unwrap_err(),
            RecoveryError::UnsupportedProvider
        );
        assert!(revision_files(&temp.path().join("recovery")).is_empty());
    }
}

#[test]
fn prepare_rejects_entry_identity_that_cannot_be_acknowledged() {
    let mut invalid_entry = board_entry(RECOVERY_ID, "safe status");
    invalid_entry.id = "board/entry".to_string();
    assert_eq!(
        RecoveryIntent::new(
            RECOVERY_ID,
            RecoveryProvider::Local,
            BoardWorktreeForm::BranchBacked,
            invalid_entry,
        )
        .unwrap_err(),
        RecoveryError::InvalidEntryId
    );

    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let mut bypassed_constructor = intent(RECOVERY_ID, "safe status");
    bypassed_constructor.entry.id = "board/entry".to_string();
    bypassed_constructor.payload_digest =
        board_entry_payload_digest(&bypassed_constructor.entry).unwrap();
    assert_eq!(
        store
            .prepare("invalid-entry-id", bypassed_constructor)
            .unwrap_err(),
        RecoveryError::InvalidEntryId
    );
    assert!(revision_files(&temp.path().join("recovery")).is_empty());
}

#[test]
fn persistence_accepts_public_relative_references_and_explanations() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let body = "Reason: crates/gwt/src/main.rs, docs/usr/layout.md, and crates/var/schema.md keep state=active, not_secret: false, access_tokens=0, and SC=072; probe /api/v1/health and /users/public-profile; see https://github.com/akiojin/gwt/issues/1974";

    let prepared = store
        .prepare("public-body", intent(RECOVERY_ID, body))
        .unwrap();

    assert_eq!(prepared.record.intent.entry.body, body);
    let files = revision_files(&temp.path().join("recovery"));
    assert_eq!(files.len(), 1);
    assert!(fs::read_to_string(&files[0]).unwrap().contains(body));
}

#[test]
fn conflicted_state_is_durable_terminal_and_idempotent() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    let pending = store
        .prepare("prepare-1974", intent(RECOVERY_ID, "safe status"))
        .unwrap()
        .record;
    let conflict = store
        .mark_conflicted(
            RECOVERY_ID,
            pending.revision,
            "conflict-1974",
            RecoveryConflictKind::PayloadMismatch,
        )
        .unwrap();
    assert_eq!(conflict.record.state, RecoveryState::Conflicted);
    drop(store);

    let reopened = open_store(&temp);
    assert_eq!(
        reopened.get(RECOVERY_ID).unwrap(),
        Some(conflict.record.clone())
    );
    let replay = reopened
        .mark_conflicted(
            RECOVERY_ID,
            pending.revision,
            "conflict-1974",
            RecoveryConflictKind::PayloadMismatch,
        )
        .unwrap();
    assert_eq!(replay.disposition, RecoveryWriteDisposition::Replayed);
    assert_eq!(replay.record, conflict.record);

    let error = reopened
        .acknowledge(
            RECOVERY_ID,
            2,
            "ack-after-conflict",
            acknowledgement(&pending),
        )
        .unwrap_err();
    assert_eq!(error, RecoveryError::TerminalState);
    assert_eq!(revision_files(&temp.path().join("recovery")).len(), 2);
}

#[test]
fn concurrent_compare_and_swap_has_one_winner() {
    let temp = TempDir::new().unwrap();
    let store = Arc::new(open_store(&temp));
    let pending = store
        .prepare("prepare-1974", intent(RECOVERY_ID, "safe status"))
        .unwrap()
        .record;
    let barrier = Arc::new(Barrier::new(3));
    let mut handles = Vec::new();
    for index in 0..2 {
        let store = Arc::clone(&store);
        let pending = pending.clone();
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            barrier.wait();
            store.acknowledge(
                RECOVERY_ID,
                pending.revision,
                &format!("concurrent-ack-{index}"),
                acknowledgement(&pending),
            )
        }));
    }
    barrier.wait();
    let results = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect::<Vec<_>>();

    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(RecoveryError::StaleRevision { .. })))
            .count(),
        1
    );
    assert_eq!(revision_files(&temp.path().join("recovery")).len(), 2);
}

#[test]
fn tampered_committed_revision_fails_closed() {
    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    store
        .prepare("prepare-1974", intent(RECOVERY_ID, "safe status"))
        .unwrap();
    let revision = revision_files(&temp.path().join("recovery"))
        .into_iter()
        .next()
        .unwrap();
    let mut json: Value = serde_json::from_slice(&fs::read(&revision).unwrap()).unwrap();
    json["record"]["intent"]["entry"]["body"] = Value::String("tampered".to_string());
    fs::write(&revision, serde_json::to_vec_pretty(&json).unwrap()).unwrap();

    assert_eq!(
        store.get(RECOVERY_ID).unwrap_err(),
        RecoveryError::CorruptStore
    );
    assert_eq!(revision_files(&temp.path().join("recovery")).len(), 1);
}

#[cfg(unix)]
#[test]
fn recovery_directories_and_files_are_private() {
    use std::os::unix::fs::PermissionsExt;

    let temp = TempDir::new().unwrap();
    let store = open_store(&temp);
    store
        .prepare("prepare-1974", intent(RECOVERY_ID, "safe status"))
        .unwrap();
    let root = temp.path().join("recovery");
    assert_eq!(
        fs::metadata(&root).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let revision = revision_files(&root).into_iter().next().unwrap();
    let head = head_files(&root).into_iter().next().unwrap();
    let revisions_dir = revision.parent().unwrap();
    let record_dir = revisions_dir.parent().unwrap();
    let authority_dir = record_dir.parent().unwrap();
    let heads_dir = head.parent().unwrap();
    assert_eq!(head.file_name(), revision.file_name());
    assert_eq!(heads_dir.parent(), Some(record_dir));
    assert_eq!(
        fs::metadata(authority_dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(record_dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(revisions_dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(heads_dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(&revision).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(&head).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(record_dir.join(".lock"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}
