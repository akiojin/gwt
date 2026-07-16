//! Project-scoped durable recovery records for agent sessions.
//!
//! Recovery data deliberately lives outside worktrees. Every mutation is an
//! immutable, uniquely named event followed by an immutable snapshot. This
//! avoids replacement gaps on Windows and lets startup ignore partial temp
//! files while replaying the highest complete generation.

use std::{
    cell::RefCell,
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    rc::Rc,
};

use chrono::{DateTime, Duration, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

mod attachment;

pub use attachment::{
    read_recovery_attachment_bytes, read_recovery_attachment_bytes_with_limit,
    RecoveryAttachmentRef, MAX_RECOVERY_ATTACHMENT_BYTES,
};

const STORE_SCHEMA_VERSION: u32 = 1;
const TOMBSTONE_RETENTION_DAYS: i64 = 30;
const SNAPSHOTS_TO_KEEP: usize = 2;
const MAX_RECOVERY_VISIBLE_ITEMS: usize = 128;
const MAX_RECOVERY_COLLECTION_ITEMS: usize = 128;
const MAX_BOARD_ENTRY_IDS: usize = 1_024;
const MAX_BOARD_ENTRY_ID_HISTORY_BYTES: usize = 256 * 1_024;
const MAX_RECOVERY_IDENTIFIER_CHARS: usize = 4_096;
const MAX_RECOVERY_TEXT_FIELD_CHARS: usize = 65_536;
const MAX_RECOVERY_INITIAL_PROMPT_CHARS: usize = 262_144;
const MAX_RECOVERY_SEMANTIC_PAYLOAD_BYTES: usize = 512 * 1_024;
const MAX_RECOVERY_EVENT_BYTES: usize = 1_024 * 1_024;
const MAX_RECOVERY_RECORD_BYTES: usize = 2 * 1_024 * 1_024;
const MAX_RECOVERY_SNAPSHOT_BYTES: usize = 4 * 1_024 * 1_024;
const MAX_RECOVERY_EVENT_FILES: usize = 4_096;
const MAX_RECOVERY_EVENT_TOTAL_BYTES: usize = 64 * 1_024 * 1_024;
const MAX_RECOVERY_SNAPSHOT_FILES: usize = 128;
const MAX_RECOVERY_SNAPSHOT_TOTAL_BYTES: usize = 64 * 1_024 * 1_024;
const MAX_RECOVERY_RECEIPT_FILES: usize = 16_384;
const MAX_RECOVERY_RECEIPT_BYTES: usize = 64 * 1_024;
const MAX_RECOVERY_RECEIPT_TOTAL_BYTES: usize = 128 * 1_024 * 1_024;
const MAX_RECOVERY_TOMBSTONE_FILES: usize = 4_096;
const MAX_RECOVERY_TOMBSTONE_BYTES: usize = 256 * 1_024;
const MAX_RECOVERY_TOMBSTONE_TOTAL_BYTES: usize = 64 * 1_024 * 1_024;
const MAX_RECOVERY_TRANSACTION_FILES: usize = 1_024;
const MAX_RECOVERY_TRANSACTION_BYTES: usize = 4 * 1_024 * 1_024;
const MAX_RECOVERY_TRANSACTION_TOTAL_BYTES: usize = 64 * 1_024 * 1_024;
const MAX_PROVIDER_ROOT_CLAIM_EVENT_FILES: usize = 128;
const MAX_PROVIDER_ROOT_CLAIM_EVENT_BYTES: usize = 64 * 1_024;
const MAX_PROVIDER_ROOT_CLAIM_EVENT_TOTAL_BYTES: usize = 4 * 1_024 * 1_024;
// Thousands of recoveries are already far beyond normal project history; this
// cap bounds startup path allocation without deleting or truncating overflow.
const MAX_RECOVERY_DIRECTORY_ENTRIES: usize = 4_096;
// Global Recovery Center/owner discovery is stricter than each record's local
// ledger limits: 65k metadata entries and 256 MiB of bytes cap the whole scan.
const MAX_GLOBAL_RECOVERY_LEDGER_ENTRIES: usize = 65_536;
const MAX_GLOBAL_RECOVERY_READ_BYTES: usize = 256 * 1024 * 1024;
const MAX_PROVIDER_ROOT_CLAIM_TTL_MINUTES: i64 = 10;
const PROVIDER_ROOT_CLAIM_EVENTS_TO_KEEP: usize = 2;
pub const MAX_BOARD_DELIVERY_ERROR_CHARS: usize = 512;

/// Result type used by the recovery store.
pub type RecoveryStoreResult<T> = Result<T, RecoveryStoreError>;

/// Semantic failures are kept distinct from I/O errors so callers can decide
/// whether a recovery needs retry, user attention, or conflict resolution.
#[derive(Debug, Error)]
pub enum RecoveryStoreError {
    #[error("recovery store I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("recovery store JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid recovery id: {0}")]
    InvalidRecoveryId(String),
    #[error("recovery not found: {0}")]
    NotFound(String),
    #[error("recovery already exists: {0}")]
    AlreadyExists(String),
    #[error("operation {operation_id} was already used with different content")]
    OperationConflict { operation_id: String },
    #[error("provider root binding conflict: expected {expected}, received {actual}")]
    RootBindingConflict { expected: String, actual: String },
    #[error("invalid provider root candidate: {0}")]
    InvalidProviderRootCandidate(String),
    #[error("provider root {root_id} is not one of the recorded recovery candidates")]
    UnknownProviderRootCandidate { root_id: String },
    #[error("provider root mismatch: expected {expected}, received {actual}")]
    RootMismatch { expected: String, actual: String },
    #[error("provider root operation requires an observed root role, got {0:?}")]
    RootRoleRejected(ProviderRootRole),
    #[error("invalid recovery launch stage transition from {current:?} to {requested:?}")]
    InvalidLaunchStageTransition {
        current: RecoveryLaunchStage,
        requested: RecoveryLaunchStage,
    },
    #[error("provider root role conflicts: current {current:?}, requested {requested:?}")]
    RootRoleConflict {
        current: ProviderRootRole,
        requested: ProviderRootRole,
    },
    #[error("checkpoint revision mismatch: expected {expected}, actual {actual}")]
    RevisionMismatch { expected: u64, actual: u64 },
    #[error("recovery generation mismatch: expected {expected}, actual {actual}")]
    GenerationMismatch { expected: u64, actual: u64 },
    #[error("recovery lease conflicts with active lease {lease_id} held by {holder_id}")]
    LeaseConflict { lease_id: String, holder_id: String },
    #[error(
        "provider root is already claimed by recovery {holder_recovery_id}, session {holder_session_id}, window {holder_window_id}"
    )]
    ProviderRootClaimConflict {
        holder_recovery_id: String,
        holder_session_id: String,
        holder_window_id: String,
    },
    #[error("invalid recovery lease: {0}")]
    InvalidLease(String),
    #[error("board milestone id {entry_id} was reused with different content")]
    BoardIntentConflict { entry_id: String },
    #[error("recovery continuation link conflicts with existing provenance")]
    ContinuationConflict,
    #[error("recovery content limit exceeded for {field}: maximum {limit} {unit}")]
    ContentLimitExceeded {
        field: String,
        limit: usize,
        unit: &'static str,
    },
    #[error("recovery {recovery_id} has no user-visible continuation context")]
    NoContinuationContext { recovery_id: String },
    #[error("invalid terminal lifecycle for purge: {0:?}")]
    InvalidTerminalLifecycle(RecoveryLifecycle),
    #[error("corrupt recovery event generation sequence for {recovery_id}")]
    GenerationConflict { recovery_id: String },
    #[error("tombstone conflict for recovery {0}")]
    TombstoneConflict(String),
    #[error("injected recovery store fault after {0:?}")]
    InjectedFault(RecoveryStoreFaultPoint),
}

/// Durable publication boundaries exposed for deterministic crash tests.
///
/// Production stores never enable a fault. Keeping the seam in the real I/O
/// path lets integration tests prove recovery from the same fsync/rename
/// boundaries used in production.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryStoreFaultPoint {
    AfterEventPublication,
    AfterOperationReceiptPublication,
    AfterSnapshotPublication,
    AfterContinuationIntentPublication,
    AfterContinuationSourcePublication,
    AfterContinuationTargetPublication,
    AfterContinuationIntentCleanup,
    AfterSuccessorReadyObservation,
    AfterProviderRootClaimPublication,
    AfterSnapshotPruneDeletion,
    AfterEventCompactionDeletion,
    AfterTombstonePublication,
    AfterRecoveryPayloadPurge,
    AfterAttachmentPublication,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecoverySessionKind {
    Intake,
    Execution,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryLifecycle {
    Launching,
    Running,
    Interrupted,
    Recovering,
    Attention,
    Resolved,
    Discarded,
}

/// Durable launch progress owned by the recovery store.
///
/// This is intentionally independent from [`RecoveryLifecycle`]. A recovery
/// can require Attention at any launch stage, while the stage records the last
/// provider boundary that was durably crossed.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Default)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryLaunchStage {
    #[default]
    Created,
    Prepared,
    WorktreeMaterialized,
    /// Durable intent written before attempting the non-transactional OS
    /// process spawn. Startup must not duplicate-launch this state.
    SpawnRequested,
    ProcessSpawned,
    ProviderBound,
    Ready,
    Resolved,
    Discarded,
}

impl RecoveryLaunchStage {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Resolved | Self::Discarded)
    }

    fn is_created(value: &Self) -> bool {
        *value == Self::Created
    }
}

/// Provider conversation role observed for this recovery.
///
/// `Unknown` is fail-closed: an ambiguous hook may not bind a provider root
/// or write root-owned checkpoint content.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProviderRootRole {
    #[default]
    Unknown,
    Root,
    Subagent,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum BindingQuality {
    Inferred,
    Confirmed,
    Preassigned,
    Verified,
}

impl BindingQuality {
    pub fn is_authoritative(self) -> bool {
        !matches!(self, Self::Inferred)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointCoverage {
    #[default]
    Unknown,
    Explicit,
    Stale,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderRootBinding {
    pub root_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_tree_id: Option<String>,
    pub quality: BindingQuality,
    pub bound_at: DateTime<Utc>,
}

/// One independently observed provider root that needs user confirmation.
///
/// Candidate evidence is intentionally descriptive rather than transcript
/// content. It lets Recovery Center explain why an id was found without
/// merging provider histories or treating an ambiguous id as authoritative.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderRootCandidate {
    pub root_id: String,
    #[serde(default)]
    pub evidence: Vec<String>,
    pub observed_at: DateTime<Utc>,
}

/// Bounded exclusive claim for one recovery launch attempt.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecoveryLease {
    pub lease_id: String,
    pub holder_id: String,
    pub acquired_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

/// Durable evidence that gwt's PTY supervisor no longer owns a provider.
///
/// A `Ready` launch stage remains monotonic after a crash. This proof is the
/// separate authority required to claim that historical Ready record for a
/// successor launch without treating every Ready record as relaunchable.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecoverySupervisorStopProof {
    pub session_id: String,
    pub observed_at: DateTime<Utc>,
}

/// Project-wide ownership of one authoritative provider conversation root.
///
/// `provider_root_hash` is the only provider-root identity written outside a
/// [`RecoveryRecord`]. The raw provider root is never serialized into the
/// claim ledger. `claim_token` is a compare-and-swap capability: a stale
/// holder cannot release a newer claim after expiry and replacement.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderRootClaim {
    pub provider_root_hash: String,
    pub claim_token: String,
    pub holder_recovery_id: String,
    pub holder_session_id: String,
    pub holder_window_id: String,
    pub acquired_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RootInput {
    pub turn_id: String,
    pub text: String,
    pub submitted_at: DateTime<Utc>,
}

/// One provider-root turn applied as a single recovery generation.
///
/// `input_text`, visible discussion, and attachment references share one
/// immutable operation id. This keeps a crash from publishing only part of a
/// user turn while still allowing attachment and discussion-only updates.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RootTurnUpdate {
    pub root_id: String,
    pub turn_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_text: Option<String>,
    #[serde(default)]
    pub visible_items: Vec<VisibleDiscussionItem>,
    #[serde(default)]
    pub attachment_refs: Vec<RecoveryAttachmentRef>,
}

/// In-memory attachment bytes imported in the same inventory lock as the turn
/// mutation. Bytes are deliberately excluded from Debug output and are never
/// serialized into recovery events.
#[derive(Clone)]
pub struct RecoveryAttachmentPayload {
    pub file_name: String,
    pub bytes: Vec<u8>,
}

impl std::fmt::Debug for RecoveryAttachmentPayload {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RecoveryAttachmentPayload")
            .field("file_name", &self.file_name)
            .field("byte_len", &self.bytes.len())
            .field("bytes", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VisibleDiscussionItem {
    pub role: String,
    pub kind: String,
    pub text: String,
    #[serde(default)]
    pub partial: bool,
}

/// Public-safe Board content queued atomically with a semantic checkpoint.
///
/// The outbox keeps discussion durability and Board visibility in the same
/// recovery generation. Callers acknowledge an entry only after the Board's
/// idempotent post succeeds.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BoardMilestoneIntent {
    pub entry_id: String,
    pub title: String,
    pub body: String,
    pub queued_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SemanticCheckpoint {
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub confirmed_decisions: Vec<String>,
    #[serde(default)]
    pub open_questions: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub as_of_turn_id: Option<String>,
    #[serde(default)]
    pub visible_items: Vec<VisibleDiscussionItem>,
    #[serde(default)]
    pub attachment_refs: Vec<RecoveryAttachmentRef>,
    #[serde(default)]
    pub board_intents: Vec<BoardMilestoneIntent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreateRecovery {
    pub recovery_id: String,
    pub session_id: String,
    pub repo_id: String,
    pub session_kind: RecoverySessionKind,
    pub worktree_path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_base_ref: Option<String>,
    pub launch_base_oid: String,
    pub launch_head_oid: String,
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub runtime: String,
    pub initial_prompt: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecoveryContinuationLink {
    pub source_recovery_id: String,
    pub target_recovery_id: String,
    pub source_checkpoint_revision: u64,
    pub definitive_reason: String,
    pub linked_at: DateTime<Utc>,
}

/// Provider-root identities retained by a fresh-root continuation.
///
/// This deliberately stores identifiers only. Provider transcript content and
/// hidden reasoning never enter continuation provenance.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct RecoveryContinuationRootProvenance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_provider_root_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_provider_root_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecoveryRecord {
    pub schema_version: u32,
    pub recovery_id: String,
    pub session_id: String,
    pub repo_id: String,
    pub generation: u64,
    pub session_kind: RecoverySessionKind,
    pub lifecycle: RecoveryLifecycle,
    #[serde(default, skip_serializing_if = "RecoveryLaunchStage::is_created")]
    pub launch_stage: RecoveryLaunchStage,
    #[serde(default, skip_serializing_if = "provider_root_role_is_unknown")]
    pub root_role: ProviderRootRole,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_lease: Option<RecoveryLease>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supervisor_stop_proof: Option<RecoverySupervisorStopProof>,
    pub worktree_path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_base_ref: Option<String>,
    pub launch_base_oid: String,
    pub launch_head_oid: String,
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub runtime: String,
    pub initial_prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_root: Option<ProviderRootBinding>,
    #[serde(default)]
    pub provider_root_candidates: Vec<ProviderRootCandidate>,
    #[serde(default)]
    pub checkpoint_revision: u64,
    #[serde(default)]
    pub checkpoint_coverage: CheckpointCoverage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<SemanticCheckpoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_root_input: Option<RootInput>,
    #[serde(default)]
    pub board_outbox: Vec<BoardMilestoneIntent>,
    #[serde(default)]
    pub board_entry_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board_delivery_error: Option<String>,
    #[serde(default)]
    pub continuation_targets: Vec<RecoveryContinuationLink>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuation_source: Option<RecoveryContinuationLink>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuation_root_provenance: Option<RecoveryContinuationRootProvenance>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle_reason: Option<String>,
}

impl RecoveryRecord {
    pub fn has_authoritative_root(&self) -> bool {
        self.provider_root
            .as_ref()
            .is_some_and(|root| root.quality.is_authoritative())
    }
}

fn provider_root_role_is_unknown(value: &ProviderRootRole) -> bool {
    *value == ProviderRootRole::Unknown
}

/// Build the user-visible prompt for a fresh provider root after exact resume
/// is definitively unavailable.
///
/// Only durable user-visible fields are included. Provider history, tool
/// payloads, hidden reasoning, and Board transport metadata are deliberately
/// excluded. The newest visible items win when the item or character budget is
/// reached.
pub fn build_checkpoint_continuation_prompt(
    record: &RecoveryRecord,
    max_visible_items: usize,
    max_chars: usize,
) -> RecoveryStoreResult<String> {
    let checkpoint = record.checkpoint.as_ref();
    let has_checkpoint_context = checkpoint.is_some_and(|checkpoint| {
        !checkpoint.summary.trim().is_empty()
            || !checkpoint.confirmed_decisions.is_empty()
            || !checkpoint.open_questions.is_empty()
            || checkpoint
                .next_action
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
            || !checkpoint.visible_items.is_empty()
    });
    let has_latest_input = record
        .latest_root_input
        .as_ref()
        .is_some_and(|input| !input.text.trim().is_empty());
    let has_initial_prompt = !record.initial_prompt.trim().is_empty();
    if !has_checkpoint_context && !has_latest_input && !has_initial_prompt {
        return Err(RecoveryStoreError::NoContinuationContext {
            recovery_id: record.recovery_id.clone(),
        });
    }
    if max_chars < 128 {
        return Err(RecoveryStoreError::NoContinuationContext {
            recovery_id: record.recovery_id.clone(),
        });
    }

    let mut sections = vec![format!(
        "Continue the interrupted {} session from the durable gwt recovery context below. Do not start over or claim that prior decisions are new.",
        match record.session_kind {
            RecoverySessionKind::Intake => "Intake",
            RecoverySessionKind::Execution => "Execution",
        }
    )];

    if record.checkpoint_coverage == CheckpointCoverage::Stale {
        sections.push(
            "Checkpoint note: a newer user turn arrived after the semantic checkpoint; treat the latest user intent below as authoritative."
                .to_string(),
        );
    }
    if let Some(input) = record.latest_root_input.as_ref() {
        if !input.text.trim().is_empty() {
            sections.push(format!("Latest user intent:\n{}", input.text.trim()));
        }
    }
    if let Some(checkpoint) = checkpoint {
        if !checkpoint.summary.trim().is_empty() {
            sections.push(format!(
                "Checkpoint summary:\n{}",
                checkpoint.summary.trim()
            ));
        }
        if !checkpoint.confirmed_decisions.is_empty() {
            sections.push(format!(
                "Confirmed decisions:\n{}",
                format_prompt_bullets(&checkpoint.confirmed_decisions)
            ));
        }
        if !checkpoint.open_questions.is_empty() {
            sections.push(format!(
                "Open questions:\n{}",
                format_prompt_bullets(&checkpoint.open_questions)
            ));
        }
        if let Some(next_action) = checkpoint
            .next_action
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            sections.push(format!("Next action:\n{next_action}"));
        }

        let visible_start = checkpoint
            .visible_items
            .len()
            .saturating_sub(max_visible_items);
        let recent_context = checkpoint.visible_items[visible_start..]
            .iter()
            .filter(|item| !item.text.trim().is_empty())
            .map(|item| {
                let partial = if item.partial { " (partial)" } else { "" };
                format!(
                    "[{} / {}{}] {}",
                    item.role.trim(),
                    item.kind.trim(),
                    partial,
                    item.text.trim()
                )
            })
            .collect::<Vec<_>>();
        if !recent_context.is_empty() {
            sections.push(format!(
                "Recent visible context:\n{}",
                recent_context.join("\n")
            ));
        }
    }
    if !has_checkpoint_context && has_initial_prompt {
        sections.push(format!(
            "Original user intent:\n{}",
            record.initial_prompt.trim()
        ));
    }
    sections.push(
        "Continue from this context, preserve the confirmed decisions, and address the next action or latest user intent."
            .to_string(),
    );

    let prompt = sections.join("\n\n");
    if prompt.chars().count() <= max_chars {
        return Ok(prompt);
    }
    let mut truncated = prompt
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    truncated.push('…');
    Ok(truncated)
}

/// Build a bounded continuation prompt and expose only verified, durable
/// attachment copies owned by this recovery store.
///
/// Arbitrary source paths are never retained by [`RecoveryAttachmentRef`].
/// Every referenced blob is checksum-verified before its project-scoped path
/// is added to the prompt. The attachment section is reserved in full; if it
/// cannot fit, continuation fails closed instead of silently dropping files.
pub fn build_checkpoint_continuation_prompt_with_attachments(
    store: &RecoveryStore,
    record: &RecoveryRecord,
    max_visible_items: usize,
    max_chars: usize,
) -> RecoveryStoreResult<String> {
    let Some(checkpoint) = record.checkpoint.as_ref() else {
        return build_checkpoint_continuation_prompt(record, max_visible_items, max_chars);
    };
    if checkpoint.attachment_refs.is_empty() {
        return build_checkpoint_continuation_prompt(record, max_visible_items, max_chars);
    }

    let mut attachment_paths = Vec::with_capacity(checkpoint.attachment_refs.len());
    for attachment in &checkpoint.attachment_refs {
        let path = store.resolve_attachment_path(attachment)?;
        attachment_paths.push(path.to_string_lossy().into_owned());
    }
    build_checkpoint_continuation_prompt_with_attachment_paths(
        record,
        &attachment_paths,
        max_visible_items,
        max_chars,
    )
}

/// Build a bounded continuation prompt using caller-verified attachment paths.
///
/// This is the container counterpart to
/// [`build_checkpoint_continuation_prompt_with_attachments`]. The caller must
/// first materialize every checkpoint attachment at the corresponding path and
/// verify its digest and byte length. Only the paths are supplied here; names,
/// content ids, and byte lengths always come from the durable checkpoint.
/// Paths must be absolute in their target runtime and are never written back to
/// [`RecoveryAttachmentRef`].
pub fn build_checkpoint_continuation_prompt_with_attachment_paths(
    record: &RecoveryRecord,
    attachment_paths: &[String],
    max_visible_items: usize,
    max_chars: usize,
) -> RecoveryStoreResult<String> {
    let Some(checkpoint) = record.checkpoint.as_ref() else {
        if attachment_paths.is_empty() {
            return build_checkpoint_continuation_prompt(record, max_visible_items, max_chars);
        }
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "attachment paths require a semantic checkpoint",
        )
        .into());
    };
    if checkpoint.attachment_refs.len() != attachment_paths.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "attachment path count does not match the durable checkpoint",
        )
        .into());
    }
    if checkpoint.attachment_refs.is_empty() {
        return build_checkpoint_continuation_prompt(record, max_visible_items, max_chars);
    }

    let mut attachment_lines = Vec::with_capacity(checkpoint.attachment_refs.len());
    for (attachment, path) in checkpoint
        .attachment_refs
        .iter()
        .zip(attachment_paths.iter())
    {
        attachment::validate_reference(attachment)?;
        let path = path.trim();
        if path.is_empty()
            || path.contains(['\n', '\r', '\t'])
            || !(Path::new(path).is_absolute() || path.starts_with('/'))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "attachment prompt path must be an absolute single-line path",
            )
            .into());
        }
        attachment_lines.push(format!(
            "- {}: {} ({}, {} bytes)",
            attachment.file_name, path, attachment.content_id, attachment.byte_len
        ));
    }
    let attachment_section = format!(
        "Recovered attachments (verified project-scoped durable copies):\n{}",
        attachment_lines.join("\n")
    );
    let attachment_chars = attachment_section.chars().count();
    let separator_chars = 2;
    let Some(semantic_budget) = max_chars
        .checked_sub(attachment_chars)
        .and_then(|remaining| remaining.checked_sub(separator_chars))
        .filter(|remaining| *remaining >= 128)
    else {
        return Err(RecoveryStoreError::NoContinuationContext {
            recovery_id: record.recovery_id.clone(),
        });
    };

    let semantic_prompt = match build_checkpoint_continuation_prompt(
        record,
        max_visible_items,
        semantic_budget,
    ) {
        Ok(prompt) => prompt,
        Err(RecoveryStoreError::NoContinuationContext { .. }) => {
            let prompt = format!(
                "Continue the interrupted {} session using the verified user-provided attachments below. Preserve their role as prior session context.",
                match record.session_kind {
                    RecoverySessionKind::Intake => "Intake",
                    RecoverySessionKind::Execution => "Execution",
                }
            );
            if prompt.chars().count() <= semantic_budget {
                prompt
            } else {
                let mut truncated = prompt
                    .chars()
                    .take(semantic_budget.saturating_sub(1))
                    .collect::<String>();
                truncated.push('…');
                truncated
            }
        }
        Err(error) => return Err(error),
    };

    Ok(format!("{semantic_prompt}\n\n{attachment_section}"))
}

