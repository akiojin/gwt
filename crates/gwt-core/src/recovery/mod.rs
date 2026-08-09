//! Durable recovery intents for exact Board delivery.
//!
//! Recovery storage is deliberately separate from the Board event log. A
//! pending intent is committed before delivery, and an acknowledgement is a
//! second immutable revision after the provider receipt has been validated.

use std::{
    collections::HashSet,
    fmt,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    coordination::{
        board_entry_payload_digest, normalize_board_audience, normalize_board_mentions,
        sanitize_board_terms, AuthorKind, BoardEntry, BoardEntryKind, BoardMention,
        BoardWorktreeForm,
    },
    paths::{gwt_project_dir_for_repo_path, project_scope_hash},
};

const RECOVERY_SCHEMA_VERSION: u32 = 1;
const RECOVERY_BOARD_PAYLOAD_SCHEMA_VERSION: u32 = 1;
const RECOVERY_HEAD_SCHEMA_VERSION: u32 = 1;
const DIGEST_VERSION: u32 = 1;
const REVISION_DIGITS: usize = 20;
const MAX_AUTHORITY_LEN: usize = 256;
const MAX_ENTRY_ID_LEN: usize = 256;
const MAX_OPERATION_ID_LEN: usize = 256;
const MAX_RECOVERY_ID_LEN: usize = 1024;
const MAX_PROVIDER_RECEIPT_LEN: usize = 512;

pub type RecoveryResult<T> = std::result::Result<T, RecoveryError>;

/// Errors deliberately omit caller values and filesystem paths so a failed
/// recovery operation cannot disclose an authority, credential, or intent ID
/// through `Display` / `Debug`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RecoveryError {
    #[error("recovery authority is invalid")]
    InvalidAuthority,
    #[error("recovery identifier is invalid")]
    InvalidRecoveryId,
    #[error("recovery operation identifier is invalid")]
    InvalidOperationId,
    #[error("recovery provider receipt is not public-safe")]
    InvalidProviderReceipt,
    #[error("recovery delivery provider does not support exact acknowledgement")]
    UnsupportedProvider,
    #[error("Board recovery entry identifier cannot be acknowledged exactly")]
    InvalidEntryId,
    #[error("Board recovery entry is not in canonical sanitized form")]
    UnsanitizedEntry,
    #[error("recovery intent payload digest is invalid")]
    InvalidPayloadDigest,
    #[error("recovery authority does not own this record")]
    AuthorityMismatch,
    #[error("recovery record was not found")]
    NotFound,
    #[error("a recovery record already exists for this identity")]
    AlreadyExists,
    #[error("recovery operation identifier was reused with another payload")]
    OperationConflict,
    #[error("recovery compare-and-swap revision is stale (expected {expected}, actual {actual})")]
    StaleRevision { expected: u64, actual: u64 },
    #[error("recovery acknowledgement does not match the pending intent: {0:?}")]
    AcknowledgementMismatch(AcknowledgementMismatchKind),
    #[error("recovery record is already terminal")]
    TerminalState,
    #[error("recovery store contains a corrupt committed revision")]
    CorruptStore,
    #[error("recovery store I/O failed")]
    Storage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcknowledgementMismatchKind {
    RecoveryIdentity,
    EntryIdentity,
    ProjectAuthority,
    SessionAuthority,
    Provider,
    PayloadDigest,
    ProviderReceipt,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryProvider {
    Local,
    Slack,
    Teams,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RecoveryAuthority {
    pub project_id: String,
    pub session_id: String,
}

impl fmt::Debug for RecoveryAuthority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RecoveryAuthority")
            .field("project_id", &"<redacted>")
            .field("session_id", &"<redacted>")
            .finish()
    }
}

impl RecoveryAuthority {
    pub fn new(
        project_id: impl Into<String>,
        session_id: impl Into<String>,
    ) -> RecoveryResult<Self> {
        let authority = Self {
            project_id: project_id.into(),
            session_id: session_id.into(),
        };
        validate_authority(&authority)?;
        Ok(authority)
    }
}

/// Opaque provider acknowledgement safe to expose in a public recovery
/// read-model. Remote response bodies and credentials never enter this type.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct RecoveryProviderReceipt(String);

impl fmt::Debug for RecoveryProviderReceipt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("RecoveryProviderReceipt")
            .field(&"<redacted>")
            .finish()
    }
}

impl RecoveryProviderReceipt {
    pub fn new(value: impl Into<String>) -> RecoveryResult<Self> {
        let value = value.into();
        if !is_public_token(&value, MAX_PROVIDER_RECEIPT_LEN)
            || !recovery_text_is_public_safe(&value)
        {
            return Err(RecoveryError::InvalidProviderReceipt);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct RecoveryIntent {
    pub recovery_id: String,
    pub provider: RecoveryProvider,
    pub worktree_form: BoardWorktreeForm,
    pub entry: BoardEntry,
    pub payload_digest: String,
}

impl fmt::Debug for RecoveryIntent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RecoveryIntent")
            .field("recovery_id", &"<redacted>")
            .field("provider", &self.provider)
            .field("worktree_form", &self.worktree_form)
            .field("entry", &"<redacted>")
            .field("payload_digest", &"<redacted>")
            .finish()
    }
}

impl RecoveryIntent {
    pub fn new(
        recovery_id: impl Into<String>,
        provider: RecoveryProvider,
        worktree_form: BoardWorktreeForm,
        entry: BoardEntry,
    ) -> RecoveryResult<Self> {
        let recovery_id = recovery_id.into();
        validate_recovery_id(&recovery_id)?;
        validate_provider(provider)?;
        validate_sanitized_entry(&entry, &recovery_id, worktree_form)?;
        let payload_digest =
            board_entry_payload_digest(&entry).map_err(|_| RecoveryError::UnsanitizedEntry)?;
        Ok(Self {
            recovery_id,
            provider,
            worktree_form,
            entry,
            payload_digest,
        })
    }
}

/// Stable allowlisted wire shape for the only Board payload that may enter a
/// recovery revision. Keeping this separate from `BoardEntry` prevents a
/// future private or derived Board field from becoming durable implicitly.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedBoardPayloadV1 {
    schema_version: u32,
    id: String,
    author_kind: AuthorKind,
    author: String,
    kind: BoardEntryKind,
    body: String,
    title: Option<String>,
    title_summary: Option<String>,
    state: Option<String>,
    parent_id: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    related_topics: Vec<String>,
    related_owners: Vec<String>,
    origin_branch: Option<String>,
    origin_session_id: Option<String>,
    origin_agent_id: Option<String>,
    origin_worktree_form: Option<BoardWorktreeForm>,
    origin_recovery_id: Option<String>,
    target_owners: Vec<String>,
    mentions: Vec<BoardMention>,
    audience: Vec<String>,
}

