//! Durable, exact Board delivery for caller-stable intents (SPEC-1921 Phase 80).
//!
//! This path is intentionally separate from ordinary Board posting. It first
//! proves that the selected provider supports deterministic acknowledgement,
//! then validates the ambient durable Session, commits a Pending recovery
//! revision, performs the exact provider append, and settles the revision only
//! after validating the complete receipt.

use std::path::Path;

use gwt_agent::{
    session::GWT_SESSION_ID_ENV, LaunchRuntimeTarget, Session, SessionExecutionIdentity,
};
use gwt_core::{
    coordination::{
        AuthorKind, BoardEntry, BoardEntryDraft, BoardEntryKind, BoardExactAppendConflictKind,
        BoardExactAppendDisposition, BoardExactAppendError, BoardMention, BoardOrigin,
        BoardProvider, BoardRecoveryCapability, BoardWorktreeForm,
    },
    paths::{gwt_sessions_dir, project_scope_hash},
    recovery::{
        RecoveryAcknowledgement, RecoveryAuthority, RecoveryConflictKind, RecoveryError,
        RecoveryIntent, RecoveryProvider, RecoveryProviderReceipt, RecoveryRecord, RecoveryState,
        RecoveryStore,
    },
};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{
    board_audience::{current_session_board_scope, post_audience_for_session},
    board_provider,
};

const MAX_INTENT_ID_LEN: usize = 256;

/// Raw public Board content accepted by the recovery route. Authority and
/// deterministic identities are deliberately absent and are derived only
/// after provider capability preflight.
#[derive(Clone, PartialEq, Eq)]
pub struct RecoveryDeliveryInput {
    pub kind: BoardEntryKind,
    pub body: String,
    pub title: Option<String>,
    pub title_summary: Option<String>,
    pub parent: Option<String>,
    pub topics: Vec<String>,
    pub owners: Vec<String>,
    pub targets: Vec<String>,
    pub mentions: Vec<BoardMention>,
    pub workspace_audience: Vec<String>,
    pub broadcast: bool,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryDeliveryState {
    Acknowledged,
    Pending,
    Conflicted,
    Refused,
}

/// Public-safe result. It intentionally contains no intent/recovery identity,
/// Session/project authority, path, digest, provider receipt, or raw error.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RecoveryDeliveryReport {
    pub state: RecoveryDeliveryState,
    pub code: &'static str,
    pub retryable: bool,
    pub provider: &'static str,
    pub worktree_form: BoardWorktreeForm,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disposition: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_refresh_pending: Option<bool>,
}

impl RecoveryDeliveryReport {
    fn refused(provider: RecoveryProvider, code: &'static str, retryable: bool) -> Self {
        Self {
            state: RecoveryDeliveryState::Refused,
            code,
            retryable,
            provider: provider_name(provider),
            worktree_form: BoardWorktreeForm::Unknown,
            entry_id: None,
            disposition: None,
            revision: None,
            snapshot_refresh_pending: None,
        }
    }

    fn pending(
        provider: RecoveryProvider,
        worktree_form: BoardWorktreeForm,
        record: Option<&RecoveryRecord>,
        code: &'static str,
    ) -> Self {
        Self {
            state: RecoveryDeliveryState::Pending,
            code,
            retryable: true,
            provider: provider_name(provider),
            worktree_form,
            entry_id: record.map(|record| record.intent.entry.id.clone()),
            disposition: None,
            revision: record.map(|record| record.revision),
            snapshot_refresh_pending: None,
        }
    }

    fn conflicted(
        provider: RecoveryProvider,
        worktree_form: BoardWorktreeForm,
        record: Option<&RecoveryRecord>,
        code: &'static str,
    ) -> Self {
        Self {
            state: RecoveryDeliveryState::Conflicted,
            code,
            retryable: false,
            provider: provider_name(provider),
            worktree_form,
            entry_id: record.map(|record| record.intent.entry.id.clone()),
            disposition: None,
            revision: record.map(|record| record.revision),
            snapshot_refresh_pending: None,
        }
    }