fn format_prompt_bullets(items: &[String]) -> String {
    items
        .iter()
        .map(|item| item.trim())
        .filter(|item| !item.is_empty())
        .map(|item| format!("- {item}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn bounded_board_delivery_error(value: &str) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = normalized.chars();
    let mut bounded = chars
        .by_ref()
        .take(MAX_BOARD_DELIVERY_ERROR_CHARS)
        .collect::<String>();
    if chars.next().is_some() {
        bounded.pop();
        bounded.push('…');
    }
    bounded
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecoveryTombstone {
    pub schema_version: u32,
    pub recovery_id: String,
    pub lifecycle: RecoveryLifecycle,
    #[serde(default, skip_serializing_if = "RecoveryLaunchStage::is_created")]
    pub launch_stage: RecoveryLaunchStage,
    #[serde(default, skip_serializing_if = "provider_root_role_is_unknown")]
    pub root_role: ProviderRootRole,
    pub terminal_operation_id: String,
    pub last_generation: u64,
    pub session_identity_hash: String,
    /// Recorded commit retained only to CAS-delete the gwt-owned Git pin on a
    /// post-tombstone retry. It is repository metadata, not user content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_base_oid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_identity_hash: Option<String>,
    pub purged_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
enum RecoveryMutation {
    Create(CreateRecovery),
    AdvanceLaunchStage {
        stage: RecoveryLaunchStage,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        root_role: Option<ProviderRootRole>,
    },
    BindRoot(ProviderRootBinding),
    BindRootSemantic {
        root_id: String,
        session_tree_id: Option<String>,
        quality: BindingQuality,
    },
    RecordProviderRootCandidates(Vec<ProviderRootCandidate>),
    ReplaceCheckpoint {
        root_id: String,
        expected_revision: u64,
        checkpoint: SemanticCheckpoint,
    },
    RecordRootInput {
        root_id: String,
        turn_id: String,
        text: String,
    },
    RecordRootTurn(RootTurnUpdate),
    SetLifecycle {
        lifecycle: RecoveryLifecycle,
        reason: Option<String>,
    },
    ClaimRecovery {
        expected_generation: u64,
        lease: RecoveryLease,
        reason: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        confirmed_root_id: Option<String>,
    },
    ClaimInterruptedRecovery {
        expected_generation: u64,
        lease: RecoveryLease,
        reason: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        confirmed_root_id: Option<String>,
    },
    InterruptAfterSupervisorStop {
        expected_generation: u64,
        session_id: String,
        reason: String,
    },
    CompleteClaimedProviderReady {
        claim_holder_recovery_id: String,
        claim_token: String,
    },
    CompleteProviderReady,
    AckBoardEntry {
        entry_id: String,
    },
    SetBoardDeliveryError {
        error: Option<String>,
    },
    LinkContinuation {
        link: RecoveryContinuationLink,
        role: RecoveryContinuationRole,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        inherited_state: Option<Box<RecoveryContinuationState>>,
    },
    CancelContinuation {
        link: RecoveryContinuationLink,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct RecoveryContinuationState {
    checkpoint_revision: u64,
    checkpoint_coverage: CheckpointCoverage,
    checkpoint: Option<SemanticCheckpoint>,
    latest_root_input: Option<RootInput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_provider_root_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum RecoveryContinuationRole {
    Source,
    Target,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ContinuationTransactionBody {
    schema_version: u32,
    operation_id: String,
    link: RecoveryContinuationLink,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    inherited_state: Option<Box<RecoveryContinuationState>>,
    /// Prepared successor intents remain durable after both link mutations.
    /// They are removed only after a Ready successor has safely finalized the
    /// source (or a terminal source tombstone proves that already happened).
    #[serde(default)]
    await_source_finalization: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct StoredContinuationTransaction {
    body: ContinuationTransactionBody,
    checksum: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SuccessorTargetDisposition {
    Pending,
    Ready,
    Discarded,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EventBody {
    schema_version: u32,
    recovery_id: String,
    generation: u64,
    operation_id: String,
    recorded_at: DateTime<Utc>,
    mutation_digest: String,
    mutation: RecoveryMutation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredEvent {
    body: EventBody,
    checksum: String,
}

/// Immutable, content-addressed acknowledgement that one operation id was
/// committed. Receipts keep long-lived idempotency outside the bounded
/// recovery snapshot without exposing the raw operation id in a filename.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct OperationReceiptBody {
    schema_version: u32,
    recovery_id: String,
    operation_id: String,
    generation: u64,
    mutation_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredOperationReceipt {
    body: OperationReceiptBody,
    checksum: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SnapshotBody {
    schema_version: u32,
    recovery_id: String,
    generation: u64,
    record: RecoveryRecord,
    /// Legacy inline operation ledger. New snapshots always publish this as
    /// an empty map; readers retain it only long enough to migrate each entry
    /// to an immutable operation receipt.
    #[serde(default)]
    operation_digests: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredSnapshot {
    body: SnapshotBody,
    checksum: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredTombstone {
    body: RecoveryTombstone,
    checksum: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ProviderRootClaimEventBody {
    schema_version: u32,
    provider_root_hash: String,
    generation: u64,
    recorded_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    active_claim: Option<ProviderRootClaim>,
    /// The token is retained in a release event so a corrupted or reordered
    /// ledger cannot make a stale release look like a valid state change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    released_claim_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredProviderRootClaimEvent {
    body: ProviderRootClaimEventBody,
    checksum: String,
}

#[derive(Debug, Default)]
struct LoadedState {
    record: Option<RecoveryRecord>,
    /// Only operations replayed after the selected snapshot, or imported from
    /// a legacy inline ledger. Durable history otherwise lives in receipts.
    operation_digests: BTreeMap<String, String>,
    operation_generations: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CommittedOperation {
    generation: u64,
    mutation_digest: String,
}

/// Filesystem-backed project recovery store.
#[derive(Debug, Clone)]
pub struct RecoveryStore {
    root: PathBuf,
    fault_point: Option<RecoveryStoreFaultPoint>,
}

impl RecoveryStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            fault_point: None,
        }
    }

    /// Create a store rooted at a repo-hash-scoped project data directory.
    /// Records are written below `<project_dir>/recoveries/`, outside every
    /// worktree owned by that repository.
    pub fn for_project_dir(project_dir: impl Into<PathBuf>) -> Self {
        Self::new(project_dir)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Enable one deterministic publication failure for integration tests.
    /// A separate healthy store should be used to model the restarted process.
    #[doc(hidden)]
    pub fn with_fault_injection_for_test(mut self, point: RecoveryStoreFaultPoint) -> Self {
        self.fault_point = Some(point);
        self
    }

    pub fn create(
        &self,
        mut request: CreateRecovery,
        operation_id: impl Into<String>,
    ) -> RecoveryStoreResult<RecoveryRecord> {
        validate_recovery_id(&request.recovery_id)?;
        sanitize_bounded_text(
            "initial_prompt",
            &mut request.initial_prompt,
            MAX_RECOVERY_INITIAL_PROMPT_CHARS,
        )?;
        ensure_serialized_limit("create_recovery", &request)?;
        let recovery_id = request.recovery_id.clone();
        self.persist_mutation(
            &recovery_id,
            operation_id.into(),
            request.created_at,
            RecoveryMutation::Create(request),
        )
    }

    pub fn bind_root(
        &self,
        recovery_id: &str,
        binding: ProviderRootBinding,
        operation_id: impl Into<String>,
    ) -> RecoveryStoreResult<RecoveryRecord> {
        self.persist_mutation(
            recovery_id,
            operation_id.into(),
            binding.bound_at,
            RecoveryMutation::BindRoot(binding),
        )
    }

    /// Durably advance the last provider launch boundary.
    ///
    /// Forward transitions are monotonic. Re-observing an earlier boundary is
    /// an idempotent no-op (but remains a durable operation), while leaving a
    /// terminal stage or changing an observed role fails closed.
    pub fn advance_launch_stage(
        &self,
        recovery_id: &str,
        stage: RecoveryLaunchStage,
        root_role: Option<ProviderRootRole>,
        operation_id: impl Into<String>,
    ) -> RecoveryStoreResult<RecoveryRecord> {
        self.persist_mutation(
            recovery_id,
            operation_id.into(),
            Utc::now(),
            RecoveryMutation::AdvanceLaunchStage { stage, root_role },
        )
    }

    /// Atomically publish the normal/fresh provider Ready boundary.
    ///
    /// Launch stage and lifecycle must share one immutable operation: a crash
    /// after publishing `Ready` but before `Running` would otherwise leave a
    /// record that startup refuses to claim and Recovery Center cannot safely
    /// relaunch.
    pub fn complete_provider_ready(
        &self,
        recovery_id: &str,
        ready_at: DateTime<Utc>,
        operation_id: impl Into<String>,
    ) -> RecoveryStoreResult<RecoveryRecord> {
        self.persist_mutation(
            recovery_id,
            operation_id.into(),
            ready_at,
            RecoveryMutation::CompleteProviderReady,
        )
    }

    /// Bind a provider root with a retry-stable semantic payload.
    ///
    /// Unlike [`Self::bind_root`], the observation timestamp is event metadata,
    /// not part of the mutation digest. Reusing an operation id with the same
    /// root succeeds idempotently; changing the root/session payload conflicts.
    pub fn bind_root_semantic(
        &self,
        recovery_id: &str,
        root_id: &str,
        session_tree_id: Option<String>,
        quality: BindingQuality,
        operation_id: impl Into<String>,
    ) -> RecoveryStoreResult<RecoveryRecord> {
        self.persist_mutation(
            recovery_id,
            operation_id.into(),
            Utc::now(),
            RecoveryMutation::BindRootSemantic {
                root_id: root_id.to_string(),
                session_tree_id,
                quality,
            },
        )
    }

    /// Persist independently observed provider roots without selecting one.
    ///
    /// This is used by legacy import when more than one exact provider id is
    /// plausible. Recording candidates never creates an authoritative root;
    /// Recovery Center must confirm one explicitly before exact resume.
    pub fn record_provider_root_candidates(
        &self,
        recovery_id: &str,
        candidates: Vec<ProviderRootCandidate>,
        operation_id: impl Into<String>,
    ) -> RecoveryStoreResult<RecoveryRecord> {
        let candidates = normalize_provider_root_candidates(candidates)?;
        self.persist_mutation(
            recovery_id,
            operation_id.into(),
            candidates
                .iter()
                .map(|candidate| candidate.observed_at)
                .max()
                .unwrap_or_else(Utc::now),
            RecoveryMutation::RecordProviderRootCandidates(candidates),
        )
    }

    pub fn replace_checkpoint(
        &self,
        recovery_id: &str,
        root_id: &str,
        expected_revision: u64,
        mut checkpoint: SemanticCheckpoint,
        operation_id: impl Into<String>,
    ) -> RecoveryStoreResult<RecoveryRecord> {
        sanitize_checkpoint(&mut checkpoint)?;
        self.persist_mutation(
            recovery_id,
            operation_id.into(),
            Utc::now(),
            RecoveryMutation::ReplaceCheckpoint {
                root_id: root_id.to_string(),
                expected_revision,
                checkpoint,
            },
        )
    }

    /// Copy attachment paths and replace a semantic checkpoint as one
    /// attachment-inventory transaction.
    ///
    /// The inventory lock remains held from blob publication through durable
    /// event publication. Global GC therefore cannot delete a newly copied
    /// blob before the checkpoint that references it becomes replayable.
    pub fn replace_checkpoint_with_attachments(
        &self,
        recovery_id: &str,
        root_id: &str,
        expected_revision: u64,
        checkpoint: SemanticCheckpoint,
        attachment_paths: &[PathBuf],
        operation_id: impl Into<String>,
    ) -> RecoveryStoreResult<RecoveryRecord> {
        self.replace_checkpoint_with_attachments_inner(
            recovery_id,
            (root_id, expected_revision),
            checkpoint,
            attachment_paths,
            operation_id.into(),
            || {},
        )
    }

    fn replace_checkpoint_with_attachments_inner(
        &self,
        recovery_id: &str,
        root_revision: (&str, u64),
        mut checkpoint: SemanticCheckpoint,
        attachment_paths: &[PathBuf],
        operation_id: String,
        after_attachment_publication: impl FnOnce(),
    ) -> RecoveryStoreResult<RecoveryRecord> {
        let (root_id, expected_revision) = root_revision;
        ensure_collection_limit(
            "checkpoint.attachment_refs",
            checkpoint
                .attachment_refs
                .len()
                .saturating_add(attachment_paths.len()),
        )?;
        sanitize_checkpoint(&mut checkpoint)?;
        self.with_attachment_inventory_lock(|| {
            self.prune_attachment_temp_files_unlocked()?;
            for source in attachment_paths {
                let attachment = self.copy_attachment_unlocked(source)?;
                if !checkpoint.attachment_refs.contains(&attachment) {
                    checkpoint.attachment_refs.push(attachment);
                }
            }
            after_attachment_publication();
            self.persist_mutation(
                recovery_id,
                operation_id,
                Utc::now(),
                RecoveryMutation::ReplaceCheckpoint {
                    root_id: root_id.to_string(),
                    expected_revision,
                    checkpoint,
                },
            )
        })
    }

    pub fn record_root_input(
        &self,
        recovery_id: &str,
        root_id: &str,
        turn_id: &str,
        text: &str,
        operation_id: impl Into<String>,
    ) -> RecoveryStoreResult<RecoveryRecord> {
        let mut text = text.to_string();
        sanitize_bounded_text(
            "root_input.text",
            &mut text,
            MAX_RECOVERY_INITIAL_PROMPT_CHARS,
        )?;
        ensure_text_limit("root_input.root_id", root_id, MAX_RECOVERY_IDENTIFIER_CHARS)?;
        ensure_text_limit("root_input.turn_id", turn_id, MAX_RECOVERY_IDENTIFIER_CHARS)?;
        self.persist_mutation(
            recovery_id,
            operation_id.into(),
            Utc::now(),
            RecoveryMutation::RecordRootInput {
                root_id: root_id.to_string(),
                turn_id: turn_id.to_string(),
                text,
            },
        )
    }

    /// Atomically apply one root input and its semantic checkpoint delta.
    ///
    /// The operation payload excludes generated timestamps and checkpoint CAS
    /// revisions, so a semantic retry can be compared directly. A reused
    /// operation id with changed text, visible items, or attachment refs is an
    /// [`RecoveryStoreError::OperationConflict`].
    pub fn record_root_turn(
        &self,
        recovery_id: &str,
        mut update: RootTurnUpdate,
        operation_id: impl Into<String>,
    ) -> RecoveryStoreResult<RecoveryRecord> {
        sanitize_root_turn_update(&mut update)?;
        self.persist_mutation(
            recovery_id,
            operation_id.into(),
            Utc::now(),
            RecoveryMutation::RecordRootTurn(update),
        )
    }

    /// Import attachment bytes and commit their root turn under one global
    /// attachment-inventory lock. GC cannot observe a newly published blob
    /// before the recovery event that references it.
    pub fn record_root_turn_with_attachments(
        &self,
        recovery_id: &str,
        update: RootTurnUpdate,
        attachments: Vec<RecoveryAttachmentPayload>,
        operation_id: impl Into<String>,
    ) -> RecoveryStoreResult<RecoveryRecord> {
        ensure_collection_limit(
            "root_turn.attachments",
            update
                .attachment_refs
                .len()
                .saturating_add(attachments.len()),
        )?;
        self.record_root_turn_with_attachments_inner(
            recovery_id,
            update,
            attachments,
            operation_id.into(),
            || {},
        )
    }

    fn record_root_turn_with_attachments_inner(
        &self,
        recovery_id: &str,
        mut update: RootTurnUpdate,
        attachments: Vec<RecoveryAttachmentPayload>,
        operation_id: String,
        after_attachment_publication: impl FnOnce(),
    ) -> RecoveryStoreResult<RecoveryRecord> {
        self.with_attachment_inventory_lock(|| {
            self.prune_attachment_temp_files_unlocked()?;
            for attachment in attachments {
                update.attachment_refs.push(
                    self.copy_attachment_bytes_unlocked(&attachment.file_name, &attachment.bytes)?,
                );
            }
            after_attachment_publication();
            sanitize_root_turn_update(&mut update)?;
            self.persist_mutation(
                recovery_id,
                operation_id,
                Utc::now(),
                RecoveryMutation::RecordRootTurn(update),
            )
        })
    }

    pub fn set_lifecycle(
        &self,
        recovery_id: &str,
        lifecycle: RecoveryLifecycle,
        reason: Option<String>,
        operation_id: impl Into<String>,
    ) -> RecoveryStoreResult<RecoveryRecord> {
        self.persist_mutation(
            recovery_id,
            operation_id.into(),
            Utc::now(),
            RecoveryMutation::SetLifecycle { lifecycle, reason },
        )
    }

    /// Atomically record that gwt's PTY supervisor stopped owning this
    /// provider process.
    ///
    /// The launch stage is deliberately not regressed. In particular, a
    /// `Ready` record remains `Ready`; the separately persisted proof is what
    /// authorizes the narrowly-scoped interrupted-recovery claim APIs.
    pub fn interrupt_after_supervisor_stop(
        &self,
        recovery_id: &str,
        expected_generation: u64,
        session_id: &str,
        observed_at: DateTime<Utc>,
        reason: &str,
        operation_id: impl Into<String>,
    ) -> RecoveryStoreResult<RecoveryRecord> {
        let session_id = session_id.trim();
        if session_id.is_empty() {
            return Err(RecoveryStoreError::InvalidLease(
                "supervisor stop proof requires a Session identity".to_string(),
            ));
        }
        ensure_text_limit(
            "supervisor_stop.session_id",
            session_id,
            MAX_RECOVERY_IDENTIFIER_CHARS,
        )?;
        let reason = sanitize_visible_text(reason);
        if reason.trim().is_empty() {
            return Err(RecoveryStoreError::InvalidLease(
                "supervisor stop proof requires a reason".to_string(),
            ));
        }
        self.persist_mutation(
            recovery_id,
            operation_id.into(),
            observed_at,
            RecoveryMutation::InterruptAfterSupervisorStop {
                expected_generation,
                session_id: session_id.to_string(),
                reason,
            },
        )
    }

    /// Atomically claim one recovery generation for a bounded launch attempt.
    ///
    /// Generation CAS prevents two clients that rendered the same candidate
    /// from both launching it. An unexpired lease also blocks a newer view;
    /// after expiry a fresh holder may take over without manual cleanup.
    pub fn claim_recovery(
        &self,
        recovery_id: &str,
        expected_generation: u64,
        lease: RecoveryLease,
        reason: &str,
        operation_id: impl Into<String>,
    ) -> RecoveryStoreResult<RecoveryRecord> {
        validate_recovery_lease(&lease)?;
        let reason = sanitize_visible_text(reason);
        if reason.trim().is_empty() {
            return Err(RecoveryStoreError::InvalidLease(
                "claim reason must not be empty".to_string(),
            ));
        }
        self.persist_mutation(
            recovery_id,
            operation_id.into(),
            lease.acquired_at,
            RecoveryMutation::ClaimRecovery {
                expected_generation,
                lease,
                reason,
                confirmed_root_id: None,
            },
        )
    }

    /// Claim a historically Ready record only after a durable supervisor-stop
    /// proof established that its previous provider can no longer be live.
    ///
    /// Normal [`Self::claim_recovery`] remains fail-closed for every Ready
    /// record. Keeping this as a separate mutation prevents a generic
    /// lifecycle write from accidentally enabling duplicate provider launch.
    pub fn claim_interrupted_recovery(
        &self,
        recovery_id: &str,
        expected_generation: u64,
        lease: RecoveryLease,
        reason: &str,
        operation_id: impl Into<String>,
    ) -> RecoveryStoreResult<RecoveryRecord> {
        validate_recovery_lease(&lease)?;
        let reason = sanitize_visible_text(reason);
        if reason.trim().is_empty() {
            return Err(RecoveryStoreError::InvalidLease(
                "interrupted recovery claim reason must not be empty".to_string(),
            ));
        }
        self.persist_mutation(
            recovery_id,
            operation_id.into(),
            lease.acquired_at,
            RecoveryMutation::ClaimInterruptedRecovery {
                expected_generation,
                lease,
                reason,
                confirmed_root_id: None,
            },
        )
    }

    /// Confirm one recorded ambiguous provider root and claim its launch in a
    /// single generation-CAS mutation.
    ///
    /// Keeping confirmation and lease acquisition atomic prevents two stale
    /// Recovery Center clients from selecting different roots for one record.
    pub fn claim_recovery_with_confirmed_root(
        &self,
        recovery_id: &str,
        expected_generation: u64,
        provider_root_id: &str,
        lease: RecoveryLease,
        reason: &str,
        operation_id: impl Into<String>,
    ) -> RecoveryStoreResult<RecoveryRecord> {
        validate_recovery_lease(&lease)?;
        let provider_root_id = normalize_provider_root_id(provider_root_id)?;
        let reason = sanitize_visible_text(reason);
        if reason.trim().is_empty() {
            return Err(RecoveryStoreError::InvalidLease(
                "claim reason must not be empty".to_string(),
            ));
        }
        self.persist_mutation(
            recovery_id,
            operation_id.into(),
            lease.acquired_at,
            RecoveryMutation::ClaimRecovery {
                expected_generation,
                lease,
                reason,
                confirmed_root_id: Some(provider_root_id),
            },
        )
    }

    /// Atomically claim an authoritative provider root at project scope and
    /// the corresponding Recovery Record generation.
    ///
    /// The project claim is published before the per-record recovery event.
    /// A crash at that boundary therefore fails closed: a second process sees
    /// the bounded claim and cannot launch the same provider conversation.
    /// `confirm_candidate` selects whether `provider_root_id` must be promoted
    /// from the record's ambiguity set in the same record mutation.
    #[allow(clippy::too_many_arguments)]
    pub fn claim_recovery_with_provider_root(
        &self,
        recovery_id: &str,
        expected_generation: u64,
        provider_root_id: &str,
        confirm_candidate: bool,
        lease: RecoveryLease,
        holder_window_id: &str,
        reason: &str,
        operation_id: impl Into<String>,
    ) -> RecoveryStoreResult<RecoveryRecord> {
        self.claim_recovery_with_provider_root_inner(
            recovery_id,
            expected_generation,
            provider_root_id,
            confirm_candidate,
            lease,
            holder_window_id,
            reason,
            false,
            operation_id.into(),
        )
    }

    /// Project-wide provider-root claim for a historically Ready recovery
    /// whose previous PTY supervisor is durably proven stopped.
    ///
    /// This is intentionally separate from the ordinary claim API: only the
    /// dedicated interrupted mutation accepts a Ready launch stage, and it
    /// validates the persisted supervisor proof under the same record lock as
    /// the generation CAS.
    #[allow(clippy::too_many_arguments)]
    pub fn claim_interrupted_recovery_with_provider_root(
        &self,
        recovery_id: &str,
        expected_generation: u64,
        provider_root_id: &str,
        confirm_candidate: bool,
        lease: RecoveryLease,
        holder_window_id: &str,
        reason: &str,
        operation_id: impl Into<String>,
    ) -> RecoveryStoreResult<RecoveryRecord> {
        self.claim_recovery_with_provider_root_inner(
            recovery_id,
            expected_generation,
            provider_root_id,
            confirm_candidate,
            lease,
            holder_window_id,
            reason,
            true,
            operation_id.into(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn claim_recovery_with_provider_root_inner(
        &self,
        recovery_id: &str,
        expected_generation: u64,
        provider_root_id: &str,
        confirm_candidate: bool,
        lease: RecoveryLease,
        holder_window_id: &str,
        reason: &str,
        interrupted_ready: bool,
        operation_id: String,
    ) -> RecoveryStoreResult<RecoveryRecord> {
        validate_recovery_id(recovery_id)?;
        validate_recovery_lease(&lease)?;
        let provider_root_id = normalize_provider_root_id(provider_root_id)?;
        ensure_text_limit(
            "provider_root_claim.window_id",
            holder_window_id,
            MAX_RECOVERY_IDENTIFIER_CHARS,
        )?;
        if holder_window_id.trim().is_empty() {
            return Err(RecoveryStoreError::InvalidLease(
                "provider root claim window id must not be empty".to_string(),
            ));
        }
        validate_provider_root_claim_expiry(lease.acquired_at, lease.expires_at)?;
        let reason = sanitize_visible_text(reason);
        if reason.trim().is_empty() {
            return Err(RecoveryStoreError::InvalidLease(
                "claim reason must not be empty".to_string(),
            ));
        }
        if operation_id.trim().is_empty() {
            return Err(RecoveryStoreError::OperationConflict { operation_id });
        }
        ensure_text_limit("operation_id", &operation_id, MAX_RECOVERY_IDENTIFIER_CHARS)?;
        self.repair_pending_continuations_for(recovery_id)?;

        self.with_provider_root_claim_lock(|| {
            self.with_recovery_lock(recovery_id, || {
                let mut state = self.load_unlocked(recovery_id)?;
                self.migrate_operation_receipts_unlocked(recovery_id, &mut state)?;
                let current = state
                    .record
                    .as_ref()
                    .ok_or_else(|| RecoveryStoreError::NotFound(recovery_id.to_string()))?;
                let selected_root_is_authoritative =
                    current.provider_root.as_ref().is_some_and(|root| {
                        root.quality.is_authoritative() && root.root_id == provider_root_id
                    });
                let selected_root_is_candidate = current
                    .provider_root_candidates
                    .iter()
                    .any(|candidate| candidate.root_id == provider_root_id);
                if (!confirm_candidate && !selected_root_is_authoritative)
                    || (confirm_candidate && !selected_root_is_candidate)
                {
                    return Err(RecoveryStoreError::UnknownProviderRootCandidate {
                        root_id: provider_root_id.clone(),
                    });
                }

                let mutation = if interrupted_ready {
                    RecoveryMutation::ClaimInterruptedRecovery {
                        expected_generation,
                        lease: lease.clone(),
                        reason: reason.clone(),
                        confirmed_root_id: confirm_candidate.then(|| provider_root_id.clone()),
                    }
                } else {
                    RecoveryMutation::ClaimRecovery {
                        expected_generation,
                        lease: lease.clone(),
                        reason: reason.clone(),
                        confirmed_root_id: confirm_candidate.then(|| provider_root_id.clone()),
                    }
                };
                let mutation_digest = checksum_json(&mutation)?;
                if let Some(committed) =
                    self.committed_operation_unlocked(recovery_id, &state, &operation_id)?
                {
                    if committed.mutation_digest == mutation_digest {
                        return Ok(current.clone());
                    }
                    return Err(RecoveryStoreError::OperationConflict {
                        operation_id: operation_id.clone(),
                    });
                }

                // Validate the full record transition before publishing the
                // independent project claim. Only the injected crash seam may
                // intentionally leave a claim without its record event.
                let mut probe = state.record.clone();
                apply_mutation(
                    &mut probe,
                    recovery_id,
                    current.generation.saturating_add(1),
                    lease.acquired_at,
                    &mutation,
                )?;

                let provider_root_hash =
                    provider_root_claim_hash(&current.provider, &provider_root_id);
                if let Some(owner) = self.find_live_provider_root_owner_unlocked(
                    &provider_root_hash,
                    Some(recovery_id),
                )? {
                    return Err(RecoveryStoreError::ProviderRootClaimConflict {
                        holder_recovery_id: owner.recovery_id,
                        holder_session_id: owner.session_id,
                        holder_window_id: "durable-live-owner".to_string(),
                    });
                }
                let claim = ProviderRootClaim {
                    provider_root_hash: provider_root_hash.clone(),
                    claim_token: lease.lease_id.clone(),
                    holder_recovery_id: current.recovery_id.clone(),
                    holder_session_id: current.session_id.clone(),
                    holder_window_id: holder_window_id.to_string(),
                    acquired_at: lease.acquired_at,
                    expires_at: lease.expires_at,
                };
                self.acquire_provider_root_claim_unlocked(&provider_root_hash, claim)?;

                self.persist_mutation_unlocked(
                    recovery_id,
                    operation_id,
                    lease.acquired_at,
                    mutation,
                )
                // Every semantic failure was rejected by the probe above.
                // Any error here may follow durable event publication (for
                // example a snapshot write failure), so the bounded project
                // claim must remain fail-closed until retry or expiry.
            })
        })
    }

    /// Read the unexpired project-wide claim for a provider root.
    pub fn active_provider_root_claim(
        &self,
        provider: &str,
        provider_root_id: &str,
        now: DateTime<Utc>,
    ) -> RecoveryStoreResult<Option<ProviderRootClaim>> {
        let provider_root_id = normalize_provider_root_id(provider_root_id)?;
        let provider_root_hash = provider_root_claim_hash(provider, &provider_root_id);
        self.with_provider_root_claim_lock(|| {
            let claim = self.load_provider_root_claim_unlocked(&provider_root_hash)?;
            let active = match claim {
                Some(claim)
                    if claim.expires_at > now
                        || self.provider_root_claim_holder_is_live_unlocked(
                            &provider_root_hash,
                            &claim,
                        )? =>
                {
                    Some(claim)
                }
                _ => None,
            };
            Ok(active)
        })
    }

    /// Return whether another durable live Recovery Record or unexpired
    /// launch claim owns this provider root.
    pub fn provider_root_owned_by_other_recovery(
        &self,
        provider: &str,
        provider_root_id: &str,
        recovery_id: &str,
        now: DateTime<Utc>,
    ) -> RecoveryStoreResult<bool> {
        validate_recovery_id(recovery_id)?;
        let provider_root_id = normalize_provider_root_id(provider_root_id)?;
        let provider_root_hash = provider_root_claim_hash(provider, &provider_root_id);
        self.with_provider_root_claim_lock(|| {
            if self
                .load_provider_root_claim_unlocked(&provider_root_hash)?
                .is_some_and(|claim| {
                    claim.holder_recovery_id != recovery_id && claim.expires_at > now
                })
            {
                return Ok(true);
            }
            Ok(self
                .find_live_provider_root_owner_unlocked(&provider_root_hash, Some(recovery_id))?
                .is_some())
        })
    }

    /// Renew one still-live claim and optionally attach the actual launched
    /// window identity. The token and recovery identity must still match.
    pub fn renew_provider_root_claim_for_recovery(
        &self,
        recovery_id: &str,
        claim_token: &str,
        holder_window_id: &str,
        renewed_at: DateTime<Utc>,
        expires_at: DateTime<Utc>,
    ) -> RecoveryStoreResult<ProviderRootClaim> {
        validate_recovery_id(recovery_id)?;
        validate_provider_root_claim_expiry(renewed_at, expires_at)?;
        ensure_text_limit(
            "provider_root_claim.window_id",
            holder_window_id,
            MAX_RECOVERY_IDENTIFIER_CHARS,
        )?;
        self.with_provider_root_claim_lock(|| {
            self.with_recovery_lock(recovery_id, || {
                let record = self
                    .load_unlocked(recovery_id)?
                    .record
                    .ok_or_else(|| RecoveryStoreError::NotFound(recovery_id.to_string()))?;
                if record
                    .recovery_lease
                    .as_ref()
                    .is_none_or(|lease| lease.lease_id != claim_token)
                {
                    return Err(RecoveryStoreError::InvalidLease(
                        "provider root claim token no longer owns this recovery".to_string(),
                    ));
                }
                let root = record
                    .provider_root
                    .as_ref()
                    .filter(|root| root.quality.is_authoritative())
                    .ok_or_else(|| {
                        RecoveryStoreError::InvalidLease(
                            "recovery has no authoritative provider root".to_string(),
                        )
                    })?;
                let provider_root_hash = provider_root_claim_hash(&record.provider, &root.root_id);
                let mut claim = self
                    .load_provider_root_claim_unlocked(&provider_root_hash)?
                    .ok_or_else(|| {
                        RecoveryStoreError::InvalidLease(
                            "provider root claim is no longer active".to_string(),
                        )
                    })?;
                if claim.claim_token != claim_token
                    || claim.holder_recovery_id != recovery_id
                    || claim.expires_at <= renewed_at
                {
                    return Err(RecoveryStoreError::ProviderRootClaimConflict {
                        holder_recovery_id: claim.holder_recovery_id,
                        holder_session_id: claim.holder_session_id,
                        holder_window_id: claim.holder_window_id,
                    });
                }
                claim.holder_window_id = holder_window_id.to_string();
                claim.acquired_at = renewed_at;
                claim.expires_at = expires_at;
                self.write_provider_root_claim_state_unlocked(
                    &provider_root_hash,
                    renewed_at,
                    Some(claim.clone()),
                    None,
                )?;
                Ok(claim)
            })
        })
    }

    /// Clear a crashed launch's expired lease after startup has durably
    /// established that its owning Session is interrupted.
    ///
    /// The provider-claim and record locks are held in the same order as
    /// claim/renew. A renewal that won the race keeps the record Recovering;
    /// otherwise the expired lease is cleared through an Interrupted mutation
    /// and any matching stale project claim is CAS-released.
    pub fn interrupt_expired_recovery_lease(
        &self,
        recovery_id: &str,
        now: DateTime<Utc>,
        reason: &str,
        operation_id: impl Into<String>,
    ) -> RecoveryStoreResult<RecoveryRecord> {
        validate_recovery_id(recovery_id)?;
        let reason = sanitize_visible_text(reason);
        if reason.trim().is_empty() {
            return Err(RecoveryStoreError::InvalidLease(
                "expired recovery lease interruption reason must not be empty".to_string(),
            ));
        }
        let operation_id = operation_id.into();
        if operation_id.trim().is_empty() {
            return Err(RecoveryStoreError::OperationConflict { operation_id });
        }
        ensure_text_limit("operation_id", &operation_id, MAX_RECOVERY_IDENTIFIER_CHARS)?;
        self.repair_pending_continuations_for(recovery_id)?;

        self.with_provider_root_claim_lock(|| {
            self.with_recovery_lock(recovery_id, || {
                let current = self
                    .load_unlocked(recovery_id)?
                    .record
                    .ok_or_else(|| RecoveryStoreError::NotFound(recovery_id.to_string()))?;
                let interrupted_ready_claim = current.launch_stage == RecoveryLaunchStage::Ready
                    && current
                        .supervisor_stop_proof
                        .as_ref()
                        .is_some_and(|proof| proof.session_id == current.session_id);
                if current.lifecycle != RecoveryLifecycle::Recovering
                    || current.launch_stage.is_terminal()
                    || (current.launch_stage >= RecoveryLaunchStage::Ready
                        && !interrupted_ready_claim)
                {
                    return Ok(current);
                }
                let Some(lease) = current.recovery_lease.as_ref() else {
                    return Ok(current);
                };
                if lease.expires_at > now {
                    return Ok(current);
                }

                let provider_root_hash = current
                    .provider_root
                    .as_ref()
                    .filter(|root| root.quality.is_authoritative())
                    .map(|root| provider_root_claim_hash(&current.provider, &root.root_id));
                let active_claim = provider_root_hash
                    .as_deref()
                    .map(|hash| self.load_provider_root_claim_unlocked(hash))
                    .transpose()?
                    .flatten();
                if active_claim.as_ref().is_some_and(|claim| {
                    claim.holder_recovery_id == recovery_id && claim.expires_at > now
                }) {
                    return Ok(current);
                }

                let interrupted = self.persist_mutation_unlocked(
                    recovery_id,
                    operation_id.clone(),
                    now,
                    RecoveryMutation::SetLifecycle {
                        lifecycle: RecoveryLifecycle::Interrupted,
                        reason: Some(reason.clone()),
                    },
                )?;
                if let (Some(provider_root_hash), Some(claim)) =
                    (provider_root_hash.as_deref(), active_claim.as_ref())
                {
                    if claim.claim_token == lease.lease_id
                        && claim.holder_recovery_id == recovery_id
                        && claim.expires_at <= now
                    {
                        self.release_provider_root_claim_unlocked(
                            provider_root_hash,
                            &lease.lease_id,
                            now,
                        )?;
                    }
                }
                Ok(interrupted)
            })
        })
    }

    /// Cross the provider Ready barrier only while the exact-resume launch
    /// still owns its project-wide provider-root claim.
    ///
    /// Exact recovery launches create a new target Recovery Record while the
    /// bounded claim remains attached to the interrupted source record. This
    /// method therefore validates the source claim and advances the target as
    /// one project-claim critical section. The target becomes the durable live
    /// owner before the claim is released, so expiry/replacement can never
    /// leave two provider processes accepted as Ready.
    #[allow(clippy::too_many_arguments)]
    pub fn complete_claimed_provider_ready(
        &self,
        claim_holder_recovery_id: &str,
        ready_recovery_id: &str,
        claim_token: &str,
        ready_at: DateTime<Utc>,
        operation_id: impl Into<String>,
    ) -> RecoveryStoreResult<RecoveryRecord> {
        validate_recovery_id(claim_holder_recovery_id)?;
        validate_recovery_id(ready_recovery_id)?;
        if claim_token.trim().is_empty() {
            return Err(RecoveryStoreError::InvalidLease(
                "provider root claim token must not be empty".to_string(),
            ));
        }
        ensure_text_limit(
            "provider_root_claim.token",
            claim_token,
            MAX_RECOVERY_IDENTIFIER_CHARS,
        )?;
        let operation_id = operation_id.into();
        if operation_id.trim().is_empty() {
            return Err(RecoveryStoreError::OperationConflict { operation_id });
        }
        ensure_text_limit("operation_id", &operation_id, MAX_RECOVERY_IDENTIFIER_CHARS)?;
        self.repair_pending_continuations_for(claim_holder_recovery_id)?;
        if ready_recovery_id != claim_holder_recovery_id {
            self.repair_pending_continuations_for(ready_recovery_id)?;
        }

        let mutation = RecoveryMutation::CompleteClaimedProviderReady {
            claim_holder_recovery_id: claim_holder_recovery_id.to_string(),
            claim_token: claim_token.to_string(),
        };
        let mutation_digest = checksum_json(&mutation)?;
        let complete = || {
            let source = self
                .load_unlocked(claim_holder_recovery_id)?
                .record
                .ok_or_else(|| {
                    RecoveryStoreError::NotFound(claim_holder_recovery_id.to_string())
                })?;
            let mut ready_state = self.load_unlocked(ready_recovery_id)?;
            self.migrate_operation_receipts_unlocked(ready_recovery_id, &mut ready_state)?;
            if let Some(committed) =
                self.committed_operation_unlocked(ready_recovery_id, &ready_state, &operation_id)?
            {
                if committed.mutation_digest == mutation_digest {
                    if let Some(root) = source
                        .provider_root
                        .as_ref()
                        .filter(|root| root.quality.is_authoritative())
                    {
                        let provider_root_hash =
                            provider_root_claim_hash(&source.provider, &root.root_id);
                        if self
                            .load_provider_root_claim_unlocked(&provider_root_hash)?
                            .is_some_and(|claim| {
                                claim.claim_token == claim_token
                                    && claim.holder_recovery_id == claim_holder_recovery_id
                            })
                        {
                            self.release_provider_root_claim_unlocked(
                                &provider_root_hash,
                                claim_token,
                                ready_at,
                            )?;
                        }
                    }
                    return ready_state.record.ok_or_else(|| {
                        RecoveryStoreError::NotFound(ready_recovery_id.to_string())
                    });
                }
                return Err(RecoveryStoreError::OperationConflict {
                    operation_id: operation_id.clone(),
                });
            }
            let ready = ready_state
                .record
                .as_ref()
                .ok_or_else(|| RecoveryStoreError::NotFound(ready_recovery_id.to_string()))?;
            if source
                .recovery_lease
                .as_ref()
                .is_none_or(|lease| lease.lease_id != claim_token)
            {
                return Err(RecoveryStoreError::InvalidLease(
                    "provider root claim token no longer owns the source recovery".to_string(),
                ));
            }
            if ready.launch_stage >= RecoveryLaunchStage::Ready
                || ready.launch_stage.is_terminal()
                || matches!(
                    ready.lifecycle,
                    RecoveryLifecycle::Running
                        | RecoveryLifecycle::Resolved
                        | RecoveryLifecycle::Discarded
                )
            {
                return Err(RecoveryStoreError::InvalidLease(
                    "provider Ready target already crossed its claim barrier".to_string(),
                ));
            }
            let source_root = source
                .provider_root
                .as_ref()
                .filter(|root| root.quality.is_authoritative())
                .ok_or_else(|| {
                    RecoveryStoreError::InvalidLease(
                        "source recovery has no authoritative provider root".to_string(),
                    )
                })?;
            let ready_root = ready
                .provider_root
                .as_ref()
                .filter(|root| root.quality.is_authoritative())
                .ok_or_else(|| {
                    RecoveryStoreError::InvalidLease(
                        "provider Ready target has no authoritative provider root".to_string(),
                    )
                })?;
            let source_root_hash = provider_root_claim_hash(&source.provider, &source_root.root_id);
            let ready_root_hash = provider_root_claim_hash(&ready.provider, &ready_root.root_id);
            if source_root_hash != ready_root_hash {
                return Err(RecoveryStoreError::RootBindingConflict {
                    expected: source_root.root_id.clone(),
                    actual: ready_root.root_id.clone(),
                });
            }
            let claim = self
                .load_provider_root_claim_unlocked(&source_root_hash)?
                .ok_or_else(|| {
                    RecoveryStoreError::InvalidLease(
                        "provider root claim is no longer active at Ready".to_string(),
                    )
                })?;
            if claim.claim_token != claim_token
                || claim.holder_recovery_id != claim_holder_recovery_id
                || claim.holder_session_id != source.session_id
                || claim.expires_at <= ready_at
            {
                return Err(RecoveryStoreError::ProviderRootClaimConflict {
                    holder_recovery_id: claim.holder_recovery_id,
                    holder_session_id: claim.holder_session_id,
                    holder_window_id: claim.holder_window_id,
                });
            }

            let completed = self.persist_mutation_unlocked(
                ready_recovery_id,
                operation_id.clone(),
                ready_at,
                mutation.clone(),
            )?;
            if !self.release_provider_root_claim_unlocked(
                &source_root_hash,
                claim_token,
                ready_at,
            )? {
                return Err(RecoveryStoreError::InvalidLease(
                    "provider root claim changed while crossing Ready".to_string(),
                ));
            }
            Ok(completed)
        };

        self.with_provider_root_claim_lock(|| {
            if claim_holder_recovery_id == ready_recovery_id {
                self.with_recovery_lock(ready_recovery_id, complete)
            } else {
                self.with_recovery_locks(claim_holder_recovery_id, ready_recovery_id, complete)
            }
        })
    }

    /// Release exactly the current claim token owned by one Recovery Record.
    /// A stale holder returns `false` and cannot delete a replacement claim.
    pub fn release_provider_root_claim_for_recovery(
        &self,
        recovery_id: &str,
        claim_token: &str,
        released_at: DateTime<Utc>,
    ) -> RecoveryStoreResult<bool> {
        validate_recovery_id(recovery_id)?;
        self.with_provider_root_claim_lock(|| {
            self.with_recovery_lock(recovery_id, || {
                let record = self
                    .load_unlocked(recovery_id)?
                    .record
                    .ok_or_else(|| RecoveryStoreError::NotFound(recovery_id.to_string()))?;
                if record
                    .recovery_lease
                    .as_ref()
                    .is_none_or(|lease| lease.lease_id != claim_token)
                {
                    return Ok(false);
                }
                let Some(root) = record
                    .provider_root
                    .as_ref()
                    .filter(|root| root.quality.is_authoritative())
                else {
                    return Ok(false);
                };
                let provider_root_hash = provider_root_claim_hash(&record.provider, &root.root_id);
                self.release_provider_root_claim_unlocked(
                    &provider_root_hash,
                    claim_token,
                    released_at,
                )
            })
        })
    }

    /// Publish a crash-atomic source-to-successor decision before the target
    /// Session or RecoveryRecord is created.
    ///
    /// Only one prepared successor may exist for a source. A semantic retry
    /// (same source revision and mode/reason) returns the already assigned
    /// target id, preventing a restarted UI/startup queue from launching a
    /// second replacement after losing the first response.
    pub fn prepare_successor(
        &self,
        mut proposed: RecoveryContinuationLink,
        operation_id: impl Into<String>,
    ) -> RecoveryStoreResult<RecoveryContinuationLink> {
        validate_successor_link(&mut proposed)?;
        let operation_id = operation_id.into();
        validate_continuation_operation_id(&operation_id)?;

        self.with_continuation_lock(|| {
            self.repair_pending_continuations_unlocked()?;
            self.cancel_discarded_successors_unlocked()?;
            let transactions = self.read_continuation_transactions_unlocked()?;
            if let Some(existing) = transactions.iter().find(|transaction| {
                transaction.await_source_finalization
                    && transaction.link.source_recovery_id == proposed.source_recovery_id
            }) {
                if successor_request_semantics_match(&existing.link, &proposed) {
                    return Ok(existing.link.clone());
                }
                return Err(RecoveryStoreError::ContinuationConflict);
            }

            let source_recovery_id = proposed.source_recovery_id.clone();
            let target_recovery_id = proposed.target_recovery_id.clone();
            self.with_recovery_locks(&source_recovery_id, &target_recovery_id, || {
                let source_state = self.load_unlocked(&proposed.source_recovery_id)?;
                let source = source_state.record.as_ref().ok_or_else(|| {
                    RecoveryStoreError::NotFound(proposed.source_recovery_id.clone())
                })?;
                if source.checkpoint_revision != proposed.source_checkpoint_revision {
                    return Err(RecoveryStoreError::RevisionMismatch {
                        expected: proposed.source_checkpoint_revision,
                        actual: source.checkpoint_revision,
                    });
                }
                if let Some(existing) = source.continuation_targets.first() {
                    if successor_request_semantics_match(existing, &proposed) {
                        proposed = existing.clone();
                    } else {
                        return Err(RecoveryStoreError::ContinuationConflict);
                    }
                }
                if let Some(target) = self.load_unlocked(&proposed.target_recovery_id)?.record {
                    let already_linked = target
                        .continuation_source
                        .as_ref()
                        .is_some_and(|link| continuation_link_semantics_match(link, &proposed));
                    if !already_linked {
                        return Err(RecoveryStoreError::ContinuationConflict);
                    }
                }
                if let Some(checkpoint) = source.checkpoint.as_ref() {
                    for attachment in &checkpoint.attachment_refs {
                        self.verify_attachment(attachment)?;
                    }
                }
                let transaction = ContinuationTransactionBody {
                    schema_version: STORE_SCHEMA_VERSION,
                    operation_id: operation_id.clone(),
                    link: proposed.clone(),
                    inherited_state: continuation_inherited_state(source),
                    await_source_finalization: true,
                };
                self.write_continuation_transaction_unlocked(&transaction)?;
                self.inject_fault(RecoveryStoreFaultPoint::AfterContinuationIntentPublication)?;
                Ok(proposed.clone())
            })
        })
    }

    /// Return the durable prepared successor for a source, if one exists.
    pub fn prepared_successor_for_source(
        &self,
        source_recovery_id: &str,
    ) -> RecoveryStoreResult<Option<RecoveryContinuationLink>> {
        validate_recovery_id(source_recovery_id)?;
        self.with_continuation_lock(|| {
            self.repair_pending_continuations_unlocked()?;
            self.cancel_discarded_successors_unlocked()?;
            Ok(self
                .read_continuation_transactions_unlocked()?
                .into_iter()
                .find(|transaction| {
                    transaction.await_source_finalization
                        && transaction.link.source_recovery_id == source_recovery_id
                })
                .map(|transaction| transaction.link))
        })
    }

    /// Repair prepared successor links and retire sources whose successor has
    /// crossed the durable Ready barrier. Safe to call repeatedly at startup
    /// or before presenting Recovery Center inventory.
    pub fn reconcile_successors(&self) -> RecoveryStoreResult<Vec<RecoveryContinuationLink>> {
        let scan_budget = active_recovery_scan_budget().unwrap_or_default();
        with_active_recovery_scan_budget(&scan_budget, || self.reconcile_successors_unscoped())
    }

    fn reconcile_successors_unscoped(&self) -> RecoveryStoreResult<Vec<RecoveryContinuationLink>> {
        let mut completed = Vec::new();
        loop {
            let transactions = self.with_continuation_lock(|| {
                self.repair_pending_continuations_unlocked()?;
                self.cancel_discarded_successors_unlocked()?;
                Ok(self
                    .read_continuation_transactions_unlocked()?
                    .into_iter()
                    .filter(|transaction| transaction.await_source_finalization)
                    .collect::<Vec<_>>())
            })?;
            let mut made_progress = false;
            for transaction in transactions {
                if self.successor_target_disposition(&transaction.link.target_recovery_id)?
                    != SuccessorTargetDisposition::Ready
                {
                    continue;
                }

                if let Some(source) = self.load(&transaction.link.source_recovery_id)? {
                    if !source.board_outbox.is_empty() {
                        // Board delivery remains source-owned. Never purge its
                        // outbox merely because the replacement became Ready.
                        continue;
                    }
                    self.inject_fault(RecoveryStoreFaultPoint::AfterSuccessorReadyObservation)?;
                    let operation_id = successor_terminal_operation_id("resolve", &transaction)?;
                    self.finalize_and_purge(
                        &transaction.link.source_recovery_id,
                        RecoveryLifecycle::Resolved,
                        Utc::now(),
                        operation_id,
                    )?;
                } else if self
                    .load_tombstone(&transaction.link.source_recovery_id)?
                    .is_none()
                {
                    return Err(invalid_recovery_data(
                        "prepared successor source disappeared without a tombstone",
                    ));
                }

                self.with_continuation_lock(|| {
                    let path = self.continuation_transaction_path(&transaction)?;
                    if path.is_file() {
                        let current = self.read_continuation_transaction(&path)?;
                        if current.body != transaction {
                            return Err(RecoveryStoreError::ContinuationConflict);
                        }
                        self.remove_continuation_transaction_unlocked(&transaction)?;
                        self.inject_fault(RecoveryStoreFaultPoint::AfterContinuationIntentCleanup)?;
                    }
                    Ok(())
                })?;
                completed.push(transaction.link);
                made_progress = true;
            }
            if !made_progress {
                break;
            }
        }
        Ok(completed)
    }

    /// Durably link a fresh-provider continuation to the exact/checkpoint
    /// recovery it supersedes. Both records receive the same provenance. The
    /// source checkpoint revision is checked again under the source mutation
    /// lock so a continuation cannot silently claim newer context than the
    /// prompt actually contained.
    pub fn link_continuation(
        &self,
        mut link: RecoveryContinuationLink,
        operation_id: impl Into<String>,
    ) -> RecoveryStoreResult<(RecoveryRecord, RecoveryRecord)> {
        validate_recovery_id(&link.source_recovery_id)?;
        validate_recovery_id(&link.target_recovery_id)?;
        if link.source_recovery_id == link.target_recovery_id {
            return Err(RecoveryStoreError::ContinuationConflict);
        }
        link.definitive_reason = sanitize_visible_text(&link.definitive_reason);
        if link.definitive_reason.trim().is_empty() {
            return Err(RecoveryStoreError::ContinuationConflict);
        }
        let operation_id = operation_id.into();
        if operation_id.trim().is_empty() {
            return Err(RecoveryStoreError::OperationConflict { operation_id });
        }
        ensure_text_limit("operation_id", &operation_id, MAX_RECOVERY_IDENTIFIER_CHARS)?;
        ensure_text_limit(
            "operation_id",
            &format!("{operation_id}:source"),
            MAX_RECOVERY_IDENTIFIER_CHARS,
        )?;
        ensure_text_limit(
            "operation_id",
            &format!("{operation_id}:target"),
            MAX_RECOVERY_IDENTIFIER_CHARS,
        )?;

        self.with_continuation_lock(|| {
            self.repair_pending_continuations_unlocked()?;
            self.with_recovery_locks(&link.source_recovery_id, &link.target_recovery_id, || {
                let mut source_state = self.load_unlocked(&link.source_recovery_id)?;
                let mut target_state = self.load_unlocked(&link.target_recovery_id)?;
                self.migrate_operation_receipts_unlocked(
                    &link.source_recovery_id,
                    &mut source_state,
                )?;
                self.migrate_operation_receipts_unlocked(
                    &link.target_recovery_id,
                    &mut target_state,
                )?;
                let source = source_state
                    .record
                    .as_ref()
                    .ok_or_else(|| RecoveryStoreError::NotFound(link.source_recovery_id.clone()))?;
                let target = target_state
                    .record
                    .as_ref()
                    .ok_or_else(|| RecoveryStoreError::NotFound(link.target_recovery_id.clone()))?;

                if continuation_retry_matches(source, target, &link) {
                    return Ok((source.clone(), target.clone()));
                }

                let source_operation_id = format!("{operation_id}:source");
                let target_operation_id = format!("{operation_id}:target");
                let source_committed = self
                    .committed_operation_unlocked(
                        &link.source_recovery_id,
                        &source_state,
                        &source_operation_id,
                    )?
                    .is_some();
                let target_committed = self
                    .committed_operation_unlocked(
                        &link.target_recovery_id,
                        &target_state,
                        &target_operation_id,
                    )?
                    .is_some();
                if source_committed || target_committed {
                    if source_committed
                        && target_committed
                        && continuation_retry_matches(source, target, &link)
                    {
                        return Ok((source.clone(), target.clone()));
                    }
                    return Err(RecoveryStoreError::OperationConflict { operation_id });
                }

                if source.repo_id != target.repo_id {
                    return Err(RecoveryStoreError::ContinuationConflict);
                }
                if source.checkpoint_revision != link.source_checkpoint_revision {
                    return Err(RecoveryStoreError::RevisionMismatch {
                        expected: link.source_checkpoint_revision,
                        actual: source.checkpoint_revision,
                    });
                }
                if let Some(checkpoint) = source.checkpoint.as_ref() {
                    for attachment in &checkpoint.attachment_refs {
                        self.verify_attachment(attachment)?;
                    }
                }
                let has_inherited_state = source.checkpoint.is_some()
                    || source.latest_root_input.is_some()
                    || source.provider_root.is_some();
                let inherited_state = has_inherited_state.then(|| {
                    let mut checkpoint = source.checkpoint.clone();
                    if let Some(checkpoint) = checkpoint.as_mut() {
                        // Board delivery remains owned by the source
                        // recovery. The target inherits semantic and
                        // attachment context only.
                        checkpoint.board_intents.clear();
                    }
                    Box::new(RecoveryContinuationState {
                        checkpoint_revision: source.checkpoint_revision,
                        checkpoint_coverage: source.checkpoint_coverage,
                        checkpoint,
                        latest_root_input: source.latest_root_input.clone(),
                        source_provider_root_id: source
                            .provider_root
                            .as_ref()
                            .map(|root| root.root_id.clone()),
                    })
                });
                let transaction = ContinuationTransactionBody {
                    schema_version: STORE_SCHEMA_VERSION,
                    operation_id: operation_id.clone(),
                    link: link.clone(),
                    inherited_state,
                    await_source_finalization: false,
                };
                self.write_continuation_transaction_unlocked(&transaction)?;
                self.inject_fault(RecoveryStoreFaultPoint::AfterContinuationIntentPublication)?;
                self.apply_continuation_transaction_locked(&transaction)
            })
        })
    }

    /// Mark a queued Board milestone as durably published.
    ///
    /// This must be called after `post_entry_idempotent` succeeds. Reusing the
    /// same operation id is safe and returns the already committed generation.
    pub fn ack_board_entry(
        &self,
        recovery_id: &str,
        entry_id: &str,
        operation_id: impl Into<String>,
    ) -> RecoveryStoreResult<RecoveryRecord> {
        self.persist_mutation(
            recovery_id,
            operation_id.into(),
            Utc::now(),
            RecoveryMutation::AckBoardEntry {
                entry_id: entry_id.to_string(),
            },
        )
    }

    /// Persist the latest Board outbox delivery diagnostic without changing
    /// checkpoint revision or lifecycle. The value is whitespace-normalized
    /// and bounded before it enters the durable recovery log.
    pub fn set_board_delivery_error(
        &self,
        recovery_id: &str,
        error: Option<&str>,
    ) -> RecoveryStoreResult<RecoveryRecord> {
        let error = error.map(bounded_board_delivery_error);
        let current = self
            .load(recovery_id)?
            .ok_or_else(|| RecoveryStoreError::NotFound(recovery_id.to_string()))?;
        if current.board_delivery_error == error {
            return Ok(current);
        }
        self.persist_mutation(
            recovery_id,
            format!(
                "board-delivery-error-{}-{}",
                current.generation.saturating_add(1),
                Uuid::new_v4().simple()
            ),
            Utc::now(),
            RecoveryMutation::SetBoardDeliveryError { error },
        )
    }

    pub fn load(&self, recovery_id: &str) -> RecoveryStoreResult<Option<RecoveryRecord>> {
        validate_recovery_id(recovery_id)?;
        self.repair_pending_continuations_for(recovery_id)?;
        let state = self.with_recovery_lock(recovery_id, || self.load_unlocked(recovery_id))?;
        Ok(state.record)
    }

    /// Return whether an immutable operation id is already part of this
    /// recovery's committed history.
    ///
    /// Bridge retries use this before reconstructing a CAS mutation whose
    /// expected revision has necessarily advanced after the first commit.
    pub fn has_committed_operation(
        &self,
        recovery_id: &str,
        operation_id: &str,
    ) -> RecoveryStoreResult<bool> {
        validate_recovery_id(recovery_id)?;
        if operation_id.trim().is_empty() {
            return Ok(false);
        }
        self.repair_pending_continuations_for(recovery_id)?;
        self.with_recovery_lock(recovery_id, || {
            let state = self.load_unlocked(recovery_id)?;
            Ok(self
                .committed_operation_unlocked(recovery_id, &state, operation_id)?
                .is_some())
        })
    }

    /// Load every durable recovery record, newest activity first.
    ///
    /// Partial temp files and unrelated directory entries are ignored by the
    /// per-record loader. A corrupt committed record remains an error so the
    /// caller can surface an attention state instead of silently hiding it.
    pub fn list(&self) -> RecoveryStoreResult<Vec<RecoveryRecord>> {
        self.ensure_root_dirs()?;
        let scan_budget = active_recovery_scan_budget().unwrap_or_default();
        with_active_recovery_scan_budget(&scan_budget, || {
            self.reconcile_successors_unscoped()?;
            let mut records = Vec::new();
            let mut explicit_scan_budget = scan_budget.clone();
            for recovery_id in self.recovery_ids_unlocked()? {
                let state = self.with_recovery_lock(&recovery_id, || {
                    self.load_unlocked_with_scan(&recovery_id, Some(&mut explicit_scan_budget))
                })?;
                if let Some(record) = state.record {
                    records.push(record);
                }
            }
            records.sort_by(|left, right| {
                right
                    .updated_at
                    .cmp(&left.updated_at)
                    .then_with(|| right.created_at.cmp(&left.created_at))
                    .then_with(|| right.recovery_id.cmp(&left.recovery_id))
            });
            Ok(records)
        })
    }

    pub fn load_tombstone(
        &self,
        recovery_id: &str,
    ) -> RecoveryStoreResult<Option<RecoveryTombstone>> {
        validate_recovery_id(recovery_id)?;
        let path = self.tombstone_path(recovery_id);
        if !path.is_file() {
            return Ok(None);
        }
        let bytes =
            read_bounded_store_file(&path, "recovery_tombstone", MAX_RECOVERY_TOMBSTONE_BYTES)?;
        let stored: StoredTombstone = serde_json::from_slice(&bytes)?;
        if checksum_json(&stored.body)? != stored.checksum {
            return Err(
                io::Error::new(io::ErrorKind::InvalidData, "invalid tombstone checksum").into(),
            );
        }
        Ok(Some(stored.body))
    }

    /// Commit terminal metadata outside the content directory, then remove
    /// every gwt-owned recovery payload. Provider stores are not under this
    /// root and are intentionally untouched.
    pub fn finalize_and_purge(
        &self,
        recovery_id: &str,
        lifecycle: RecoveryLifecycle,
        purged_at: DateTime<Utc>,
        operation_id: impl Into<String>,
    ) -> RecoveryStoreResult<RecoveryTombstone> {
        validate_recovery_id(recovery_id)?;
        self.repair_pending_continuations_for(recovery_id)?;
        if !matches!(
            lifecycle,
            RecoveryLifecycle::Resolved | RecoveryLifecycle::Discarded
        ) {
            return Err(RecoveryStoreError::InvalidTerminalLifecycle(lifecycle));
        }
        // Terminal publication releases only the CAS token recorded on this
        // recovery. If expiry already allowed another record to claim the
        // same root, the stale release is a no-op and cannot delete it.
        if let Some(claim_token) = self
            .load(recovery_id)?
            .and_then(|record| record.recovery_lease.map(|lease| lease.lease_id))
        {
            self.release_provider_root_claim_for_recovery(recovery_id, &claim_token, purged_at)?;
        }
        let operation_id = operation_id.into();
        let tombstone = self.with_recovery_lock(recovery_id, || {
            if let Some(existing) = self.load_tombstone(recovery_id)? {
                if existing.terminal_operation_id == operation_id && existing.lifecycle == lifecycle
                {
                    self.remove_recovery_payload(recovery_id)?;
                    return Ok(existing);
                }
                return Err(RecoveryStoreError::TombstoneConflict(
                    recovery_id.to_string(),
                ));
            }

            let state = self.load_unlocked(recovery_id)?;
            let record = state
                .record
                .ok_or_else(|| RecoveryStoreError::NotFound(recovery_id.to_string()))?;
            let provider_identity_hash = record
                .provider_root
                .as_ref()
                .map(|root| prefixed_sha256(&format!("{}:{}", record.provider, root.root_id)));
            let tombstone = RecoveryTombstone {
                schema_version: STORE_SCHEMA_VERSION,
                recovery_id: recovery_id.to_string(),
                lifecycle,
                launch_stage: match lifecycle {
                    RecoveryLifecycle::Resolved => RecoveryLaunchStage::Resolved,
                    RecoveryLifecycle::Discarded => RecoveryLaunchStage::Discarded,
                    _ => unreachable!("terminal lifecycle checked above"),
                },
                root_role: record.root_role,
                terminal_operation_id: operation_id,
                last_generation: record.generation,
                session_identity_hash: prefixed_sha256(&format!(
                    "{}:{}:{}",
                    record.repo_id, record.session_id, record.recovery_id
                )),
                launch_base_oid: Some(record.launch_base_oid.clone()),
                provider_identity_hash,
                purged_at,
                expires_at: purged_at + Duration::days(TOMBSTONE_RETENTION_DAYS),
            };
            self.write_tombstone(&tombstone)?;
            self.inject_fault(RecoveryStoreFaultPoint::AfterTombstonePublication)?;
            self.remove_recovery_payload(recovery_id)?;
            Ok(tombstone)
        })?;
        // A retry after a crash between content purge and blob GC reaches this
        // same call through the idempotent tombstone path and completes the
        // cleanup. Inventory errors fail closed and keep shared blobs.
        self.prune_unreferenced_attachments()?;
        Ok(tombstone)
    }

    pub fn remove_expired_tombstones(&self, now: DateTime<Utc>) -> RecoveryStoreResult<usize> {
        self.ensure_root_dirs()?;
        let mut removed = 0;
        let mut read_budget = RecoveryLedgerReadBudget::new(
            "recovery_tombstones",
            MAX_RECOVERY_TOMBSTONE_TOTAL_BYTES,
        );
        for path in bounded_json_paths(
            &self.tombstones_dir(),
            "recovery_tombstones",
            MAX_RECOVERY_TOMBSTONE_FILES,
            MAX_RECOVERY_TOMBSTONE_BYTES,
            MAX_RECOVERY_TOMBSTONE_TOTAL_BYTES,
        )? {
            let bytes = match read_budget.read(&path, MAX_RECOVERY_TOMBSTONE_BYTES, None) {
                Ok(bytes) => bytes,
                Err(error @ RecoveryStoreError::ContentLimitExceeded { .. }) => return Err(error),
                Err(_) => continue,
            };
            let stored: StoredTombstone = match serde_json::from_slice(&bytes) {
                Ok(stored) => stored,
                Err(_) => continue,
            };
            if checksum_json(&stored.body).ok().as_deref() != Some(&stored.checksum) {
                continue;
            }
            if stored.body.expires_at <= now {
                fs::remove_file(path)?;
                removed += 1;
            }
        }
        if removed > 0 {
            sync_directory(&self.tombstones_dir())?;
        }
        Ok(removed)
    }

    /// Repair any coordinator intent involving `recovery_id` before exposing
    /// or mutating that record. A durable intent is the transaction's commit
    /// decision; replay is therefore safe after every process-crash boundary.
    fn repair_pending_continuations_for(&self, recovery_id: &str) -> RecoveryStoreResult<()> {
        self.with_continuation_lock(|| {
            for transaction in self.read_continuation_transactions_unlocked()? {
                if transaction.link.source_recovery_id != recovery_id
                    && transaction.link.target_recovery_id != recovery_id
                {
                    continue;
                }
                self.with_recovery_locks(
                    &transaction.link.source_recovery_id,
                    &transaction.link.target_recovery_id,
                    || self.apply_continuation_if_materialized_locked(&transaction),
                )?;
            }
            Ok(())
        })
    }

    /// Repair all older intents while the global continuation lock is held.
    fn repair_pending_continuations_unlocked(&self) -> RecoveryStoreResult<()> {
        for transaction in self.read_continuation_transactions_unlocked()? {
            self.with_recovery_locks(
                &transaction.link.source_recovery_id,
                &transaction.link.target_recovery_id,
                || self.apply_continuation_if_materialized_locked(&transaction),
            )?;
        }
        Ok(())
    }

    /// Cancel a prepared successor whose target was explicitly discarded
    /// before Ready. The source link is removed with its own immutable
    /// operation before the coordinator intent is deleted, so every crash
    /// boundary retries to the same state and a later semantic retry may
    /// allocate a new target identity.
    fn cancel_discarded_successors_unlocked(&self) -> RecoveryStoreResult<()> {
        for transaction in self
            .read_continuation_transactions_unlocked()?
            .into_iter()
            .filter(|transaction| transaction.await_source_finalization)
        {
            let discarded = self.with_recovery_locks(
                &transaction.link.source_recovery_id,
                &transaction.link.target_recovery_id,
                || {
                    if self.successor_target_disposition_unlocked(
                        &transaction.link.target_recovery_id,
                    )? != SuccessorTargetDisposition::Discarded
                    {
                        return Ok(false);
                    }
                    let source_state = self.load_unlocked(&transaction.link.source_recovery_id)?;
                    if source_state.record.is_some() {
                        self.persist_mutation_unlocked(
                            &transaction.link.source_recovery_id,
                            successor_terminal_operation_id("cancel", &transaction)?,
                            Utc::now(),
                            RecoveryMutation::CancelContinuation {
                                link: transaction.link.clone(),
                            },
                        )?;
                        self.inject_fault(
                            RecoveryStoreFaultPoint::AfterContinuationSourcePublication,
                        )?;
                    } else if self
                        .load_tombstone(&transaction.link.source_recovery_id)?
                        .is_none()
                    {
                        return Err(invalid_recovery_data(
                            "discarded successor source disappeared without a tombstone",
                        ));
                    }
                    Ok(true)
                },
            )?;
            if discarded {
                self.remove_continuation_transaction_unlocked(&transaction)?;
                self.inject_fault(RecoveryStoreFaultPoint::AfterContinuationIntentCleanup)?;
            }
        }
        Ok(())
    }

    fn successor_target_disposition(
        &self,
        target_recovery_id: &str,
    ) -> RecoveryStoreResult<SuccessorTargetDisposition> {
        if let Some(tombstone) = self.load_tombstone(target_recovery_id)? {
            return Ok(successor_terminal_lifecycle_disposition(
                tombstone.lifecycle,
            ));
        }
        if let Some(target) = self.load(target_recovery_id)? {
            return Ok(successor_record_disposition(&target));
        }
        Ok(SuccessorTargetDisposition::Pending)
    }

    /// Same disposition read while the caller owns the target recovery lock.
    fn successor_target_disposition_unlocked(
        &self,
        target_recovery_id: &str,
    ) -> RecoveryStoreResult<SuccessorTargetDisposition> {
        if let Some(tombstone) = self.load_tombstone(target_recovery_id)? {
            return Ok(successor_terminal_lifecycle_disposition(
                tombstone.lifecycle,
            ));
        }
        if let Some(target) = self.load_unlocked(target_recovery_id)?.record {
            return Ok(successor_record_disposition(&target));
        }
        Ok(SuccessorTargetDisposition::Pending)
    }

    fn apply_continuation_if_materialized_locked(
        &self,
        transaction: &ContinuationTransactionBody,
    ) -> RecoveryStoreResult<()> {
        if transaction.await_source_finalization {
            // A terminal tombstone is authoritative even if a crash left the
            // old payload directory behind. Never replay link mutations into
            // a tombstoned target; Ready/Discarded reconciliation owns the
            // remaining source decision.
            if self
                .load_tombstone(&transaction.link.source_recovery_id)?
                .is_some()
                || self
                    .load_tombstone(&transaction.link.target_recovery_id)?
                    .is_some()
            {
                return Ok(());
            }
            let source_exists = self
                .load_unlocked(&transaction.link.source_recovery_id)?
                .record
                .is_some();
            let target_exists = self
                .load_unlocked(&transaction.link.target_recovery_id)?
                .record
                .is_some();
            if !source_exists || !target_exists {
                return Ok(());
            }
        }
        self.apply_continuation_transaction_locked(transaction)
            .map(|_| ())
    }

    /// Apply both sides while both recovery locks and the coordinator lock are
    /// held. Per-record operation ids make every replay idempotent.
    fn apply_continuation_transaction_locked(
        &self,
        transaction: &ContinuationTransactionBody,
    ) -> RecoveryStoreResult<(RecoveryRecord, RecoveryRecord)> {
        let source = self.persist_mutation_unlocked(
            &transaction.link.source_recovery_id,
            format!("{}:source", transaction.operation_id),
            transaction.link.linked_at,
            RecoveryMutation::LinkContinuation {
                link: transaction.link.clone(),
                role: RecoveryContinuationRole::Source,
                inherited_state: transaction.inherited_state.clone(),
            },
        )?;
        self.inject_fault(RecoveryStoreFaultPoint::AfterContinuationSourcePublication)?;
        let target = self.persist_mutation_unlocked(
            &transaction.link.target_recovery_id,
            format!("{}:target", transaction.operation_id),
            transaction.link.linked_at,
            RecoveryMutation::LinkContinuation {
                link: transaction.link.clone(),
                role: RecoveryContinuationRole::Target,
                inherited_state: transaction.inherited_state.clone(),
            },
        )?;
        self.inject_fault(RecoveryStoreFaultPoint::AfterContinuationTargetPublication)?;
        if !transaction.await_source_finalization {
            self.remove_continuation_transaction_unlocked(transaction)?;
            self.inject_fault(RecoveryStoreFaultPoint::AfterContinuationIntentCleanup)?;
        }
        Ok((source, target))
    }

    fn write_continuation_transaction_unlocked(
        &self,
        transaction: &ContinuationTransactionBody,
    ) -> RecoveryStoreResult<()> {
        let path = self.continuation_transaction_path(transaction)?;
        if path.is_file() {
            let stored = self.read_continuation_transaction(&path)?;
            if stored.body == *transaction {
                return Ok(());
            }
            return Err(RecoveryStoreError::OperationConflict {
                operation_id: transaction.operation_id.clone(),
            });
        }
        let stored = StoredContinuationTransaction {
            body: transaction.clone(),
            checksum: checksum_json(transaction)?,
        };
        ensure_serialized_limit_with(
            "recovery_continuation_transaction",
            &stored,
            MAX_RECOVERY_TRANSACTION_BYTES,
        )?;
        let existing = bounded_json_paths(
            &self.continuations_dir(),
            "recovery_continuation_transactions",
            MAX_RECOVERY_TRANSACTION_FILES,
            MAX_RECOVERY_TRANSACTION_BYTES,
            MAX_RECOVERY_TRANSACTION_TOTAL_BYTES,
        )?;
        if existing.len() >= MAX_RECOVERY_TRANSACTION_FILES {
            return Err(content_limit_error(
                "recovery_continuation_transactions",
                MAX_RECOVERY_TRANSACTION_FILES,
                "files",
            ));
        }
        let filename = path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid intent path"))?;
        write_unique_json(&self.continuations_dir(), filename, &stored)
    }

    fn read_continuation_transactions_unlocked(
        &self,
    ) -> RecoveryStoreResult<Vec<ContinuationTransactionBody>> {
        self.ensure_root_dirs()?;
        let paths = bounded_json_paths(
            &self.continuations_dir(),
            "recovery_continuation_transactions",
            MAX_RECOVERY_TRANSACTION_FILES,
            MAX_RECOVERY_TRANSACTION_BYTES,
            MAX_RECOVERY_TRANSACTION_TOTAL_BYTES,
        )?;
        let mut transactions = Vec::with_capacity(paths.len());
        let mut read_budget = RecoveryLedgerReadBudget::new(
            "recovery_continuation_transactions",
            MAX_RECOVERY_TRANSACTION_TOTAL_BYTES,
        );
        for path in paths {
            let bytes = read_budget.read(&path, MAX_RECOVERY_TRANSACTION_BYTES, None)?;
            let stored = decode_continuation_transaction(&bytes)?;
            validate_recovery_id(&stored.body.link.source_recovery_id)?;
            validate_recovery_id(&stored.body.link.target_recovery_id)?;
            if stored.body.schema_version != STORE_SCHEMA_VERSION
                || stored.body.link.source_recovery_id == stored.body.link.target_recovery_id
                || stored.body.operation_id.trim().is_empty()
                || self.continuation_transaction_path(&stored.body)? != path
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid continuation transaction intent",
                )
                .into());
            }
            ensure_text_limit(
                "operation_id",
                &stored.body.operation_id,
                MAX_RECOVERY_IDENTIFIER_CHARS,
            )?;
            ensure_text_limit(
                "operation_id",
                &format!("{}:source", stored.body.operation_id),
                MAX_RECOVERY_IDENTIFIER_CHARS,
            )?;
            ensure_text_limit(
                "operation_id",
                &format!("{}:target", stored.body.operation_id),
                MAX_RECOVERY_IDENTIFIER_CHARS,
            )?;
            transactions.push(stored.body);
        }
        Ok(transactions)
    }

    fn read_continuation_transaction(
        &self,
        path: &Path,
    ) -> RecoveryStoreResult<StoredContinuationTransaction> {
        let bytes = read_bounded_store_file(
            path,
            "recovery_continuation_transaction",
            MAX_RECOVERY_TRANSACTION_BYTES,
        )?;
        decode_continuation_transaction(&bytes)
    }

    fn remove_continuation_transaction_unlocked(
        &self,
        transaction: &ContinuationTransactionBody,
    ) -> RecoveryStoreResult<()> {
        let path = self.continuation_transaction_path(transaction)?;
        if path.exists() {
            fs::remove_file(path)?;
            sync_directory(&self.continuations_dir())?;
        }
        Ok(())
    }

    fn continuation_transaction_path(
        &self,
        transaction: &ContinuationTransactionBody,
    ) -> RecoveryStoreResult<PathBuf> {
        let identity = (
            transaction.operation_id.as_str(),
            transaction.link.source_recovery_id.as_str(),
            transaction.link.target_recovery_id.as_str(),
        );
        Ok(self
            .continuations_dir()
            .join(format!("{}.json", checksum_json(&identity)?)))
    }

    fn acquire_provider_root_claim_unlocked(
        &self,
        provider_root_hash: &str,
        claim: ProviderRootClaim,
    ) -> RecoveryStoreResult<()> {
        if claim.provider_root_hash != provider_root_hash {
            return Err(RecoveryStoreError::InvalidLease(
                "provider root claim hash mismatch".to_string(),
            ));
        }
        if let Some(existing) = self.load_provider_root_claim_unlocked(provider_root_hash)? {
            if existing.claim_token == claim.claim_token {
                if existing == claim {
                    return Ok(());
                }
                return Err(RecoveryStoreError::InvalidLease(
                    "provider root claim token was reused with different content".to_string(),
                ));
            }
            let durable_live_owner = existing.holder_recovery_id != claim.holder_recovery_id
                && self
                    .provider_root_claim_holder_is_live_unlocked(provider_root_hash, &existing)?;
            if existing.expires_at > claim.acquired_at || durable_live_owner {
                return Err(RecoveryStoreError::ProviderRootClaimConflict {
                    holder_recovery_id: existing.holder_recovery_id,
                    holder_session_id: existing.holder_session_id,
                    holder_window_id: existing.holder_window_id,
                });
            }
        }
        self.write_provider_root_claim_state_unlocked(
            provider_root_hash,
            claim.acquired_at,
            Some(claim),
            None,
        )
    }

    fn provider_root_claim_holder_is_live_unlocked(
        &self,
        provider_root_hash: &str,
        claim: &ProviderRootClaim,
    ) -> RecoveryStoreResult<bool> {
        // Provider claim publication and recovery events/snapshots are
        // immutable atomic files. Do not take a second recovery lock while a
        // claim caller holds its current record lock: continuation pairs use
        // lexical lock ordering and a B -> A acquisition here could deadlock
        // with their A -> B order. Any concurrent compaction/read failure is
        // returned and therefore fails the launch closed.
        let Some(record) = self.load_unlocked(&claim.holder_recovery_id)?.record else {
            return Ok(false);
        };
        let still_same_root = record.provider_root.as_ref().is_some_and(|root| {
            root.quality.is_authoritative()
                && provider_root_claim_hash(&record.provider, &root.root_id) == provider_root_hash
        });
        let terminal = record.launch_stage.is_terminal()
            || matches!(
                record.lifecycle,
                RecoveryLifecycle::Resolved | RecoveryLifecycle::Discarded
            );
        Ok(still_same_root
            && !terminal
            && (record.launch_stage >= RecoveryLaunchStage::Ready
                || record.lifecycle == RecoveryLifecycle::Running))
    }

    fn find_live_provider_root_owner_unlocked(
        &self,
        provider_root_hash: &str,
        excluded_recovery_id: Option<&str>,
    ) -> RecoveryStoreResult<Option<RecoveryRecord>> {
        self.ensure_root_dirs()?;
        let mut scan_budget = active_recovery_scan_budget().unwrap_or_default();
        for recovery_id in self.recovery_ids_unlocked()? {
            if excluded_recovery_id == Some(recovery_id.as_str()) {
                continue;
            }
            let record = self
                .load_unlocked_with_scan(&recovery_id, Some(&mut scan_budget))?
                .record;
            let Some(record) = record else {
                continue;
            };
            let terminal = record.launch_stage.is_terminal()
                || matches!(
                    record.lifecycle,
                    RecoveryLifecycle::Resolved | RecoveryLifecycle::Discarded
                );
            let live = record.launch_stage >= RecoveryLaunchStage::Ready
                || record.lifecycle == RecoveryLifecycle::Running;
            let same_root = record.provider_root.as_ref().is_some_and(|root| {
                root.quality.is_authoritative()
                    && provider_root_claim_hash(&record.provider, &root.root_id)
                        == provider_root_hash
            });
            if !terminal && live && same_root {
                return Ok(Some(record));
            }
        }
        Ok(None)
    }

    fn recovery_ids_unlocked(&self) -> RecoveryStoreResult<Vec<String>> {
        self.recovery_ids_unlocked_with_limit(MAX_RECOVERY_DIRECTORY_ENTRIES)
    }

    fn recovery_ids_unlocked_with_limit(
        &self,
        max_entries: usize,
    ) -> RecoveryStoreResult<Vec<String>> {
        let mut entry_count = 0_usize;
        let mut recovery_ids = Vec::new();
        for entry in fs::read_dir(self.recoveries_dir())? {
            let entry = entry?;
            entry_count = entry_count.saturating_add(1);
            if entry_count > max_entries {
                return Err(RecoveryStoreError::ContentLimitExceeded {
                    field: "recovery_directory_entries".to_string(),
                    limit: max_entries,
                    unit: "entries",
                });
            }
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let recovery_id = entry.file_name().to_string_lossy().into_owned();
            if validate_recovery_id(&recovery_id).is_ok() {
                recovery_ids.push(recovery_id);
            }
        }
        recovery_ids.sort();
        Ok(recovery_ids)
    }

    fn release_provider_root_claim_unlocked(
        &self,
        provider_root_hash: &str,
        claim_token: &str,
        released_at: DateTime<Utc>,
    ) -> RecoveryStoreResult<bool> {
        let Some(existing) = self.load_provider_root_claim_unlocked(provider_root_hash)? else {
            return Ok(false);
        };
        if existing.claim_token != claim_token {
            return Ok(false);
        }
        self.write_provider_root_claim_state_unlocked(
            provider_root_hash,
            released_at,
            None,
            Some(claim_token.to_string()),
        )?;
        Ok(true)
    }

    fn load_provider_root_claim_unlocked(
        &self,
        provider_root_hash: &str,
    ) -> RecoveryStoreResult<Option<ProviderRootClaim>> {
        let dir = self.provider_root_claim_events_dir(provider_root_hash);
        if !dir.is_dir() {
            return Ok(None);
        }
        let mut latest: Option<ProviderRootClaimEventBody> = None;
        let mut read_budget = RecoveryLedgerReadBudget::new(
            "provider_root_claim_events",
            MAX_PROVIDER_ROOT_CLAIM_EVENT_TOTAL_BYTES,
        );
        for path in bounded_json_paths(
            &dir,
            "provider_root_claim_events",
            MAX_PROVIDER_ROOT_CLAIM_EVENT_FILES,
            MAX_PROVIDER_ROOT_CLAIM_EVENT_BYTES,
            MAX_PROVIDER_ROOT_CLAIM_EVENT_TOTAL_BYTES,
        )? {
            let bytes = read_budget.read(&path, MAX_PROVIDER_ROOT_CLAIM_EVENT_BYTES, None)?;
            let stored: StoredProviderRootClaimEvent = serde_json::from_slice(&bytes)?;
            if stored.body.schema_version != STORE_SCHEMA_VERSION
                || stored.body.provider_root_hash != provider_root_hash
                || checksum_json(&stored.body)? != stored.checksum
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid provider root claim event",
                )
                .into());
            }
            if stored.body.active_claim.is_some() == stored.body.released_claim_token.is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid provider root claim state",
                )
                .into());
            }
            if let Some(claim) = stored.body.active_claim.as_ref() {
                if claim.provider_root_hash != provider_root_hash {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "provider root claim identity mismatch",
                    )
                    .into());
                }
                validate_provider_root_claim_expiry(claim.acquired_at, claim.expires_at)?;
            }
            match latest.as_ref() {
                Some(current) if current.generation == stored.body.generation => {
                    if current != &stored.body {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "conflicting provider root claim generation",
                        )
                        .into());
                    }
                }
                Some(current) if current.generation > stored.body.generation => {}
                _ => latest = Some(stored.body),
            }
        }
        Ok(latest.and_then(|event| event.active_claim))
    }

    fn write_provider_root_claim_state_unlocked(
        &self,
        provider_root_hash: &str,
        recorded_at: DateTime<Utc>,
        active_claim: Option<ProviderRootClaim>,
        released_claim_token: Option<String>,
    ) -> RecoveryStoreResult<()> {
        let dir = self.provider_root_claim_events_dir(provider_root_hash);
        create_private_dir_all(&dir)?;
        let generation = self
            .latest_provider_root_claim_generation_unlocked(provider_root_hash)?
            .saturating_add(1);
        let body = ProviderRootClaimEventBody {
            schema_version: STORE_SCHEMA_VERSION,
            provider_root_hash: provider_root_hash.to_string(),
            generation,
            recorded_at,
            active_claim,
            released_claim_token,
        };
        let stored = StoredProviderRootClaimEvent {
            checksum: checksum_json(&body)?,
            body,
        };
        ensure_serialized_limit_with(
            "provider_root_claim_event",
            &stored,
            MAX_PROVIDER_ROOT_CLAIM_EVENT_BYTES,
        )?;
        let existing = bounded_json_paths(
            &dir,
            "provider_root_claim_events",
            MAX_PROVIDER_ROOT_CLAIM_EVENT_FILES,
            MAX_PROVIDER_ROOT_CLAIM_EVENT_BYTES,
            MAX_PROVIDER_ROOT_CLAIM_EVENT_TOTAL_BYTES,
        )?;
        if existing.len() >= MAX_PROVIDER_ROOT_CLAIM_EVENT_FILES {
            return Err(content_limit_error(
                "provider_root_claim_events",
                MAX_PROVIDER_ROOT_CLAIM_EVENT_FILES,
                "files",
            ));
        }
        write_unique_json(
            &dir,
            &format!("{generation:020}-{}.json", Uuid::new_v4().simple()),
            &stored,
        )?;
        self.inject_fault(RecoveryStoreFaultPoint::AfterProviderRootClaimPublication)?;
        self.prune_provider_root_claim_events_unlocked(provider_root_hash)
    }

    fn latest_provider_root_claim_generation_unlocked(
        &self,
        provider_root_hash: &str,
    ) -> RecoveryStoreResult<u64> {
        let dir = self.provider_root_claim_events_dir(provider_root_hash);
        if !dir.is_dir() {
            return Ok(0);
        }
        let mut latest = 0;
        for path in bounded_json_paths(
            &dir,
            "provider_root_claim_events",
            MAX_PROVIDER_ROOT_CLAIM_EVENT_FILES,
            MAX_PROVIDER_ROOT_CLAIM_EVENT_BYTES,
            MAX_PROVIDER_ROOT_CLAIM_EVENT_TOTAL_BYTES,
        )? {
            let generation = path
                .file_name()
                .and_then(|value| value.to_str())
                .and_then(|value| value.split_once('-'))
                .and_then(|(generation, _)| generation.parse::<u64>().ok());
            latest = latest.max(generation.unwrap_or(0));
        }
        Ok(latest)
    }

    fn prune_provider_root_claim_events_unlocked(
        &self,
        provider_root_hash: &str,
    ) -> RecoveryStoreResult<()> {
        let dir = self.provider_root_claim_events_dir(provider_root_hash);
        let events = bounded_json_paths(
            &dir,
            "provider_root_claim_events",
            MAX_PROVIDER_ROOT_CLAIM_EVENT_FILES,
            MAX_PROVIDER_ROOT_CLAIM_EVENT_BYTES,
            MAX_PROVIDER_ROOT_CLAIM_EVENT_TOTAL_BYTES,
        )?;
        let remove_count = events
            .len()
            .saturating_sub(PROVIDER_ROOT_CLAIM_EVENTS_TO_KEEP);
        for path in events.into_iter().take(remove_count) {
            fs::remove_file(path)?;
        }
        if remove_count > 0 {
            sync_directory(&dir)?;
        }
        Ok(())
    }

    fn persist_mutation(
        &self,
        recovery_id: &str,
        operation_id: String,
        recorded_at: DateTime<Utc>,
        mutation: RecoveryMutation,
    ) -> RecoveryStoreResult<RecoveryRecord> {
        validate_recovery_id(recovery_id)?;
        if operation_id.trim().is_empty() {
            return Err(RecoveryStoreError::OperationConflict { operation_id });
        }
        ensure_text_limit("operation_id", &operation_id, MAX_RECOVERY_IDENTIFIER_CHARS)?;
        self.repair_pending_continuations_for(recovery_id)?;
        self.with_recovery_lock(recovery_id, || {
            self.persist_mutation_unlocked(recovery_id, operation_id, recorded_at, mutation)
        })
    }

    /// Persist while the caller holds this recovery's exclusive lock.
    fn persist_mutation_unlocked(
        &self,
        recovery_id: &str,
        operation_id: String,
        recorded_at: DateTime<Utc>,
        mutation: RecoveryMutation,
    ) -> RecoveryStoreResult<RecoveryRecord> {
        // A retained tombstone is a terminal identity fence, not merely an
        // inventory hint. In particular, a discarded prepared successor must
        // never be resurrected by replaying its original Create operation.
        if self.load_tombstone(recovery_id)?.is_some() {
            return Err(RecoveryStoreError::TombstoneConflict(
                recovery_id.to_string(),
            ));
        }
        let mut state = self.load_unlocked(recovery_id)?;
        // Inline ledgers from older snapshots, plus events replayed after a
        // crash, are converted before accepting another mutation. The old
        // snapshot remains the source of truth until every receipt and the
        // clean replacement snapshot have been durably published.
        self.migrate_operation_receipts_unlocked(recovery_id, &mut state)?;
        let mutation_digest = checksum_json(&mutation)?;
        if let Some(committed) =
            self.committed_operation_unlocked(recovery_id, &state, &operation_id)?
        {
            if committed.mutation_digest == mutation_digest {
                return state
                    .record
                    .ok_or_else(|| RecoveryStoreError::NotFound(recovery_id.to_string()));
            }
            return Err(RecoveryStoreError::OperationConflict { operation_id });
        }

        let generation = state
            .record
            .as_ref()
            .map_or(1, |record| record.generation.saturating_add(1));
        apply_mutation(
            &mut state.record,
            recovery_id,
            generation,
            recorded_at,
            &mutation,
        )?;
        ensure_serialized_limit_with(
            "recovery_record",
            state
                .record
                .as_ref()
                .ok_or_else(|| RecoveryStoreError::NotFound(recovery_id.to_string()))?,
            MAX_RECOVERY_RECORD_BYTES,
        )?;
        let event_body = EventBody {
            schema_version: STORE_SCHEMA_VERSION,
            recovery_id: recovery_id.to_string(),
            generation,
            operation_id: operation_id.clone(),
            recorded_at,
            mutation_digest: mutation_digest.clone(),
            mutation,
        };
        ensure_serialized_limit_with("recovery_event", &event_body, MAX_RECOVERY_EVENT_BYTES)?;
        let snapshot_size_probe = SnapshotBody {
            schema_version: STORE_SCHEMA_VERSION,
            recovery_id: recovery_id.to_string(),
            generation,
            record: state
                .record
                .clone()
                .ok_or_else(|| RecoveryStoreError::NotFound(recovery_id.to_string()))?,
            operation_digests: BTreeMap::new(),
        };
        ensure_serialized_limit_with(
            "recovery_snapshot",
            &snapshot_size_probe,
            MAX_RECOVERY_SNAPSHOT_BYTES,
        )?;
        self.write_event(recovery_id, &event_body)?;
        self.inject_fault(RecoveryStoreFaultPoint::AfterEventPublication)?;
        self.write_operation_receipt(
            recovery_id,
            &OperationReceiptBody {
                schema_version: STORE_SCHEMA_VERSION,
                recovery_id: recovery_id.to_string(),
                operation_id,
                generation,
                mutation_digest,
            },
        )?;
        self.inject_fault(RecoveryStoreFaultPoint::AfterOperationReceiptPublication)?;
        self.write_snapshot(recovery_id, &state)?;
        self.inject_fault(RecoveryStoreFaultPoint::AfterSnapshotPublication)?;
        self.prune_old_snapshots(recovery_id)?;
        self.prune_events_covered_by_retained_snapshot(recovery_id)?;
        state
            .record
            .ok_or_else(|| RecoveryStoreError::NotFound(recovery_id.to_string()))
    }

    fn load_unlocked(&self, recovery_id: &str) -> RecoveryStoreResult<LoadedState> {
        self.load_unlocked_with_scan(recovery_id, None)
    }

    fn load_unlocked_with_scan(
        &self,
        recovery_id: &str,
        mut scan_budget: Option<&mut RecoveryGlobalScanBudget>,
    ) -> RecoveryStoreResult<LoadedState> {
        let recovery_dir = self.recovery_dir(recovery_id);
        if !recovery_dir.is_dir() {
            return Ok(LoadedState::default());
        }

        // Receipts are addressed by operation digest rather than enumerated
        // during replay. Validate their inventory once per load so a hostile
        // directory cannot evade the same count and aggregate-byte bounds as
        // the event and snapshot ledgers.
        bounded_json_paths_with_scan(
            &self.operations_dir(recovery_id),
            "recovery_operation_receipts",
            MAX_RECOVERY_RECEIPT_FILES,
            MAX_RECOVERY_RECEIPT_BYTES,
            MAX_RECOVERY_RECEIPT_TOTAL_BYTES,
            scan_budget.as_deref_mut(),
        )?;

        let mut state =
            self.load_highest_valid_snapshot_with_scan(recovery_id, scan_budget.as_deref_mut())?;
        let mut receipt_read_budget = RecoveryLedgerReadBudget::new(
            "recovery_operation_receipts",
            MAX_RECOVERY_RECEIPT_TOTAL_BYTES,
        );
        let mut events = self.load_valid_events_after_with_scan(
            recovery_id,
            state.record.as_ref().map_or(0, |r| r.generation),
            scan_budget.as_deref_mut(),
        )?;
        events.sort_by_key(|event| event.body.generation);
        for event in events {
            if event.body.recovery_id != recovery_id {
                continue;
            }
            if let Some(existing) = state.operation_digests.get(&event.body.operation_id) {
                if existing != &event.body.mutation_digest {
                    return Err(RecoveryStoreError::OperationConflict {
                        operation_id: event.body.operation_id,
                    });
                }
                if state
                    .operation_generations
                    .get(&event.body.operation_id)
                    .is_some_and(|generation| *generation != event.body.generation)
                {
                    return Err(RecoveryStoreError::GenerationConflict {
                        recovery_id: recovery_id.to_string(),
                    });
                }
                continue;
            }
            if let Some(receipt) = self.read_operation_receipt_with_scan(
                recovery_id,
                &event.body.operation_id,
                scan_budget.as_deref_mut(),
                Some(&mut receipt_read_budget),
            )? {
                if receipt.mutation_digest != event.body.mutation_digest
                    || receipt.generation != event.body.generation
                {
                    return Err(invalid_recovery_data(
                        "operation receipt does not match its recovery event",
                    ));
                }
            }
            let expected_generation = state
                .record
                .as_ref()
                .map_or(1, |record| record.generation.saturating_add(1));
            if event.body.generation != expected_generation {
                return Err(RecoveryStoreError::GenerationConflict {
                    recovery_id: recovery_id.to_string(),
                });
            }
            apply_mutation(
                &mut state.record,
                recovery_id,
                event.body.generation,
                event.body.recorded_at,
                &event.body.mutation,
            )?;
            state
                .operation_digests
                .insert(event.body.operation_id.clone(), event.body.mutation_digest);
            state
                .operation_generations
                .insert(event.body.operation_id, event.body.generation);
        }
        Ok(state)
    }

    fn load_highest_valid_snapshot_with_scan(
        &self,
        recovery_id: &str,
        mut scan_budget: Option<&mut RecoveryGlobalScanBudget>,
    ) -> RecoveryStoreResult<LoadedState> {
        let dir = self.snapshots_dir(recovery_id);
        if !dir.is_dir() {
            return Ok(LoadedState::default());
        }
        let mut best: Option<StoredSnapshot> = None;
        let mut read_budget =
            RecoveryLedgerReadBudget::new("recovery_snapshots", MAX_RECOVERY_SNAPSHOT_TOTAL_BYTES);
        for path in bounded_json_paths_with_scan(
            &dir,
            "recovery_snapshots",
            MAX_RECOVERY_SNAPSHOT_FILES,
            MAX_RECOVERY_SNAPSHOT_BYTES,
            MAX_RECOVERY_SNAPSHOT_TOTAL_BYTES,
            scan_budget.as_deref_mut(),
        )? {
            let bytes = read_budget.read(
                &path,
                MAX_RECOVERY_SNAPSHOT_BYTES,
                scan_budget.as_deref_mut(),
            )?;
            let Ok(stored) = serde_json::from_slice::<StoredSnapshot>(&bytes) else {
                continue;
            };
            if stored.body.recovery_id != recovery_id
                || checksum_json(&stored.body).ok().as_deref() != Some(&stored.checksum)
            {
                continue;
            }
            if best.as_ref().is_none_or(|current| {
                stored.body.generation > current.body.generation
                    || (stored.body.generation == current.body.generation
                        && stored.body.operation_digests.is_empty()
                        && !current.body.operation_digests.is_empty())
            }) {
                best = Some(stored);
            }
        }
        Ok(best.map_or_else(LoadedState::default, |stored| {
            let generation = stored.body.generation;
            let mut record = stored.body.record;
            normalize_recovery_record(&mut record);
            let operation_generations = stored
                .body
                .operation_digests
                .keys()
                .map(|operation_id| (operation_id.clone(), generation))
                .collect();
            LoadedState {
                record: Some(record),
                operation_digests: stored.body.operation_digests,
                operation_generations,
            }
        }))
    }

    /// Resolve one operation through the bounded replay/legacy state and the
    /// immutable receipt ledger. If both sources exist they must agree
    /// exactly; disagreement is durable corruption and is never guessed away.
    fn committed_operation_unlocked(
        &self,
        recovery_id: &str,
        state: &LoadedState,
        operation_id: &str,
    ) -> RecoveryStoreResult<Option<CommittedOperation>> {
        let transient = state.operation_digests.get(operation_id).map(|digest| {
            let generation = state
                .operation_generations
                .get(operation_id)
                .copied()
                .or_else(|| state.record.as_ref().map(|record| record.generation))
                .unwrap_or_default();
            CommittedOperation {
                generation,
                mutation_digest: digest.clone(),
            }
        });
        let receipt = self
            .read_operation_receipt(recovery_id, operation_id)?
            .map(|receipt| CommittedOperation {
                generation: receipt.generation,
                mutation_digest: receipt.mutation_digest,
            });

        if let (Some(transient), Some(receipt)) = (&transient, &receipt) {
            if transient != receipt {
                return Err(invalid_recovery_data(
                    "operation receipt disagrees with replayed recovery history",
                ));
            }
        }
        let committed = receipt.or(transient);
        if let Some(committed) = committed.as_ref() {
            let Some(record) = state.record.as_ref() else {
                return Err(invalid_recovery_data(
                    "operation receipt exists without a recovery record",
                ));
            };
            if committed.generation == 0 || committed.generation > record.generation {
                return Err(invalid_recovery_data(
                    "operation receipt generation exceeds recovery history",
                ));
            }
        }
        Ok(committed)
    }

    /// Convert the legacy inline map (or a post-crash event replay) into
    /// immutable receipts. A clean same-generation snapshot is published only
    /// after every receipt is durable, so interruption leaves a replayable SOT.
    fn migrate_operation_receipts_unlocked(
        &self,
        recovery_id: &str,
        state: &mut LoadedState,
    ) -> RecoveryStoreResult<()> {
        if state.operation_digests.is_empty() {
            return Ok(());
        }
        let record = state
            .record
            .as_ref()
            .ok_or_else(|| RecoveryStoreError::NotFound(recovery_id.to_string()))?;
        for (operation_id, mutation_digest) in &state.operation_digests {
            let generation = state
                .operation_generations
                .get(operation_id)
                .copied()
                .unwrap_or(record.generation);
            self.write_operation_receipt(
                recovery_id,
                &OperationReceiptBody {
                    schema_version: STORE_SCHEMA_VERSION,
                    recovery_id: recovery_id.to_string(),
                    operation_id: operation_id.clone(),
                    generation,
                    mutation_digest: mutation_digest.clone(),
                },
            )?;
            self.inject_fault(RecoveryStoreFaultPoint::AfterOperationReceiptPublication)?;
        }

        state.operation_digests.clear();
        state.operation_generations.clear();
        self.write_snapshot(recovery_id, state)?;
        self.prune_old_snapshots(recovery_id)?;
        self.prune_events_covered_by_retained_snapshot(recovery_id)?;
        Ok(())
    }

    fn read_operation_receipt(
        &self,
        recovery_id: &str,
        operation_id: &str,
    ) -> RecoveryStoreResult<Option<OperationReceiptBody>> {
        self.read_operation_receipt_with_scan(recovery_id, operation_id, None, None)
    }

    fn read_operation_receipt_with_scan(
        &self,
        recovery_id: &str,
        operation_id: &str,
        scan_budget: Option<&mut RecoveryGlobalScanBudget>,
        read_budget: Option<&mut RecoveryLedgerReadBudget>,
    ) -> RecoveryStoreResult<Option<OperationReceiptBody>> {
        let path = self.operation_receipt_path(recovery_id, operation_id);
        if !path.is_file() {
            return Ok(None);
        }
        let bytes = if let Some(read_budget) = read_budget {
            read_budget.read(&path, MAX_RECOVERY_RECEIPT_BYTES, scan_budget)?
        } else {
            read_bounded_store_file_with_scan(
                &path,
                "recovery_operation_receipt",
                MAX_RECOVERY_RECEIPT_BYTES,
                scan_budget,
            )?
        };
        let stored: StoredOperationReceipt = serde_json::from_slice(&bytes)?;
        if stored.body.schema_version != STORE_SCHEMA_VERSION
            || stored.body.recovery_id != recovery_id
            || stored.body.operation_id != operation_id
            || stored.body.generation == 0
            || stored.body.mutation_digest.len() != 64
            || !stored
                .body
                .mutation_digest
                .bytes()
                .all(|value| value.is_ascii_hexdigit())
            || checksum_json(&stored.body)? != stored.checksum
        {
            return Err(invalid_recovery_data("invalid operation receipt"));
        }
        Ok(Some(stored.body))
    }

    fn write_operation_receipt(
        &self,
        recovery_id: &str,
        body: &OperationReceiptBody,
    ) -> RecoveryStoreResult<()> {
        if body.schema_version != STORE_SCHEMA_VERSION
            || body.recovery_id != recovery_id
            || body.generation == 0
        {
            return Err(invalid_recovery_data("invalid operation receipt body"));
        }
        let dir = self.operations_dir(recovery_id);
        create_private_dir_all(&dir)?;
        if let Some(existing) = self.read_operation_receipt(recovery_id, &body.operation_id)? {
            if existing == *body {
                return Ok(());
            }
            if existing.mutation_digest != body.mutation_digest {
                return Err(RecoveryStoreError::OperationConflict {
                    operation_id: body.operation_id.clone(),
                });
            }
            return Err(invalid_recovery_data(
                "operation receipt generation cannot be replaced",
            ));
        }
        let stored = StoredOperationReceipt {
            checksum: checksum_json(body)?,
            body: body.clone(),
        };
        ensure_serialized_limit_with(
            "recovery_operation_receipt",
            &stored,
            MAX_RECOVERY_RECEIPT_BYTES,
        )?;
        let receipts = bounded_json_paths(
            &dir,
            "recovery_operation_receipts",
            MAX_RECOVERY_RECEIPT_FILES,
            MAX_RECOVERY_RECEIPT_BYTES,
            MAX_RECOVERY_RECEIPT_TOTAL_BYTES,
        )?;
        if receipts.len() >= MAX_RECOVERY_RECEIPT_FILES {
            return Err(content_limit_error(
                "recovery_operation_receipts",
                MAX_RECOVERY_RECEIPT_FILES,
                "files",
            ));
        }
        let filename = operation_receipt_filename(&body.operation_id);
        write_unique_json(&dir, &filename, &stored)?;
        let Some(reloaded) = self.read_operation_receipt(recovery_id, &body.operation_id)? else {
            return Err(invalid_recovery_data(
                "operation receipt publication was not durable",
            ));
        };
        if reloaded != *body {
            return Err(invalid_recovery_data(
                "operation receipt verification failed",
            ));
        }
        Ok(())
    }

    fn load_valid_events_after_with_scan(
        &self,
        recovery_id: &str,
        generation: u64,
        mut scan_budget: Option<&mut RecoveryGlobalScanBudget>,
    ) -> RecoveryStoreResult<Vec<StoredEvent>> {
        let dir = self.events_dir(recovery_id);
        if !dir.is_dir() {
            return Ok(Vec::new());
        }
        let mut events = Vec::new();
        let mut read_budget =
            RecoveryLedgerReadBudget::new("recovery_events", MAX_RECOVERY_EVENT_TOTAL_BYTES);
        for path in bounded_json_paths_with_scan(
            &dir,
            "recovery_events",
            MAX_RECOVERY_EVENT_FILES,
            MAX_RECOVERY_EVENT_BYTES,
            MAX_RECOVERY_EVENT_TOTAL_BYTES,
            scan_budget.as_deref_mut(),
        )? {
            let bytes =
                read_budget.read(&path, MAX_RECOVERY_EVENT_BYTES, scan_budget.as_deref_mut())?;
            let stored: StoredEvent = serde_json::from_slice(&bytes)?;
            if stored.body.generation <= generation {
                continue;
            }
            if checksum_json(&stored.body)? != stored.checksum
                || checksum_json(&stored.body.mutation)? != stored.body.mutation_digest
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid recovery event checksum",
                )
                .into());
            }
            events.push(stored);
        }
        Ok(events)
    }

    fn write_event(&self, recovery_id: &str, body: &EventBody) -> RecoveryStoreResult<()> {
        let dir = self.events_dir(recovery_id);
        create_private_dir_all(&dir)?;
        let stored = StoredEvent {
            checksum: checksum_json(body)?,
            body: body.clone(),
        };
        ensure_serialized_limit_with("recovery_event", &stored, MAX_RECOVERY_EVENT_BYTES)?;
        let events = bounded_json_paths(
            &dir,
            "recovery_events",
            MAX_RECOVERY_EVENT_FILES,
            MAX_RECOVERY_EVENT_BYTES,
            MAX_RECOVERY_EVENT_TOTAL_BYTES,
        )?;
        if events.len() >= MAX_RECOVERY_EVENT_FILES {
            return Err(content_limit_error(
                "recovery_events",
                MAX_RECOVERY_EVENT_FILES,
                "files",
            ));
        }
        let filename = format!("{:020}-{}.json", body.generation, Uuid::new_v4().simple());
        write_unique_json(&dir, &filename, &stored)
    }

    fn write_snapshot(&self, recovery_id: &str, state: &LoadedState) -> RecoveryStoreResult<()> {
        let record = state
            .record
            .as_ref()
            .ok_or_else(|| RecoveryStoreError::NotFound(recovery_id.to_string()))?;
        let dir = self.snapshots_dir(recovery_id);
        create_private_dir_all(&dir)?;
        let body = SnapshotBody {
            schema_version: STORE_SCHEMA_VERSION,
            recovery_id: recovery_id.to_string(),
            generation: record.generation,
            record: record.clone(),
            operation_digests: BTreeMap::new(),
        };
        ensure_serialized_limit_with("recovery_snapshot", &body, MAX_RECOVERY_SNAPSHOT_BYTES)?;
        let stored = StoredSnapshot {
            checksum: checksum_json(&body)?,
            body,
        };
        ensure_serialized_limit_with("recovery_snapshot", &stored, MAX_RECOVERY_SNAPSHOT_BYTES)?;
        let snapshots = bounded_json_paths(
            &dir,
            "recovery_snapshots",
            MAX_RECOVERY_SNAPSHOT_FILES,
            MAX_RECOVERY_SNAPSHOT_BYTES,
            MAX_RECOVERY_SNAPSHOT_TOTAL_BYTES,
        )?;
        if snapshots.len() >= MAX_RECOVERY_SNAPSHOT_FILES {
            return Err(content_limit_error(
                "recovery_snapshots",
                MAX_RECOVERY_SNAPSHOT_FILES,
                "files",
            ));
        }
        let filename = format!("{:020}-{}.json", record.generation, Uuid::new_v4().simple());
        write_unique_json(&dir, &filename, &stored)?;

        let bytes = read_bounded_store_file(
            &dir.join(filename),
            "recovery_snapshot",
            MAX_RECOVERY_SNAPSHOT_BYTES,
        )?;
        let reloaded: StoredSnapshot = serde_json::from_slice(&bytes)?;
        if reloaded.checksum != checksum_json(&reloaded.body)? {
            return Err(
                io::Error::new(io::ErrorKind::InvalidData, "snapshot verification failed").into(),
            );
        }
        Ok(())
    }

    fn write_tombstone(&self, tombstone: &RecoveryTombstone) -> RecoveryStoreResult<()> {
        let dir = self.tombstones_dir();
        create_private_dir_all(&dir)?;
        let stored = StoredTombstone {
            checksum: checksum_json(tombstone)?,
            body: tombstone.clone(),
        };
        ensure_serialized_limit_with("recovery_tombstone", &stored, MAX_RECOVERY_TOMBSTONE_BYTES)?;
        let tombstones = bounded_json_paths(
            &dir,
            "recovery_tombstones",
            MAX_RECOVERY_TOMBSTONE_FILES,
            MAX_RECOVERY_TOMBSTONE_BYTES,
            MAX_RECOVERY_TOMBSTONE_TOTAL_BYTES,
        )?;
        if tombstones.len() >= MAX_RECOVERY_TOMBSTONE_FILES {
            return Err(content_limit_error(
                "recovery_tombstones",
                MAX_RECOVERY_TOMBSTONE_FILES,
                "files",
            ));
        }
        let filename = format!("{}.json", tombstone.recovery_id);
        write_unique_json(&dir, &filename, &stored)
    }

    fn prune_old_snapshots(&self, recovery_id: &str) -> RecoveryStoreResult<()> {
        let dir = self.snapshots_dir(recovery_id);
        let mut valid_paths = Vec::new();
        let mut read_budget =
            RecoveryLedgerReadBudget::new("recovery_snapshots", MAX_RECOVERY_SNAPSHOT_TOTAL_BYTES);
        for path in bounded_json_paths(
            &dir,
            "recovery_snapshots",
            MAX_RECOVERY_SNAPSHOT_FILES,
            MAX_RECOVERY_SNAPSHOT_BYTES,
            MAX_RECOVERY_SNAPSHOT_TOTAL_BYTES,
        )? {
            let bytes = read_budget.read(&path, MAX_RECOVERY_SNAPSHOT_BYTES, None)?;
            let Ok(stored) = serde_json::from_slice::<StoredSnapshot>(&bytes) else {
                continue;
            };
            if checksum_json(&stored.body).ok().as_deref() == Some(&stored.checksum) {
                valid_paths.push((stored.body.generation, path));
            }
        }
        valid_paths.sort_by_key(|(generation, _)| *generation);
        let remove_count = valid_paths.len().saturating_sub(SNAPSHOTS_TO_KEEP);
        for (_, path) in valid_paths.into_iter().take(remove_count) {
            fs::remove_file(path)?;
            self.inject_fault(RecoveryStoreFaultPoint::AfterSnapshotPruneDeletion)?;
        }
        if remove_count > 0 {
            sync_directory(&dir)?;
        }
        Ok(())
    }

    /// Compact only events covered by the oldest of at least two verified
    /// retained snapshots. The newer snapshot plus later events remain a
    /// replay path if the newest snapshot is later corrupted.
    fn prune_events_covered_by_retained_snapshot(
        &self,
        recovery_id: &str,
    ) -> RecoveryStoreResult<()> {
        let snapshot_dir = self.snapshots_dir(recovery_id);
        let mut generations = Vec::new();
        let mut read_budget =
            RecoveryLedgerReadBudget::new("recovery_snapshots", MAX_RECOVERY_SNAPSHOT_TOTAL_BYTES);
        for path in bounded_json_paths(
            &snapshot_dir,
            "recovery_snapshots",
            MAX_RECOVERY_SNAPSHOT_FILES,
            MAX_RECOVERY_SNAPSHOT_BYTES,
            MAX_RECOVERY_SNAPSHOT_TOTAL_BYTES,
        )? {
            let bytes = read_budget.read(&path, MAX_RECOVERY_SNAPSHOT_BYTES, None)?;
            let Ok(stored) = serde_json::from_slice::<StoredSnapshot>(&bytes) else {
                continue;
            };
            if stored.body.recovery_id == recovery_id
                && checksum_json(&stored.body).ok().as_deref() == Some(&stored.checksum)
            {
                generations.push(stored.body.generation);
            }
        }
        generations.sort_unstable();
        generations.dedup();
        if generations.len() < 2 {
            return Ok(());
        }
        let covered_generation = generations[0];
        let events_dir = self.events_dir(recovery_id);
        if !events_dir.is_dir() {
            return Ok(());
        }
        let mut removed = 0;
        for path in bounded_json_paths(
            &events_dir,
            "recovery_events",
            MAX_RECOVERY_EVENT_FILES,
            MAX_RECOVERY_EVENT_BYTES,
            MAX_RECOVERY_EVENT_TOTAL_BYTES,
        )? {
            let Some(generation) = path
                .file_name()
                .and_then(|value| value.to_str())
                .and_then(|value| value.split_once('-'))
                .and_then(|(generation, _)| generation.parse::<u64>().ok())
            else {
                continue;
            };
            if generation <= covered_generation {
                fs::remove_file(path)?;
                removed += 1;
                self.inject_fault(RecoveryStoreFaultPoint::AfterEventCompactionDeletion)?;
            }
        }
        if removed > 0 {
            sync_directory(&events_dir)?;
        }
        Ok(())
    }

    fn with_recovery_lock<T>(
        &self,
        recovery_id: &str,
        operation: impl FnOnce() -> RecoveryStoreResult<T>,
    ) -> RecoveryStoreResult<T> {
        self.ensure_root_dirs()?;
        let lock_path = self.locks_dir().join(format!("{recovery_id}.lock"));
        let lock = open_private_lock(&lock_path)?;
        FileExt::lock_exclusive(&lock)?;
        let result = operation();
        let unlock_result = FileExt::unlock(&lock);
        match (result, unlock_result) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error.into()),
        }
    }

    fn with_continuation_lock<T>(
        &self,
        operation: impl FnOnce() -> RecoveryStoreResult<T>,
    ) -> RecoveryStoreResult<T> {
        self.ensure_root_dirs()?;
        // Recovery ids cannot contain '.', so this coordinator filename can
        // never alias a per-record `<recovery_id>.lock` file.
        let lock = open_private_lock(&self.locks_dir().join(".continuations.lock"))?;
        FileExt::lock_exclusive(&lock)?;
        let result = operation();
        let unlock_result = FileExt::unlock(&lock);
        match (result, unlock_result) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error.into()),
        }
    }

    fn with_provider_root_claim_lock<T>(
        &self,
        operation: impl FnOnce() -> RecoveryStoreResult<T>,
    ) -> RecoveryStoreResult<T> {
        self.ensure_root_dirs()?;
        let lock = open_private_lock(&self.locks_dir().join(".provider-root-claims.lock"))?;
        FileExt::lock_exclusive(&lock)?;
        let result = operation();
        let unlock_result = FileExt::unlock(&lock);
        match (result, unlock_result) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error.into()),
        }
    }

    /// Lock a continuation pair in lexical recovery-id order. All code that
    /// needs two record locks uses this helper, preventing AB/BA deadlocks.
    fn with_recovery_locks<T>(
        &self,
        left_recovery_id: &str,
        right_recovery_id: &str,
        operation: impl FnOnce() -> RecoveryStoreResult<T>,
    ) -> RecoveryStoreResult<T> {
        self.ensure_root_dirs()?;
        let mut recovery_ids = [left_recovery_id, right_recovery_id];
        recovery_ids.sort_unstable();
        let mut locks = Vec::with_capacity(recovery_ids.len());
        for recovery_id in recovery_ids {
            let lock = open_private_lock(&self.locks_dir().join(format!("{recovery_id}.lock")))?;
            FileExt::lock_exclusive(&lock)?;
            locks.push(lock);
        }
        let result = operation();
        let mut unlock_error = None;
        for lock in locks.iter().rev() {
            if let Err(error) = FileExt::unlock(lock) {
                unlock_error.get_or_insert(error);
            }
        }
        match (result, unlock_error) {
            (Ok(value), None) => Ok(value),
            (Err(error), _) => Err(error),
            (Ok(_), Some(error)) => Err(error.into()),
        }
    }

    pub(super) fn with_attachment_inventory_lock<T>(
        &self,
        operation: impl FnOnce() -> RecoveryStoreResult<T>,
    ) -> RecoveryStoreResult<T> {
        self.ensure_root_dirs()?;
        let lock_path = self.locks_dir().join("attachment-inventory.lock");
        let lock = open_private_lock(&lock_path)?;
        FileExt::lock_exclusive(&lock)?;
        let result = operation();
        let unlock_result = FileExt::unlock(&lock);
        match (result, unlock_result) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error.into()),
        }
    }

    fn ensure_root_dirs(&self) -> RecoveryStoreResult<()> {
        create_private_dir_all(&self.root)?;
        create_private_dir_all(&self.recoveries_dir())?;
        create_private_dir_all(&self.tombstones_dir())?;
        create_private_dir_all(&self.continuations_dir())?;
        create_private_dir_all(&self.provider_root_claims_dir())?;
        create_private_dir_all(&self.locks_dir())?;
        Ok(())
    }

    fn remove_recovery_payload(&self, recovery_id: &str) -> RecoveryStoreResult<()> {
        let recovery_dir = self.recovery_dir(recovery_id);
        if recovery_dir.exists() {
            fs::remove_dir_all(&recovery_dir)?;
            sync_directory(recovery_dir.parent().unwrap_or(&self.root))?;
        }
        self.inject_fault(RecoveryStoreFaultPoint::AfterRecoveryPayloadPurge)
    }

    pub(super) fn inject_fault(&self, point: RecoveryStoreFaultPoint) -> RecoveryStoreResult<()> {
        if self.fault_point == Some(point) {
            Err(RecoveryStoreError::InjectedFault(point))
        } else {
            Ok(())
        }
    }

    fn recoveries_dir(&self) -> PathBuf {
        self.root.join("recoveries")
    }

    fn tombstones_dir(&self) -> PathBuf {
        self.root.join("tombstones")
    }

    fn continuations_dir(&self) -> PathBuf {
        self.root.join("continuations")
    }

    fn provider_root_claims_dir(&self) -> PathBuf {
        self.root.join("provider-root-claims")
    }

    fn provider_root_claim_events_dir(&self, provider_root_hash: &str) -> PathBuf {
        self.provider_root_claims_dir()
            .join(provider_root_hash_path_component(provider_root_hash))
    }

    fn locks_dir(&self) -> PathBuf {
        self.root.join("locks")
    }

    fn recovery_dir(&self, recovery_id: &str) -> PathBuf {
        self.recoveries_dir().join(recovery_id)
    }

    fn events_dir(&self, recovery_id: &str) -> PathBuf {
        self.recovery_dir(recovery_id).join("events")
    }

    fn snapshots_dir(&self, recovery_id: &str) -> PathBuf {
        self.recovery_dir(recovery_id).join("snapshots")
    }

    fn operations_dir(&self, recovery_id: &str) -> PathBuf {
        self.recovery_dir(recovery_id).join("operations")
    }

    fn operation_receipt_path(&self, recovery_id: &str, operation_id: &str) -> PathBuf {
        self.operations_dir(recovery_id)
            .join(operation_receipt_filename(operation_id))
    }

    fn tombstone_path(&self, recovery_id: &str) -> PathBuf {
        self.tombstones_dir().join(format!("{recovery_id}.json"))
    }
}