impl From<&BoardEntry> for PersistedBoardPayloadV1 {
    fn from(entry: &BoardEntry) -> Self {
        Self {
            schema_version: RECOVERY_BOARD_PAYLOAD_SCHEMA_VERSION,
            id: entry.id.clone(),
            author_kind: entry.author_kind.clone(),
            author: entry.author.clone(),
            kind: entry.kind.clone(),
            body: entry.body.clone(),
            title: entry.title.clone(),
            title_summary: entry.title_summary.clone(),
            state: entry.state.clone(),
            parent_id: entry.parent_id.clone(),
            created_at: entry.created_at,
            updated_at: entry.updated_at,
            related_topics: entry.related_topics.clone(),
            related_owners: entry.related_owners.clone(),
            origin_branch: entry.origin_branch.clone(),
            origin_session_id: entry.origin_session_id.clone(),
            origin_agent_id: entry.origin_agent_id.clone(),
            origin_worktree_form: entry.origin_worktree_form,
            origin_recovery_id: entry.origin_recovery_id.clone(),
            target_owners: entry.target_owners.clone(),
            mentions: entry.mentions.clone(),
            audience: entry.audience.clone(),
        }
    }
}

impl PersistedBoardPayloadV1 {
    fn into_board_entry(self) -> BoardEntry {
        BoardEntry {
            id: self.id,
            author_kind: self.author_kind,
            author: self.author,
            kind: self.kind,
            body: self.body,
            title: self.title,
            title_summary: self.title_summary,
            state: self.state,
            parent_id: self.parent_id,
            created_at: self.created_at,
            updated_at: self.updated_at,
            related_topics: self.related_topics,
            related_owners: self.related_owners,
            origin_branch: self.origin_branch,
            origin_session_id: self.origin_session_id,
            origin_agent_id: self.origin_agent_id,
            origin_worktree_form: self.origin_worktree_form,
            origin_recovery_id: self.origin_recovery_id,
            target_owners: self.target_owners,
            mentions: self.mentions,
            audience: self.audience,
            body_html: None,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedRecoveryIntent {
    recovery_id: String,
    provider: RecoveryProvider,
    worktree_form: BoardWorktreeForm,
    entry: PersistedBoardPayloadV1,
    payload_digest: String,
}

impl Serialize for RecoveryIntent {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        PersistedRecoveryIntent {
            recovery_id: self.recovery_id.clone(),
            provider: self.provider,
            worktree_form: self.worktree_form,
            entry: PersistedBoardPayloadV1::from(&self.entry),
            payload_digest: self.payload_digest.clone(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for RecoveryIntent {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let persisted = PersistedRecoveryIntent::deserialize(deserializer)?;
        if persisted.entry.schema_version != RECOVERY_BOARD_PAYLOAD_SCHEMA_VERSION {
            return Err(serde::de::Error::custom(
                "unsupported recovery Board payload schema",
            ));
        }
        Ok(Self {
            recovery_id: persisted.recovery_id,
            provider: persisted.provider,
            worktree_form: persisted.worktree_form,
            entry: persisted.entry.into_board_entry(),
            payload_digest: persisted.payload_digest,
        })
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RecoveryAcknowledgement {
    pub recovery_id: String,
    pub entry_id: String,
    pub authority: RecoveryAuthority,
    pub provider: RecoveryProvider,
    pub payload_digest: String,
    pub provider_receipt: RecoveryProviderReceipt,
}

impl fmt::Debug for RecoveryAcknowledgement {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RecoveryAcknowledgement")
            .field("recovery_id", &"<redacted>")
            .field("entry_id", &"<redacted>")
            .field("authority", &"<redacted>")
            .field("provider", &self.provider)
            .field("payload_digest", &"<redacted>")
            .field("provider_receipt", &"<redacted>")
            .finish()
    }
}

impl RecoveryAcknowledgement {
    pub fn for_record(
        record: &RecoveryRecord,
        provider_receipt: RecoveryProviderReceipt,
    ) -> RecoveryResult<Self> {
        let acknowledgement = Self {
            recovery_id: record.recovery_id.clone(),
            entry_id: record.intent.entry.id.clone(),
            authority: record.authority.clone(),
            provider: record.intent.provider,
            payload_digest: record.intent.payload_digest.clone(),
            provider_receipt,
        };
        validate_acknowledgement_shape(&acknowledgement)?;
        Ok(acknowledgement)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryState {
    Pending,
    Acknowledged,
    Conflicted,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryConflictKind {
    PayloadMismatch,
    DuplicateIdentity,
    ReceiptMismatch,
    StorageUncertain,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RecoveryRecord {
    pub recovery_id: String,
    pub revision: u64,
    pub state: RecoveryState,
    pub authority: RecoveryAuthority,
    pub intent: RecoveryIntent,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acknowledgement: Option<RecoveryAcknowledgement>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflict: Option<RecoveryConflictKind>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl fmt::Debug for RecoveryRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RecoveryRecord")
            .field("recovery_id", &"<redacted>")
            .field("revision", &self.revision)
            .field("state", &self.state)
            .field("authority", &"<redacted>")
            .field("intent", &self.intent)
            .field("acknowledgement", &self.acknowledgement)
            .field("conflict", &self.conflict)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryWriteDisposition {
    Committed,
    Replayed,
}

#[derive(Clone, PartialEq, Eq)]
pub struct RecoveryWrite {
    pub record: RecoveryRecord,
    pub disposition: RecoveryWriteDisposition,
}

impl fmt::Debug for RecoveryWrite {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RecoveryWrite")
            .field("record", &self.record)
            .field("disposition", &self.disposition)
            .finish()
    }
}

#[derive(Clone)]
pub struct RecoveryStore {
    root: PathBuf,
    authority: RecoveryAuthority,
}

impl fmt::Debug for RecoveryStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RecoveryStore")
            .field("root", &"<redacted>")
            .field("authority", &"<redacted>")
            .finish()
    }
}

impl RecoveryStore {
    /// Construct a store at an explicit root. This is the deterministic entry
    /// point used by tests and isolated callers.
    pub fn new(root: impl Into<PathBuf>, authority: RecoveryAuthority) -> RecoveryResult<Self> {
        validate_authority(&authority)?;
        let base_root = root.into();
        ensure_private_dir(&base_root)?;
        let root = base_root.join(authority_storage_key(&authority));
        ensure_private_dir(&root)?;
        Ok(Self { root, authority })
    }

    /// Construct the canonical machine-local store for a repository/session.
    pub fn for_repo(
        repo_root: impl AsRef<Path>,
        session_id: impl Into<String>,
    ) -> RecoveryResult<Self> {
        let repo_root = repo_root.as_ref();
        let project_hash = project_scope_hash(repo_root);
        let authority = RecoveryAuthority::new(project_hash.as_str(), session_id)?;
        let recovery_root = gwt_project_dir_for_repo_path(repo_root).join("recovery");
        ensure_private_dir(&recovery_root)?;
        Self::new(recovery_root.join("intents"), authority)
    }

    pub fn authority(&self) -> &RecoveryAuthority {
        &self.authority
    }

    pub fn prepare(
        &self,
        operation_id: &str,
        intent: RecoveryIntent,
    ) -> RecoveryResult<RecoveryWrite> {
        validate_operation_id(operation_id)?;
        self.validate_intent(&intent)?;
        let operation_digest = prepare_operation_digest(&intent)?;
        let mut locked = self.lock_record(&intent.recovery_id, true)?;

        if let Some(replay) = replay_operation(&locked.revisions, operation_id, &operation_digest)?
        {
            return Ok(replay);
        }
        if !locked.revisions.is_empty() {
            return Err(RecoveryError::AlreadyExists);
        }

        let now = Utc::now();
        let record = RecoveryRecord {
            recovery_id: intent.recovery_id.clone(),
            revision: 1,
            state: RecoveryState::Pending,
            authority: self.authority.clone(),
            intent,
            acknowledgement: None,
            conflict: None,
            created_at: now,
            updated_at: now,
        };
        let envelope = RevisionEnvelope::new(operation_id, operation_digest, None, record)?;
        persist_revision(&locked.revisions_dir, &locked.heads_dir, &envelope)?;
        let record = envelope.record.clone();
        locked.revisions.push(envelope);
        Ok(RecoveryWrite {
            record,
            disposition: RecoveryWriteDisposition::Committed,
        })
    }

    pub fn acknowledge(
        &self,
        recovery_id: &str,
        expected_revision: u64,
        operation_id: &str,
        acknowledgement: RecoveryAcknowledgement,
    ) -> RecoveryResult<RecoveryWrite> {
        validate_recovery_id(recovery_id)?;
        validate_operation_id(operation_id)?;
        validate_acknowledgement_shape(&acknowledgement)?;
        let operation_digest = acknowledgement_operation_digest(&acknowledgement)?;
        let mut locked = self.lock_record(recovery_id, false)?;

        if let Some(replay) = replay_operation(&locked.revisions, operation_id, &operation_digest)?
        {
            return Ok(replay);
        }
        let current = locked.revisions.last().ok_or(RecoveryError::NotFound)?;
        if current.record.revision != expected_revision {
            return Err(RecoveryError::StaleRevision {
                expected: expected_revision,
                actual: current.record.revision,
            });
        }
        if current.record.state != RecoveryState::Pending {
            return Err(RecoveryError::TerminalState);
        }
        validate_exact_acknowledgement(recovery_id, &current.record, &acknowledgement)?;

        let mut record = current.record.clone();
        record.revision += 1;
        record.state = RecoveryState::Acknowledged;
        record.acknowledgement = Some(acknowledgement);
        record.updated_at = Utc::now();
        let envelope = RevisionEnvelope::new(
            operation_id,
            operation_digest,
            Some(current.revision_digest.clone()),
            record,
        )?;
        persist_revision(&locked.revisions_dir, &locked.heads_dir, &envelope)?;
        let record = envelope.record.clone();
        locked.revisions.push(envelope);
        Ok(RecoveryWrite {
            record,
            disposition: RecoveryWriteDisposition::Committed,
        })
    }

    pub fn mark_conflicted(
        &self,
        recovery_id: &str,
        expected_revision: u64,
        operation_id: &str,
        conflict: RecoveryConflictKind,
    ) -> RecoveryResult<RecoveryWrite> {
        validate_recovery_id(recovery_id)?;
        validate_operation_id(operation_id)?;
        let operation_digest = conflict_operation_digest(recovery_id, conflict)?;
        let mut locked = self.lock_record(recovery_id, false)?;

        if let Some(replay) = replay_operation(&locked.revisions, operation_id, &operation_digest)?
        {
            return Ok(replay);
        }
        let current = locked.revisions.last().ok_or(RecoveryError::NotFound)?;
        if current.record.revision != expected_revision {
            return Err(RecoveryError::StaleRevision {
                expected: expected_revision,
                actual: current.record.revision,
            });
        }
        if current.record.state != RecoveryState::Pending {
            return Err(RecoveryError::TerminalState);
        }

        let mut record = current.record.clone();
        record.revision += 1;
        record.state = RecoveryState::Conflicted;
        record.conflict = Some(conflict);
        record.updated_at = Utc::now();
        let envelope = RevisionEnvelope::new(
            operation_id,
            operation_digest,
            Some(current.revision_digest.clone()),
            record,
        )?;
        persist_revision(&locked.revisions_dir, &locked.heads_dir, &envelope)?;
        let record = envelope.record.clone();
        locked.revisions.push(envelope);
        Ok(RecoveryWrite {
            record,
            disposition: RecoveryWriteDisposition::Committed,
        })
    }

    pub fn get(&self, recovery_id: &str) -> RecoveryResult<Option<RecoveryRecord>> {
        validate_recovery_id(recovery_id)?;
        let record_dir = self.record_dir(recovery_id);
        if !record_dir.exists() {
            return Ok(None);
        }
        let locked = self.lock_record(recovery_id, false)?;
        Ok(locked
            .revisions
            .last()
            .map(|revision| revision.record.clone()))
    }

    pub fn list(&self) -> RecoveryResult<Vec<RecoveryRecord>> {
        let mut records = Vec::new();
        let entries = fs::read_dir(&self.root).map_err(|_| RecoveryError::Storage)?;
        for entry in entries {
            let entry = entry.map_err(|_| RecoveryError::Storage)?;
            let file_type = entry.file_type().map_err(|_| RecoveryError::Storage)?;
            if !file_type.is_dir() {
                continue;
            }
            let name = entry.file_name();
            let Some(key) = name.to_str() else {
                return Err(RecoveryError::CorruptStore);
            };
            if !is_storage_key(key) {
                continue;
            }
            let locked = self.lock_record_dir(entry.path())?;
            if let Some(latest) = locked.revisions.last() {
                records.push(latest.record.clone());
            }
        }
        records.sort_by(|left, right| left.recovery_id.cmp(&right.recovery_id));
        Ok(records)
    }

    fn validate_intent(&self, intent: &RecoveryIntent) -> RecoveryResult<()> {
        validate_recovery_id(&intent.recovery_id)?;
        validate_provider(intent.provider)?;
        validate_sanitized_entry(&intent.entry, &intent.recovery_id, intent.worktree_form)?;
        if intent.entry.origin_session_id.as_deref() != Some(self.authority.session_id.as_str()) {
            return Err(RecoveryError::AuthorityMismatch);
        }
        let expected_digest = board_entry_payload_digest(&intent.entry)
            .map_err(|_| RecoveryError::UnsanitizedEntry)?;
        if intent.payload_digest != expected_digest || !is_versioned_digest(&intent.payload_digest)
        {
            return Err(RecoveryError::InvalidPayloadDigest);
        }
        Ok(())
    }

    fn record_dir(&self, recovery_id: &str) -> PathBuf {
        self.root.join(storage_key(recovery_id))
    }

    fn lock_record(&self, recovery_id: &str, create: bool) -> RecoveryResult<LockedRecord> {
        let record_dir = self.record_dir(recovery_id);
        if !record_dir.exists() && !create {
            return Err(RecoveryError::NotFound);
        }
        if create {
            ensure_private_dir(&record_dir)?;
            ensure_private_dir(&record_dir.join("revisions"))?;
            ensure_private_dir(&record_dir.join("heads"))?;
        }
        self.lock_record_dir(record_dir)
    }

    fn lock_record_dir(&self, record_dir: PathBuf) -> RecoveryResult<LockedRecord> {
        if !record_dir.is_dir() {
            return Err(RecoveryError::CorruptStore);
        }
        ensure_private_dir(&record_dir)?;
        let revisions_dir = record_dir.join("revisions");
        ensure_private_dir(&revisions_dir)?;
        let heads_dir = record_dir.join("heads");
        ensure_private_dir(&heads_dir)?;
        let lock = open_private_lock(&record_dir.join(".lock"))?;
        crate::operation_deadline::lock_exclusive(&lock).map_err(|_| RecoveryError::Storage)?;
        let revisions = load_revisions(&revisions_dir, &heads_dir, &record_dir, &self.authority)?;
        Ok(LockedRecord {
            _lock: lock,
            revisions_dir,
            heads_dir,
            revisions,
        })
    }
}

struct LockedRecord {
    _lock: File,
    revisions_dir: PathBuf,
    heads_dir: PathBuf,
    revisions: Vec<RevisionEnvelope>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct RevisionEnvelope {
    schema_version: u32,
    recovery_id: String,
    revision: u64,
    operation_id: String,
    operation_digest: String,
    previous_revision_digest: Option<String>,
    committed_at: DateTime<Utc>,
    record: RecoveryRecord,
    revision_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct RevisionHead {
    schema_version: u32,
    revision: u64,
    revision_digest: String,
}

impl RevisionEnvelope {
    fn new(
        operation_id: &str,
        operation_digest: String,
        previous_revision_digest: Option<String>,
        record: RecoveryRecord,
    ) -> RecoveryResult<Self> {
        let mut envelope = Self {
            schema_version: RECOVERY_SCHEMA_VERSION,
            recovery_id: record.recovery_id.clone(),
            revision: record.revision,
            operation_id: operation_id.to_string(),
            operation_digest,
            previous_revision_digest,
            committed_at: record.updated_at,
            record,
            revision_digest: String::new(),
        };
        envelope.revision_digest = calculate_revision_digest(&envelope)?;
        Ok(envelope)
    }
}

fn validate_authority(authority: &RecoveryAuthority) -> RecoveryResult<()> {
    if !is_public_token(&authority.project_id, MAX_AUTHORITY_LEN)
        || !is_public_token(&authority.session_id, MAX_AUTHORITY_LEN)
        || !recovery_text_is_public_safe(&authority.project_id)
        || !recovery_text_is_public_safe(&authority.session_id)
    {
        return Err(RecoveryError::InvalidAuthority);
    }
    Ok(())
}

fn validate_recovery_id(recovery_id: &str) -> RecoveryResult<()> {
    if recovery_id.is_empty()
        || recovery_id.len() > MAX_RECOVERY_ID_LEN
        || recovery_id.trim() != recovery_id
        || recovery_id.chars().any(char::is_control)
        || !recovery_text_is_public_safe(recovery_id)
    {
        return Err(RecoveryError::InvalidRecoveryId);
    }
    Ok(())
}

fn validate_operation_id(operation_id: &str) -> RecoveryResult<()> {
    if !is_public_token(operation_id, MAX_OPERATION_ID_LEN)
        || !recovery_text_is_public_safe(operation_id)
    {
        return Err(RecoveryError::InvalidOperationId);
    }
    Ok(())
}

fn validate_provider(provider: RecoveryProvider) -> RecoveryResult<()> {
    if provider != RecoveryProvider::Local {
        return Err(RecoveryError::UnsupportedProvider);
    }
    Ok(())
}

fn validate_entry_id(entry_id: &str) -> RecoveryResult<()> {
    if !is_public_token(entry_id, MAX_ENTRY_ID_LEN) || !recovery_text_is_public_safe(entry_id) {
        return Err(RecoveryError::InvalidEntryId);
    }
    Ok(())
}

fn is_public_token(value: &str, max_len: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_len
        && value.trim() == value
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'@')
        })
}

fn validate_sanitized_entry(
    entry: &BoardEntry,
    recovery_id: &str,
    worktree_form: BoardWorktreeForm,
) -> RecoveryResult<()> {
    validate_entry_id(&entry.id)?;
    let optional_string_is_canonical = |value: &Option<String>| {
        value
            .as_deref()
            .is_none_or(|value| !value.is_empty() && value.trim() == value && !value.contains('\0'))
    };
    if entry.body_html.is_some()
        || entry.body.is_empty()
        || entry.body.trim() != entry.body
        || entry.author.is_empty()
        || entry.author.trim() != entry.author
        || entry.origin_recovery_id.as_deref() != Some(recovery_id)
        || entry.origin_worktree_form != Some(worktree_form)
        || !optional_string_is_canonical(&entry.title)
        || !optional_string_is_canonical(&entry.title_summary)
        || !optional_string_is_canonical(&entry.state)
        || !optional_string_is_canonical(&entry.parent_id)
        || !optional_string_is_canonical(&entry.origin_branch)
        || !optional_string_is_canonical(&entry.origin_session_id)
        || !optional_string_is_canonical(&entry.origin_agent_id)
        || !optional_string_is_canonical(&entry.origin_recovery_id)
        || entry.related_topics != sanitize_board_terms(&entry.related_topics)
        || entry.related_owners != sanitize_board_terms(&entry.related_owners)
        || entry.target_owners != sanitize_board_terms(&entry.target_owners)
        || entry.mentions != normalize_board_mentions(&entry.mentions)
        || entry.audience != normalize_board_audience(entry.audience.clone())
        || !board_entry_is_public_safe(entry)
    {
        return Err(RecoveryError::UnsanitizedEntry);
    }
    Ok(())
}

/// Recovery snapshots are deliberately stricter than ordinary Board storage:
/// they retain only the already-sanitized public payload needed for an exact
/// retry. The delivery coordinator owns construction of that payload; this
/// validation is the final fail-closed boundary before durable preparation.
fn board_entry_is_public_safe(entry: &BoardEntry) -> bool {
    let optional_text_is_safe =
        |value: &Option<String>| value.as_deref().is_none_or(recovery_text_is_public_safe);
    entry_text_values(entry)
        .into_iter()
        .all(recovery_text_is_public_safe)
        && optional_text_is_safe(&entry.title)
        && optional_text_is_safe(&entry.title_summary)
        && optional_text_is_safe(&entry.state)
        && optional_text_is_safe(&entry.parent_id)
        && optional_text_is_safe(&entry.origin_branch)
        && optional_text_is_safe(&entry.origin_session_id)
        && optional_text_is_safe(&entry.origin_agent_id)
        && optional_text_is_safe(&entry.origin_recovery_id)
        && entry.mentions.iter().all(|mention| {
            recovery_text_is_public_safe(&mention.target) && optional_text_is_safe(&mention.label)
        })
}

fn entry_text_values(entry: &BoardEntry) -> Vec<&str> {
    let mut values = vec![
        entry.id.as_str(),
        entry.author.as_str(),
        entry.body.as_str(),
    ];
    values.extend(entry.related_topics.iter().map(String::as_str));
    values.extend(entry.related_owners.iter().map(String::as_str));
    values.extend(entry.target_owners.iter().map(String::as_str));
    values.extend(entry.audience.iter().map(String::as_str));
    values
}

fn recovery_text_is_public_safe(value: &str) -> bool {
    if crate::process_console::redact_line(value) != value
        || contains_absolute_private_path(value)
        || contains_environment_assignment(value)
        || contains_sensitive_key_value(value)
    {
        return false;
    }

    let lowercase = value.to_ascii_lowercase();
    const PRIVATE_MARKERS: &[&str] = &[
        "<analysis>",
        "</analysis>",
        "<think>",
        "</think>",
        "chain of thought",
        "chain-of-thought",
        "hidden reasoning",
        "internal reasoning",
        "provider response body",
        "provider_response_body",
        "access_token=",
        "api_key=",
        "apikey=",
        "password=",
        "credential=",
    ];
    !PRIVATE_MARKERS
        .iter()
        .any(|marker| lowercase.contains(marker))
}

fn contains_absolute_private_path(value: &str) -> bool {
    value
        .split(|character: char| {
            character.is_whitespace()
                || matches!(
                    character,
                    '=' | '(' | ')' | '[' | ']' | '<' | '>' | '"' | '\'' | '`' | ',' | ';'
                )
        })
        .filter(|token| !token.is_empty())
        .any(|token| {
            let normalized = token.replace('\\', "/");
            let lowercase = normalized.to_ascii_lowercase();
            if lowercase.starts_with("http://") || lowercase.starts_with("https://") {
                return false;
            }
            const PRIVATE_UNIX_ROOTS: &[&str] = &[
                "/etc",
                "/home",
                "/opt",
                "/private",
                "/root",
                "/tmp",
                "/usr",
                "/var",
                "/volumes",
                "/workspace",
            ];
            let mac_user_path = token_contains_absolute_root(&normalized, "/Users");
            let unix_absolute = PRIVATE_UNIX_ROOTS
                .iter()
                .any(|root| token_contains_absolute_root(&lowercase, root));
            let windows_absolute = normalized.as_bytes().windows(3).any(|window| {
                window[0].is_ascii_alphabetic() && window[1] == b':' && window[2] == b'/'
            });
            mac_user_path
                || unix_absolute
                || windows_absolute
                || normalized.contains("~/")
                || lowercase.contains("$home/")
                || lowercase.contains("${home}/")
                || normalized.starts_with("//")
        })
}

fn token_contains_absolute_root(token: &str, root: &str) -> bool {
    token.match_indices(root).any(|(index, _)| {
        let suffix_index = index.saturating_add(root.len());
        let suffix_is_boundary = suffix_index == token.len()
            || token
                .as_bytes()
                .get(suffix_index)
                .is_some_and(|byte| *byte == b'/');
        let prefix = &token[..index];
        let prefix_is_boundary = index == 0
            || prefix.ends_with(':')
            || prefix.ends_with('{')
            || prefix.ends_with("file://");
        prefix_is_boundary && suffix_is_boundary
    })
}

fn contains_environment_assignment(value: &str) -> bool {
    value.split_whitespace().any(|token| {
        let token = token.trim_matches(|character: char| {
            matches!(
                character,
                '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>' | '"' | '\'' | '`' | ',' | ';'
            )
        });
        let Some((name, assigned_value)) = token.split_once('=') else {
            return false;
        };
        const PRIVATE_ENV_NAMES: &[&str] = &[
            "HOME",
            "OLDPWD",
            "PATH",
            "PWD",
            "SHELL",
            "TMPDIR",
            "USER",
            "USERNAME",
            "USERPROFILE",
        ];
        let secret_name = ["AUTH", "CREDENTIAL", "KEY", "PASSWORD", "SECRET", "TOKEN"]
            .iter()
            .any(|marker| name.contains(marker));
        let public_numeric_explanation =
            name == "SC" && assigned_value.bytes().all(|byte| byte.is_ascii_digit());
        !assigned_value.is_empty()
            && !name.is_empty()
            && name
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
            && name
                .as_bytes()
                .first()
                .is_some_and(|byte| byte.is_ascii_uppercase() || *byte == b'_')
            && (name.contains('_')
                || PRIVATE_ENV_NAMES.contains(&name)
                || secret_name
                || !public_numeric_explanation)
    })
}

fn contains_sensitive_key_value(value: &str) -> bool {
    let inspection = normalize_escaped_quotes_for_inspection(value);
    let compact = inspection
        .chars()
        .filter(|character| !character.is_ascii_whitespace())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    const SENSITIVE_KEYS: &[&str] = &[
        "access_token",
        "accesstoken",
        "api_key",
        "apikey",
        "authorization",
        "client_secret",
        "credential",
        "password",
        "private_key",
        "refresh_token",
        "refreshtoken",
        "secret",
        "secret_key",
        "token",
    ];
    SENSITIVE_KEYS.iter().any(|key| {
        let quoted = [format!("\"{key}\":"), format!("'{key}':")]
            .iter()
            .any(|pattern| compact.contains(pattern));
        quoted
            || compact.match_indices(key).any(|(start, _)| {
                let before_is_boundary = start == 0
                    || !compact.as_bytes()[start - 1].is_ascii_alphanumeric()
                        && compact.as_bytes()[start - 1] != b'_';
                let after = start.saturating_add(key.len());
                before_is_boundary
                    && matches!(compact.as_bytes().get(after), Some(b':') | Some(b'='))
            })
    })
}

fn normalize_escaped_quotes_for_inspection(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len());
    let mut pending_backslashes = 0usize;
    for character in value.chars() {
        if character == '\\' {
            pending_backslashes = pending_backslashes.saturating_add(1);
            continue;
        }
        if matches!(character, '"' | '\'') {
            pending_backslashes = 0;
        } else {
            for _ in 0..pending_backslashes {
                normalized.push('\\');
            }
            pending_backslashes = 0;
        }
        normalized.push(character);
    }
    for _ in 0..pending_backslashes {
        normalized.push('\\');
    }
    normalized
}

fn validate_acknowledgement_shape(ack: &RecoveryAcknowledgement) -> RecoveryResult<()> {
    validate_recovery_id(&ack.recovery_id)?;
    validate_authority(&ack.authority)?;
    validate_entry_id(&ack.entry_id)?;
    if ack.payload_digest.is_empty()
        || ack.payload_digest.len() > MAX_PROVIDER_RECEIPT_LEN
        || ack.payload_digest.chars().any(char::is_control)
    {
        return Err(RecoveryError::InvalidPayloadDigest);
    }
    if !is_public_token(ack.provider_receipt.as_str(), MAX_PROVIDER_RECEIPT_LEN)
        || !recovery_text_is_public_safe(ack.provider_receipt.as_str())
    {
        return Err(RecoveryError::InvalidProviderReceipt);
    }
    Ok(())
}

fn validate_exact_acknowledgement(
    recovery_id: &str,
    record: &RecoveryRecord,
    ack: &RecoveryAcknowledgement,
) -> RecoveryResult<()> {
    let mismatch = if ack.recovery_id != recovery_id || ack.recovery_id != record.recovery_id {
        Some(AcknowledgementMismatchKind::RecoveryIdentity)
    } else if ack.entry_id != record.intent.entry.id {
        Some(AcknowledgementMismatchKind::EntryIdentity)
    } else if ack.authority.project_id != record.authority.project_id {
        Some(AcknowledgementMismatchKind::ProjectAuthority)
    } else if ack.authority.session_id != record.authority.session_id {
        Some(AcknowledgementMismatchKind::SessionAuthority)
    } else if ack.provider != record.intent.provider {
        Some(AcknowledgementMismatchKind::Provider)
    } else if ack.payload_digest != record.intent.payload_digest {
        Some(AcknowledgementMismatchKind::PayloadDigest)
    } else if ack.provider == RecoveryProvider::Local
        && ack.provider_receipt.as_str() != record.intent.entry.id
    {
        Some(AcknowledgementMismatchKind::ProviderReceipt)
    } else {
        None
    };
    mismatch.map_or(Ok(()), |kind| {
        Err(RecoveryError::AcknowledgementMismatch(kind))
    })
}

fn prepare_operation_digest(intent: &RecoveryIntent) -> RecoveryResult<String> {
    #[derive(Serialize)]
    struct PrepareDigest<'a> {
        kind: &'static str,
        recovery_id: &'a str,
        provider: RecoveryProvider,
        worktree_form: BoardWorktreeForm,
        entry_id: &'a str,
        payload_digest: &'a str,
    }
    digest_serializable(&PrepareDigest {
        kind: "prepare",
        recovery_id: &intent.recovery_id,
        provider: intent.provider,
        worktree_form: intent.worktree_form,
        entry_id: &intent.entry.id,
        payload_digest: &intent.payload_digest,
    })
}

fn acknowledgement_operation_digest(ack: &RecoveryAcknowledgement) -> RecoveryResult<String> {
    #[derive(Serialize)]
    struct AcknowledgementDigest<'a> {
        kind: &'static str,
        acknowledgement: &'a RecoveryAcknowledgement,
    }
    digest_serializable(&AcknowledgementDigest {
        kind: "acknowledge",
        acknowledgement: ack,
    })
}

fn conflict_operation_digest(
    recovery_id: &str,
    conflict: RecoveryConflictKind,
) -> RecoveryResult<String> {
    #[derive(Serialize)]
    struct ConflictDigest<'a> {
        kind: &'static str,
        recovery_id: &'a str,
        conflict: RecoveryConflictKind,
    }
    digest_serializable(&ConflictDigest {
        kind: "mark_conflicted",
        recovery_id,
        conflict,
    })
}

fn digest_serializable(value: &impl Serialize) -> RecoveryResult<String> {
    let bytes = serde_json::to_vec(value).map_err(|_| RecoveryError::Storage)?;
    Ok(format!(
        "v{DIGEST_VERSION}:sha256:{}",
        hex::encode(Sha256::digest(bytes))
    ))
}

fn is_versioned_digest(value: &str) -> bool {
    let prefix = format!("v{DIGEST_VERSION}:sha256:");
    value.strip_prefix(&prefix).is_some_and(|digest| {
        digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

fn replay_operation(
    revisions: &[RevisionEnvelope],
    operation_id: &str,
    operation_digest: &str,
) -> RecoveryResult<Option<RecoveryWrite>> {
    let Some(revision) = revisions
        .iter()
        .find(|revision| revision.operation_id == operation_id)
    else {
        return Ok(None);
    };
    if revision.operation_digest != operation_digest {
        return Err(RecoveryError::OperationConflict);
    }
    Ok(Some(RecoveryWrite {
        record: revision.record.clone(),
        disposition: RecoveryWriteDisposition::Replayed,
    }))
}

fn storage_key(recovery_id: &str) -> String {
    hex::encode(Sha256::digest(recovery_id.as_bytes()))
}

fn authority_storage_key(authority: &RecoveryAuthority) -> String {
    let mut digest = Sha256::new();
    digest.update(b"gwt-recovery-authority-v1\0");
    digest.update(authority.project_id.as_bytes());
    digest.update(b"\0");
    digest.update(authority.session_id.as_bytes());
    hex::encode(digest.finalize())
}

fn is_storage_key(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn load_revisions(
    revisions_dir: &Path,
    heads_dir: &Path,
    record_dir: &Path,
    authority: &RecoveryAuthority,
) -> RecoveryResult<Vec<RevisionEnvelope>> {
    let storage_key = record_dir
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| is_storage_key(value))
        .ok_or(RecoveryError::CorruptStore)?;
    let mut paths = Vec::new();
    for entry in fs::read_dir(revisions_dir).map_err(|_| RecoveryError::Storage)? {
        let entry = entry.map_err(|_| RecoveryError::Storage)?;
        let file_type = entry.file_type().map_err(|_| RecoveryError::Storage)?;
        if !file_type.is_file() {
            return Err(RecoveryError::CorruptStore);
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            return Err(RecoveryError::CorruptStore);
        };
        if let Some(revision) = parse_revision_name(name) {
            paths.push((revision, entry.path()));
        } else if name.ends_with(".json") {
            return Err(RecoveryError::CorruptStore);
        }
    }
    paths.sort_by_key(|(revision, _)| *revision);

    let mut revisions = Vec::with_capacity(paths.len());
    let mut operation_ids = HashSet::new();
    for (index, (revision_number, path)) in paths.into_iter().enumerate() {
        let expected_revision = u64::try_from(index)
            .map_err(|_| RecoveryError::CorruptStore)?
            .saturating_add(1);
        if revision_number != expected_revision {
            return Err(RecoveryError::CorruptStore);
        }
        let bytes = fs::read(path).map_err(|_| RecoveryError::Storage)?;
        let raw: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| RecoveryError::CorruptStore)?;
        let envelope: RevisionEnvelope =
            serde_json::from_value(raw.clone()).map_err(|_| RecoveryError::CorruptStore)?;
        let canonical = serde_json::to_value(&envelope).map_err(|_| RecoveryError::CorruptStore)?;
        if raw != canonical
            || envelope.schema_version != RECOVERY_SCHEMA_VERSION
            || envelope.revision != expected_revision
            || envelope.record.revision != expected_revision
            || envelope.recovery_id != envelope.record.recovery_id
            || storage_key != storage_key_for_envelope(&envelope)
            || !operation_ids.insert(envelope.operation_id.clone())
            || validate_operation_id(&envelope.operation_id).is_err()
            || !is_versioned_digest(&envelope.operation_digest)
            || envelope.committed_at != envelope.record.updated_at
            || envelope.revision_digest != calculate_revision_digest(&envelope)?
        {
            return Err(RecoveryError::CorruptStore);
        }
        if &envelope.record.authority != authority {
            return Err(RecoveryError::AuthorityMismatch);
        }
        let expected_previous = revisions
            .last()
            .map(|previous: &RevisionEnvelope| previous.revision_digest.as_str());
        if envelope.previous_revision_digest.as_deref() != expected_previous {
            return Err(RecoveryError::CorruptStore);
        }
        validate_loaded_record(&envelope.record)?;
        if envelope.operation_digest
            != expected_operation_digest(&envelope.record)
                .map_err(|_| RecoveryError::CorruptStore)?
        {
            return Err(RecoveryError::CorruptStore);
        }
        validate_revision_transition(
            revisions
                .last()
                .map(|previous: &RevisionEnvelope| &previous.record),
            &envelope.record,
        )?;
        revisions.push(envelope);
    }
    reconcile_revision_heads(heads_dir, &revisions)?;
    Ok(revisions)
}

fn expected_operation_digest(record: &RecoveryRecord) -> RecoveryResult<String> {
    match record.state {
        RecoveryState::Pending => prepare_operation_digest(&record.intent),
        RecoveryState::Acknowledged => acknowledgement_operation_digest(
            record
                .acknowledgement
                .as_ref()
                .ok_or(RecoveryError::CorruptStore)?,
        ),
        RecoveryState::Conflicted => conflict_operation_digest(
            &record.recovery_id,
            record.conflict.ok_or(RecoveryError::CorruptStore)?,
        ),
    }
}

fn validate_revision_transition(
    previous: Option<&RecoveryRecord>,
    current: &RecoveryRecord,
) -> RecoveryResult<()> {
    let valid = match previous {
        None => current.revision == 1 && current.state == RecoveryState::Pending,
        Some(previous) => {
            previous.state == RecoveryState::Pending
                && current.state != RecoveryState::Pending
                && current.revision == previous.revision.saturating_add(1)
                && current.recovery_id == previous.recovery_id
                && current.authority == previous.authority
                && current.intent == previous.intent
                && current.created_at == previous.created_at
        }
    };
    if !valid {
        return Err(RecoveryError::CorruptStore);
    }
    Ok(())
}

fn reconcile_revision_heads(
    heads_dir: &Path,
    revisions: &[RevisionEnvelope],
) -> RecoveryResult<()> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(heads_dir).map_err(|_| RecoveryError::Storage)? {
        let entry = entry.map_err(|_| RecoveryError::Storage)?;
        let file_type = entry.file_type().map_err(|_| RecoveryError::Storage)?;
        if !file_type.is_file() {
            return Err(RecoveryError::CorruptStore);
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            return Err(RecoveryError::CorruptStore);
        };
        if let Some(revision) = parse_revision_name(name) {
            paths.push((revision, entry.path()));
        } else if name.ends_with(".json") {
            return Err(RecoveryError::CorruptStore);
        }
    }
    paths.sort_by_key(|(revision, _)| *revision);

    for (index, (revision_number, path)) in paths.iter().enumerate() {
        let expected_revision = u64::try_from(index)
            .map_err(|_| RecoveryError::CorruptStore)?
            .saturating_add(1);
        if *revision_number != expected_revision {
            return Err(RecoveryError::CorruptStore);
        }
        let revision_index = usize::try_from(expected_revision.saturating_sub(1))
            .map_err(|_| RecoveryError::CorruptStore)?;
        let revision = revisions
            .get(revision_index)
            .ok_or(RecoveryError::CorruptStore)?;
        let bytes = fs::read(path).map_err(|_| RecoveryError::Storage)?;
        let raw: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| RecoveryError::CorruptStore)?;
        let head: RevisionHead =
            serde_json::from_value(raw.clone()).map_err(|_| RecoveryError::CorruptStore)?;
        let canonical = serde_json::to_value(&head).map_err(|_| RecoveryError::CorruptStore)?;
        if raw != canonical
            || head.schema_version != RECOVERY_HEAD_SCHEMA_VERSION
            || head.revision != expected_revision
            || head.revision_digest != revision.revision_digest
        {
            return Err(RecoveryError::CorruptStore);
        }
    }

    for revision in revisions.iter().skip(paths.len()) {
        persist_revision_head(heads_dir, revision)?;
    }
    Ok(())
}

fn storage_key_for_envelope(envelope: &RevisionEnvelope) -> String {
    storage_key(&envelope.recovery_id)
}

fn validate_loaded_record(record: &RecoveryRecord) -> RecoveryResult<()> {
    validate_authority(&record.authority).map_err(|_| RecoveryError::CorruptStore)?;
    validate_recovery_id(&record.recovery_id).map_err(|_| RecoveryError::CorruptStore)?;
    if record.intent.recovery_id != record.recovery_id {
        return Err(RecoveryError::CorruptStore);
    }
    validate_sanitized_entry(
        &record.intent.entry,
        &record.intent.recovery_id,
        record.intent.worktree_form,
    )
    .map_err(|_| RecoveryError::CorruptStore)?;
    validate_provider(record.intent.provider).map_err(|_| RecoveryError::CorruptStore)?;
    if record.intent.entry.origin_session_id.as_deref()
        != Some(record.authority.session_id.as_str())
    {
        return Err(RecoveryError::CorruptStore);
    }
    let digest = board_entry_payload_digest(&record.intent.entry)
        .map_err(|_| RecoveryError::CorruptStore)?;
    if digest != record.intent.payload_digest || !is_versioned_digest(&record.intent.payload_digest)
    {
        return Err(RecoveryError::CorruptStore);
    }
    match record.state {
        RecoveryState::Pending if record.acknowledgement.is_none() && record.conflict.is_none() => {
        }
        RecoveryState::Acknowledged
            if record.acknowledgement.is_some() && record.conflict.is_none() =>
        {
            validate_acknowledgement_shape(record.acknowledgement.as_ref().expect("checked Some"))
                .map_err(|_| RecoveryError::CorruptStore)?;
            validate_exact_acknowledgement(
                &record.recovery_id,
                record,
                record.acknowledgement.as_ref().expect("checked Some"),
            )
            .map_err(|_| RecoveryError::CorruptStore)?;
        }
        RecoveryState::Conflicted
            if record.acknowledgement.is_none() && record.conflict.is_some() => {}
        _ => return Err(RecoveryError::CorruptStore),
    }
    Ok(())
}

fn parse_revision_name(name: &str) -> Option<u64> {
    let stem = name.strip_suffix(".json")?;
    if stem.len() != REVISION_DIGITS || !stem.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    stem.parse().ok()
}

fn calculate_revision_digest(envelope: &RevisionEnvelope) -> RecoveryResult<String> {
    #[derive(Serialize)]
    struct RevisionDigest<'a> {
        schema_version: u32,
        recovery_id: &'a str,
        revision: u64,
        operation_id: &'a str,
        operation_digest: &'a str,
        previous_revision_digest: &'a Option<String>,
        committed_at: &'a DateTime<Utc>,
        record: &'a RecoveryRecord,
    }
    digest_serializable(&RevisionDigest {
        schema_version: envelope.schema_version,
        recovery_id: &envelope.recovery_id,
        revision: envelope.revision,
        operation_id: &envelope.operation_id,
        operation_digest: &envelope.operation_digest,
        previous_revision_digest: &envelope.previous_revision_digest,
        committed_at: &envelope.committed_at,
        record: &envelope.record,
    })
}

fn persist_revision(
    revisions_dir: &Path,
    heads_dir: &Path,
    envelope: &RevisionEnvelope,
) -> RecoveryResult<()> {
    persist_immutable_json(revisions_dir, envelope.revision, envelope)?;
    persist_revision_head(heads_dir, envelope)
}

fn persist_revision_head(heads_dir: &Path, envelope: &RevisionEnvelope) -> RecoveryResult<()> {
    let head = RevisionHead {
        schema_version: RECOVERY_HEAD_SCHEMA_VERSION,
        revision: envelope.revision,
        revision_digest: envelope.revision_digest.clone(),
    };
    persist_immutable_json(heads_dir, envelope.revision, &head)
}

fn persist_immutable_json(
    directory: &Path,
    revision: u64,
    value: &impl Serialize,
) -> RecoveryResult<()> {
    let final_path = directory.join(format!(
        "{:0width$}.json",
        revision,
        width = REVISION_DIGITS
    ));
    if final_path.exists() {
        return Err(RecoveryError::CorruptStore);
    }
    let temp_path = directory.join(format!(
        ".{:0width$}.json.tmp-{}",
        revision,
        Uuid::new_v4(),
        width = REVISION_DIGITS
    ));
    let bytes = serde_json::to_vec_pretty(value).map_err(|_| RecoveryError::Storage)?;
    let mut file = open_private_new(&temp_path)?;
    file.write_all(&bytes).map_err(|_| RecoveryError::Storage)?;
    file.write_all(b"\n").map_err(|_| RecoveryError::Storage)?;
    file.sync_all().map_err(|_| RecoveryError::Storage)?;
    drop(file);

    if fs::hard_link(&temp_path, &final_path).is_err() {
        let _ = fs::remove_file(&temp_path);
        return Err(RecoveryError::Storage);
    }
    set_private_file_permissions(&final_path)?;
    sync_directory(directory)?;
    if fs::remove_file(&temp_path).is_ok() {
        sync_directory(directory)?;
    }
    Ok(())
}

fn ensure_private_dir(path: &Path) -> RecoveryResult<()> {
    let existed = path.exists();
    fs::create_dir_all(path).map_err(|_| RecoveryError::Storage)?;
    if !path.is_dir() {
        return Err(RecoveryError::Storage);
    }
    set_private_dir_permissions(path)?;
    if !existed {
        sync_directory(path)?;
        if let Some(parent) = path.parent() {
            sync_directory(parent)?;
        }
    }
    Ok(())
}

fn open_private_lock(path: &Path) -> RecoveryResult<File> {
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    set_private_open_mode(&mut options);
    let file = options.open(path).map_err(|_| RecoveryError::Storage)?;
    set_private_file_permissions(path)?;
    Ok(file)
}

fn open_private_new(path: &Path) -> RecoveryResult<File> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    set_private_open_mode(&mut options);
    let file = options.open(path).map_err(|_| RecoveryError::Storage)?;
    set_private_file_permissions(path)?;
    Ok(file)
}

#[cfg(unix)]
fn set_private_open_mode(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    options.mode(0o600);
}

#[cfg(not(unix))]
fn set_private_open_mode(_options: &mut OpenOptions) {}

#[cfg(unix)]
fn set_private_dir_permissions(path: &Path) -> RecoveryResult<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|_| RecoveryError::Storage)
}

#[cfg(not(unix))]
fn set_private_dir_permissions(_path: &Path) -> RecoveryResult<()> {
    Ok(())
}

#[cfg(unix)]
fn set_private_file_permissions(path: &Path) -> RecoveryResult<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|_| RecoveryError::Storage)
}

#[cfg(not(unix))]
fn set_private_file_permissions(_path: &Path) -> RecoveryResult<()> {
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> RecoveryResult<()> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| RecoveryError::Storage)
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> RecoveryResult<()> {
    Ok(())
}