    fn acknowledged(
        provider: RecoveryProvider,
        worktree_form: BoardWorktreeForm,
        record: &RecoveryRecord,
        disposition: Option<BoardExactAppendDisposition>,
        snapshot_refresh_pending: bool,
    ) -> Self {
        Self {
            state: RecoveryDeliveryState::Acknowledged,
            code: "acknowledged",
            retryable: false,
            provider: provider_name(provider),
            worktree_form,
            entry_id: Some(record.intent.entry.id.clone()),
            disposition: disposition.map(|value| match value {
                BoardExactAppendDisposition::Appended => "appended".to_string(),
                BoardExactAppendDisposition::Replayed => "replayed".to_string(),
            }),
            revision: Some(record.revision),
            snapshot_refresh_pending: snapshot_refresh_pending.then_some(true),
        }
    }
}

fn provider_name(provider: RecoveryProvider) -> &'static str {
    match provider {
        RecoveryProvider::Local => "local",
        RecoveryProvider::Slack => "slack",
        RecoveryProvider::Teams => "teams",
    }
}

/// One provider selection captured once for the entire delivery attempt.
pub(crate) struct RecoveryDeliveryRoute {
    provider_kind: RecoveryProvider,
    provider: Box<dyn BoardProvider>,
}

impl RecoveryDeliveryRoute {
    pub(crate) fn new(provider_kind: RecoveryProvider, provider: Box<dyn BoardProvider>) -> Self {
        Self {
            provider_kind,
            provider,
        }
    }
}

/// Validated, non-public delivery authority. The opaque Recovery authority is
/// exposed only to the Store factory; callers cannot supply any of its fields.
#[derive(Clone)]
pub(crate) struct RecoveryDeliveryAuthority {
    authority: RecoveryAuthority,
    author: String,
    agent_id: String,
    origin_branch: Option<String>,
    worktree_form: BoardWorktreeForm,
}

impl RecoveryDeliveryAuthority {
    pub(crate) fn new(
        project_id: impl Into<String>,
        session_id: impl Into<String>,
        author: impl Into<String>,
        agent_id: impl Into<String>,
        origin_branch: Option<String>,
        worktree_form: BoardWorktreeForm,
    ) -> Result<Self, RecoveryError> {
        Ok(Self {
            authority: RecoveryAuthority::new(project_id, session_id)?,
            author: author.into(),
            agent_id: agent_id.into(),
            origin_branch,
            worktree_form,
        })
    }

    #[cfg(test)]
    pub(crate) fn authority(&self) -> &RecoveryAuthority {
        &self.authority
    }

    #[cfg(test)]
    pub(crate) fn origin_branch(&self) -> Option<&str> {
        self.origin_branch.as_deref()
    }