fn apply_mutation(
    record: &mut Option<RecoveryRecord>,
    recovery_id: &str,
    generation: u64,
    recorded_at: DateTime<Utc>,
    mutation: &RecoveryMutation,
) -> RecoveryStoreResult<()> {
    match mutation {
        RecoveryMutation::Create(request) => {
            if record.is_some() {
                return Err(RecoveryStoreError::AlreadyExists(recovery_id.to_string()));
            }
            *record = Some(RecoveryRecord {
                schema_version: STORE_SCHEMA_VERSION,
                recovery_id: request.recovery_id.clone(),
                session_id: request.session_id.clone(),
                repo_id: request.repo_id.clone(),
                generation,
                session_kind: request.session_kind,
                lifecycle: RecoveryLifecycle::Launching,
                // Recovery records are created only after the worktree and
                // launch base have been durably materialized.
                launch_stage: RecoveryLaunchStage::WorktreeMaterialized,
                root_role: ProviderRootRole::Unknown,
                recovery_lease: None,
                supervisor_stop_proof: None,
                worktree_path: request.worktree_path.clone(),
                launch_base_ref: request.launch_base_ref.clone(),
                launch_base_oid: request.launch_base_oid.clone(),
                launch_head_oid: request.launch_head_oid.clone(),
                provider: request.provider.clone(),
                model: request.model.clone(),
                runtime: request.runtime.clone(),
                initial_prompt: request.initial_prompt.clone(),
                provider_root: None,
                provider_root_candidates: Vec::new(),
                checkpoint_revision: 0,
                checkpoint_coverage: CheckpointCoverage::Unknown,
                checkpoint: None,
                latest_root_input: None,
                board_outbox: Vec::new(),
                board_entry_ids: Vec::new(),
                board_delivery_error: None,
                continuation_targets: Vec::new(),
                continuation_source: None,
                continuation_root_provenance: None,
                created_at: request.created_at,
                updated_at: request.created_at,
                lifecycle_reason: None,
            });
        }
        RecoveryMutation::AdvanceLaunchStage { stage, root_role } => {
            let current = require_record_mut(record, recovery_id)?;
            apply_launch_stage(current, *stage, *root_role)?;
            current.generation = generation;
            current.updated_at = recorded_at;
        }
        RecoveryMutation::BindRoot(binding) => {
            let current = require_record_mut(record, recovery_id)?;
            require_bindable_root_role(current)?;
            apply_root_binding(current, binding.clone())?;
            if binding.quality != BindingQuality::Preassigned {
                apply_launch_stage(
                    current,
                    RecoveryLaunchStage::ProviderBound,
                    Some(ProviderRootRole::Root),
                )?;
            } else {
                apply_launch_stage(current, current.launch_stage, Some(ProviderRootRole::Root))?;
            }
            update_continuation_target_root_provenance(current)?;
            current.generation = generation;
            current.updated_at = recorded_at;
        }
        RecoveryMutation::BindRootSemantic {
            root_id,
            session_tree_id,
            quality,
        } => {
            let current = require_record_mut(record, recovery_id)?;
            require_bindable_root_role(current)?;
            apply_root_binding(
                current,
                ProviderRootBinding {
                    root_id: root_id.clone(),
                    session_tree_id: session_tree_id.clone(),
                    quality: *quality,
                    bound_at: recorded_at,
                },
            )?;
            if *quality != BindingQuality::Preassigned {
                apply_launch_stage(
                    current,
                    RecoveryLaunchStage::ProviderBound,
                    Some(ProviderRootRole::Root),
                )?;
            } else {
                apply_launch_stage(current, current.launch_stage, Some(ProviderRootRole::Root))?;
            }
            update_continuation_target_root_provenance(current)?;
            current.generation = generation;
            current.updated_at = recorded_at;
        }
        RecoveryMutation::RecordProviderRootCandidates(candidates) => {
            let current = require_record_mut(record, recovery_id)?;
            for candidate in candidates {
                if let Some(existing) = current
                    .provider_root_candidates
                    .iter_mut()
                    .find(|existing| existing.root_id == candidate.root_id)
                {
                    for evidence in &candidate.evidence {
                        if !existing.evidence.contains(evidence) {
                            existing.evidence.push(evidence.clone());
                        }
                    }
                    existing.evidence.sort();
                    existing.observed_at = existing.observed_at.max(candidate.observed_at);
                } else {
                    current.provider_root_candidates.push(candidate.clone());
                }
            }
            current
                .provider_root_candidates
                .sort_by(|left, right| left.root_id.cmp(&right.root_id));
            current.generation = generation;
            current.updated_at = recorded_at;
        }
        RecoveryMutation::ReplaceCheckpoint {
            root_id,
            expected_revision,
            checkpoint,
        } => {
            let current = require_record_mut(record, recovery_id)?;
            require_root(current, root_id)?;
            if current.checkpoint_revision != *expected_revision {
                return Err(RecoveryStoreError::RevisionMismatch {
                    expected: *expected_revision,
                    actual: current.checkpoint_revision,
                });
            }
            current.checkpoint_revision = current.checkpoint_revision.saturating_add(1);
            current.checkpoint_coverage = CheckpointCoverage::Explicit;
            for intent in &checkpoint.board_intents {
                if current.board_entry_ids.contains(&intent.entry_id) {
                    continue;
                }
                if let Some(existing) = current
                    .board_outbox
                    .iter()
                    .find(|existing| existing.entry_id == intent.entry_id)
                {
                    if existing != intent {
                        return Err(RecoveryStoreError::BoardIntentConflict {
                            entry_id: intent.entry_id.clone(),
                        });
                    }
                    continue;
                }
                current.board_outbox.push(intent.clone());
            }
            current.checkpoint = Some(checkpoint.clone());
            current.generation = generation;
            current.updated_at = recorded_at;
        }
        RecoveryMutation::RecordRootInput {
            root_id,
            turn_id,
            text,
        } => {
            let current = require_record_mut(record, recovery_id)?;
            require_root(current, root_id)?;
            current.latest_root_input = Some(RootInput {
                turn_id: turn_id.clone(),
                text: text.clone(),
                submitted_at: recorded_at,
            });
            if current.checkpoint_revision > 0 {
                current.checkpoint_coverage = CheckpointCoverage::Stale;
            }
            current.generation = generation;
            current.updated_at = recorded_at;
        }
        RecoveryMutation::RecordRootTurn(update) => {
            let current = require_record_mut(record, recovery_id)?;
            require_root(current, &update.root_id)?;

            if let Some(text) = &update.input_text {
                current.latest_root_input = Some(RootInput {
                    turn_id: update.turn_id.clone(),
                    text: text.clone(),
                    submitted_at: recorded_at,
                });
                if current.checkpoint_revision > 0 {
                    current.checkpoint_coverage = CheckpointCoverage::Stale;
                }
            }

            let mut checkpoint_changed = false;
            if !update.visible_items.is_empty() || !update.attachment_refs.is_empty() {
                let checkpoint = current.checkpoint.get_or_insert_with(Default::default);
                for attachment in &update.attachment_refs {
                    if !checkpoint.attachment_refs.contains(attachment) {
                        checkpoint.attachment_refs.push(attachment.clone());
                        checkpoint_changed = true;
                    }
                }
                if !update.visible_items.is_empty() {
                    checkpoint.as_of_turn_id = Some(update.turn_id.clone());
                    checkpoint
                        .visible_items
                        .extend(update.visible_items.clone());
                    if checkpoint.visible_items.len() > MAX_RECOVERY_VISIBLE_ITEMS {
                        let remove_count =
                            checkpoint.visible_items.len() - MAX_RECOVERY_VISIBLE_ITEMS;
                        checkpoint.visible_items.drain(..remove_count);
                    }
                    checkpoint_changed = true;
                    current.checkpoint_coverage = CheckpointCoverage::Explicit;
                } else if update.input_text.is_some() {
                    // The attachment and its user input are one turn. The
                    // checkpoint contains the evidence but still predates the
                    // provider's response to that input.
                    current.checkpoint_coverage = CheckpointCoverage::Stale;
                } else if checkpoint_changed {
                    current.checkpoint_coverage = CheckpointCoverage::Explicit;
                }
            }
            if checkpoint_changed {
                current.checkpoint_revision = current.checkpoint_revision.saturating_add(1);
            }
            current.generation = generation;
            current.updated_at = recorded_at;
        }
        RecoveryMutation::SetLifecycle { lifecycle, reason } => {
            let current = require_record_mut(record, recovery_id)?;
            match lifecycle {
                RecoveryLifecycle::Resolved => {
                    apply_launch_stage(current, RecoveryLaunchStage::Resolved, None)?
                }
                RecoveryLifecycle::Discarded => {
                    apply_launch_stage(current, RecoveryLaunchStage::Discarded, None)?
                }
                _ => {}
            }
            current.lifecycle = *lifecycle;
            current.lifecycle_reason = reason.clone();
            if *lifecycle != RecoveryLifecycle::Recovering {
                current.recovery_lease = None;
            }
            if *lifecycle == RecoveryLifecycle::Running {
                current.supervisor_stop_proof = None;
            }
            current.generation = generation;
            current.updated_at = recorded_at;
        }
        RecoveryMutation::InterruptAfterSupervisorStop {
            expected_generation,
            session_id,
            reason,
        } => {
            let current = require_record_mut(record, recovery_id)?;
            if current.generation != *expected_generation {
                return Err(RecoveryStoreError::GenerationMismatch {
                    expected: *expected_generation,
                    actual: current.generation,
                });
            }
            if current.session_kind != RecoverySessionKind::Intake
                || current.session_id != *session_id
            {
                return Err(RecoveryStoreError::InvalidLease(
                    "supervisor stop proof does not match the Intake Session".to_string(),
                ));
            }
            if current.launch_stage < RecoveryLaunchStage::SpawnRequested
                || current.launch_stage.is_terminal()
                || !matches!(
                    current.lifecycle,
                    RecoveryLifecycle::Launching
                        | RecoveryLifecycle::Running
                        | RecoveryLifecycle::Interrupted
                )
                || current.recovery_lease.is_some()
            {
                return Err(RecoveryStoreError::InvalidLease(
                    "recovery is not a live supervisor-owned provider".to_string(),
                ));
            }
            current.lifecycle = RecoveryLifecycle::Interrupted;
            current.lifecycle_reason = Some(reason.clone());
            current.supervisor_stop_proof = Some(RecoverySupervisorStopProof {
                session_id: session_id.clone(),
                observed_at: recorded_at,
            });
            current.generation = generation;
            current.updated_at = recorded_at;
        }
        RecoveryMutation::ClaimRecovery {
            expected_generation,
            lease,
            reason,
            confirmed_root_id,
        }
        | RecoveryMutation::ClaimInterruptedRecovery {
            expected_generation,
            lease,
            reason,
            confirmed_root_id,
        } => {
            let current = require_record_mut(record, recovery_id)?;
            let interrupted_claim =
                matches!(mutation, RecoveryMutation::ClaimInterruptedRecovery { .. });
            if current.generation != *expected_generation {
                return Err(RecoveryStoreError::GenerationMismatch {
                    expected: *expected_generation,
                    actual: current.generation,
                });
            }
            if interrupted_claim {
                let proof_matches = current
                    .supervisor_stop_proof
                    .as_ref()
                    .is_some_and(|proof| proof.session_id == current.session_id);
                if current.lifecycle != RecoveryLifecycle::Interrupted
                    || current.launch_stage > RecoveryLaunchStage::Ready
                    || !proof_matches
                {
                    return Err(RecoveryStoreError::InvalidLease(
                        "interrupted recovery claim requires durable supervisor-stop proof"
                            .to_string(),
                    ));
                }
            } else if current.launch_stage >= RecoveryLaunchStage::Ready
                || current.launch_stage.is_terminal()
            {
                return Err(RecoveryStoreError::InvalidLease(
                    "provider-ready or terminal recovery cannot be claimed for launch".to_string(),
                ));
            }
            if let Some(existing) = current.recovery_lease.as_ref() {
                let same_claim =
                    existing.lease_id == lease.lease_id && existing.holder_id == lease.holder_id;
                if !same_claim && existing.expires_at > lease.acquired_at {
                    return Err(RecoveryStoreError::LeaseConflict {
                        lease_id: existing.lease_id.clone(),
                        holder_id: existing.holder_id.clone(),
                    });
                }
            }
            if let Some(root_id) = confirmed_root_id {
                require_bindable_root_role(current)?;
                if !current
                    .provider_root_candidates
                    .iter()
                    .any(|candidate| candidate.root_id == *root_id)
                {
                    return Err(RecoveryStoreError::UnknownProviderRootCandidate {
                        root_id: root_id.clone(),
                    });
                }
                if let Some(existing) = current.provider_root.as_ref() {
                    if existing.root_id != *root_id {
                        return Err(RecoveryStoreError::RootBindingConflict {
                            expected: existing.root_id.clone(),
                            actual: root_id.clone(),
                        });
                    }
                }
                let should_confirm = current
                    .provider_root
                    .as_ref()
                    .is_none_or(|existing| existing.quality < BindingQuality::Confirmed);
                if should_confirm {
                    current.provider_root = Some(ProviderRootBinding {
                        root_id: root_id.clone(),
                        session_tree_id: None,
                        quality: BindingQuality::Confirmed,
                        bound_at: lease.acquired_at,
                    });
                }
                // Candidate confirmation selects an exact-resume target but
                // does not prove that a replacement provider process bound it.
                apply_launch_stage(current, current.launch_stage, Some(ProviderRootRole::Root))?;
            }
            current.recovery_lease = Some(lease.clone());
            current.lifecycle = RecoveryLifecycle::Recovering;
            current.lifecycle_reason = Some(reason.clone());
            current.generation = generation;
            current.updated_at = recorded_at;
        }
        RecoveryMutation::CompleteClaimedProviderReady {
            claim_holder_recovery_id: _,
            claim_token: _,
        } => {
            let current = require_record_mut(record, recovery_id)?;
            apply_launch_stage(
                current,
                RecoveryLaunchStage::Ready,
                Some(ProviderRootRole::Root),
            )?;
            current.lifecycle = RecoveryLifecycle::Running;
            current.lifecycle_reason = None;
            current.recovery_lease = None;
            current.supervisor_stop_proof = None;
            current.generation = generation;
            current.updated_at = recorded_at;
        }
        RecoveryMutation::CompleteProviderReady => {
            let current = require_record_mut(record, recovery_id)?;
            apply_launch_stage(
                current,
                RecoveryLaunchStage::Ready,
                Some(ProviderRootRole::Root),
            )?;
            current.lifecycle = RecoveryLifecycle::Running;
            current.lifecycle_reason = None;
            current.recovery_lease = None;
            current.supervisor_stop_proof = None;
            current.generation = generation;
            current.updated_at = recorded_at;
        }
        RecoveryMutation::AckBoardEntry { entry_id } => {
            let current = require_record_mut(record, recovery_id)?;
            if let Some(index) = current
                .board_outbox
                .iter()
                .position(|intent| intent.entry_id == *entry_id)
            {
                current.board_outbox.remove(index);
            }
            if !current.board_entry_ids.contains(entry_id) {
                current.board_entry_ids.push(entry_id.clone());
                normalize_board_entry_ids(&mut current.board_entry_ids);
            }
            if current.board_outbox.is_empty() {
                current.board_delivery_error = None;
            }
            current.generation = generation;
            current.updated_at = recorded_at;
        }
        RecoveryMutation::SetBoardDeliveryError { error } => {
            let current = require_record_mut(record, recovery_id)?;
            current.board_delivery_error = error.clone();
            current.generation = generation;
            current.updated_at = recorded_at;
        }
        RecoveryMutation::LinkContinuation {
            link,
            role,
            inherited_state,
        } => {
            let current = require_record_mut(record, recovery_id)?;
            match role {
                RecoveryContinuationRole::Source => {
                    if current.recovery_id != link.source_recovery_id
                        || current.checkpoint_revision != link.source_checkpoint_revision
                    {
                        return Err(RecoveryStoreError::RevisionMismatch {
                            expected: link.source_checkpoint_revision,
                            actual: current.checkpoint_revision,
                        });
                    }
                    if let Some(existing) = current
                        .continuation_targets
                        .iter()
                        .find(|existing| existing.target_recovery_id == link.target_recovery_id)
                    {
                        if existing != link {
                            return Err(RecoveryStoreError::ContinuationConflict);
                        }
                    } else {
                        current.continuation_targets.push(link.clone());
                    }
                }
                RecoveryContinuationRole::Target => {
                    if current.recovery_id != link.target_recovery_id {
                        return Err(RecoveryStoreError::ContinuationConflict);
                    }
                    if current
                        .continuation_source
                        .as_ref()
                        .is_some_and(|existing| existing != link)
                    {
                        return Err(RecoveryStoreError::ContinuationConflict);
                    }
                    current.continuation_source = Some(link.clone());
                    merge_continuation_root_provenance(
                        current,
                        inherited_state
                            .as_deref()
                            .and_then(|state| state.source_provider_root_id.clone()),
                    )?;
                    if let Some(inherited) = inherited_state {
                        if current.checkpoint.is_none() && current.checkpoint_revision == 0 {
                            current.checkpoint_revision = inherited.checkpoint_revision;
                            current.checkpoint_coverage = inherited.checkpoint_coverage;
                            current.checkpoint = inherited.checkpoint.clone();
                        }
                        if current.latest_root_input.is_none() {
                            current.latest_root_input = inherited.latest_root_input.clone();
                        }
                    }
                }
            }
            current.generation = generation;
            current.updated_at = recorded_at;
        }
        RecoveryMutation::CancelContinuation { link } => {
            let current = require_record_mut(record, recovery_id)?;
            if current.recovery_id != link.source_recovery_id {
                return Err(RecoveryStoreError::ContinuationConflict);
            }
            if let Some(index) = current
                .continuation_targets
                .iter()
                .position(|existing| existing == link)
            {
                current.continuation_targets.remove(index);
            } else if current.continuation_targets.iter().any(|existing| {
                existing.target_recovery_id == link.target_recovery_id
                    && !continuation_link_semantics_match(existing, link)
            }) {
                return Err(RecoveryStoreError::ContinuationConflict);
            }
            current.generation = generation;
            current.updated_at = recorded_at;
        }
    }
    Ok(())
}