    #[cfg(test)]
    pub(crate) fn worktree_form(&self) -> BoardWorktreeForm {
        self.worktree_form
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RecoveryDeliveryAuthorityError;

/// Resolve the current durable Host Session without migrating or synthesizing
/// a ledger. Exact worktree/repository/binding checks fail closed. Worktree
/// form is structural: attached HEAD is Branch-backed, detached HEAD is
/// Ephemeral, and an unreadable HEAD is Unknown; filesystem name prefixes are
/// never consulted.
pub(crate) fn resolve_delivery_authority(
    repo_root: &Path,
    ambient_session_id: Option<&str>,
) -> Result<RecoveryDeliveryAuthority, RecoveryDeliveryAuthorityError> {
    let session_id = ambient_session_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(RecoveryDeliveryAuthorityError)?;
    gwt_agent::validate_session_id_path_component(session_id)
        .map_err(|_| RecoveryDeliveryAuthorityError)?;
    let path = gwt_sessions_dir().join(format!("{session_id}.toml"));
    let session = Session::load(&path).map_err(|_| RecoveryDeliveryAuthorityError)?;
    if session.id != session_id || session.runtime_target != LaunchRuntimeTarget::Host {
        return Err(RecoveryDeliveryAuthorityError);
    }
    SessionExecutionIdentity::from_session(&session)
        .map_err(|_| RecoveryDeliveryAuthorityError)?
        .ok_or(RecoveryDeliveryAuthorityError)?;

    let invocation = dunce::canonicalize(repo_root).map_err(|_| RecoveryDeliveryAuthorityError)?;
    let session_worktree =
        dunce::canonicalize(&session.worktree_path).map_err(|_| RecoveryDeliveryAuthorityError)?;
    if invocation != session_worktree {
        return Err(RecoveryDeliveryAuthorityError);
    }
    let repository =
        gwt_git::Repository::discover(&invocation).map_err(|_| RecoveryDeliveryAuthorityError)?;
    let repository_root =
        dunce::canonicalize(repository.path()).map_err(|_| RecoveryDeliveryAuthorityError)?;
    if repository_root != invocation {
        return Err(RecoveryDeliveryAuthorityError);
    }

    let project_state_root = crate::validated_project_state_root_for_session_recovery(&session)
        .map_err(|_| RecoveryDeliveryAuthorityError)?;
    if project_state_root.as_os_str().is_empty() {
        return Err(RecoveryDeliveryAuthorityError);
    }
    let observed_project = project_scope_hash(&invocation);
    if session.repo_hash.as_deref() != Some(observed_project.as_str()) {
        return Err(RecoveryDeliveryAuthorityError);
    }

    let (worktree_form, origin_branch) = classify_worktree_origin(&repository);
    if let Some(branch) = origin_branch.as_deref() {
        if canonical_branch(&session.branch) != canonical_branch(branch) {
            return Err(RecoveryDeliveryAuthorityError);
        }
    }
    RecoveryDeliveryAuthority::new(
        observed_project.as_str(),
        session.id,
        session.display_name,
        session.agent_id.command(),
        origin_branch,
        worktree_form,
    )
    .map_err(|_| RecoveryDeliveryAuthorityError)
}

fn canonical_branch(value: &str) -> &str {
    let value = value.trim();
    let value = value.strip_prefix("refs/heads/").unwrap_or(value);
    let value = value.strip_prefix("refs/remotes/").unwrap_or(value);
    value.strip_prefix("origin/").unwrap_or(value)
}

/// Distinguish a genuine detached HEAD from an unreadable/corrupt HEAD. The
/// repository wrapper deliberately maps every non-zero `symbolic-ref` result
/// to `None`, so an object-resolution probe is required before treating that
/// value as structural evidence of an ephemeral worktree.
pub(crate) fn classify_worktree_origin(
    repository: &gwt_git::Repository,
) -> (BoardWorktreeForm, Option<String>) {
    match repository.current_branch() {
        Ok(Some(branch)) if !branch.trim().is_empty() => {
            (BoardWorktreeForm::BranchBacked, Some(branch))
        }
        Ok(Some(_)) | Err(_) => (BoardWorktreeForm::Unknown, None),
        Ok(None) => {
            let head_is_readable = gwt_core::process::hidden_command("git")
                .arg("-C")
                .arg(repository.path())
                .args(["rev-parse", "--verify", "HEAD"])
                .output()
                .is_ok_and(|output| output.status.success());
            if head_is_readable {
                (BoardWorktreeForm::Ephemeral, None)
            } else {
                (BoardWorktreeForm::Unknown, None)
            }
        }
    }
}

/// Production entrypoint used only when `board.post` carries `intent_id`.
pub fn deliver_board_recovery(
    repo_root: &Path,
    intent_id: &str,
    input: RecoveryDeliveryInput,
) -> RecoveryDeliveryReport {
    deliver_with(
        repo_root,
        intent_id,
        input,
        board_provider::recovery_route_for,
        |repo_root| {
            resolve_delivery_authority(repo_root, std::env::var(GWT_SESSION_ID_ENV).ok().as_deref())
        },
        |repo_root, authority| {
            let store = RecoveryStore::for_repo(repo_root, authority.authority.session_id.clone())?;
            if store.authority() != &authority.authority {
                return Err(RecoveryError::AuthorityMismatch);
            }
            Ok(store)
        },
    )
}

/// Testable orchestration kernel. The three closures make the ordering proof
/// observable without adding global provider/Store overrides.
pub(crate) fn deliver_with<ResolveRoute, ResolveAuthority, StoreFactory>(
    repo_root: &Path,
    intent_id: &str,
    input: RecoveryDeliveryInput,
    resolve_route: ResolveRoute,
    resolve_authority: ResolveAuthority,
    store_factory: StoreFactory,
) -> RecoveryDeliveryReport
where
    ResolveRoute: FnOnce(&Path) -> RecoveryDeliveryRoute,
    ResolveAuthority:
        FnOnce(&Path) -> Result<RecoveryDeliveryAuthority, RecoveryDeliveryAuthorityError>,
    StoreFactory: FnOnce(&Path, &RecoveryDeliveryAuthority) -> Result<RecoveryStore, RecoveryError>,
{
    // Provider selection is captured once. Unsupported providers return before
    // Session/project derivation, payload construction, or Store creation.
    let RecoveryDeliveryRoute {
        provider_kind,
        provider,
    } = resolve_route(repo_root);
    if provider.recovery_capability() != BoardRecoveryCapability::ExactAcknowledgement {
        return RecoveryDeliveryReport::refused(provider_kind, "unsupported_provider", false);
    }
    if provider_kind != RecoveryProvider::Local {
        return RecoveryDeliveryReport::refused(provider_kind, "unsupported_provider", false);
    }

    if !valid_intent_id(intent_id) {
        return RecoveryDeliveryReport::refused(provider_kind, "invalid_intent", false);
    }
    let authority = match resolve_authority(repo_root) {
        Ok(authority) => authority,
        Err(_) => {
            return RecoveryDeliveryReport::refused(provider_kind, "authority_unavailable", false)
        }
    };
    let recovery_id = derive_identity("gwt-board-recovery-id-v1", &authority, intent_id);
    let entry_id = derive_identity("gwt-board-recovery-entry-v1", &authority, intent_id);
    let prepare_operation = derive_identity("gwt-board-recovery-prepare-v1", &authority, intent_id);
    let acknowledge_operation =
        derive_identity("gwt-board-recovery-acknowledge-v1", &authority, intent_id);

    let entry = match canonical_entry(repo_root, input, &authority, &recovery_id, &entry_id) {
        Ok(entry) => entry,
        Err(_) => {
            return RecoveryDeliveryReport::refused(
                provider_kind,
                "unsafe_or_invalid_payload",
                false,
            )
        }
    };
    let intent = match RecoveryIntent::new(
        recovery_id.clone(),
        provider_kind,
        authority.worktree_form,
        entry,
    ) {
        Ok(intent) => intent,
        Err(_) => {
            return RecoveryDeliveryReport::refused(
                provider_kind,
                "unsafe_or_invalid_payload",
                false,
            )
        }
    };

    // Store construction is intentionally lazy: capability, authority, and
    // canonical payload validation have all completed before this point.
    let store = match store_factory(repo_root, &authority) {
        Ok(store) => store,
        Err(_) => return RecoveryDeliveryReport::refused(provider_kind, "store_unavailable", true),
    };
    match store.prepare(&prepare_operation, intent) {
        Ok(_) => {}
        Err(RecoveryError::OperationConflict | RecoveryError::AlreadyExists) => {
            return settle_changed_intent(
                &store,
                &recovery_id,
                provider_kind,
                authority.worktree_form,
                &authority,
                intent_id,
            );
        }
        Err(RecoveryError::Storage) => {
            return latest_after_prepare_unknown(
                &store,
                &recovery_id,
                provider_kind,
                authority.worktree_form,
            )
        }
        Err(_) => {
            return RecoveryDeliveryReport::refused(provider_kind, "prepare_unavailable", true)
        }
    }

    // A prepare replay returns the revision on which that operation first
    // committed, so always reread the current head before deciding effects.
    let record = match store.get(&recovery_id) {
        Ok(Some(record)) => record,
        Ok(None) | Err(_) => {
            return RecoveryDeliveryReport::pending(
                provider_kind,
                authority.worktree_form,
                None,
                "prepared_state_unknown",
            )
        }
    };
    match record.state {
        RecoveryState::Acknowledged => {
            return RecoveryDeliveryReport::acknowledged(
                provider_kind,
                record.intent.worktree_form,
                &record,
                None,
                false,
            )
        }
        RecoveryState::Conflicted => {
            return RecoveryDeliveryReport::conflicted(
                provider_kind,
                record.intent.worktree_form,
                Some(&record),
                "conflicted",
            )
        }
        RecoveryState::Pending => {}
    }

    let receipt = match provider.post_recovery_entry_exact(repo_root, record.intent.entry.clone()) {
        Ok(receipt) => receipt,
        Err(BoardExactAppendError::Storage(_)) => {
            return RecoveryDeliveryReport::pending(
                provider_kind,
                record.intent.worktree_form,
                Some(&record),
                "provider_outcome_unknown",
            )
        }
        Err(BoardExactAppendError::Conflict(conflict)) => {
            let (kind, code) = match conflict.kind {
                BoardExactAppendConflictKind::PayloadMismatch => (
                    RecoveryConflictKind::PayloadMismatch,
                    "provider_payload_conflict",
                ),
                BoardExactAppendConflictKind::DuplicateIdentity => (
                    RecoveryConflictKind::DuplicateIdentity,
                    "provider_duplicate_identity",
                ),
            };
            return settle_conflict(
                &store,
                &record,
                provider_kind,
                &authority,
                intent_id,
                kind,
                code,
            );
        }
        Err(BoardExactAppendError::Unsupported) => {
            return settle_conflict(
                &store,
                &record,
                provider_kind,
                &authority,
                intent_id,
                RecoveryConflictKind::ReceiptMismatch,
                "provider_capability_changed",
            );
        }
    };

    if receipt.entry_id != record.intent.entry.id
        || receipt.payload_digest != record.intent.payload_digest
        || record.intent.provider != provider_kind
    {
        return settle_conflict(
            &store,
            &record,
            provider_kind,
            &authority,
            intent_id,
            RecoveryConflictKind::ReceiptMismatch,
            "receipt_mismatch",
        );
    }
    let provider_receipt = match RecoveryProviderReceipt::new(receipt.entry_id.clone()) {
        Ok(receipt) if receipt.as_str() == record.intent.entry.id => receipt,
        _ => {
            return settle_conflict(
                &store,
                &record,
                provider_kind,
                &authority,
                intent_id,
                RecoveryConflictKind::ReceiptMismatch,
                "receipt_mismatch",
            )
        }
    };
    let acknowledgement = match RecoveryAcknowledgement::for_record(&record, provider_receipt) {
        Ok(acknowledgement) => acknowledgement,
        Err(_) => {
            return settle_conflict(
                &store,
                &record,
                provider_kind,
                &authority,
                intent_id,
                RecoveryConflictKind::ReceiptMismatch,
                "receipt_mismatch",
            )
        }
    };

    match store.acknowledge(
        &recovery_id,
        record.revision,
        &acknowledge_operation,
        acknowledgement,
    ) {
        Ok(write) => RecoveryDeliveryReport::acknowledged(
            provider_kind,
            write.record.intent.worktree_form,
            &write.record,
            Some(receipt.disposition),
            receipt.refresh_error.is_some(),
        ),
        Err(RecoveryError::StaleRevision { .. } | RecoveryError::TerminalState) => {
            latest_after_race(
                &store,
                &recovery_id,
                provider_kind,
                authority.worktree_form,
                "acknowledgement_race",
            )
        }
        Err(_) => RecoveryDeliveryReport::pending(
            provider_kind,
            record.intent.worktree_form,
            Some(&record),
            "acknowledgement_unknown",
        ),
    }
}

fn latest_after_prepare_unknown(
    store: &RecoveryStore,
    recovery_id: &str,
    provider: RecoveryProvider,
    worktree_form: BoardWorktreeForm,
) -> RecoveryDeliveryReport {
    match store.get(recovery_id) {
        Ok(Some(record)) if record.state == RecoveryState::Acknowledged => {
            RecoveryDeliveryReport::acknowledged(
                provider,
                record.intent.worktree_form,
                &record,
                None,
                false,
            )
        }
        Ok(Some(record)) if record.state == RecoveryState::Conflicted => {
            RecoveryDeliveryReport::conflicted(
                provider,
                record.intent.worktree_form,
                Some(&record),
                "conflicted",
            )
        }
        Ok(Some(record)) => RecoveryDeliveryReport::pending(
            provider,
            record.intent.worktree_form,
            Some(&record),
            "prepare_outcome_unknown",
        ),
        _ => RecoveryDeliveryReport::pending(
            provider,
            worktree_form,
            None,
            "prepare_outcome_unknown",
        ),
    }
}

fn settle_changed_intent(
    store: &RecoveryStore,
    recovery_id: &str,
    provider: RecoveryProvider,
    worktree_form: BoardWorktreeForm,
    authority: &RecoveryDeliveryAuthority,
    intent_id: &str,
) -> RecoveryDeliveryReport {
    let latest = match store.get(recovery_id) {
        Ok(Some(record)) => record,
        _ => {
            return RecoveryDeliveryReport::conflicted(
                provider,
                worktree_form,
                None,
                "intent_payload_changed",
            )
        }
    };
    if latest.state != RecoveryState::Pending {
        return RecoveryDeliveryReport::conflicted(
            provider,
            latest.intent.worktree_form,
            Some(&latest),
            "intent_payload_changed",
        );
    }
    settle_conflict(
        store,
        &latest,
        provider,
        authority,
        intent_id,
        RecoveryConflictKind::PayloadMismatch,
        "intent_payload_changed",
    )
}

fn settle_conflict(
    store: &RecoveryStore,
    record: &RecoveryRecord,
    provider: RecoveryProvider,
    authority: &RecoveryDeliveryAuthority,
    intent_id: &str,
    kind: RecoveryConflictKind,
    code: &'static str,
) -> RecoveryDeliveryReport {
    let operation_id = derive_identity(conflict_domain(kind), authority, intent_id);
    match store.mark_conflicted(&record.recovery_id, record.revision, &operation_id, kind) {
        Ok(write) => RecoveryDeliveryReport::conflicted(
            provider,
            write.record.intent.worktree_form,
            Some(&write.record),
            code,
        ),
        Err(RecoveryError::StaleRevision { .. } | RecoveryError::TerminalState) => {
            latest_after_race(
                store,
                &record.recovery_id,
                provider,
                record.intent.worktree_form,
                code,
            )
        }
        Err(_) => RecoveryDeliveryReport::pending(
            provider,
            record.intent.worktree_form,
            Some(record),
            "conflict_settlement_unknown",
        ),
    }
}

fn latest_after_race(
    store: &RecoveryStore,
    recovery_id: &str,
    provider: RecoveryProvider,
    worktree_form: BoardWorktreeForm,
    pending_code: &'static str,
) -> RecoveryDeliveryReport {
    match store.get(recovery_id) {
        Ok(Some(record)) if record.state == RecoveryState::Acknowledged => {
            RecoveryDeliveryReport::acknowledged(
                provider,
                record.intent.worktree_form,
                &record,
                None,
                false,
            )
        }
        Ok(Some(record)) if record.state == RecoveryState::Conflicted => {
            RecoveryDeliveryReport::conflicted(
                provider,
                record.intent.worktree_form,
                Some(&record),
                pending_code,
            )
        }
        Ok(Some(record)) => RecoveryDeliveryReport::pending(
            provider,
            record.intent.worktree_form,
            Some(&record),
            pending_code,
        ),
        _ => RecoveryDeliveryReport::pending(provider, worktree_form, None, pending_code),
    }
}

fn conflict_domain(kind: RecoveryConflictKind) -> &'static str {
    match kind {
        RecoveryConflictKind::PayloadMismatch => "gwt-board-recovery-conflict-payload-v1",
        RecoveryConflictKind::DuplicateIdentity => "gwt-board-recovery-conflict-duplicate-v1",
        RecoveryConflictKind::ReceiptMismatch => "gwt-board-recovery-conflict-receipt-v1",
        RecoveryConflictKind::StorageUncertain => "gwt-board-recovery-conflict-storage-v1",
    }
}

fn valid_intent_id(intent_id: &str) -> bool {
    !intent_id.is_empty()
        && intent_id.len() <= MAX_INTENT_ID_LEN
        && intent_id.trim() == intent_id
        && intent_id.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'@')
        })
}