fn require_record_mut<'a>(
    record: &'a mut Option<RecoveryRecord>,
    recovery_id: &str,
) -> RecoveryStoreResult<&'a mut RecoveryRecord> {
    record
        .as_mut()
        .ok_or_else(|| RecoveryStoreError::NotFound(recovery_id.to_string()))
}

fn require_bindable_root_role(record: &RecoveryRecord) -> RecoveryStoreResult<()> {
    if record.root_role == ProviderRootRole::Subagent {
        return Err(RecoveryStoreError::RootRoleRejected(record.root_role));
    }
    Ok(())
}

fn apply_launch_stage(
    record: &mut RecoveryRecord,
    requested: RecoveryLaunchStage,
    observed_role: Option<ProviderRootRole>,
) -> RecoveryStoreResult<()> {
    if let Some(role) = observed_role {
        match (record.root_role, role) {
            (ProviderRootRole::Unknown, role) => record.root_role = role,
            (current, requested) if current == requested => {}
            (current, requested) => {
                return Err(RecoveryStoreError::RootRoleConflict { current, requested });
            }
        }
    }

    if requested >= RecoveryLaunchStage::ProviderBound
        && requested <= RecoveryLaunchStage::Ready
        && record.root_role != ProviderRootRole::Root
    {
        return Err(RecoveryStoreError::RootRoleRejected(record.root_role));
    }
    if record.launch_stage.is_terminal() && requested != record.launch_stage {
        return Err(RecoveryStoreError::InvalidLaunchStageTransition {
            current: record.launch_stage,
            requested,
        });
    }
    if requested > record.launch_stage {
        record.launch_stage = requested;
    }
    Ok(())
}

fn apply_root_binding(
    current: &mut RecoveryRecord,
    binding: ProviderRootBinding,
) -> RecoveryStoreResult<()> {
    if let Some(existing) = &current.provider_root {
        if existing.root_id != binding.root_id {
            return Err(RecoveryStoreError::RootBindingConflict {
                expected: existing.root_id.clone(),
                actual: binding.root_id,
            });
        }
        if existing
            .session_tree_id
            .as_ref()
            .zip(binding.session_tree_id.as_ref())
            .is_some_and(|(left, right)| left != right)
        {
            return Err(RecoveryStoreError::RootBindingConflict {
                expected: existing.root_id.clone(),
                actual: binding.root_id,
            });
        }
        if binding.quality >= existing.quality {
            current.provider_root = Some(binding);
        }
    } else {
        current.provider_root = Some(binding);
    }
    Ok(())
}

fn successor_terminal_operation_id(
    outcome: &str,
    transaction: &ContinuationTransactionBody,
) -> RecoveryStoreResult<String> {
    Ok(format!(
        "successor-{outcome}:{}",
        checksum_json(&(
            transaction.operation_id.as_str(),
            transaction.link.source_recovery_id.as_str(),
            transaction.link.target_recovery_id.as_str(),
        ))?
    ))
}

fn successor_record_disposition(record: &RecoveryRecord) -> SuccessorTargetDisposition {
    match record.launch_stage {
        RecoveryLaunchStage::Ready | RecoveryLaunchStage::Resolved => {
            SuccessorTargetDisposition::Ready
        }
        RecoveryLaunchStage::Discarded => SuccessorTargetDisposition::Discarded,
        _ => successor_terminal_lifecycle_disposition(record.lifecycle),
    }
}