fn derive_identity(domain: &str, authority: &RecoveryDeliveryAuthority, intent_id: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(domain.as_bytes());
    digest.update([0]);
    digest.update(authority.authority.project_id.as_bytes());
    digest.update([0]);
    digest.update(authority.authority.session_id.as_bytes());
    digest.update([0]);
    digest.update(intent_id.as_bytes());
    format!("v1-{}", hex::encode(digest.finalize()))
}

fn canonical_entry(
    repo_root: &Path,
    input: RecoveryDeliveryInput,
    authority: &RecoveryDeliveryAuthority,
    recovery_id: &str,
    entry_id: &str,
) -> Result<BoardEntry, ()> {
    let mentions = input
        .mentions
        .into_iter()
        .map(|mut mention| {
            mention.target = redact(&mention.target);
            mention.label = mention.label.map(|label| redact(&label));
            mention
        })
        .collect::<Vec<_>>();
    let mut audience = Vec::new();
    if !input.broadcast {
        if let gwt_core::coordination::BoardAudienceScope::Workspace(workspace_id) =
            current_session_board_scope(repo_root, Some(&authority.authority.session_id))
                .map_err(|_| ())?
        {
            audience.push(workspace_id);
        }
        audience.extend(
            input
                .workspace_audience
                .into_iter()
                .map(|value| redact(&value)),
        );
        audience.extend(
            post_audience_for_session(repo_root, None, &mentions, false)
                .map_err(|_| ())?
                .unwrap_or_default(),
        );
    }

    let mut draft = BoardEntryDraft::new(
        AuthorKind::Agent,
        redact(&authority.author),
        input.kind,
        redact(&input.body),
    );
    draft.title = input.title.map(|value| redact(&value));
    draft.title_summary = input.title_summary.map(|value| redact(&value));
    draft.parent_id = input.parent.map(|value| redact(&value));
    draft.related_topics = input
        .topics
        .into_iter()
        .map(|value| redact(&value))
        .collect();
    draft.related_owners = input
        .owners
        .into_iter()
        .map(|value| redact(&value))
        .collect();
    draft.target_owners = input
        .targets
        .into_iter()
        .map(|value| redact(&value))
        .collect();
    draft.mentions = mentions;
    draft.audience = audience;
    draft.origin = BoardOrigin::new(
        authority.origin_branch.clone().unwrap_or_default(),
        authority.authority.session_id.clone(),
        redact(&authority.agent_id),
    )
    .with_worktree_form(authority.worktree_form)
    .with_recovery_id(recovery_id);
    let mut entry = draft.finalize().map_err(|_| ())?;
    entry.id = entry_id.to_string();
    entry.body_html = None;
    Ok(entry)
}

fn redact(value: &str) -> String {
    gwt_core::process_console::redact_line(value)
}