fn successor_terminal_lifecycle_disposition(
    lifecycle: RecoveryLifecycle,
) -> SuccessorTargetDisposition {
    match lifecycle {
        RecoveryLifecycle::Resolved => SuccessorTargetDisposition::Ready,
        RecoveryLifecycle::Discarded => SuccessorTargetDisposition::Discarded,
        _ => SuccessorTargetDisposition::Pending,
    }
}

fn continuation_retry_matches(
    source: &RecoveryRecord,
    target: &RecoveryRecord,
    requested: &RecoveryContinuationLink,
) -> bool {
    let source_link = source
        .continuation_targets
        .iter()
        .find(|link| continuation_link_semantics_match(link, requested));
    let target_link = target.continuation_source.as_ref();
    source_link.is_some_and(|source_link| {
        target_link.is_some_and(|target_link| {
            source_link == target_link && continuation_link_semantics_match(target_link, requested)
        })
    })
}

fn validate_successor_link(link: &mut RecoveryContinuationLink) -> RecoveryStoreResult<()> {
    validate_recovery_id(&link.source_recovery_id)?;
    validate_recovery_id(&link.target_recovery_id)?;
    if link.source_recovery_id == link.target_recovery_id {
        return Err(RecoveryStoreError::ContinuationConflict);
    }
    link.definitive_reason = sanitize_visible_text(&link.definitive_reason);
    if link.definitive_reason.trim().is_empty() {
        return Err(RecoveryStoreError::ContinuationConflict);
    }
    Ok(())
}

fn validate_continuation_operation_id(operation_id: &str) -> RecoveryStoreResult<()> {
    if operation_id.trim().is_empty() {
        return Err(RecoveryStoreError::OperationConflict {
            operation_id: operation_id.to_string(),
        });
    }
    ensure_text_limit("operation_id", operation_id, MAX_RECOVERY_IDENTIFIER_CHARS)?;
    ensure_text_limit(
        "operation_id",
        &format!("{operation_id}:source"),
        MAX_RECOVERY_IDENTIFIER_CHARS,
    )?;
    ensure_text_limit(
        "operation_id",
        &format!("{operation_id}:target"),
        MAX_RECOVERY_IDENTIFIER_CHARS,
    )
}

fn successor_request_semantics_match(
    existing: &RecoveryContinuationLink,
    requested: &RecoveryContinuationLink,
) -> bool {
    existing.source_recovery_id == requested.source_recovery_id
        && existing.source_checkpoint_revision == requested.source_checkpoint_revision
        && existing.definitive_reason == requested.definitive_reason
}

fn continuation_inherited_state(source: &RecoveryRecord) -> Option<Box<RecoveryContinuationState>> {
    let has_inherited_state = source.checkpoint.is_some()
        || source.latest_root_input.is_some()
        || source.provider_root.is_some();
    has_inherited_state.then(|| {
        let mut checkpoint = source.checkpoint.clone();
        if let Some(checkpoint) = checkpoint.as_mut() {
            // Board delivery remains owned by the source recovery. The target
            // inherits semantic and attachment context only.
            checkpoint.board_intents.clear();
        }
        Box::new(RecoveryContinuationState {
            checkpoint_revision: source.checkpoint_revision,
            checkpoint_coverage: source.checkpoint_coverage,
            checkpoint,
            latest_root_input: source.latest_root_input.clone(),
            source_provider_root_id: source
                .provider_root
                .as_ref()
                .map(|root| root.root_id.clone()),
        })
    })
}

fn continuation_link_semantics_match(
    left: &RecoveryContinuationLink,
    right: &RecoveryContinuationLink,
) -> bool {
    left.source_recovery_id == right.source_recovery_id
        && left.target_recovery_id == right.target_recovery_id
        && left.source_checkpoint_revision == right.source_checkpoint_revision
        && left.definitive_reason == right.definitive_reason
}

fn update_continuation_target_root_provenance(
    current: &mut RecoveryRecord,
) -> RecoveryStoreResult<()> {
    if current.continuation_source.is_some() {
        merge_continuation_root_provenance(current, None)?;
    }
    Ok(())
}

fn merge_continuation_root_provenance(
    current: &mut RecoveryRecord,
    source_provider_root_id: Option<String>,
) -> RecoveryStoreResult<()> {
    let target_provider_root_id = current
        .provider_root
        .as_ref()
        .map(|root| root.root_id.clone());
    if source_provider_root_id.is_none()
        && target_provider_root_id.is_none()
        && current.continuation_root_provenance.is_none()
    {
        return Ok(());
    }
    let provenance = current
        .continuation_root_provenance
        .get_or_insert_with(RecoveryContinuationRootProvenance::default);
    if let Some(source_provider_root_id) = source_provider_root_id {
        if provenance
            .source_provider_root_id
            .as_ref()
            .is_some_and(|existing| existing != &source_provider_root_id)
        {
            return Err(RecoveryStoreError::ContinuationConflict);
        }
        provenance.source_provider_root_id = Some(source_provider_root_id);
    }
    if let Some(target_provider_root_id) = target_provider_root_id {
        if provenance
            .target_provider_root_id
            .as_ref()
            .is_some_and(|existing| existing != &target_provider_root_id)
        {
            return Err(RecoveryStoreError::ContinuationConflict);
        }
        provenance.target_provider_root_id = Some(target_provider_root_id);
    }
    Ok(())
}

fn normalize_provider_root_candidates(
    candidates: Vec<ProviderRootCandidate>,
) -> RecoveryStoreResult<Vec<ProviderRootCandidate>> {
    if candidates.is_empty() {
        return Err(RecoveryStoreError::InvalidProviderRootCandidate(
            "at least one candidate is required".to_string(),
        ));
    }
    let mut normalized = BTreeMap::<String, ProviderRootCandidate>::new();
    for candidate in candidates {
        let root_id = normalize_provider_root_id(&candidate.root_id)?;
        let mut evidence = candidate
            .evidence
            .into_iter()
            .map(|value| sanitize_visible_text(&value).trim().to_string())
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        if evidence.iter().any(|value| value.chars().count() > 256) {
            return Err(RecoveryStoreError::InvalidProviderRootCandidate(
                "candidate evidence must be at most 256 characters".to_string(),
            ));
        }
        evidence.sort();
        evidence.dedup();
        if evidence.is_empty() || evidence.len() > 16 {
            return Err(RecoveryStoreError::InvalidProviderRootCandidate(
                "each candidate requires between one and sixteen evidence labels".to_string(),
            ));
        }
        match normalized.get_mut(&root_id) {
            Some(existing) => {
                for item in evidence {
                    if !existing.evidence.contains(&item) {
                        existing.evidence.push(item);
                    }
                }
                existing.evidence.sort();
                existing.observed_at = existing.observed_at.max(candidate.observed_at);
            }
            None => {
                normalized.insert(
                    root_id.clone(),
                    ProviderRootCandidate {
                        root_id,
                        evidence,
                        observed_at: candidate.observed_at,
                    },
                );
            }
        }
    }
    if normalized.len() > 32 {
        return Err(RecoveryStoreError::InvalidProviderRootCandidate(
            "at most 32 provider root candidates may be recorded".to_string(),
        ));
    }
    Ok(normalized.into_values().collect())
}

fn normalize_provider_root_id(root_id: &str) -> RecoveryStoreResult<String> {
    let root_id = root_id.trim();
    if root_id.is_empty()
        || root_id.chars().count() > 1024
        || root_id
            .chars()
            .any(|character| character.is_control() || character == '\u{1b}')
    {
        return Err(RecoveryStoreError::InvalidProviderRootCandidate(
            "root id must be a non-empty single-line visible identifier".to_string(),
        ));
    }
    Ok(root_id.to_string())
}

fn validate_recovery_lease(lease: &RecoveryLease) -> RecoveryStoreResult<()> {
    let valid_identity = |value: &str| {
        !value.trim().is_empty()
            && value.len() <= 256
            && value
                .chars()
                .all(|character| !character.is_control() && character != '\u{1b}')
    };
    if !valid_identity(&lease.lease_id) || !valid_identity(&lease.holder_id) {
        return Err(RecoveryStoreError::InvalidLease(
            "lease and holder ids must be non-empty visible identifiers".to_string(),
        ));
    }
    if lease.expires_at <= lease.acquired_at {
        return Err(RecoveryStoreError::InvalidLease(
            "expiry must be later than acquisition".to_string(),
        ));
    }
    Ok(())
}

fn validate_provider_root_claim_expiry(
    acquired_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
) -> RecoveryStoreResult<()> {
    if expires_at <= acquired_at {
        return Err(RecoveryStoreError::InvalidLease(
            "provider root claim expiry must be later than acquisition".to_string(),
        ));
    }
    if expires_at - acquired_at > Duration::minutes(MAX_PROVIDER_ROOT_CLAIM_TTL_MINUTES) {
        return Err(RecoveryStoreError::InvalidLease(format!(
            "provider root claim expiry must be within {MAX_PROVIDER_ROOT_CLAIM_TTL_MINUTES} minutes"
        )));
    }
    Ok(())
}

fn provider_root_claim_hash(provider: &str, provider_root_id: &str) -> String {
    prefixed_sha256(&format!(
        "provider-root-claim-v1\0{}\0{}",
        provider.trim().to_ascii_lowercase(),
        provider_root_id
    ))
}

fn provider_root_hash_path_component(provider_root_hash: &str) -> String {
    provider_root_hash
        .strip_prefix("sha256:")
        .unwrap_or(provider_root_hash)
        .to_string()
}

fn require_root(record: &RecoveryRecord, actual: &str) -> RecoveryStoreResult<()> {
    if record.root_role != ProviderRootRole::Root {
        return Err(RecoveryStoreError::RootRoleRejected(record.root_role));
    }
    let expected = record
        .provider_root
        .as_ref()
        .map(|binding| binding.root_id.as_str())
        .unwrap_or_default();
    if expected != actual {
        return Err(RecoveryStoreError::RootMismatch {
            expected: expected.to_string(),
            actual: actual.to_string(),
        });
    }
    Ok(())
}

fn sanitize_root_turn_update(update: &mut RootTurnUpdate) -> RecoveryStoreResult<()> {
    ensure_text_limit(
        "root_turn.root_id",
        &update.root_id,
        MAX_RECOVERY_IDENTIFIER_CHARS,
    )?;
    ensure_text_limit(
        "root_turn.turn_id",
        &update.turn_id,
        MAX_RECOVERY_IDENTIFIER_CHARS,
    )?;
    ensure_collection_limit("root_turn.visible_items", update.visible_items.len())?;
    ensure_collection_limit("root_turn.attachment_refs", update.attachment_refs.len())?;
    if let Some(text) = &mut update.input_text {
        sanitize_bounded_text(
            "root_turn.input_text",
            text,
            MAX_RECOVERY_INITIAL_PROMPT_CHARS,
        )?;
    }
    for item in &mut update.visible_items {
        sanitize_bounded_text(
            "root_turn.visible_item.role",
            &mut item.role,
            MAX_RECOVERY_IDENTIFIER_CHARS,
        )?;
        sanitize_bounded_text(
            "root_turn.visible_item.kind",
            &mut item.kind,
            MAX_RECOVERY_IDENTIFIER_CHARS,
        )?;
        sanitize_bounded_text(
            "root_turn.visible_item.text",
            &mut item.text,
            MAX_RECOVERY_TEXT_FIELD_CHARS,
        )?;
    }
    for attachment in &mut update.attachment_refs {
        sanitize_bounded_text(
            "root_turn.attachment.file_name",
            &mut attachment.file_name,
            MAX_RECOVERY_IDENTIFIER_CHARS,
        )?;
        attachment::validate_reference(attachment)?;
    }
    ensure_serialized_limit("root_turn", update)?;
    Ok(())
}

fn sanitize_checkpoint(checkpoint: &mut SemanticCheckpoint) -> RecoveryStoreResult<()> {
    ensure_collection_limit(
        "checkpoint.confirmed_decisions",
        checkpoint.confirmed_decisions.len(),
    )?;
    ensure_collection_limit("checkpoint.open_questions", checkpoint.open_questions.len())?;
    ensure_collection_limit("checkpoint.visible_items", checkpoint.visible_items.len())?;
    ensure_collection_limit(
        "checkpoint.attachment_refs",
        checkpoint.attachment_refs.len(),
    )?;
    ensure_collection_limit("checkpoint.board_intents", checkpoint.board_intents.len())?;
    sanitize_bounded_text(
        "checkpoint.summary",
        &mut checkpoint.summary,
        MAX_RECOVERY_TEXT_FIELD_CHARS,
    )?;
    for decision in &mut checkpoint.confirmed_decisions {
        sanitize_bounded_text(
            "checkpoint.confirmed_decision",
            decision,
            MAX_RECOVERY_TEXT_FIELD_CHARS,
        )?;
    }
    for question in &mut checkpoint.open_questions {
        sanitize_bounded_text(
            "checkpoint.open_question",
            question,
            MAX_RECOVERY_TEXT_FIELD_CHARS,
        )?;
    }
    if let Some(next_action) = &mut checkpoint.next_action {
        sanitize_bounded_text(
            "checkpoint.next_action",
            next_action,
            MAX_RECOVERY_TEXT_FIELD_CHARS,
        )?;
    }
    if let Some(turn_id) = &mut checkpoint.as_of_turn_id {
        sanitize_bounded_text(
            "checkpoint.as_of_turn_id",
            turn_id,
            MAX_RECOVERY_IDENTIFIER_CHARS,
        )?;
    }
    for item in &mut checkpoint.visible_items {
        sanitize_bounded_text(
            "checkpoint.visible_item.role",
            &mut item.role,
            MAX_RECOVERY_IDENTIFIER_CHARS,
        )?;
        sanitize_bounded_text(
            "checkpoint.visible_item.kind",
            &mut item.kind,
            MAX_RECOVERY_IDENTIFIER_CHARS,
        )?;
        sanitize_bounded_text(
            "checkpoint.visible_item.text",
            &mut item.text,
            MAX_RECOVERY_TEXT_FIELD_CHARS,
        )?;
    }
    for attachment in &mut checkpoint.attachment_refs {
        sanitize_bounded_text(
            "checkpoint.attachment.file_name",
            &mut attachment.file_name,
            MAX_RECOVERY_IDENTIFIER_CHARS,
        )?;
        attachment::validate_reference(attachment)?;
    }
    for intent in &mut checkpoint.board_intents {
        sanitize_bounded_text(
            "checkpoint.board_intent.entry_id",
            &mut intent.entry_id,
            MAX_RECOVERY_IDENTIFIER_CHARS,
        )?;
        sanitize_bounded_text(
            "checkpoint.board_intent.title",
            &mut intent.title,
            MAX_RECOVERY_TEXT_FIELD_CHARS,
        )?;
        sanitize_bounded_text(
            "checkpoint.board_intent.body",
            &mut intent.body,
            MAX_RECOVERY_TEXT_FIELD_CHARS,
        )?;
    }
    ensure_serialized_limit("checkpoint", checkpoint)?;
    Ok(())
}

fn sanitize_bounded_text(
    field: &'static str,
    value: &mut String,
    max_chars: usize,
) -> RecoveryStoreResult<()> {
    ensure_text_limit(field, value, max_chars)?;
    *value = sanitize_visible_text(value);
    Ok(())
}

fn ensure_text_limit(
    field: &'static str,
    value: &str,
    max_chars: usize,
) -> RecoveryStoreResult<()> {
    if value.len() <= max_chars || value.chars().take(max_chars + 1).count() <= max_chars {
        return Ok(());
    }
    Err(RecoveryStoreError::ContentLimitExceeded {
        field: field.to_string(),
        limit: max_chars,
        unit: "characters",
    })
}

fn ensure_collection_limit(field: &'static str, len: usize) -> RecoveryStoreResult<()> {
    if len <= MAX_RECOVERY_COLLECTION_ITEMS {
        return Ok(());
    }
    Err(RecoveryStoreError::ContentLimitExceeded {
        field: field.to_string(),
        limit: MAX_RECOVERY_COLLECTION_ITEMS,
        unit: "items",
    })
}

fn ensure_serialized_limit<T: Serialize>(
    field: &'static str,
    value: &T,
) -> RecoveryStoreResult<()> {
    ensure_serialized_limit_with(field, value, MAX_RECOVERY_SEMANTIC_PAYLOAD_BYTES)
}

fn ensure_serialized_limit_with<T: Serialize>(
    field: &'static str,
    value: &T,
    max_bytes: usize,
) -> RecoveryStoreResult<()> {
    let len = serde_json::to_vec(value)?.len();
    if len <= max_bytes {
        return Ok(());
    }
    Err(RecoveryStoreError::ContentLimitExceeded {
        field: field.to_string(),
        limit: max_bytes,
        unit: "bytes",
    })
}

fn sanitize_visible_text(value: &str) -> String {
    value
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .chars()
        .filter(|character| {
            *character == '\n'
                || *character == '\t'
                || (!character.is_control() && *character != '\u{1b}')
        })
        .collect()
}

fn checksum_json<T: Serialize>(value: &T) -> RecoveryStoreResult<String> {
    let bytes = serde_json::to_vec(value)?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

fn operation_receipt_filename(operation_id: &str) -> String {
    format!(
        "{}.json",
        hex::encode(Sha256::digest(operation_id.as_bytes()))
    )
}

fn normalize_recovery_record(record: &mut RecoveryRecord) {
    normalize_board_entry_ids(&mut record.board_entry_ids);
}

fn normalize_board_entry_ids(entry_ids: &mut Vec<String>) {
    if entry_ids.len() > MAX_BOARD_ENTRY_IDS {
        let remove_count = entry_ids.len() - MAX_BOARD_ENTRY_IDS;
        entry_ids.drain(..remove_count);
    }
    let mut total_bytes = entry_ids.iter().map(String::len).sum::<usize>();
    let mut remove_count = 0;
    while total_bytes > MAX_BOARD_ENTRY_ID_HISTORY_BYTES && remove_count < entry_ids.len() {
        total_bytes = total_bytes.saturating_sub(entry_ids[remove_count].len());
        remove_count += 1;
    }
    if remove_count > 0 {
        entry_ids.drain(..remove_count);
    }
}

fn invalid_recovery_data(message: &'static str) -> RecoveryStoreError {
    io::Error::new(io::ErrorKind::InvalidData, message).into()
}

fn decode_continuation_transaction(
    bytes: &[u8],
) -> RecoveryStoreResult<StoredContinuationTransaction> {
    let stored: StoredContinuationTransaction = serde_json::from_slice(bytes)?;
    if checksum_json(&stored.body)? != stored.checksum {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid continuation transaction checksum",
        )
        .into());
    }
    Ok(stored)
}

fn prefixed_sha256(value: &str) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(value.as_bytes())))
}

fn validate_recovery_id(recovery_id: &str) -> RecoveryStoreResult<()> {
    let valid = !recovery_id.is_empty()
        && recovery_id.len() <= 128
        && recovery_id != "."
        && recovery_id != ".."
        && recovery_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'));
    if valid {
        Ok(())
    } else {
        Err(RecoveryStoreError::InvalidRecoveryId(
            recovery_id.to_string(),
        ))
    }
}

fn write_unique_json<T: Serialize>(
    dir: &Path,
    filename: &str,
    value: &T,
) -> RecoveryStoreResult<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    let temp_path = dir.join(format!(".{filename}.{}.tmp", Uuid::new_v4().simple()));
    let final_path = dir.join(filename);
    let mut file = open_private_new(&temp_path)?;
    file.write_all(&bytes)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temp_path, &final_path)?;
    sync_directory(dir)?;
    Ok(())
}

/// Enumerate one immutable JSON ledger without allowing directory fan-out or
/// aggregate file size to become an unbounded startup input. Entry count is
/// checked before retaining a path; file size is checked from metadata before
/// any file content is allocated or read.
fn bounded_json_paths(
    dir: &Path,
    field: &str,
    max_files: usize,
    max_file_bytes: usize,
    max_total_bytes: usize,
) -> RecoveryStoreResult<Vec<PathBuf>> {
    bounded_json_paths_with_scan(dir, field, max_files, max_file_bytes, max_total_bytes, None)
}

fn bounded_json_paths_with_scan(
    dir: &Path,
    field: &str,
    max_files: usize,
    max_file_bytes: usize,
    max_total_bytes: usize,
    scan_budget: Option<&mut RecoveryGlobalScanBudget>,
) -> RecoveryStoreResult<Vec<PathBuf>> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    let mut entries_seen = 0usize;
    let mut total_bytes = 0usize;
    let scan_budget = scan_budget
        .as_deref()
        .cloned()
        .or_else(active_recovery_scan_budget);
    for entry in fs::read_dir(dir)? {
        if let Some(budget) = scan_budget.as_ref() {
            budget.charge_entry()?;
        }
        entries_seen = entries_seen.saturating_add(1);
        if entries_seen > max_files {
            return Err(content_limit_error(field, max_files, "files"));
        }
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        if !entry.file_type()?.is_file() {
            return Err(invalid_recovery_data(
                "recovery ledger contains a non-regular JSON entry",
            ));
        }
        let byte_len = usize::try_from(entry.metadata()?.len())
            .map_err(|_| content_limit_error(field, max_file_bytes, "bytes per file"))?;
        if byte_len > max_file_bytes {
            return Err(content_limit_error(field, max_file_bytes, "bytes per file"));
        }
        total_bytes = total_bytes
            .checked_add(byte_len)
            .ok_or_else(|| content_limit_error(field, max_total_bytes, "aggregate bytes"))?;
        if total_bytes > max_total_bytes {
            return Err(content_limit_error(
                field,
                max_total_bytes,
                "aggregate bytes",
            ));
        }
        paths.push(path);
    }
    paths.sort();
    Ok(paths)
}

fn read_bounded_store_file(
    path: &Path,
    field: &str,
    max_bytes: usize,
) -> RecoveryStoreResult<Vec<u8>> {
    read_bounded_store_file_with_scan(path, field, max_bytes, None)
}

fn read_bounded_store_file_with_scan(
    path: &Path,
    field: &str,
    max_bytes: usize,
    scan_budget: Option<&mut RecoveryGlobalScanBudget>,
) -> RecoveryStoreResult<Vec<u8>> {
    let scan_budget = scan_budget
        .as_deref()
        .cloned()
        .or_else(active_recovery_scan_budget);
    let effective_max = scan_budget
        .as_ref()
        .map_or(max_bytes, |budget| max_bytes.min(budget.remaining_bytes()));
    if effective_max == 0 {
        return Err(scan_budget.as_ref().map_or_else(
            || {
                content_limit_error(
                    "global_recovery_scan",
                    MAX_GLOBAL_RECOVERY_READ_BYTES,
                    "aggregate bytes",
                )
            },
            RecoveryGlobalScanBudget::byte_limit_error,
        ));
    }
    let bytes =
        match crate::bounded_file::read_bounded_regular_file(path, effective_max as u64, field) {
            Ok(bytes) => bytes,
            Err(error)
                if effective_max < max_bytes
                    && error.kind() == io::ErrorKind::InvalidInput
                    && error.to_string().contains("size limit") =>
            {
                return Err(scan_budget.as_ref().map_or_else(
                    || {
                        content_limit_error(
                            "global_recovery_scan",
                            MAX_GLOBAL_RECOVERY_READ_BYTES,
                            "aggregate bytes",
                        )
                    },
                    RecoveryGlobalScanBudget::byte_limit_error,
                ));
            }
            Err(error)
                if error.kind() == io::ErrorKind::InvalidInput
                    && error.to_string().contains("size limit") =>
            {
                return Err(content_limit_error(field, max_bytes, "bytes per file"));
            }
            Err(error) => return Err(error.into()),
        };
    if let Some(budget) = scan_budget {
        budget.consume_bytes(bytes.len())?;
    }
    Ok(bytes)
}

struct RecoveryLedgerReadBudget {
    field: &'static str,
    limit: usize,
    remaining: usize,
}

impl RecoveryLedgerReadBudget {
    fn new(field: &'static str, limit: usize) -> Self {
        Self {
            field,
            limit,
            remaining: limit,
        }
    }

    fn read(
        &mut self,
        path: &Path,
        per_file_limit: usize,
        scan_budget: Option<&mut RecoveryGlobalScanBudget>,
    ) -> RecoveryStoreResult<Vec<u8>> {
        if self.remaining == 0 {
            return Err(content_limit_error(
                self.field,
                self.limit,
                "aggregate bytes",
            ));
        }
        let effective_max = per_file_limit.min(self.remaining);
        let bytes =
            match read_bounded_store_file_with_scan(path, self.field, effective_max, scan_budget) {
                Ok(bytes) => bytes,
                Err(RecoveryStoreError::ContentLimitExceeded {
                    ref field,
                    unit: "bytes per file",
                    ..
                }) if effective_max < per_file_limit && field == self.field => {
                    return Err(content_limit_error(
                        self.field,
                        self.limit,
                        "aggregate bytes",
                    ));
                }
                Err(error) => return Err(error),
            };
        self.remaining = self
            .remaining
            .checked_sub(bytes.len())
            .ok_or_else(|| content_limit_error(self.field, self.limit, "aggregate bytes"))?;
        Ok(bytes)
    }
}

#[derive(Clone)]
struct RecoveryGlobalScanBudget {
    state: Rc<RefCell<RecoveryGlobalScanBudgetState>>,
}

struct RecoveryGlobalScanBudgetState {
    entry_limit: usize,
    byte_limit: usize,
    remaining_entries: usize,
    remaining_bytes: usize,
}

impl Default for RecoveryGlobalScanBudget {
    fn default() -> Self {
        Self::with_limits(
            MAX_GLOBAL_RECOVERY_LEDGER_ENTRIES,
            MAX_GLOBAL_RECOVERY_READ_BYTES,
        )
    }
}

impl RecoveryGlobalScanBudget {
    fn with_limits(entries: usize, bytes: usize) -> Self {
        Self {
            state: Rc::new(RefCell::new(RecoveryGlobalScanBudgetState {
                entry_limit: entries,
                byte_limit: bytes,
                remaining_entries: entries,
                remaining_bytes: bytes,
            })),
        }
    }

    fn charge_entry(&self) -> RecoveryStoreResult<()> {
        let mut state = self.state.borrow_mut();
        state.remaining_entries = state.remaining_entries.checked_sub(1).ok_or_else(|| {
            content_limit_error("global_recovery_scan", state.entry_limit, "ledger entries")
        })?;
        Ok(())
    }

    fn remaining_bytes(&self) -> usize {
        self.state.borrow().remaining_bytes
    }

    fn consume_bytes(&self, byte_len: usize) -> RecoveryStoreResult<()> {
        let mut state = self.state.borrow_mut();
        state.remaining_bytes = state.remaining_bytes.checked_sub(byte_len).ok_or_else(|| {
            content_limit_error("global_recovery_scan", state.byte_limit, "aggregate bytes")
        })?;
        Ok(())
    }

    fn byte_limit_error(&self) -> RecoveryStoreError {
        content_limit_error(
            "global_recovery_scan",
            self.state.borrow().byte_limit,
            "aggregate bytes",
        )
    }
}

thread_local! {
    static ACTIVE_RECOVERY_SCAN_BUDGETS: RefCell<Vec<RecoveryGlobalScanBudget>> =
        const { RefCell::new(Vec::new()) };
}

struct ActiveRecoveryScanBudgetGuard;

impl Drop for ActiveRecoveryScanBudgetGuard {
    fn drop(&mut self) {
        ACTIVE_RECOVERY_SCAN_BUDGETS.with(|budgets| {
            budgets.borrow_mut().pop();
        });
    }
}

fn active_recovery_scan_budget() -> Option<RecoveryGlobalScanBudget> {
    ACTIVE_RECOVERY_SCAN_BUDGETS.with(|budgets| budgets.borrow().last().cloned())
}

fn with_active_recovery_scan_budget<T>(
    budget: &RecoveryGlobalScanBudget,
    action: impl FnOnce() -> RecoveryStoreResult<T>,
) -> RecoveryStoreResult<T> {
    ACTIVE_RECOVERY_SCAN_BUDGETS.with(|budgets| budgets.borrow_mut().push(budget.clone()));
    let _guard = ActiveRecoveryScanBudgetGuard;
    action()
}

fn content_limit_error(field: &str, limit: usize, unit: &'static str) -> RecoveryStoreError {
    RecoveryStoreError::ContentLimitExceeded {
        field: field.to_string(),
        limit,
        unit,
    }
}

fn create_private_dir_all(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn open_private_new(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

fn open_private_lock(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

fn sync_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        File::open(path)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::mpsc, time::Duration as StdDuration};

    use super::*;

    fn create_recovery(store: &RecoveryStore, recovery_id: &str, root_id: Option<&str>) {
        store
            .create(
                CreateRecovery {
                    recovery_id: recovery_id.to_string(),
                    session_id: format!("session-{recovery_id}"),
                    repo_id: "repo".to_string(),
                    session_kind: RecoverySessionKind::Intake,
                    worktree_path: PathBuf::from(format!("/tmp/{recovery_id}")),
                    launch_base_ref: None,
                    launch_base_oid: "base".to_string(),
                    launch_head_oid: "head".to_string(),
                    provider: "codex".to_string(),
                    model: None,
                    runtime: "host".to_string(),
                    initial_prompt: "Recover".to_string(),
                    created_at: Utc::now(),
                },
                format!("create-{recovery_id}"),
            )
            .unwrap();
        if let Some(root_id) = root_id {
            store
                .bind_root_semantic(
                    recovery_id,
                    root_id,
                    None,
                    BindingQuality::Verified,
                    format!("bind-{recovery_id}"),
                )
                .unwrap();
        }
    }

    #[test]
    fn recovery_directory_inventory_fails_closed_instead_of_truncating() {
        let temp = tempfile::tempdir().unwrap();
        let store = RecoveryStore::new(temp.path().join("recovery"));
        store.ensure_root_dirs().unwrap();
        for name in ["one", "two", "unrelated"] {
            fs::create_dir(store.recoveries_dir().join(name)).unwrap();
        }

        let error = store
            .recovery_ids_unlocked_with_limit(2)
            .expect_err("all recovery-directory entries must consume the scan budget");
        assert!(matches!(
            error,
            RecoveryStoreError::ContentLimitExceeded {
                ref field,
                limit: 2,
                unit: "entries"
            } if field == "recovery_directory_entries"
        ));
    }

    #[test]
    fn global_recovery_scan_shares_entry_and_actual_byte_budgets() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("ledger");
        fs::create_dir(&dir).unwrap();
        let first = dir.join("one.json");
        let second = dir.join("two.json");
        fs::write(&first, b"one").unwrap();
        fs::write(&second, b"two").unwrap();

        let mut entry_budget = RecoveryGlobalScanBudget::with_limits(1, 100);
        let entry_error =
            bounded_json_paths_with_scan(&dir, "test_ledger", 10, 10, 100, Some(&mut entry_budget))
                .expect_err("the global entry budget must span the complete ledger scan");
        assert!(matches!(
            entry_error,
            RecoveryStoreError::ContentLimitExceeded { ref field, .. }
                if field == "global_recovery_scan"
        ));

        let mut byte_budget = RecoveryGlobalScanBudget::with_limits(10, 5);
        assert_eq!(
            read_bounded_store_file_with_scan(&first, "test_ledger", 10, Some(&mut byte_budget),)
                .unwrap(),
            b"one"
        );
        let byte_error =
            read_bounded_store_file_with_scan(&second, "test_ledger", 10, Some(&mut byte_budget))
                .expect_err("actual bytes must share the global scan budget");
        assert!(matches!(
            byte_error,
            RecoveryStoreError::ContentLimitExceeded { ref field, .. }
                if field == "global_recovery_scan"
        ));

        let mut ledger_budget = RecoveryLedgerReadBudget::new("test_ledger", 5);
        assert_eq!(ledger_budget.read(&first, 10, None).unwrap(), b"one");
        let ledger_error = ledger_budget
            .read(&second, 10, None)
            .expect_err("actual bytes must also enforce each ledger's aggregate bound");
        assert!(matches!(
            ledger_error,
            RecoveryStoreError::ContentLimitExceeded { ref field, .. }
                if field == "test_ledger"
        ));
    }

    #[test]
    fn list_keeps_one_global_budget_across_successor_reconciliation_retries() {
        let temp = tempfile::tempdir().unwrap();
        let store = RecoveryStore::new(temp.path().join("recovery"));
        store.ensure_root_dirs().unwrap();

        let linked_at = Utc::now();
        for index in 0..64 {
            store
                .write_continuation_transaction_unlocked(&ContinuationTransactionBody {
                    schema_version: STORE_SCHEMA_VERSION,
                    operation_id: format!("duplicate-pair-{index}"),
                    link: RecoveryContinuationLink {
                        source_recovery_id: "same-source".to_string(),
                        target_recovery_id: "same-target".to_string(),
                        source_checkpoint_revision: 0,
                        definitive_reason: "test pending successor".to_string(),
                        linked_at,
                    },
                    inherited_state: None,
                    await_source_finalization: true,
                })
                .unwrap();
        }

        let transaction_paths = bounded_json_paths(
            &store.continuations_dir(),
            "recovery_continuation_transactions",
            MAX_RECOVERY_TRANSACTION_FILES,
            MAX_RECOVERY_TRANSACTION_BYTES,
            MAX_RECOVERY_TRANSACTION_TOTAL_BYTES,
        )
        .unwrap();
        let transaction_bytes = transaction_paths
            .iter()
            .map(|path| usize::try_from(fs::metadata(path).unwrap().len()).unwrap())
            .sum();
        let scan_budget = RecoveryGlobalScanBudget::with_limits(10_000, transaction_bytes);

        let error = with_active_recovery_scan_budget(&scan_budget, || store.list())
            .expect_err("successor retry reads must not reset the list scan budget");
        assert!(matches!(
            error,
            RecoveryStoreError::ContentLimitExceeded {
                ref field,
                limit,
                unit: "aggregate bytes"
            } if field == "global_recovery_scan" && limit == transaction_bytes
        ));
        assert_eq!(
            bounded_json_paths(
                &store.continuations_dir(),
                "recovery_continuation_transactions",
                MAX_RECOVERY_TRANSACTION_FILES,
                MAX_RECOVERY_TRANSACTION_BYTES,
                MAX_RECOVERY_TRANSACTION_TOTAL_BYTES,
            )
            .unwrap()
            .len(),
            64,
            "a failed bounded reconciliation must leave every intent retryable"
        );
    }

    #[test]
    fn attachment_publication_and_turn_commit_exclude_concurrent_global_gc() {
        let temp = tempfile::tempdir().unwrap();
        let store = RecoveryStore::new(temp.path().join("recovery"));
        create_recovery(&store, "target", Some("root-target"));
        create_recovery(&store, "purged", None);

        let (published_tx, published_rx) = mpsc::sync_channel(1);
        let (continue_tx, continue_rx) = mpsc::sync_channel(1);
        let atomic_store = store.clone();
        let atomic = std::thread::spawn(move || {
            atomic_store.record_root_turn_with_attachments_inner(
                "target",
                RootTurnUpdate {
                    root_id: "root-target".to_string(),
                    turn_id: "turn-1".to_string(),
                    input_text: Some("Keep the new attachment".to_string()),
                    visible_items: Vec::new(),
                    attachment_refs: Vec::new(),
                },
                vec![RecoveryAttachmentPayload {
                    file_name: "race.png".to_string(),
                    bytes: b"published before checkpoint".to_vec(),
                }],
                "target-turn-1".to_string(),
                || {
                    published_tx.send(()).unwrap();
                    continue_rx.recv().unwrap();
                },
            )
        });
        published_rx
            .recv()
            .expect("blob published under inventory lock");

        let (purge_started_tx, purge_started_rx) = mpsc::sync_channel(1);
        let (purged_tx, purged_rx) = mpsc::sync_channel(1);
        let purge_store = store.clone();
        let purge = std::thread::spawn(move || {
            purge_started_tx.send(()).unwrap();
            let result = purge_store.finalize_and_purge(
                "purged",
                RecoveryLifecycle::Discarded,
                Utc::now(),
                "discard-purged",
            );
            purged_tx.send(()).unwrap();
            result
        });
        purge_started_rx.recv().expect("purge thread started");
        assert!(
            purged_rx
                .recv_timeout(StdDuration::from_millis(100))
                .is_err(),
            "global GC must wait until attachment publication and turn commit finish"
        );

        continue_tx.send(()).unwrap();
        let record = atomic.join().unwrap().unwrap();
        purge.join().unwrap().unwrap();
        purged_rx
            .recv()
            .expect("purge completed after atomic commit");

        let attachment = &record
            .checkpoint
            .as_ref()
            .expect("checkpoint")
            .attachment_refs[0];
        store.verify_attachment(attachment).unwrap();
    }

    #[test]
    fn attachment_publication_and_checkpoint_commit_exclude_concurrent_global_gc() {
        let temp = tempfile::tempdir().unwrap();
        let store = RecoveryStore::new(temp.path().join("recovery"));
        create_recovery(&store, "target", Some("root-target"));
        create_recovery(&store, "purged", None);
        let source = temp.path().join("checkpoint-race.png");
        fs::write(&source, b"published before checkpoint replacement").unwrap();

        let (published_tx, published_rx) = mpsc::sync_channel(1);
        let (continue_tx, continue_rx) = mpsc::sync_channel(1);
        let atomic_store = store.clone();
        let atomic = std::thread::spawn(move || {
            atomic_store.replace_checkpoint_with_attachments_inner(
                "target",
                ("root-target", 0),
                SemanticCheckpoint {
                    summary: "Keep the checkpoint attachment".to_string(),
                    ..SemanticCheckpoint::default()
                },
                &[source],
                "target-checkpoint-1".to_string(),
                || {
                    published_tx.send(()).unwrap();
                    continue_rx.recv().unwrap();
                },
            )
        });
        published_rx
            .recv()
            .expect("blob published under inventory lock");

        let (purge_started_tx, purge_started_rx) = mpsc::sync_channel(1);
        let (purged_tx, purged_rx) = mpsc::sync_channel(1);
        let purge_store = store.clone();
        let purge = std::thread::spawn(move || {
            purge_started_tx.send(()).unwrap();
            let result = purge_store.finalize_and_purge(
                "purged",
                RecoveryLifecycle::Discarded,
                Utc::now(),
                "discard-purged",
            );
            purged_tx.send(()).unwrap();
            result
        });
        purge_started_rx.recv().expect("purge thread started");
        assert!(
            purged_rx
                .recv_timeout(StdDuration::from_millis(100))
                .is_err(),
            "global GC must wait until attachment publication and checkpoint commit finish"
        );

        continue_tx.send(()).unwrap();
        let record = atomic.join().unwrap().unwrap();
        purge.join().unwrap().unwrap();
        purged_rx
            .recv()
            .expect("purge completed after atomic checkpoint commit");

        let attachment = &record
            .checkpoint
            .as_ref()
            .expect("checkpoint")
            .attachment_refs[0];
        store.verify_attachment(attachment).unwrap();
    }

    #[test]
    fn immutable_json_ledger_enumeration_rejects_count_and_byte_overflow() {
        let temp = tempfile::tempdir().unwrap();
        for name in ["one.json", "two.json", "three.json"] {
            fs::write(temp.path().join(name), b"{}").unwrap();
        }
        let count_error = bounded_json_paths(temp.path(), "test_ledger", 2, 16, 32)
            .expect_err("third entry must exceed the pre-allocation count bound");
        assert!(matches!(
            count_error,
            RecoveryStoreError::ContentLimitExceeded { unit, .. } if unit == "files"
        ));

        fs::remove_file(temp.path().join("three.json")).unwrap();
        fs::write(temp.path().join("two.json"), [0_u8; 17]).unwrap();
        let byte_error = bounded_json_paths(temp.path(), "test_ledger", 2, 16, 32)
            .expect_err("metadata must reject an oversized file before reading it");
        assert!(matches!(
            byte_error,
            RecoveryStoreError::ContentLimitExceeded { unit, .. }
                if unit == "bytes per file"
        ));

        fs::write(temp.path().join("one.json"), [0_u8; 9]).unwrap();
        fs::write(temp.path().join("two.json"), [0_u8; 9]).unwrap();
        let total_error = bounded_json_paths(temp.path(), "test_ledger", 2, 16, 16)
            .expect_err("aggregate metadata must be bounded before either file is read");
        assert!(matches!(
            total_error,
            RecoveryStoreError::ContentLimitExceeded { unit, .. }
                if unit == "aggregate bytes"
        ));
    }

    #[test]
    fn recovery_store_ledger_readers_fail_closed_before_oversized_reads() {
        fn sparse(path: &Path, len: usize) {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            File::create(path).unwrap().set_len(len as u64).unwrap();
        }

        let temp = tempfile::tempdir().unwrap();
        let event_store = RecoveryStore::new(temp.path().join("event-store"));
        create_recovery(&event_store, "bounded", None);
        sparse(
            &event_store.events_dir("bounded").join("oversized.json"),
            MAX_RECOVERY_EVENT_BYTES + 1,
        );
        assert!(matches!(
            event_store.load("bounded"),
            Err(RecoveryStoreError::ContentLimitExceeded { .. })
        ));

        let snapshot_store = RecoveryStore::new(temp.path().join("snapshot-store"));
        create_recovery(&snapshot_store, "bounded", None);
        sparse(
            &snapshot_store
                .snapshots_dir("bounded")
                .join("oversized.json"),
            MAX_RECOVERY_SNAPSHOT_BYTES + 1,
        );
        assert!(matches!(
            snapshot_store.load("bounded"),
            Err(RecoveryStoreError::ContentLimitExceeded { .. })
        ));

        let receipt_store = RecoveryStore::new(temp.path().join("receipt-store"));
        create_recovery(&receipt_store, "bounded", None);
        sparse(
            &receipt_store.operation_receipt_path("bounded", "create-bounded"),
            MAX_RECOVERY_RECEIPT_BYTES + 1,
        );
        assert!(matches!(
            receipt_store.load("bounded"),
            Err(RecoveryStoreError::ContentLimitExceeded { .. })
        ));

        let tombstone_store = RecoveryStore::new(temp.path().join("tombstone-store"));
        sparse(
            &tombstone_store.tombstone_path("bounded"),
            MAX_RECOVERY_TOMBSTONE_BYTES + 1,
        );
        assert!(matches!(
            tombstone_store.load_tombstone("bounded"),
            Err(RecoveryStoreError::ContentLimitExceeded { .. })
        ));

        let transaction_store = RecoveryStore::new(temp.path().join("transaction-store"));
        sparse(
            &transaction_store.continuations_dir().join("oversized.json"),
            MAX_RECOVERY_TRANSACTION_BYTES + 1,
        );
        assert!(matches!(
            transaction_store.read_continuation_transactions_unlocked(),
            Err(RecoveryStoreError::ContentLimitExceeded { .. })
        ));
    }
}
