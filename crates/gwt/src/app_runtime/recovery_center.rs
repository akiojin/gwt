//! Project-scoped Recovery Center read model and action validation.

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fmt,
    path::Path,
    sync::{Mutex, OnceLock},
};

use chrono::{Duration, SecondsFormat, Utc};
use gwt::protocol::{
    RecoveryCaptureHealth, RecoveryCenterAction,
    RecoveryCenterActionRequest as PublicRecoveryCenterActionRequest,
    RecoveryCenterActionResult as PublicRecoveryCenterActionResult,
    RecoveryCenterCandidate as PublicRecoveryCenterCandidate, RecoveryCenterError,
    RecoveryCenterErrorCode, RecoveryCenterLaunchMode, RecoveryCenterProviderChoice,
    RecoveryCenterView as PublicRecoveryCenterView,
};
use gwt::WindowGeometry;
use gwt_core::recovery::{
    build_checkpoint_continuation_prompt, build_checkpoint_continuation_prompt_with_attachments,
    CheckpointCoverage, RecoveryContinuationLink, RecoveryLaunchStage, RecoveryLease,
    RecoveryLifecycle, RecoveryRecord, RecoverySessionKind, RecoveryStore, RecoveryStoreError,
    RecoveryTombstone,
};
use sha2::{Digest, Sha256};

use super::{
    launch::{launch_checkpoint_continuation_config, launch_config_from_persisted_session},
    same_worktree_path, ActiveAgentSession, AppRuntime, BackendEvent, OutboundEvent,
};

const CONTINUATION_MAX_VISIBLE_ITEMS: usize = 12;
const CONTINUATION_MAX_CHARS: usize = 12_000;
const RECOVERY_LAUNCH_FAILED_REASON: &str =
    "Recovery Center launch failed before provider readiness";
const RECOVERY_CENTER_CONTINUATION_REASON: &str =
    "Recovery Center requested durable checkpoint continuation";
const RECOVERY_CENTER_EXACT_REASON: &str = "Recovery Center requested exact provider resume";
const RECOVERY_CENTER_FRESH_REASON: &str = "Recovery Center requested fresh provider start";
const RECOVERY_CLAIM_TTL_MINUTES: i64 = 2;
const RECOVERY_PROVIDER_STOPPED_REASON: &str = "Recovery provider stopped before readiness";
const RECOVERY_PROVIDER_STOPPED_AFTER_CLAIM_LOSS_PREFIX: &str =
    "Recovery provider stopped before readiness after provider-root claim loss";
const RECOVERY_ACTION_HANDLE_PREFIX: &str = "rc1_";
const RECOVERY_PROVIDER_CHOICE_HANDLE_PREFIX: &str = "rp1_";
const RECOVERY_ACTION_HANDLE_REGISTRY_CAPACITY: usize = 4096;

/// Private request resolved from a process-scoped public action handle.
#[derive(Clone, PartialEq, Eq)]
struct RecoveryCenterActionRequest {
    action_handle: String,
    recovery_id: String,
    expected_generation: u64,
    action: RecoveryCenterAction,
    provider_root_id: Option<String>,
}

impl fmt::Debug for RecoveryCenterActionRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RecoveryCenterActionRequest")
            .field("action_handle", &"[opaque]")
            .field("recovery_id", &"[redacted]")
            .field("expected_generation", &self.expected_generation)
            .field("action", &self.action)
            .field(
                "provider_root_id",
                &self.provider_root_id.as_ref().map(|_| "[redacted]"),
            )
            .finish()
    }
}

/// Private candidate state used for eligibility and launch validation. It is
/// converted to [`PublicRecoveryCenterCandidate`] only at the WebSocket edge.
#[derive(Clone, PartialEq, Eq)]
struct RecoveryCenterCandidate {
    recovery_id: String,
    session_id: String,
    generation: u64,
    session_kind: RecoverySessionKind,
    lifecycle: RecoveryLifecycle,
    attention_required: bool,
    worktree_path: String,
    worktree_available: bool,
    launch_base_ref: Option<String>,
    launch_base_oid: String,
    launch_head_oid: String,
    provider: String,
    model: Option<String>,
    runtime: String,
    initial_prompt: String,
    provider_root_id: Option<String>,
    provider_binding_quality: Option<gwt_core::recovery::BindingQuality>,
    provider_root_candidates: Vec<gwt_core::recovery::ProviderRootCandidate>,
    checkpoint_revision: u64,
    checkpoint_coverage: CheckpointCoverage,
    last_checkpoint_at: Option<String>,
    capture_health: RecoveryCaptureHealth,
    checkpoint: Option<gwt_core::recovery::SemanticCheckpoint>,
    latest_root_input: Option<gwt_core::recovery::RootInput>,
    board_entry_ids: Vec<String>,
    board_pending: usize,
    board_delivery_error: Option<String>,
    exact_available: bool,
    exact_ambiguity: Option<String>,
    details: BTreeMap<String, String>,
    lifecycle_reason: Option<String>,
    available_actions: Vec<RecoveryCenterAction>,
    created_at: String,
    updated_at: String,
}

impl fmt::Debug for RecoveryCenterCandidate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RecoveryCenterCandidate")
            .field("recovery_id", &"[redacted]")
            .field("generation", &self.generation)
            .field("session_kind", &self.session_kind)
            .field("lifecycle", &self.lifecycle)
            .field("provider", &self.provider)
            .field("runtime", &self.runtime)
            .field("private_payload", &"[redacted]")
            .finish()
    }
}

struct RecoveryCenterView {
    project_root: String,
    attention_only: bool,
    candidates: Vec<RecoveryCenterCandidate>,
}

/// Private launch command. This type must never cross the WebSocket boundary.
#[derive(Clone, PartialEq)]
struct RecoveryCenterLaunchRequest {
    mode: RecoveryCenterLaunchMode,
    recovery_id: String,
    target_recovery_id: String,
    source_session_id: String,
    expected_generation: u64,
    session_kind: RecoverySessionKind,
    worktree_path: String,
    launch_base_ref: Option<String>,
    launch_base_oid: String,
    launch_head_oid: String,
    provider: String,
    model: Option<String>,
    runtime: String,
    provider_root_id: Option<String>,
    provider_root_claim_token: Option<String>,
    initial_prompt: String,
    continuation_prompt: Option<String>,
    checkpoint_revision: u64,
    checkpoint_coverage: CheckpointCoverage,
    checkpoint: Option<gwt_core::recovery::SemanticCheckpoint>,
    latest_root_input: Option<gwt_core::recovery::RootInput>,
    bounds: Option<WindowGeometry>,
}

impl fmt::Debug for RecoveryCenterLaunchRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RecoveryCenterLaunchRequest")
            .field("mode", &self.mode)
            .field("expected_generation", &self.expected_generation)
            .field("session_kind", &self.session_kind)
            .field("provider", &self.provider)
            .field("runtime", &self.runtime)
            .field("private_payload", &"[redacted]")
            .finish()
    }
}

#[derive(Clone, PartialEq)]
enum RecoveryCenterActionResult {
    Focus {
        recovery_id: String,
        worktree_path: String,
    },
    LaunchRequested {
        request: Box<RecoveryCenterLaunchRequest>,
    },
    OpenBoard {
        recovery_id: String,
        board_entry_ids: Vec<String>,
    },
    Details {
        candidate: Box<RecoveryCenterCandidate>,
    },
    Discarded {
        recovery_id: String,
        purged_at: String,
    },
}

impl fmt::Debug for RecoveryCenterActionResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Focus { .. } => "RecoveryCenterActionResult::Focus([redacted])",
            Self::LaunchRequested { .. } => {
                "RecoveryCenterActionResult::LaunchRequested([redacted])"
            }
            Self::OpenBoard { .. } => "RecoveryCenterActionResult::OpenBoard([redacted])",
            Self::Details { .. } => "RecoveryCenterActionResult::Details([redacted])",
            Self::Discarded { .. } => "RecoveryCenterActionResult::Discarded([redacted])",
        })
    }
}

fn recovery_handle_key() -> &'static [u8; 32] {
    static KEY: OnceLock<[u8; 32]> = OnceLock::new();
    KEY.get_or_init(|| {
        let seed = format!("{}{}", uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
        Sha256::digest(seed.as_bytes()).into()
    })
}

fn recovery_handle_registry() -> &'static Mutex<VecDeque<(String, String, u64)>> {
    static REGISTRY: OnceLock<Mutex<VecDeque<(String, String, u64)>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(VecDeque::new()))
}

fn register_recovery_action_handle(handle: &str, recovery_id: &str, generation: u64) {
    let mut registry = recovery_handle_registry()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(index) = registry
        .iter()
        .position(|(known, _, _)| constant_time_eq(known, handle))
    {
        registry.remove(index);
    }
    registry.push_back((handle.to_string(), recovery_id.to_string(), generation));
    while registry.len() > RECOVERY_ACTION_HANDLE_REGISTRY_CAPACITY {
        registry.pop_front();
    }
}

fn registered_recovery_action(handle: &str) -> Option<(String, u64)> {
    recovery_handle_registry()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .find(|(known, _, _)| constant_time_eq(known, handle))
        .map(|(_, recovery_id, generation)| (recovery_id.clone(), *generation))
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    const BLOCK_SIZE: usize = 64;
    let mut normalized = [0_u8; BLOCK_SIZE];
    if key.len() > BLOCK_SIZE {
        normalized[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        normalized[..key.len()].copy_from_slice(key);
    }
    let mut inner_pad = [0x36_u8; BLOCK_SIZE];
    let mut outer_pad = [0x5c_u8; BLOCK_SIZE];
    for index in 0..BLOCK_SIZE {
        inner_pad[index] ^= normalized[index];
        outer_pad[index] ^= normalized[index];
    }
    let mut inner = Sha256::new();
    inner.update(inner_pad);
    inner.update(message);
    let inner_digest = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(outer_pad);
    outer.update(inner_digest);
    outer.finalize().into()
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.as_bytes()
        .iter()
        .zip(right.as_bytes())
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn recovery_action_handle(recovery_id: &str, generation: u64) -> String {
    let message = format!("recovery-candidate-v1\0{recovery_id}\0{generation}");
    let handle = format!(
        "{RECOVERY_ACTION_HANDLE_PREFIX}{}",
        hex::encode(hmac_sha256(recovery_handle_key(), message.as_bytes()))
    );
    register_recovery_action_handle(&handle, recovery_id, generation);
    handle
}

fn provider_choice_handle(recovery_id: &str, generation: u64, provider_root_id: &str) -> String {
    let message =
        format!("recovery-provider-choice-v1\0{recovery_id}\0{generation}\0{provider_root_id}");
    format!(
        "{RECOVERY_PROVIDER_CHOICE_HANDLE_PREFIX}{}",
        hex::encode(hmac_sha256(recovery_handle_key(), message.as_bytes()))
    )
}

fn public_recovery_candidate(candidate: &RecoveryCenterCandidate) -> PublicRecoveryCenterCandidate {
    // Prompts, checkpoint summaries, and root inputs can contain arbitrary
    // credentials. A structural label keeps candidates distinguishable via
    // worktree/provider/time evidence without projecting user content.
    let purpose_preview = match candidate.session_kind {
        RecoverySessionKind::Intake => "Intake recovery",
        RecoverySessionKind::Execution => "Execution recovery",
    }
    .to_string();
    let worktree_name = Path::new(&candidate.worktree_path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("worktree")
        .to_string();
    let provider_choices = candidate
        .provider_root_candidates
        .iter()
        .enumerate()
        .map(|(index, provider_candidate)| RecoveryCenterProviderChoice {
            choice_handle: provider_choice_handle(
                &candidate.recovery_id,
                candidate.generation,
                &provider_candidate.root_id,
            ),
            label: format!("Candidate {}", index + 1),
            evidence_count: provider_candidate.evidence.len(),
        })
        .collect();
    let mut details = BTreeMap::from([
        (
            "Lifecycle".to_string(),
            format!("{:?}", candidate.lifecycle),
        ),
        (
            "Checkpoint revision".to_string(),
            candidate.checkpoint_revision.to_string(),
        ),
    ]);
    if let Some(quality) = candidate.provider_binding_quality {
        details.insert("Provider binding".to_string(), format!("{quality:?}"));
    }
    if candidate.details.contains_key("Worktree restoration") {
        details.insert(
            "Worktree restoration".to_string(),
            "Pinned Intake base is available".to_string(),
        );
    }
    if candidate.details.contains_key("Launch source") {
        details.insert(
            "Launch source".to_string(),
            "Source Session ledger is unavailable".to_string(),
        );
    }

    PublicRecoveryCenterCandidate {
        action_handle: recovery_action_handle(&candidate.recovery_id, candidate.generation),
        session_kind: candidate.session_kind,
        lifecycle: candidate.lifecycle,
        attention_required: candidate.attention_required,
        purpose_preview,
        worktree_name,
        worktree_available: candidate.worktree_available,
        launch_base_ref: candidate.launch_base_ref.clone(),
        provider: candidate.provider.clone(),
        model: candidate.model.clone(),
        runtime: candidate.runtime.clone(),
        provider_binding_quality: candidate.provider_binding_quality,
        provider_choices,
        checkpoint_revision: candidate.checkpoint_revision,
        checkpoint_coverage: candidate.checkpoint_coverage,
        last_checkpoint_at: candidate.last_checkpoint_at.clone(),
        capture_health: candidate.capture_health,
        board_pending: candidate.board_pending,
        board_delivery_failed: candidate.board_delivery_error.is_some(),
        exact_available: candidate.exact_available,
        exact_ambiguous: candidate.exact_ambiguity.is_some(),
        details,
        available_actions: candidate.available_actions.clone(),
        created_at: candidate.created_at.clone(),
        updated_at: candidate.updated_at.clone(),
    }
}

fn public_recovery_center(center: &RecoveryCenterView) -> PublicRecoveryCenterView {
    let project_name = Path::new(&center.project_root)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("Project")
        .to_string();
    PublicRecoveryCenterView {
        project_name,
        attention_only: center.attention_only,
        candidates: center
            .candidates
            .iter()
            .map(public_recovery_candidate)
            .collect(),
    }
}

fn resolve_public_recovery_action(
    store: &RecoveryStore,
    request: PublicRecoveryCenterActionRequest,
) -> Result<RecoveryCenterActionRequest, RecoveryCenterError> {
    let action_handle = request.action_handle.trim().to_string();
    if !action_handle.starts_with(RECOVERY_ACTION_HANDLE_PREFIX) {
        return Err(recovery_error(
            Some(action_handle),
            Some(request.action),
            RecoveryCenterErrorCode::NotFound,
            "Recovery candidate is unavailable; refresh before retrying",
        ));
    }
    let Some((recovery_id, expected_generation)) = registered_recovery_action(&action_handle)
    else {
        return Err(recovery_error(
            Some(action_handle),
            Some(request.action),
            RecoveryCenterErrorCode::StaleCandidate,
            "Recovery candidate changed; refresh before retrying",
        ));
    };
    let record = match store.load(&recovery_id).map_err(|_| {
        recovery_error(
            Some(action_handle.clone()),
            Some(request.action),
            RecoveryCenterErrorCode::Store,
            "Failed to resolve the recovery candidate",
        )
    })? {
        Some(record) if record.generation == expected_generation => record,
        None if request.action == RecoveryCenterAction::Discard
            && store.load_tombstone(&recovery_id).ok().flatten().is_some() =>
        {
            return Ok(RecoveryCenterActionRequest {
                action_handle,
                recovery_id,
                expected_generation,
                action: request.action,
                provider_root_id: None,
            });
        }
        _ => {
            return Err(recovery_error(
                Some(action_handle),
                Some(request.action),
                RecoveryCenterErrorCode::StaleCandidate,
                "Recovery candidate changed; refresh before retrying",
            ));
        }
    };

    let provider_root_id = match request.provider_choice_handle.as_deref() {
        Some(choice_handle) if request.action == RecoveryCenterAction::ConfirmResume => {
            let selected = record.provider_root_candidates.iter().find(|candidate| {
                constant_time_eq(
                    &provider_choice_handle(
                        &record.recovery_id,
                        record.generation,
                        &candidate.root_id,
                    ),
                    choice_handle,
                )
            });
            let Some(selected) = selected else {
                return Err(recovery_error(
                    Some(action_handle),
                    Some(request.action),
                    RecoveryCenterErrorCode::ActionUnavailable,
                    "Provider choice changed; refresh before retrying",
                ));
            };
            Some(selected.root_id.clone())
        }
        Some(_) => {
            return Err(recovery_error(
                Some(action_handle),
                Some(request.action),
                RecoveryCenterErrorCode::ActionUnavailable,
                "Provider choice is valid only for exact resume",
            ));
        }
        None => None,
    };

    Ok(RecoveryCenterActionRequest {
        action_handle,
        recovery_id: record.recovery_id,
        expected_generation: record.generation,
        action: request.action,
        provider_root_id,
    })
}

fn record_has_attachments(record: &RecoveryRecord) -> bool {
    record
        .checkpoint
        .as_ref()
        .is_some_and(|checkpoint| !checkpoint.attachment_refs.is_empty())
}

fn continuation_prompt_for_record(
    store: &RecoveryStore,
    record: &RecoveryRecord,
) -> Result<String, RecoveryStoreError> {
    if record.runtime != "docker" || !record_has_attachments(record) {
        return build_checkpoint_continuation_prompt_with_attachments(
            store,
            record,
            CONTINUATION_MAX_VISIBLE_ITEMS,
            CONTINUATION_MAX_CHARS,
        );
    }
    if !record.provider.eq_ignore_ascii_case("codex") {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Docker checkpoint attachments require Codex private sidecar staging",
        )
        .into());
    }
    match build_checkpoint_continuation_prompt(
        record,
        CONTINUATION_MAX_VISIBLE_ITEMS,
        CONTINUATION_MAX_CHARS,
    ) {
        Ok(prompt) => Ok(prompt),
        Err(RecoveryStoreError::NoContinuationContext { .. }) => Ok(format!(
            "Continue the interrupted {} session using its verified user-provided attachments.",
            match record.session_kind {
                RecoverySessionKind::Intake => "Intake",
                RecoverySessionKind::Execution => "Execution",
            }
        )),
        Err(error) => Err(error),
    }
}

fn load_recovery_center_from_store(
    project_root: &Path,
    store: &RecoveryStore,
) -> Result<RecoveryCenterView, RecoveryCenterError> {
    let repo_id = gwt_core::paths::project_scope_hash(project_root).to_string();
    let records = store.list().map_err(|error| {
        tracing::warn!(error = %error, "failed to load Recovery Center candidates");
        recovery_error(
            None,
            None,
            RecoveryCenterErrorCode::Store,
            "Failed to load recovery candidates",
        )
    })?;
    let candidates = records
        .into_iter()
        .filter(|record| record.repo_id == repo_id)
        .filter(|record| !is_terminal(record.lifecycle))
        .map(|record| candidate_from_record(store, record))
        .collect::<Vec<_>>();
    let attention_only = !candidates.is_empty()
        && candidates
            .iter()
            .all(|candidate| candidate.attention_required);
    Ok(RecoveryCenterView {
        project_root: project_root.to_string_lossy().into_owned(),
        attention_only,
        candidates,
    })
}

fn handle_recovery_center_action(
    store: &RecoveryStore,
    request: RecoveryCenterActionRequest,
) -> Result<RecoveryCenterActionResult, RecoveryCenterError> {
    let record = match store.load(&request.recovery_id).map_err(|error| {
        tracing::warn!(error = %error, action = request.action.as_str(), "failed to load Recovery Center candidate");
        action_error(
            &request,
            RecoveryCenterErrorCode::Store,
            "Failed to load recovery candidate",
        )
    })? {
        Some(record) => record,
        None if request.action == RecoveryCenterAction::Discard => {
            return repeat_discard_result(store, &request);
        }
        None => {
            return Err(action_error(
                &request,
                RecoveryCenterErrorCode::NotFound,
                "Recovery candidate no longer exists",
            ));
        }
    };

    if record.generation != request.expected_generation {
        return Err(action_error(
            &request,
            RecoveryCenterErrorCode::StaleCandidate,
            "Recovery candidate changed; refresh before retrying",
        ));
    }
    if is_terminal(record.lifecycle) {
        return Err(action_error(
            &request,
            RecoveryCenterErrorCode::ActionUnavailable,
            "Recovery candidate is already terminal",
        ));
    }
    if request.action == RecoveryCenterAction::Discard
        && !recovery_is_safely_discardable(store, &record).map_err(|error| {
            tracing::warn!(error = %error, "failed to verify Recovery Center discard safety");
            action_error(
                &request,
                RecoveryCenterErrorCode::Store,
                "Failed to verify recovery discard safety",
            )
        })?
    {
        return Err(action_error(
            &request,
            RecoveryCenterErrorCode::ActionUnavailable,
            "Stop the recovery provider and wait for an Interrupted or confirmed stopped state before discarding",
        ));
    }
    if matches!(
        request.action,
        RecoveryCenterAction::ConfirmResume
            | RecoveryCenterAction::ContinueCheckpoint
            | RecoveryCenterAction::StartFresh
    ) && !recovery_can_launch_without_duplicate_provider(&record)
    {
        return Err(action_error(
            &request,
            RecoveryCenterErrorCode::ActionUnavailable,
            "The previous provider spawn is not confirmed stopped; focus it or wait for durable interruption evidence before relaunching",
        ));
    }
    if matches!(
        request.action,
        RecoveryCenterAction::ConfirmResume
            | RecoveryCenterAction::ContinueCheckpoint
            | RecoveryCenterAction::StartFresh
    ) && recovery_has_active_provider_root_claim(store, &record).map_err(|error| {
        tracing::warn!(error = %error, "failed to verify Recovery Center launch ownership");
        action_error(
            &request,
            RecoveryCenterErrorCode::Store,
            "Failed to verify recovery launch ownership",
        )
    })? {
        return Err(action_error(
            &request,
            RecoveryCenterErrorCode::ActionUnavailable,
            "Another recovery launch already owns this exact provider conversation",
        ));
    }

    match request.action {
        RecoveryCenterAction::Focus => {
            require_worktree(&record, &request)?;
            Ok(RecoveryCenterActionResult::Focus {
                recovery_id: record.recovery_id,
                worktree_path: record.worktree_path.to_string_lossy().into_owned(),
            })
        }
        RecoveryCenterAction::ConfirmResume => {
            require_worktree(&record, &request)?;
            let authoritative_root_id = record
                .provider_root
                .as_ref()
                .filter(|root| root.quality.is_authoritative())
                .map(|root| root.root_id.trim())
                .filter(|root_id| !root_id.is_empty())
                .map(str::to_string);
            let (provider_root_id, confirm_candidate) = if let Some(root_id) = authoritative_root_id
            {
                if request
                    .provider_root_id
                    .as_deref()
                    .map(str::trim)
                    .is_some_and(|selected| selected != root_id)
                {
                    return Err(action_error(
                        &request,
                        RecoveryCenterErrorCode::ActionUnavailable,
                        "Selected provider root no longer matches the authoritative recovery root",
                    ));
                }
                (root_id, false)
            } else {
                let selected = request
                    .provider_root_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|root_id| !root_id.is_empty())
                    .ok_or_else(|| {
                        action_error(
                            &request,
                            RecoveryCenterErrorCode::ActionUnavailable,
                            "Choose one recorded provider root before exact resume",
                        )
                    })?;
                if !record
                    .provider_root_candidates
                    .iter()
                    .any(|candidate| candidate.root_id == selected)
                {
                    return Err(action_error(
                        &request,
                        RecoveryCenterErrorCode::ActionUnavailable,
                        "Selected provider root is not one of this recovery's recorded candidates",
                    ));
                }
                (selected.to_string(), true)
            };
            let claimed = claim_launch(
                store,
                &record,
                &request,
                "exact provider resume",
                confirm_candidate.then_some(provider_root_id.as_str()),
            )?;
            let successor = prepare_launch_successor(
                store,
                &claimed,
                RecoveryCenterLaunchMode::ConfirmResume,
                &request,
            )?;
            Ok(launch_result(
                &claimed,
                &successor,
                RecoveryCenterLaunchMode::ConfirmResume,
                Some(provider_root_id),
                false,
                None,
            ))
        }
        RecoveryCenterAction::ContinueCheckpoint => {
            require_worktree(&record, &request)?;
            let continuation_prompt = continuation_prompt(store, &record, &request)?;
            let claimed = claim_launch(store, &record, &request, "checkpoint continuation", None)?;
            let successor = prepare_launch_successor(
                store,
                &claimed,
                RecoveryCenterLaunchMode::ContinueCheckpoint,
                &request,
            )?;
            Ok(launch_result(
                &claimed,
                &successor,
                RecoveryCenterLaunchMode::ContinueCheckpoint,
                None,
                true,
                Some(continuation_prompt),
            ))
        }
        RecoveryCenterAction::StartFresh => {
            require_worktree(&record, &request)?;
            if record.initial_prompt.trim().is_empty() {
                return Err(action_error(
                    &request,
                    RecoveryCenterErrorCode::ActionUnavailable,
                    "Original initial request is unavailable",
                ));
            }
            let claimed = claim_launch(store, &record, &request, "fresh provider start", None)?;
            let successor = prepare_launch_successor(
                store,
                &claimed,
                RecoveryCenterLaunchMode::StartFresh,
                &request,
            )?;
            Ok(launch_result(
                &claimed,
                &successor,
                RecoveryCenterLaunchMode::StartFresh,
                None,
                false,
                None,
            ))
        }
        RecoveryCenterAction::OpenBoard => Ok(RecoveryCenterActionResult::OpenBoard {
            recovery_id: record.recovery_id.clone(),
            board_entry_ids: related_board_entry_ids(&record),
        }),
        RecoveryCenterAction::Details => Ok(RecoveryCenterActionResult::Details {
            candidate: Box::new(candidate_from_record(store, record)),
        }),
        RecoveryCenterAction::Discard => {
            if let Some(claim_token) = record
                .recovery_lease
                .as_ref()
                .map(|lease| lease.lease_id.as_str())
            {
                store
                    .release_provider_root_claim_for_recovery(
                        &record.recovery_id,
                        claim_token,
                        Utc::now(),
                    )
                    .map_err(|error| {
                        tracing::warn!(error = %error, "failed to release Recovery Center provider claim");
                        action_error(
                            &request,
                            RecoveryCenterErrorCode::Store,
                            "Failed to release recovery launch ownership",
                        )
                    })?;
            }
            let tombstone = store
                .finalize_and_purge(
                    &request.recovery_id,
                    RecoveryLifecycle::Discarded,
                    Utc::now(),
                    discard_operation_id(&request),
                )
                .map_err(|error| {
                    tracing::warn!(error = %error, "failed to discard Recovery Center candidate");
                    action_error(
                        &request,
                        RecoveryCenterErrorCode::Store,
                        "Failed to discard recovery candidate",
                    )
                })?;
            Ok(discard_result(tombstone))
        }
    }
}

#[cfg(test)]
pub(super) fn handle_recovery_center_action_for_test(
    store: &RecoveryStore,
    recovery_id: &str,
    expected_generation: u64,
    action: RecoveryCenterAction,
) -> Result<(), RecoveryCenterError> {
    handle_recovery_center_action(
        store,
        RecoveryCenterActionRequest {
            action_handle: recovery_action_handle(recovery_id, expected_generation),
            recovery_id: recovery_id.to_string(),
            expected_generation,
            action,
            provider_root_id: None,
        },
    )
    .map(|_| ())
}

fn recovery_successor_reason(mode: RecoveryCenterLaunchMode) -> &'static str {
    match mode {
        RecoveryCenterLaunchMode::ConfirmResume => RECOVERY_CENTER_EXACT_REASON,
        RecoveryCenterLaunchMode::ContinueCheckpoint => RECOVERY_CENTER_CONTINUATION_REASON,
        RecoveryCenterLaunchMode::StartFresh => RECOVERY_CENTER_FRESH_REASON,
    }
}

fn prepare_launch_successor(
    store: &RecoveryStore,
    record: &RecoveryRecord,
    mode: RecoveryCenterLaunchMode,
    request: &RecoveryCenterActionRequest,
) -> Result<RecoveryContinuationLink, RecoveryCenterError> {
    store
        .prepare_successor(
            RecoveryContinuationLink {
                source_recovery_id: record.recovery_id.clone(),
                target_recovery_id: uuid::Uuid::new_v4().to_string(),
                source_checkpoint_revision: record.checkpoint_revision,
                definitive_reason: recovery_successor_reason(mode).to_string(),
                linked_at: Utc::now(),
            },
            format!(
                "recovery-center-successor-v1:{}:{}:{}",
                record.recovery_id,
                record.checkpoint_revision,
                recovery_launch_mode_name(mode)
            ),
        )
        .map_err(|error| {
            tracing::warn!(error = %error, "failed to prepare Recovery Center successor");
            action_error(
                request,
                RecoveryCenterErrorCode::Store,
                "Failed to prepare recovery successor",
            )
        })
}

fn launch_result(
    record: &RecoveryRecord,
    successor: &RecoveryContinuationLink,
    mode: RecoveryCenterLaunchMode,
    provider_root_id: Option<String>,
    include_checkpoint: bool,
    continuation_prompt: Option<String>,
) -> RecoveryCenterActionResult {
    let initial_prompt = match mode {
        RecoveryCenterLaunchMode::ConfirmResume => String::new(),
        RecoveryCenterLaunchMode::ContinueCheckpoint => {
            continuation_prompt.clone().unwrap_or_default()
        }
        RecoveryCenterLaunchMode::StartFresh => record.initial_prompt.clone(),
    };
    RecoveryCenterActionResult::LaunchRequested {
        request: Box::new(RecoveryCenterLaunchRequest {
            mode,
            recovery_id: record.recovery_id.clone(),
            target_recovery_id: successor.target_recovery_id.clone(),
            source_session_id: record.session_id.clone(),
            expected_generation: record.generation,
            session_kind: record.session_kind,
            worktree_path: record.worktree_path.to_string_lossy().into_owned(),
            launch_base_ref: record.launch_base_ref.clone(),
            launch_base_oid: record.launch_base_oid.clone(),
            launch_head_oid: record.launch_head_oid.clone(),
            provider: record.provider.clone(),
            model: record.model.clone(),
            runtime: record.runtime.clone(),
            provider_root_id,
            provider_root_claim_token: (mode == RecoveryCenterLaunchMode::ConfirmResume)
                .then(|| {
                    record
                        .recovery_lease
                        .as_ref()
                        .map(|lease| lease.lease_id.clone())
                })
                .flatten(),
            initial_prompt,
            continuation_prompt,
            checkpoint_revision: record.checkpoint_revision,
            checkpoint_coverage: record.checkpoint_coverage,
            checkpoint: include_checkpoint
                .then(|| record.checkpoint.clone())
                .flatten(),
            latest_root_input: include_checkpoint
                .then(|| record.latest_root_input.clone())
                .flatten(),
            bounds: None,
        }),
    }
}

fn recovery_is_safely_discardable(
    store: &RecoveryStore,
    record: &RecoveryRecord,
) -> Result<bool, RecoveryStoreError> {
    if record.recovery_lease.is_some() {
        return Ok(false);
    }
    if recovery_has_active_provider_root_claim(store, record)? {
        return Ok(false);
    }
    let attention_is_known_stopped = record.lifecycle == RecoveryLifecycle::Attention
        && (record.launch_stage <= RecoveryLaunchStage::WorktreeMaterialized
            || recovery_has_durable_provider_stop_evidence(record));
    Ok(record.lifecycle == RecoveryLifecycle::Interrupted || attention_is_known_stopped)
}

fn recovery_has_durable_provider_stop_evidence(record: &RecoveryRecord) -> bool {
    record.lifecycle == RecoveryLifecycle::Interrupted
        || (record.lifecycle == RecoveryLifecycle::Attention
            && record.lifecycle_reason.as_deref().is_some_and(|reason| {
                reason == RECOVERY_PROVIDER_STOPPED_REASON
                    || reason.starts_with(RECOVERY_PROVIDER_STOPPED_AFTER_CLAIM_LOSS_PREFIX)
            }))
}

fn recovery_can_launch_without_duplicate_provider(record: &RecoveryRecord) -> bool {
    if record.lifecycle == RecoveryLifecycle::Running {
        return false;
    }
    if record.launch_stage >= RecoveryLaunchStage::Ready {
        return record.launch_stage == RecoveryLaunchStage::Ready
            && record.lifecycle == RecoveryLifecycle::Interrupted
            && record
                .supervisor_stop_proof
                .as_ref()
                .is_some_and(|proof| proof.session_id == record.session_id);
    }
    record.launch_stage < RecoveryLaunchStage::SpawnRequested
        || recovery_has_durable_provider_stop_evidence(record)
}

fn recovery_has_active_provider_root_claim(
    store: &RecoveryStore,
    record: &RecoveryRecord,
) -> Result<bool, RecoveryStoreError> {
    if let Some(root) = record
        .provider_root
        .as_ref()
        .filter(|root| root.quality.is_authoritative())
    {
        if store
            .active_provider_root_claim(&record.provider, &root.root_id, Utc::now())?
            .is_some()
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn candidate_from_record(store: &RecoveryStore, record: RecoveryRecord) -> RecoveryCenterCandidate {
    let worktree_available = record.worktree_path.is_dir();
    let authoritative_root = record
        .provider_root
        .as_ref()
        .is_some_and(|root| root.quality.is_authoritative() && !root.root_id.trim().is_empty());
    let exact_available = worktree_available && authoritative_root;
    let active_lease = record
        .recovery_lease
        .as_ref()
        .is_some_and(|lease| lease.expires_at > Utc::now())
        || recovery_has_active_provider_root_claim(store, &record).unwrap_or(true);
    let root_claimed_elsewhere = if let Some(root) = record
        .provider_root
        .as_ref()
        .filter(|root| root.quality.is_authoritative())
    {
        store
            .provider_root_owned_by_other_recovery(
                &record.provider,
                &root.root_id,
                &record.recovery_id,
                Utc::now(),
            )
            // A corrupt/unreadable claim ledger must not enable a launch that
            // could duplicate an authoritative provider root.
            .unwrap_or(true)
    } else if !record.provider_root_candidates.is_empty() {
        record.provider_root_candidates.iter().all(|candidate| {
            store
                .provider_root_owned_by_other_recovery(
                    &record.provider,
                    &candidate.root_id,
                    &record.recovery_id,
                    Utc::now(),
                )
                .unwrap_or(true)
        })
    } else {
        false
    };
    let attention_required = record.lifecycle == RecoveryLifecycle::Attention
        || !worktree_available
        || !authoritative_root
        || record.board_delivery_error.is_some();
    let continuation_available = continuation_prompt_for_record(store, &record).is_ok();
    let provider_launch_available = recovery_can_launch_without_duplicate_provider(&record);
    let mut available_actions = Vec::new();
    if worktree_available {
        available_actions.push(RecoveryCenterAction::Focus);
    }
    let root_confirmation_available = !record.provider_root_candidates.is_empty();
    if worktree_available
        && (authoritative_root || root_confirmation_available)
        && !active_lease
        && !root_claimed_elsewhere
        && provider_launch_available
    {
        available_actions.push(RecoveryCenterAction::ConfirmResume);
    }
    if worktree_available && continuation_available && !active_lease && provider_launch_available {
        available_actions.push(RecoveryCenterAction::ContinueCheckpoint);
    }
    if worktree_available
        && !active_lease
        && !record.initial_prompt.trim().is_empty()
        && provider_launch_available
    {
        available_actions.push(RecoveryCenterAction::StartFresh);
    }
    available_actions.extend([
        RecoveryCenterAction::OpenBoard,
        RecoveryCenterAction::Details,
    ]);
    if recovery_is_safely_discardable(store, &record).unwrap_or(false) {
        available_actions.push(RecoveryCenterAction::Discard);
    }
    let provider_root_id = record
        .provider_root
        .as_ref()
        .map(|root| root.root_id.clone());
    let provider_binding_quality = record.provider_root.as_ref().map(|root| root.quality);
    let board_entry_ids = related_board_entry_ids(&record);
    let board_pending = record.board_outbox.len();
    let board_delivery_error = record.board_delivery_error.clone();
    let last_checkpoint_at = record
        .checkpoint
        .as_ref()
        .map(|_| timestamp(record.updated_at));
    let capture_health = recovery_capture_health(&record, worktree_available);
    let exact_ambiguity = recovery_exact_ambiguity(&record, exact_available, worktree_available);
    let details = recovery_details(&record);
    RecoveryCenterCandidate {
        recovery_id: record.recovery_id,
        session_id: record.session_id,
        generation: record.generation,
        session_kind: record.session_kind,
        lifecycle: record.lifecycle,
        attention_required,
        worktree_path: record.worktree_path.to_string_lossy().into_owned(),
        worktree_available,
        launch_base_ref: record.launch_base_ref,
        launch_base_oid: record.launch_base_oid,
        launch_head_oid: record.launch_head_oid,
        provider: record.provider,
        model: record.model,
        runtime: record.runtime,
        initial_prompt: record.initial_prompt,
        provider_root_id,
        provider_binding_quality,
        provider_root_candidates: record.provider_root_candidates,
        checkpoint_revision: record.checkpoint_revision,
        checkpoint_coverage: record.checkpoint_coverage,
        last_checkpoint_at,
        capture_health,
        checkpoint: record.checkpoint,
        latest_root_input: record.latest_root_input,
        board_entry_ids,
        board_pending,
        board_delivery_error,
        exact_available,
        exact_ambiguity,
        details,
        lifecycle_reason: record.lifecycle_reason,
        available_actions,
        created_at: timestamp(record.created_at),
        updated_at: timestamp(record.updated_at),
    }
}

fn continuation_prompt(
    store: &RecoveryStore,
    record: &RecoveryRecord,
    request: &RecoveryCenterActionRequest,
) -> Result<String, RecoveryCenterError> {
    continuation_prompt_for_record(store, record).map_err(|_| {
        action_error(
            request,
            RecoveryCenterErrorCode::ActionUnavailable,
            "No user-visible continuation context is available for this candidate",
        )
    })
}

fn claim_launch(
    store: &RecoveryStore,
    record: &RecoveryRecord,
    request: &RecoveryCenterActionRequest,
    reason: &str,
    confirmed_root_id: Option<&str>,
) -> Result<RecoveryRecord, RecoveryCenterError> {
    let acquired_at = Utc::now();
    let lease_id = uuid::Uuid::new_v4().to_string();
    let lease = RecoveryLease {
        lease_id: lease_id.clone(),
        holder_id: format!("recovery-center:{}", request.action.as_str()),
        acquired_at,
        expires_at: acquired_at + Duration::minutes(RECOVERY_CLAIM_TTL_MINUTES),
    };
    let operation_id = format!(
        "recovery-center-claim-v1:{}:{}:{lease_id}",
        record.recovery_id, request.expected_generation
    );
    let interrupted_ready = record.lifecycle == RecoveryLifecycle::Interrupted
        && record.launch_stage == RecoveryLaunchStage::Ready
        && record
            .supervisor_stop_proof
            .as_ref()
            .is_some_and(|proof| proof.session_id == record.session_id);
    let claim = if request.action == RecoveryCenterAction::ConfirmResume {
        let root_id = confirmed_root_id
            .or_else(|| {
                record
                    .provider_root
                    .as_ref()
                    .map(|root| root.root_id.as_str())
            })
            .ok_or_else(|| {
                action_error(
                    request,
                    RecoveryCenterErrorCode::ActionUnavailable,
                    "Exact provider root is no longer available",
                )
            })?;
        if interrupted_ready {
            store.claim_interrupted_recovery_with_provider_root(
                &record.recovery_id,
                request.expected_generation,
                root_id,
                confirmed_root_id.is_some(),
                lease,
                &format!("recovery-center-pending:{lease_id}"),
                reason,
                operation_id,
            )
        } else {
            store.claim_recovery_with_provider_root(
                &record.recovery_id,
                request.expected_generation,
                root_id,
                confirmed_root_id.is_some(),
                lease,
                &format!("recovery-center-pending:{lease_id}"),
                reason,
                operation_id,
            )
        }
    } else if interrupted_ready {
        store.claim_interrupted_recovery(
            &record.recovery_id,
            request.expected_generation,
            lease,
            reason,
            operation_id,
        )
    } else {
        store.claim_recovery(
            &record.recovery_id,
            request.expected_generation,
            lease,
            reason,
            operation_id,
        )
    };
    claim.map_err(|error| match error {
        RecoveryStoreError::GenerationMismatch { .. } => action_error(
            request,
            RecoveryCenterErrorCode::StaleCandidate,
            "Recovery candidate changed; refresh before retrying",
        ),
        RecoveryStoreError::LeaseConflict { .. } => action_error(
            request,
            RecoveryCenterErrorCode::ActionUnavailable,
            "Another window is already recovering this candidate",
        ),
        RecoveryStoreError::ProviderRootClaimConflict { .. } => action_error(
            request,
            RecoveryCenterErrorCode::ActionUnavailable,
            "Another recovery already owns this exact provider conversation",
        ),
        RecoveryStoreError::UnknownProviderRootCandidate { .. }
        | RecoveryStoreError::RootBindingConflict { .. } => action_error(
            request,
            RecoveryCenterErrorCode::ActionUnavailable,
            "Selected provider root is no longer eligible; refresh Recovery Center",
        ),
        other => {
            tracing::warn!(error = %other, "failed to claim Recovery Center candidate");
            action_error(
                request,
                RecoveryCenterErrorCode::Store,
                "Failed to claim recovery candidate",
            )
        }
    })
}

fn recovery_launch_config(
    request: &RecoveryCenterLaunchRequest,
    source: &gwt_agent::Session,
) -> Result<gwt_agent::LaunchConfig, String> {
    validate_recovery_launch_source(request, source)?;
    let mut config = match request.mode {
        RecoveryCenterLaunchMode::ConfirmResume => {
            let provider_root_id = request
                .provider_root_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| "exact provider root is unavailable".to_string())?;
            let mut exact_source = source.clone();
            exact_source.agent_session_id = Some(provider_root_id.to_string());
            exact_source.session_mode = gwt_agent::SessionMode::Resume;
            let config = launch_config_from_persisted_session(&exact_source);
            if config.session_mode != gwt_agent::SessionMode::Resume
                || config.resume_session_id.as_deref() != Some(provider_root_id)
            {
                return Err("exact provider root could not be materialized safely".to_string());
            }
            config
        }
        RecoveryCenterLaunchMode::ContinueCheckpoint => {
            let prompt = request
                .continuation_prompt
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| "checkpoint continuation prompt is unavailable".to_string())?;
            if prompt.chars().count() > CONTINUATION_MAX_CHARS {
                return Err("checkpoint continuation prompt exceeds its safety bound".to_string());
            }
            launch_checkpoint_continuation_config(source, prompt)
        }
        RecoveryCenterLaunchMode::StartFresh => {
            let prompt = bounded_original_prompt(&request.initial_prompt, CONTINUATION_MAX_CHARS);
            if prompt.is_empty() {
                return Err("original initial request is unavailable".to_string());
            }
            launch_checkpoint_continuation_config(source, &prompt)
        }
    };
    config.recovery_continuation = Some(gwt_agent::RecoveryContinuationHandoff {
        source_session_id: request.source_session_id.clone(),
        source_recovery_id: request.recovery_id.clone(),
        target_recovery_id: request.target_recovery_id.clone(),
        source_checkpoint_revision: request.checkpoint_revision,
        reason: recovery_successor_reason(request.mode).to_string(),
        inherit_checkpoint: request.mode == RecoveryCenterLaunchMode::ContinueCheckpoint,
    });
    Ok(config)
}

fn validate_recovery_launch_source(
    request: &RecoveryCenterLaunchRequest,
    source: &gwt_agent::Session,
) -> Result<(), String> {
    if source.id != request.source_session_id {
        return Err("source Session identity no longer matches the recovery candidate".to_string());
    }
    if !same_worktree_path(&source.worktree_path, Path::new(&request.worktree_path)) {
        return Err("source Session worktree no longer matches the recovery candidate".to_string());
    }
    let source_kind = if source.is_ephemeral {
        gwt_core::recovery::RecoverySessionKind::Intake
    } else {
        gwt_core::recovery::RecoverySessionKind::Execution
    };
    if source_kind != request.session_kind {
        return Err("source Session lane no longer matches the recovery candidate".to_string());
    }
    if source.agent_id.to_string() != request.provider {
        return Err("source Session provider no longer matches the recovery candidate".to_string());
    }
    if source.model != request.model {
        return Err("source Session model no longer matches the recovery candidate".to_string());
    }
    let source_runtime = match source.runtime_target {
        gwt_agent::LaunchRuntimeTarget::Host => "host",
        gwt_agent::LaunchRuntimeTarget::Docker => "docker",
    };
    if source_runtime != request.runtime {
        return Err("source Session runtime no longer matches the recovery candidate".to_string());
    }
    Ok(())
}

fn bounded_original_prompt(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn select_live_recovery_window<F>(
    active_sessions: &std::collections::HashMap<String, ActiveAgentSession>,
    source_session_id: &str,
    worktree_path: &Path,
    mut is_live: F,
) -> Option<String>
where
    F: FnMut(&str) -> bool,
{
    active_sessions
        .iter()
        .find(|(window_id, session)| session.session_id == source_session_id && is_live(window_id))
        .or_else(|| {
            active_sessions.iter().find(|(window_id, session)| {
                same_worktree_path(&session.worktree_path, worktree_path) && is_live(window_id)
            })
        })
        .map(|(window_id, _)| window_id.clone())
}

fn recovery_capture_health(
    record: &RecoveryRecord,
    worktree_available: bool,
) -> RecoveryCaptureHealth {
    if !worktree_available {
        return RecoveryCaptureHealth::Unavailable;
    }
    match (record.checkpoint_coverage, record.checkpoint.is_some()) {
        (CheckpointCoverage::Explicit, true) => RecoveryCaptureHealth::Healthy,
        (CheckpointCoverage::Stale, true) => RecoveryCaptureHealth::Degraded,
        _ => RecoveryCaptureHealth::Unavailable,
    }
}

fn recovery_exact_ambiguity(
    record: &RecoveryRecord,
    exact_available: bool,
    worktree_available: bool,
) -> Option<String> {
    if exact_available {
        return None;
    }
    if !worktree_available {
        return Some("Recovery worktree is unavailable".to_string());
    }
    record.lifecycle_reason.clone().or_else(|| {
        Some(if record.provider_root.is_some() {
            "Provider root binding is not authoritative".to_string()
        } else {
            "Provider root binding is missing".to_string()
        })
    })
}

fn recovery_details(record: &RecoveryRecord) -> BTreeMap<String, String> {
    let mut details = BTreeMap::new();
    details.insert("Recovery ID".to_string(), record.recovery_id.clone());
    details.insert("Session ID".to_string(), record.session_id.clone());
    details.insert("Lifecycle".to_string(), format!("{:?}", record.lifecycle));
    details.insert(
        "Checkpoint revision".to_string(),
        record.checkpoint_revision.to_string(),
    );
    if let Some(root) = &record.provider_root {
        details.insert(
            "Provider root quality".to_string(),
            format!("{:?}", root.quality),
        );
    }
    if let Some(reason) = record.lifecycle_reason.as_deref() {
        if !reason.trim().is_empty() {
            details.insert(
                "Attention reason".to_string(),
                compact_visible_text(reason, 280),
            );
        }
    }
    if let Some(error) = record.board_delivery_error.as_deref() {
        if !error.trim().is_empty() {
            details.insert(
                "Board delivery error".to_string(),
                compact_visible_text(error, gwt_core::recovery::MAX_BOARD_DELIVERY_ERROR_CHARS),
            );
        }
    }
    details
}

fn compact_visible_text(value: &str, max_chars: usize) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut compact = normalized.chars().take(max_chars).collect::<String>();
    if normalized.chars().count() > max_chars {
        compact.push('…');
    }
    compact
}

fn related_board_entry_ids(record: &RecoveryRecord) -> Vec<String> {
    let mut ids = record
        .board_entry_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    ids.extend(
        record
            .board_outbox
            .iter()
            .map(|intent| intent.entry_id.clone()),
    );
    if let Some(checkpoint) = &record.checkpoint {
        ids.extend(
            checkpoint
                .board_intents
                .iter()
                .map(|intent| intent.entry_id.clone()),
        );
    }
    ids.into_iter().collect()
}

fn repeat_discard_result(
    store: &RecoveryStore,
    request: &RecoveryCenterActionRequest,
) -> Result<RecoveryCenterActionResult, RecoveryCenterError> {
    let tombstone = store
        .load_tombstone(&request.recovery_id)
        .map_err(|error| {
            tracing::warn!(error = %error, "failed to inspect discarded Recovery Center candidate");
            action_error(
                request,
                RecoveryCenterErrorCode::Store,
                "Failed to inspect discarded recovery candidate",
            )
        })?
        .ok_or_else(|| {
            action_error(
                request,
                RecoveryCenterErrorCode::NotFound,
                "Recovery candidate no longer exists",
            )
        })?;
    if tombstone.lifecycle != RecoveryLifecycle::Discarded
        || tombstone.last_generation != request.expected_generation
        || tombstone.terminal_operation_id != discard_operation_id(request)
    {
        return Err(action_error(
            request,
            RecoveryCenterErrorCode::StaleCandidate,
            "Recovery candidate was finalized by a different operation",
        ));
    }
    Ok(discard_result(tombstone))
}

fn discard_result(tombstone: RecoveryTombstone) -> RecoveryCenterActionResult {
    RecoveryCenterActionResult::Discarded {
        recovery_id: tombstone.recovery_id,
        purged_at: timestamp(tombstone.purged_at),
    }
}

fn require_worktree(
    record: &RecoveryRecord,
    request: &RecoveryCenterActionRequest,
) -> Result<(), RecoveryCenterError> {
    if record.worktree_path.is_dir() {
        Ok(())
    } else {
        Err(action_error(
            request,
            RecoveryCenterErrorCode::WorktreeUnavailable,
            "Recovery worktree is unavailable",
        ))
    }
}

fn is_terminal(lifecycle: RecoveryLifecycle) -> bool {
    matches!(
        lifecycle,
        RecoveryLifecycle::Resolved | RecoveryLifecycle::Discarded
    )
}

fn discard_operation_id(request: &RecoveryCenterActionRequest) -> String {
    format!(
        "recovery-center-discard-v1:{}:{}",
        request.recovery_id, request.expected_generation
    )
}

fn timestamp(value: chrono::DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn action_error(
    request: &RecoveryCenterActionRequest,
    code: RecoveryCenterErrorCode,
    message: impl Into<String>,
) -> RecoveryCenterError {
    recovery_error(
        Some(request.action_handle.clone()),
        Some(request.action),
        code,
        message,
    )
}

fn recovery_error(
    action_handle: Option<String>,
    action: Option<RecoveryCenterAction>,
    code: RecoveryCenterErrorCode,
    message: impl Into<String>,
) -> RecoveryCenterError {
    RecoveryCenterError {
        action_handle,
        action,
        code,
        message: message.into(),
    }
}

impl AppRuntime {
    pub(crate) fn recovery_center_events(&self, client_id: &str) -> Vec<OutboundEvent> {
        let Some(project_root) = self.active_project_root() else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::RecoveryCenterError {
                    error: recovery_error(
                        None,
                        None,
                        RecoveryCenterErrorCode::NoActiveProject,
                        "Open a project before viewing Recovery Center",
                    ),
                },
            )];
        };
        let store = RecoveryStore::for_project_dir(gwt_core::paths::gwt_project_dir_for_repo_path(
            project_root,
        ));
        match load_recovery_center_from_store(project_root, &store) {
            Ok(mut center) => {
                for candidate in &mut center.candidates {
                    self.post_process_recovery_candidate(candidate);
                }
                let center = public_recovery_center(&center);
                vec![OutboundEvent::reply(
                    client_id,
                    BackendEvent::RecoveryCenterState { center },
                )]
            }
            Err(error) => vec![OutboundEvent::reply(
                client_id,
                BackendEvent::RecoveryCenterError { error },
            )],
        }
    }

    pub(crate) fn recovery_center_action_events(
        &mut self,
        client_id: &str,
        request: PublicRecoveryCenterActionRequest,
        bounds: Option<WindowGeometry>,
    ) -> Vec<OutboundEvent> {
        let Some(project_root) = self.active_project_root().map(Path::to_path_buf) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::RecoveryCenterError {
                    error: recovery_error(
                        Some(request.action_handle),
                        Some(request.action),
                        RecoveryCenterErrorCode::NoActiveProject,
                        "Open a project before using Recovery Center",
                    ),
                },
            )];
        };
        let store = RecoveryStore::for_project_dir(gwt_core::paths::gwt_project_dir_for_repo_path(
            &project_root,
        ));
        let request = match resolve_public_recovery_action(&store, request) {
            Ok(request) => request,
            Err(error) => return vec![recovery_error_event(client_id, error)],
        };
        let action_handle = request.action_handle.clone();
        if matches!(
            request.action,
            RecoveryCenterAction::ConfirmResume
                | RecoveryCenterAction::ContinueCheckpoint
                | RecoveryCenterAction::StartFresh
        ) && store
            .load(&request.recovery_id)
            .ok()
            .flatten()
            .is_some_and(|record| {
                record.generation == request.expected_generation
                    && !recovery_source_ledger_available(&self.sessions_dir, &record.session_id)
            })
        {
            return vec![recovery_error_event(
                client_id,
                recovery_error(
                    Some(request.action_handle.clone()),
                    Some(request.action),
                    RecoveryCenterErrorCode::ActionUnavailable,
                    "The source Session ledger is unavailable; this candidate can be inspected or discarded but not launched",
                ),
            )];
        }
        if request.action == RecoveryCenterAction::Discard {
            if let Ok(Some(record)) = store.load(&request.recovery_id) {
                if self
                    .recovery_live_window_id(&record.session_id, &record.worktree_path)
                    .is_some()
                {
                    return vec![recovery_error_event(
                        client_id,
                        action_error(
                            &request,
                            RecoveryCenterErrorCode::ActionUnavailable,
                            "Stop the live recovery window before discarding this candidate",
                        ),
                    )];
                }
            }
        }
        let discarded_source_session = (request.action == RecoveryCenterAction::Discard)
            .then(|| store.load(&request.recovery_id).ok().flatten())
            .flatten()
            .map(|record| {
                (
                    record.session_id,
                    record.recovery_id,
                    record.launch_base_oid,
                )
            });
        if matches!(
            request.action,
            RecoveryCenterAction::ConfirmResume
                | RecoveryCenterAction::ContinueCheckpoint
                | RecoveryCenterAction::StartFresh
        ) {
            let record = match store.load(&request.recovery_id) {
                Ok(Some(record)) => record,
                Ok(None) => {
                    return vec![recovery_error_event(
                        client_id,
                        action_error(
                            &request,
                            RecoveryCenterErrorCode::NotFound,
                            "Recovery candidate no longer exists",
                        ),
                    )];
                }
                Err(error) => {
                    tracing::warn!(error = %error, "failed to load Recovery Center candidate for launch");
                    return vec![recovery_error_event(
                        client_id,
                        action_error(
                            &request,
                            RecoveryCenterErrorCode::Store,
                            "Failed to load recovery candidate",
                        ),
                    )];
                }
            };
            if record.generation != request.expected_generation {
                return vec![recovery_error_event(
                    client_id,
                    action_error(
                        &request,
                        RecoveryCenterErrorCode::StaleCandidate,
                        "Recovery candidate changed; refresh before retrying",
                    ),
                )];
            }
            if record.session_kind == RecoverySessionKind::Intake {
                if let Err(error) = gwt_git::recovery::ensure_recovery_base_pin(
                    &project_root,
                    &record.recovery_id,
                    &record.launch_base_oid,
                ) {
                    tracing::warn!(error = %error, "failed to pin Recovery Center launch base");
                    return vec![recovery_error_event(
                        client_id,
                        action_error(
                            &request,
                            RecoveryCenterErrorCode::WorktreeUnavailable,
                            "Recovery launch base could not be pinned safely",
                        ),
                    )];
                }
            }
            if !record.worktree_path.is_dir() {
                if let Err(error) = recreate_missing_recovery_intake(&project_root, &record) {
                    tracing::warn!(error = %error, "failed to restore Recovery Center worktree");
                    return vec![recovery_error_event(
                        client_id,
                        action_error(
                            &request,
                            RecoveryCenterErrorCode::WorktreeUnavailable,
                            "Recovery worktree could not be restored safely",
                        ),
                    )];
                }
            }
        }
        match handle_recovery_center_action(&store, request) {
            Ok(result) => {
                if let RecoveryCenterActionResult::Discarded { recovery_id, .. } = &result {
                    let cleanup_oid = discarded_source_session
                        .as_ref()
                        .map(|(_, _, oid)| oid.clone())
                        .or_else(|| {
                            store
                                .load_tombstone(recovery_id)
                                .ok()
                                .flatten()
                                .and_then(|tombstone| tombstone.launch_base_oid)
                        });
                    if let Some(launch_base_oid) = cleanup_oid.as_deref() {
                        if let Err(error) = gwt_git::recovery::remove_recovery_base_pin(
                            &project_root,
                            recovery_id,
                            launch_base_oid,
                        ) {
                            // Tombstone publication and payload purge have
                            // already completed. Keep that terminal SOT and
                            // surface the owned-ref leak for diagnosis rather
                            // than rolling back or deleting a mismatched ref.
                            tracing::warn!(
                                recovery_id,
                                error = %error,
                                "failed to remove discarded recovery base pin"
                            );
                            return vec![recovery_error_event(
                                client_id,
                                recovery_error(
                                    Some(action_handle.clone()),
                                    Some(RecoveryCenterAction::Discard),
                                    RecoveryCenterErrorCode::Store,
                                    "Recovery was discarded, but its Git base pin could not be removed safely",
                                ),
                            )];
                        }
                    }
                    if let Some((session_id, _, _)) = discarded_source_session.as_ref() {
                        if let Err(error) =
                            mark_recovery_source_session_discarded(&self.sessions_dir, session_id)
                        {
                            // The durable tombstone remains authoritative. On
                            // restart legacy import retries this Session marker
                            // before tombstone expiry/cleanup.
                            tracing::warn!(
                                session_id,
                                error = %error,
                                "failed to mark discarded recovery source Session"
                            );
                        }
                    }
                }
                self.apply_recovery_center_action(client_id, &store, result, bounds, action_handle)
            }
            Err(error) => vec![OutboundEvent::reply(
                client_id,
                BackendEvent::RecoveryCenterError { error },
            )],
        }
    }

    fn apply_recovery_center_action(
        &mut self,
        client_id: &str,
        store: &RecoveryStore,
        result: RecoveryCenterActionResult,
        bounds: Option<WindowGeometry>,
        action_handle: String,
    ) -> Vec<OutboundEvent> {
        match result {
            RecoveryCenterActionResult::Focus {
                recovery_id,
                worktree_path,
            } => self.focus_recovery_candidate(
                client_id,
                store,
                recovery_id,
                worktree_path,
                bounds,
                action_handle,
            ),
            RecoveryCenterActionResult::LaunchRequested { mut request } => {
                request.bounds = bounds;
                self.launch_recovery_candidate(client_id, store, *request, action_handle)
            }
            RecoveryCenterActionResult::Details { mut candidate } => {
                self.post_process_recovery_candidate(&mut candidate);
                vec![recovery_result_event(
                    client_id,
                    PublicRecoveryCenterActionResult::Details {
                        action_handle,
                        accepted: true,
                        candidate: Box::new(public_recovery_candidate(&candidate)),
                    },
                )]
            }
            RecoveryCenterActionResult::OpenBoard {
                board_entry_ids, ..
            } => vec![recovery_result_event(
                client_id,
                PublicRecoveryCenterActionResult::OpenBoard {
                    action_handle,
                    accepted: true,
                    board_entry_ids,
                },
            )],
            RecoveryCenterActionResult::Discarded { purged_at, .. } => {
                vec![recovery_result_event(
                    client_id,
                    PublicRecoveryCenterActionResult::Discarded {
                        action_handle,
                        accepted: true,
                        purged_at,
                    },
                )]
            }
        }
    }

    fn focus_recovery_candidate(
        &mut self,
        client_id: &str,
        store: &RecoveryStore,
        recovery_id: String,
        worktree_path: String,
        bounds: Option<WindowGeometry>,
        action_handle: String,
    ) -> Vec<OutboundEvent> {
        let record = match store.load(&recovery_id) {
            Ok(Some(record)) => record,
            Ok(None) => {
                return vec![recovery_error_event(
                    client_id,
                    recovery_error(
                        Some(action_handle.clone()),
                        Some(RecoveryCenterAction::Focus),
                        RecoveryCenterErrorCode::NotFound,
                        "Recovery candidate no longer exists",
                    ),
                )];
            }
            Err(error) => {
                tracing::warn!(error = %error, "failed to load Recovery Center focus candidate");
                return vec![recovery_error_event(
                    client_id,
                    recovery_error(
                        Some(action_handle.clone()),
                        Some(RecoveryCenterAction::Focus),
                        RecoveryCenterErrorCode::Store,
                        "Failed to load recovery candidate",
                    ),
                )];
            }
        };
        let Some(window_id) =
            self.recovery_live_window_id(&record.session_id, Path::new(&worktree_path))
        else {
            return vec![recovery_error_event(
                client_id,
                recovery_error(
                    Some(action_handle.clone()),
                    Some(RecoveryCenterAction::Focus),
                    RecoveryCenterErrorCode::ActionUnavailable,
                    "No live window is available for this recovery candidate",
                ),
            )];
        };
        let mut events = self.focus_window_events(&window_id, bounds);
        if events.is_empty() {
            return vec![recovery_error_event(
                client_id,
                recovery_error(
                    Some(action_handle.clone()),
                    Some(RecoveryCenterAction::Focus),
                    RecoveryCenterErrorCode::ActionUnavailable,
                    "Recovery window disappeared before it could be focused",
                ),
            )];
        }
        events.push(recovery_result_event(
            client_id,
            PublicRecoveryCenterActionResult::Focus {
                action_handle,
                accepted: true,
                window_id,
            },
        ));
        events
    }

    fn launch_recovery_candidate(
        &mut self,
        client_id: &str,
        store: &RecoveryStore,
        request: RecoveryCenterLaunchRequest,
        action_handle: String,
    ) -> Vec<OutboundEvent> {
        let action = recovery_action_for_launch_mode(request.mode);
        let provider_root_claim_token = request.provider_root_claim_token.clone();
        if self
            .recovery_live_window_id(
                &request.source_session_id,
                Path::new(&request.worktree_path),
            )
            .is_some()
        {
            if let Some(claim_token) = provider_root_claim_token.as_deref() {
                if let Err(error) = store.release_provider_root_claim_for_recovery(
                    &request.recovery_id,
                    claim_token,
                    Utc::now(),
                ) {
                    tracing::warn!(
                        recovery_id = %request.recovery_id,
                        error = %error,
                        "failed to release duplicate provider-root claim"
                    );
                }
            }
            if let Err(error) = store.set_lifecycle(
                &request.recovery_id,
                RecoveryLifecycle::Running,
                Some("A live window already owns this recovery".to_string()),
                format!(
                    "recovery-center-live-owner-v1:{}:{}",
                    request.recovery_id, request.expected_generation
                ),
            ) {
                tracing::warn!(
                    recovery_id = %request.recovery_id,
                    error = %error,
                    "failed to release duplicate Recovery Center claim"
                );
            }
            return vec![recovery_error_event(
                client_id,
                recovery_error(
                    Some(action_handle.clone()),
                    Some(action),
                    RecoveryCenterErrorCode::ActionUnavailable,
                    "A live window already owns this recovery candidate",
                ),
            )];
        }
        let launch = (|| -> Result<Vec<OutboundEvent>, String> {
            let source = load_recovery_source_session(&self.sessions_dir, &request)?;
            let config = recovery_launch_config(&request, &source)?;
            let bounds = request.bounds.clone().ok_or_else(|| {
                "visible canvas bounds are required to launch recovery".to_string()
            })?;
            let tab_id = self
                .active_tab_id
                .clone()
                .ok_or_else(|| "active project tab is unavailable".to_string())?;
            let existing_windows = self.window_lookup.keys().cloned().collect::<BTreeSet<_>>();
            let events = self.spawn_agent_window(&tab_id, config, bounds, None)?;
            if let Some(window_id) = self
                .window_lookup
                .keys()
                .find(|window_id| !existing_windows.contains(*window_id))
                .cloned()
            {
                self.pending_auto_resume_sources
                    .insert(window_id.clone(), source.id.clone());
                if let Some(claim_token) = provider_root_claim_token.as_ref() {
                    self.attach_pending_provider_root_claim(
                        &window_id,
                        &request.recovery_id,
                        claim_token,
                        store,
                        Duration::minutes(RECOVERY_CLAIM_TTL_MINUTES),
                    )?;
                }
            }
            Ok(events)
        })();

        match launch {
            Ok(mut events) => {
                let mode = request.mode;
                events.push(recovery_result_event(
                    client_id,
                    PublicRecoveryCenterActionResult::LaunchRequested {
                        action_handle,
                        accepted: true,
                        mode,
                    },
                ));
                events
            }
            Err(error) => {
                tracing::warn!(error = %error, "Recovery Center launch failed");
                if let Some(claim_token) = provider_root_claim_token.as_deref() {
                    if let Err(release_error) = store.release_provider_root_claim_for_recovery(
                        &request.recovery_id,
                        claim_token,
                        Utc::now(),
                    ) {
                        tracing::warn!(
                            recovery_id = %request.recovery_id,
                            error = %release_error,
                            "failed to release provider-root claim after launch failure"
                        );
                    }
                }
                let reason = format!(
                    "{RECOVERY_LAUNCH_FAILED_REASON}: {}",
                    compact_visible_text(&error, 280)
                );
                let attention_result = store.set_lifecycle(
                    &request.recovery_id,
                    RecoveryLifecycle::Attention,
                    Some(reason),
                    format!(
                        "recovery-center-launch-failed-v1:{}:{}:{}",
                        request.recovery_id,
                        request.expected_generation,
                        recovery_launch_mode_name(request.mode)
                    ),
                );
                if let Err(attention_error) = attention_result {
                    tracing::warn!(
                        recovery_id = %request.recovery_id,
                        error = %attention_error,
                        "failed to return Recovery Center candidate to Attention"
                    );
                }
                vec![recovery_error_event(
                    client_id,
                    recovery_error(
                        Some(action_handle),
                        Some(action),
                        RecoveryCenterErrorCode::ActionUnavailable,
                        "Recovery launch failed; the candidate remains available for retry",
                    ),
                )]
            }
        }
    }

    fn post_process_recovery_candidate(&self, candidate: &mut RecoveryCenterCandidate) {
        candidate
            .available_actions
            .retain(|action| *action != RecoveryCenterAction::Focus);
        let live_owner = self
            .recovery_live_window_id(&candidate.session_id, Path::new(&candidate.worktree_path))
            .is_some();
        if live_owner {
            candidate.available_actions.retain(|action| {
                !matches!(
                    action,
                    RecoveryCenterAction::ConfirmResume
                        | RecoveryCenterAction::ContinueCheckpoint
                        | RecoveryCenterAction::StartFresh
                        | RecoveryCenterAction::Discard
                )
            });
            candidate
                .available_actions
                .insert(0, RecoveryCenterAction::Focus);
        }
        if !candidate.worktree_available && !live_owner {
            if let Some(project_root) = self.active_project_root() {
                let store = RecoveryStore::for_project_dir(
                    gwt_core::paths::gwt_project_dir_for_repo_path(project_root),
                );
                if let Ok(Some(record)) = store.load(&candidate.recovery_id) {
                    if missing_recovery_intake_is_recreatable(project_root, &record).is_ok() {
                        candidate.details.insert(
                            "Worktree restoration".to_string(),
                            "Pinned Intake base can be restored at the recorded path".to_string(),
                        );
                        let active_lease = record
                            .recovery_lease
                            .as_ref()
                            .is_some_and(|lease| lease.expires_at > Utc::now())
                            || recovery_has_active_provider_root_claim(&store, &record)
                                .unwrap_or(true);
                        if !active_lease && recovery_can_launch_without_duplicate_provider(&record)
                        {
                            let exact = record.has_authoritative_root()
                                || !record.provider_root_candidates.is_empty();
                            if exact {
                                candidate
                                    .available_actions
                                    .insert(0, RecoveryCenterAction::ConfirmResume);
                                candidate.exact_available = record.has_authoritative_root();
                            }
                            if continuation_prompt_for_record(&store, &record).is_ok() {
                                let index = usize::from(exact);
                                candidate
                                    .available_actions
                                    .insert(index, RecoveryCenterAction::ContinueCheckpoint);
                            }
                            let index = candidate
                                .available_actions
                                .iter()
                                .take_while(|action| {
                                    matches!(
                                        action,
                                        RecoveryCenterAction::ConfirmResume
                                            | RecoveryCenterAction::ContinueCheckpoint
                                    )
                                })
                                .count();
                            if !candidate.initial_prompt.trim().is_empty() {
                                candidate
                                    .available_actions
                                    .insert(index, RecoveryCenterAction::StartFresh);
                            }
                        }
                    }
                }
            }
        }
        if !recovery_source_ledger_available(&self.sessions_dir, &candidate.session_id) {
            candidate.available_actions.retain(|action| {
                !matches!(
                    action,
                    RecoveryCenterAction::ConfirmResume
                        | RecoveryCenterAction::ContinueCheckpoint
                        | RecoveryCenterAction::StartFresh
                )
            });
            candidate.details.insert(
                "Launch source".to_string(),
                "Source Session ledger is unavailable".to_string(),
            );
        } else {
            candidate.details.remove("Launch source");
        }
    }

    fn recovery_live_window_id(
        &self,
        source_session_id: &str,
        worktree_path: &Path,
    ) -> Option<String> {
        select_live_recovery_window(
            &self.active_agent_sessions,
            source_session_id,
            worktree_path,
            |window_id| {
                self.window_lookup.contains_key(window_id)
                    && self.window_status(window_id).is_some_and(|status| {
                        !matches!(
                            status,
                            gwt::WindowProcessStatus::Stopped | gwt::WindowProcessStatus::Error
                        )
                    })
            },
        )
    }
}

fn missing_recovery_intake_is_recreatable(
    project_root: &Path,
    record: &RecoveryRecord,
) -> Result<(), String> {
    if record.session_kind != RecoverySessionKind::Intake {
        return Err("only an Intake recovery may recreate an ephemeral worktree".to_string());
    }
    if is_terminal(record.lifecycle) {
        return Err("terminal recovery cannot recreate a worktree".to_string());
    }
    if record
        .recovery_lease
        .as_ref()
        .is_some_and(|lease| lease.expires_at > Utc::now())
    {
        return Err("another window already owns this recovery".to_string());
    }
    let expected_repo_id = gwt_core::paths::project_scope_hash(project_root).to_string();
    if record.repo_id != expected_repo_id {
        return Err("recovery repo identity does not match the active project".to_string());
    }
    gwt_git::recovery::can_recreate_missing_intake_worktree(
        project_root,
        &record.worktree_path,
        &record.recovery_id,
        &record.launch_base_oid,
    )
    .map_err(|error| error.to_string())
}

fn recreate_missing_recovery_intake(
    project_root: &Path,
    record: &RecoveryRecord,
) -> Result<(), String> {
    missing_recovery_intake_is_recreatable(project_root, record)?;
    gwt_git::recovery::recreate_missing_intake_worktree(
        project_root,
        &record.worktree_path,
        &record.recovery_id,
        &record.launch_base_oid,
    )
    .map_err(|error| error.to_string())
}

fn recovery_source_ledger_available(sessions_dir: &Path, source_session_id: &str) -> bool {
    if source_session_id.is_empty()
        || source_session_id.len() > 128
        || !source_session_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return false;
    }
    let path = sessions_dir.join(format!("{source_session_id}.toml"));
    gwt_agent::Session::load_and_migrate(&path)
        .ok()
        .is_some_and(|session| session.id == source_session_id)
}

fn mark_recovery_source_session_discarded(
    sessions_dir: &Path,
    source_session_id: &str,
) -> Result<(), String> {
    gwt_agent::update_session(sessions_dir, source_session_id, |session| {
        session.update_status(gwt_agent::AgentStatus::Stopped);
        session.restore_window_on_startup = false;
        session
            .advance_recovery_launch_stage(gwt_agent::session::RecoveryLaunchStage::Discarded)?;
        session.recovery_lease = None;
        Ok(())
    })
    .map(|_| ())
    .map_err(|error| format!("persist discarded recovery source Session: {error}"))
}

fn load_recovery_source_session(
    sessions_dir: &Path,
    request: &RecoveryCenterLaunchRequest,
) -> Result<gwt_agent::Session, String> {
    let source_session_id = request.source_session_id.as_str();
    if source_session_id.is_empty()
        || source_session_id.len() > 128
        || !source_session_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err("recovery source Session id is invalid".to_string());
    }
    let path = sessions_dir.join(format!("{source_session_id}.toml"));
    gwt_agent::Session::load_and_migrate(&path)
        .map_err(|error| format!("load recovery source Session: {error}"))
}

fn recovery_action_for_launch_mode(mode: RecoveryCenterLaunchMode) -> RecoveryCenterAction {
    match mode {
        RecoveryCenterLaunchMode::ConfirmResume => RecoveryCenterAction::ConfirmResume,
        RecoveryCenterLaunchMode::ContinueCheckpoint => RecoveryCenterAction::ContinueCheckpoint,
        RecoveryCenterLaunchMode::StartFresh => RecoveryCenterAction::StartFresh,
    }
}

fn recovery_launch_mode_name(mode: RecoveryCenterLaunchMode) -> &'static str {
    match mode {
        RecoveryCenterLaunchMode::ConfirmResume => "confirm_resume",
        RecoveryCenterLaunchMode::ContinueCheckpoint => "continue_checkpoint",
        RecoveryCenterLaunchMode::StartFresh => "start_fresh",
    }
}

fn recovery_result_event(
    client_id: &str,
    result: PublicRecoveryCenterActionResult,
) -> OutboundEvent {
    OutboundEvent::reply(
        client_id,
        BackendEvent::RecoveryCenterActionResult { result },
    )
}

fn recovery_error_event(client_id: &str, error: RecoveryCenterError) -> OutboundEvent {
    OutboundEvent::reply(client_id, BackendEvent::RecoveryCenterError { error })
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        fs,
        path::Path,
        process::Command,
        sync::{Arc, RwLock},
    };

    use chrono::{Duration, Utc};
    use gwt::protocol::{
        BackendEvent, RecoveryCenterAction,
        RecoveryCenterActionRequest as PublicRecoveryCenterActionRequest,
        RecoveryCenterActionResult as PublicRecoveryCenterActionResult, RecoveryCenterErrorCode,
        RecoveryCenterLaunchMode,
    };
    use gwt_core::recovery::{
        BindingQuality, BoardMilestoneIntent, CreateRecovery, ProviderRootBinding,
        ProviderRootCandidate, RecoveryLaunchStage, RecoveryLifecycle, RecoverySessionKind,
        RecoveryStore, SemanticCheckpoint,
    };

    use super::{
        handle_recovery_center_action, load_recovery_center_from_store, public_recovery_center,
        recovery_action_handle, recovery_launch_config, select_live_recovery_window,
        RecoveryCenterActionRequest, RecoveryCenterActionResult, RecoveryCenterCandidate,
        RecoveryCenterLaunchRequest, RecoveryCenterView, CONTINUATION_MAX_CHARS,
    };
    use crate::{
        app_runtime::{
            persist_dispatcher::PersistDispatcher, ActiveAgentSession, AppEventProxy, AppRuntime,
            BlockingTaskSpawner, LaunchWizardMemoryCache, PendingProviderRootClaim,
            ProjectTabRuntime,
        },
        AttachmentUploadStore,
    };

    fn durable_attachment_path(
        store: &RecoveryStore,
        attachment: &gwt_core::recovery::RecoveryAttachmentRef,
    ) -> std::path::PathBuf {
        let digest = attachment.content_id.strip_prefix("sha256:").unwrap();
        store
            .root()
            .join("attachments")
            .join("sha256")
            .join(&digest[..2])
            .join(digest)
    }

    #[test]
    fn lists_multiple_nonterminal_candidates_and_derives_attention_only() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("repo");
        fs::create_dir_all(&project_root).expect("project root");
        let store = RecoveryStore::for_project_dir(temp.path().join("project-store"));
        let repo_id = gwt_core::paths::project_scope_hash(&project_root).to_string();

        create_record(&store, "exact", &project_root, &repo_id);
        bind_root(&store, "exact");
        store
            .set_lifecycle(
                "exact",
                RecoveryLifecycle::Interrupted,
                None,
                "exact:interrupt",
            )
            .expect("interrupt exact");

        create_record(&store, "attention", &project_root, &repo_id);
        store
            .set_lifecycle(
                "attention",
                RecoveryLifecycle::Attention,
                Some("ambiguous provider root".to_string()),
                "attention:mark",
            )
            .expect("mark attention");

        create_record(&store, "resolved", &project_root, &repo_id);
        store
            .set_lifecycle(
                "resolved",
                RecoveryLifecycle::Resolved,
                None,
                "resolved:mark",
            )
            .expect("resolve terminal record");

        let view = load_recovery_center_from_store(&project_root, &store).expect("load center");

        assert_eq!(view.candidates.len(), 2);
        assert!(!view.attention_only);
        let exact = candidate(&view, "exact");
        assert!(!exact.attention_required);
        assert!(exact
            .available_actions
            .contains(&RecoveryCenterAction::ConfirmResume));
        let attention = candidate(&view, "attention");
        assert!(attention.attention_required);
        assert!(!attention
            .available_actions
            .contains(&RecoveryCenterAction::ConfirmResume));

        store
            .finalize_and_purge(
                "exact",
                RecoveryLifecycle::Resolved,
                Utc::now(),
                "exact:purge",
            )
            .expect("purge exact");
        let attention_view =
            load_recovery_center_from_store(&project_root, &store).expect("reload center");
        assert_eq!(attention_view.candidates.len(), 1);
        assert!(attention_view.attention_only);
    }

    #[test]
    fn public_recovery_projection_never_serializes_private_recovery_content() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("repo-private-path");
        fs::create_dir_all(&project_root).expect("project root");
        let store = RecoveryStore::for_project_dir(temp.path().join("project-store"));
        let repo_id = gwt_core::paths::project_scope_hash(&project_root).to_string();
        create_record_with_initial_prompt(
            &store,
            "raw-recovery-secret",
            &project_root,
            &repo_id,
            "Authorization: Bearer TOP_SECRET_PROMPT_VALUE",
        );
        store
            .record_provider_root_candidates(
                "raw-recovery-secret",
                vec![ProviderRootCandidate {
                    root_id: "provider-root-secret-value".to_string(),
                    evidence: vec!["provider evidence secret value".to_string()],
                    observed_at: Utc::now(),
                }],
                "private-projection:candidates",
            )
            .expect("provider choice");
        store
            .set_board_delivery_error("raw-recovery-secret", Some("board transport secret value"))
            .expect("Board error");

        let private = load_recovery_center_from_store(&project_root, &store).expect("private view");
        let wire = serde_json::to_string(&public_recovery_center(&private)).expect("public wire");

        assert!(wire.contains("Intake recovery"));
        assert!(wire.contains("Candidate 1"));
        let private_path = project_root.to_string_lossy().into_owned();
        for private_marker in [
            "TOP_SECRET_PROMPT_VALUE",
            "raw-recovery-secret",
            "session-raw-recovery-secret",
            "provider-root-secret-value",
            "provider evidence secret value",
            "board transport secret value",
            private_path.as_str(),
        ] {
            assert!(
                !wire.contains(private_marker),
                "public Recovery Center payload exposed {private_marker}: {wire}"
            );
        }
    }

    #[test]
    fn recovery_center_surfaces_bounded_board_delivery_error_with_pending_count() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("repo");
        fs::create_dir_all(&project_root).expect("project root");
        let store = RecoveryStore::for_project_dir(temp.path().join("project-store"));
        let repo_id = gwt_core::paths::project_scope_hash(&project_root).to_string();
        create_record(&store, "board-pending", &project_root, &repo_id);
        bind_root(&store, "board-pending");
        store
            .replace_checkpoint(
                "board-pending",
                "provider-board-pending",
                0,
                checkpoint(),
                "board-pending:checkpoint",
            )
            .expect("queue Board milestone");
        let oversized = format!("Slack delivery unavailable {}", "x".repeat(900));
        store
            .set_board_delivery_error("board-pending", Some(&oversized))
            .expect("record Board diagnostic");

        let view = load_recovery_center_from_store(&project_root, &store).expect("load center");
        let pending = candidate(&view, "board-pending");
        assert!(pending.attention_required);
        assert_eq!(pending.board_pending, 1);
        let error = pending
            .board_delivery_error
            .as_deref()
            .expect("Board delivery error");
        assert!(error.starts_with("Slack delivery unavailable"));
        assert!(error.chars().count() <= gwt_core::recovery::MAX_BOARD_DELIVERY_ERROR_CHARS);
        assert_eq!(
            pending.details.get("Board delivery error"),
            Some(&error.to_string())
        );
    }

    #[test]
    fn launch_actions_return_generation_bound_bridge_requests() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("repo");
        fs::create_dir_all(&project_root).expect("project root");
        let store = RecoveryStore::for_project_dir(temp.path().join("project-store"));
        let repo_id = gwt_core::paths::project_scope_hash(&project_root).to_string();
        create_record(&store, "launch", &project_root, &repo_id);
        bind_root(&store, "launch");
        let record = store.load("launch").unwrap().unwrap();
        store
            .replace_checkpoint(
                "launch",
                "provider-launch",
                record.checkpoint_revision,
                checkpoint(),
                "launch:checkpoint",
            )
            .expect("checkpoint");
        let generation = store
            .set_lifecycle(
                "launch",
                RecoveryLifecycle::Interrupted,
                Some("provider stopped before manual recovery".to_string()),
                "launch:interrupted",
            )
            .unwrap()
            .generation;

        let confirm = handle_recovery_center_action(
            &store,
            action_request("launch", generation, RecoveryCenterAction::ConfirmResume),
        )
        .expect("confirm resume");
        let confirm = launch_request(confirm);
        assert_eq!(confirm.mode, RecoveryCenterLaunchMode::ConfirmResume);
        assert_eq!(confirm.provider_root_id.as_deref(), Some("provider-launch"));
        assert!(confirm.checkpoint.is_none());
        assert!(confirm.expected_generation > generation);
        let confirm_link = store
            .prepared_successor_for_source("launch")
            .unwrap()
            .expect("prepared exact successor");
        assert_eq!(confirm.target_recovery_id, confirm_link.target_recovery_id);

        let source = recovery_source_session(&project_root, "session-launch");
        let confirm_config =
            recovery_launch_config(&confirm, &source).expect("exact resume launch config");
        assert_eq!(confirm_config.session_mode, gwt_agent::SessionMode::Resume);
        assert_eq!(
            confirm_config.resume_session_id.as_deref(),
            Some("provider-launch")
        );
        assert_recovery_config_preserves_source(&confirm_config, &source);
        let confirm_handoff = confirm_config
            .recovery_continuation
            .as_ref()
            .expect("exact handoff");
        assert_eq!(
            confirm_handoff.target_recovery_id,
            confirm.target_recovery_id
        );
        assert_eq!(confirm_handoff.source_session_id, source.id);
        assert!(!confirm_handoff.inherit_checkpoint);
        assert_eq!(
            gwt_agent::Session::from_launch_config(&project_root, "", &confirm_config).recovery_id,
            Some(confirm.target_recovery_id.clone())
        );

        create_record(&store, "launch-checkpoint", &project_root, &repo_id);
        bind_root(&store, "launch-checkpoint");
        let checkpoint_record = store.load("launch-checkpoint").unwrap().unwrap();
        store
            .replace_checkpoint(
                "launch-checkpoint",
                "provider-launch-checkpoint",
                checkpoint_record.checkpoint_revision,
                checkpoint(),
                "launch-checkpoint:checkpoint",
            )
            .expect("checkpoint continuation checkpoint");
        let checkpoint_generation = store
            .set_lifecycle(
                "launch-checkpoint",
                RecoveryLifecycle::Interrupted,
                Some("provider stopped before checkpoint continuation".to_string()),
                "launch-checkpoint:interrupted",
            )
            .unwrap()
            .generation;

        let checkpoint = handle_recovery_center_action(
            &store,
            action_request(
                "launch-checkpoint",
                checkpoint_generation,
                RecoveryCenterAction::ContinueCheckpoint,
            ),
        )
        .expect("continue checkpoint");
        let checkpoint = launch_request(checkpoint);
        assert_eq!(
            checkpoint.mode,
            RecoveryCenterLaunchMode::ContinueCheckpoint
        );
        assert_eq!(checkpoint.provider_root_id, None);
        let continuation_prompt = checkpoint
            .continuation_prompt
            .as_deref()
            .expect("bounded continuation prompt");
        assert!(!continuation_prompt.trim().is_empty());
        assert!(continuation_prompt.chars().count() <= CONTINUATION_MAX_CHARS);
        assert_eq!(checkpoint.initial_prompt, continuation_prompt);
        assert_eq!(
            checkpoint
                .checkpoint
                .as_ref()
                .map(|checkpoint| checkpoint.summary.as_str()),
            Some("Recovered discussion")
        );
        let checkpoint_link = store
            .prepared_successor_for_source("launch-checkpoint")
            .unwrap()
            .expect("prepared checkpoint successor");
        assert_eq!(
            checkpoint.target_recovery_id,
            checkpoint_link.target_recovery_id
        );
        let checkpoint_source = recovery_source_session(&project_root, "session-launch-checkpoint");
        let checkpoint_config = recovery_launch_config(&checkpoint, &checkpoint_source)
            .expect("checkpoint launch config");
        assert_eq!(
            checkpoint_config.session_mode,
            gwt_agent::SessionMode::Normal
        );
        assert_eq!(
            checkpoint_config.initial_prompt.as_deref(),
            checkpoint.continuation_prompt.as_deref()
        );
        assert!(checkpoint_config.resume_session_id.is_none());
        assert_recovery_config_preserves_source(&checkpoint_config, &checkpoint_source);
        let checkpoint_handoff = checkpoint_config
            .recovery_continuation
            .as_ref()
            .expect("checkpoint handoff");
        assert_eq!(
            checkpoint_handoff.target_recovery_id,
            checkpoint.target_recovery_id
        );
        assert_eq!(checkpoint_handoff.source_session_id, checkpoint_source.id);
        assert!(checkpoint_handoff.inherit_checkpoint);

        create_record(&store, "launch-fresh", &project_root, &repo_id);
        let fresh_generation = store.load("launch-fresh").unwrap().unwrap().generation;

        let fresh = handle_recovery_center_action(
            &store,
            action_request(
                "launch-fresh",
                fresh_generation,
                RecoveryCenterAction::StartFresh,
            ),
        )
        .expect("start fresh");
        let fresh = launch_request(fresh);
        assert_eq!(fresh.mode, RecoveryCenterLaunchMode::StartFresh);
        assert_eq!(fresh.provider_root_id, None);
        assert_eq!(fresh.checkpoint, None);
        let fresh_link = store
            .prepared_successor_for_source("launch-fresh")
            .unwrap()
            .expect("prepared fresh successor");
        assert_eq!(fresh.target_recovery_id, fresh_link.target_recovery_id);
        let fresh_source = recovery_source_session(&project_root, "session-launch-fresh");
        let fresh_config =
            recovery_launch_config(&fresh, &fresh_source).expect("fresh launch config");
        assert_eq!(fresh_config.session_mode, gwt_agent::SessionMode::Normal);
        assert_eq!(
            fresh_config.initial_prompt.as_deref(),
            Some("Investigate interrupted Intake")
        );
        assert!(fresh_config.resume_session_id.is_none());
        assert_recovery_config_preserves_source(&fresh_config, &fresh_source);
        let fresh_handoff = fresh_config
            .recovery_continuation
            .as_ref()
            .expect("fresh handoff");
        assert_eq!(fresh_handoff.target_recovery_id, fresh.target_recovery_id);
        assert_eq!(fresh_handoff.source_session_id, fresh_source.id);
        assert!(!fresh_handoff.inherit_checkpoint);

        let stale = handle_recovery_center_action(
            &store,
            action_request(
                "launch",
                generation.saturating_sub(1),
                RecoveryCenterAction::ConfirmResume,
            ),
        )
        .expect_err("stale action must fail");
        assert_eq!(stale.code, RecoveryCenterErrorCode::StaleCandidate);
    }

    #[test]
    fn ambiguous_provider_roots_require_an_explicit_candidate_and_bind_only_that_root() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("repo");
        fs::create_dir_all(&project_root).expect("project root");
        let store = RecoveryStore::for_project_dir(temp.path().join("project-store"));
        let repo_id = gwt_core::paths::project_scope_hash(&project_root).to_string();
        create_record(&store, "ambiguous", &project_root, &repo_id);
        let observed_at = Utc::now();
        store
            .record_provider_root_candidates(
                "ambiguous",
                vec![
                    ProviderRootCandidate {
                        root_id: "provider-a".to_string(),
                        evidence: vec!["Current session id".to_string()],
                        observed_at,
                    },
                    ProviderRootCandidate {
                        root_id: "provider-b".to_string(),
                        evidence: vec!["Provider history".to_string()],
                        observed_at,
                    },
                ],
                "ambiguous:candidates",
            )
            .expect("record candidates");
        let record = store
            .set_lifecycle(
                "ambiguous",
                RecoveryLifecycle::Attention,
                Some("multiple provider roots".to_string()),
                "ambiguous:attention",
            )
            .expect("mark attention");

        let view = load_recovery_center_from_store(&project_root, &store).expect("load center");
        let candidate = candidate(&view, "ambiguous");
        assert!(!candidate.exact_available);
        assert_eq!(candidate.provider_root_candidates.len(), 2);
        assert!(candidate
            .available_actions
            .contains(&RecoveryCenterAction::ConfirmResume));

        let missing = handle_recovery_center_action(
            &store,
            action_request(
                "ambiguous",
                record.generation,
                RecoveryCenterAction::ConfirmResume,
            ),
        )
        .expect_err("ambiguous exact resume requires a selection");
        assert_eq!(missing.code, RecoveryCenterErrorCode::ActionUnavailable);

        let unknown = handle_recovery_center_action(
            &store,
            action_request_with_root(
                "ambiguous",
                record.generation,
                RecoveryCenterAction::ConfirmResume,
                "provider-unknown",
            ),
        )
        .expect_err("selection must belong to the durable candidate set");
        assert_eq!(unknown.code, RecoveryCenterErrorCode::ActionUnavailable);

        let result = handle_recovery_center_action(
            &store,
            action_request_with_root(
                "ambiguous",
                record.generation,
                RecoveryCenterAction::ConfirmResume,
                "provider-b",
            ),
        )
        .expect("confirm one provider root");
        let launch = launch_request(result);
        assert_eq!(launch.provider_root_id.as_deref(), Some("provider-b"));
        let claimed = store.load("ambiguous").unwrap().unwrap();
        assert_eq!(
            claimed
                .provider_root
                .as_ref()
                .map(|root| root.root_id.as_str()),
            Some("provider-b")
        );
        assert_eq!(
            claimed.provider_root.as_ref().map(|root| root.quality),
            Some(BindingQuality::Confirmed)
        );
        assert_eq!(claimed.provider_root_candidates.len(), 2);
        assert!(claimed.recovery_lease.is_some());
        assert!(claimed.generation > record.generation);
    }

    #[test]
    fn launch_action_claims_one_generation_and_blocks_a_second_client() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("repo");
        fs::create_dir_all(&project_root).expect("project root");
        let store = RecoveryStore::for_project_dir(temp.path().join("project-store"));
        let repo_id = gwt_core::paths::project_scope_hash(&project_root).to_string();
        create_record(&store, "claimed", &project_root, &repo_id);
        bind_root(&store, "claimed");
        interrupt_after_supervisor_stop(&store, "claimed");
        let generation = store.load("claimed").unwrap().unwrap().generation;
        let action = action_request("claimed", generation, RecoveryCenterAction::ConfirmResume);

        let first = handle_recovery_center_action(&store, action.clone()).expect("first claim");
        let launch = launch_request(first);
        assert!(launch.expected_generation > generation);
        assert!(launch.provider_root_claim_token.is_some());
        let claimed = store.load("claimed").unwrap().unwrap();
        assert_eq!(claimed.generation, launch.expected_generation);
        assert_eq!(claimed.lifecycle, RecoveryLifecycle::Recovering);
        assert!(claimed.recovery_lease.is_some());

        let second = handle_recovery_center_action(&store, action)
            .expect_err("the rendered generation may launch only once");
        assert_eq!(second.code, RecoveryCenterErrorCode::StaleCandidate);
        let view = load_recovery_center_from_store(&project_root, &store).expect("claimed view");
        let candidate = candidate(&view, "claimed");
        assert!(!candidate
            .available_actions
            .contains(&RecoveryCenterAction::ConfirmResume));
        assert!(!candidate
            .available_actions
            .contains(&RecoveryCenterAction::ContinueCheckpoint));
        assert!(!candidate
            .available_actions
            .contains(&RecoveryCenterAction::StartFresh));
    }

    #[test]
    fn manual_exact_resume_disables_and_rejects_a_second_record_for_same_root() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("repo");
        fs::create_dir_all(&project_root).expect("project root");
        let store = RecoveryStore::for_project_dir(temp.path().join("project-store"));
        let repo_id = gwt_core::paths::project_scope_hash(&project_root).to_string();
        for recovery_id in ["manual-first", "manual-second"] {
            create_record(&store, recovery_id, &project_root, &repo_id);
            store
                .bind_root(
                    recovery_id,
                    ProviderRootBinding {
                        root_id: "shared-manual-root".to_string(),
                        session_tree_id: None,
                        quality: BindingQuality::Verified,
                        bound_at: Utc::now(),
                    },
                    format!("{recovery_id}:bind-shared"),
                )
                .expect("bind shared exact root");
            interrupt_after_supervisor_stop(&store, recovery_id);
        }
        let first_generation = store.load("manual-first").unwrap().unwrap().generation;
        let first_launch = handle_recovery_center_action(
            &store,
            action_request(
                "manual-first",
                first_generation,
                RecoveryCenterAction::ConfirmResume,
            ),
        )
        .expect("first manual exact claim");
        let first_claim_token = launch_request(first_launch)
            .provider_root_claim_token
            .expect("first provider-root claim token");

        let view = load_recovery_center_from_store(&project_root, &store).expect("load center");
        let second = candidate(&view, "manual-second");
        assert!(!second
            .available_actions
            .contains(&RecoveryCenterAction::ConfirmResume));

        let error = handle_recovery_center_action(
            &store,
            action_request(
                "manual-second",
                second.generation,
                RecoveryCenterAction::ConfirmResume,
            ),
        )
        .expect_err("second record may not launch the same provider root");
        assert_eq!(error.code, RecoveryCenterErrorCode::ActionUnavailable);
        assert!(error.message.contains("already owns"));

        store
            .advance_launch_stage(
                "manual-first",
                gwt_core::recovery::RecoveryLaunchStage::Ready,
                Some(gwt_core::recovery::ProviderRootRole::Root),
                "manual-first:ready",
            )
            .expect("mark first provider ready");
        assert!(store
            .release_provider_root_claim_for_recovery(
                "manual-first",
                &first_claim_token,
                Utc::now(),
            )
            .expect("release launch claim at Ready"));
        store
            .set_lifecycle(
                "manual-first",
                RecoveryLifecycle::Running,
                None,
                "manual-first:running",
            )
            .expect("mark first running");
        let ready_view =
            load_recovery_center_from_store(&project_root, &store).expect("ready owner view");
        assert!(!candidate(&ready_view, "manual-second")
            .available_actions
            .contains(&RecoveryCenterAction::ConfirmResume));

        store
            .finalize_and_purge(
                "manual-first",
                RecoveryLifecycle::Resolved,
                Utc::now(),
                "manual-first:resolved",
            )
            .expect("resolve first owner");
        let terminal_view =
            load_recovery_center_from_store(&project_root, &store).expect("terminal owner view");
        assert!(candidate(&terminal_view, "manual-second")
            .available_actions
            .contains(&RecoveryCenterAction::ConfirmResume));
    }

    #[test]
    fn checkpoint_action_uses_verified_durable_attachment_copies() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("repo");
        fs::create_dir_all(&project_root).expect("project root");
        let store = RecoveryStore::for_project_dir(temp.path().join("project-store"));
        let repo_id = gwt_core::paths::project_scope_hash(&project_root).to_string();
        create_record(&store, "attachment", &project_root, &repo_id);
        bind_root(&store, "attachment");
        interrupt_after_supervisor_stop(&store, "attachment");
        let external_dir = temp.path().join("external");
        fs::create_dir_all(&external_dir).expect("external dir");
        let source = external_dir.join("board-gap.png");
        fs::write(&source, b"board screenshot").expect("source attachment");
        let attachment = store.copy_attachment(&source).expect("copy attachment");
        let record = store.load("attachment").unwrap().unwrap();
        let record = store
            .replace_checkpoint(
                "attachment",
                "provider-attachment",
                record.checkpoint_revision,
                SemanticCheckpoint {
                    summary: "The Board stopped at Jul 13.".to_string(),
                    attachment_refs: vec![attachment.clone()],
                    ..SemanticCheckpoint::default()
                },
                "attachment:checkpoint",
            )
            .expect("checkpoint attachment");
        fs::remove_file(&source).expect("remove external source");

        let result = handle_recovery_center_action(
            &store,
            action_request(
                "attachment",
                record.generation,
                RecoveryCenterAction::ContinueCheckpoint,
            ),
        )
        .expect("continue with copied attachment");
        let prompt = launch_request(result)
            .continuation_prompt
            .expect("continuation prompt");
        let durable = durable_attachment_path(&store, &attachment);
        assert!(prompt.contains(&durable.to_string_lossy().to_string()));
        assert!(prompt.contains("board-gap.png"));
        assert!(!prompt.contains(&external_dir.to_string_lossy().to_string()));

        fs::write(&durable, b"tampered").expect("corrupt durable blob");
        let view = load_recovery_center_from_store(&project_root, &store).expect("load center");
        assert!(!candidate(&view, "attachment")
            .available_actions
            .contains(&RecoveryCenterAction::ContinueCheckpoint));
    }

    #[test]
    fn docker_checkpoint_action_never_places_host_attachment_path_in_prompt() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("repo");
        fs::create_dir_all(&project_root).expect("project root");
        let store = RecoveryStore::for_project_dir(temp.path().join("project-store"));
        let repo_id = gwt_core::paths::project_scope_hash(&project_root).to_string();
        store
            .create(
                CreateRecovery {
                    recovery_id: "docker-attachment".to_string(),
                    session_id: "session-docker-attachment".to_string(),
                    repo_id,
                    session_kind: RecoverySessionKind::Intake,
                    worktree_path: project_root.clone(),
                    launch_base_ref: Some("origin/develop".to_string()),
                    launch_base_oid: "1".repeat(40),
                    launch_head_oid: "2".repeat(40),
                    provider: "Codex".to_string(),
                    model: Some("gpt-5".to_string()),
                    runtime: "docker".to_string(),
                    initial_prompt: "Review Docker evidence".to_string(),
                    created_at: Utc::now(),
                },
                "docker-attachment:create",
            )
            .expect("create Docker recovery");
        bind_root(&store, "docker-attachment");
        interrupt_after_supervisor_stop(&store, "docker-attachment");
        let attachment = store
            .copy_attachment_bytes("docker-evidence.png", b"docker evidence")
            .expect("copy attachment");
        let record = store.load("docker-attachment").unwrap().unwrap();
        let record = store
            .replace_checkpoint(
                "docker-attachment",
                "provider-docker-attachment",
                record.checkpoint_revision,
                SemanticCheckpoint {
                    summary: "Continue the Docker-based evidence review.".to_string(),
                    attachment_refs: vec![attachment.clone()],
                    ..SemanticCheckpoint::default()
                },
                "docker-attachment:checkpoint",
            )
            .expect("checkpoint Docker attachment");
        let host_blob = durable_attachment_path(&store, &attachment)
            .to_string_lossy()
            .into_owned();

        let result = handle_recovery_center_action(
            &store,
            action_request(
                "docker-attachment",
                record.generation,
                RecoveryCenterAction::ContinueCheckpoint,
            ),
        )
        .expect("continue Docker checkpoint");
        let request = launch_request(result);
        let prompt = request
            .continuation_prompt
            .as_deref()
            .expect("path-free seed prompt");
        assert!(!prompt.contains(&host_blob));
        assert!(!prompt.contains("/tmp/gwt-codex-recovery-"));

        let mut source = recovery_source_session(&project_root, "session-docker-attachment");
        source.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;
        let config = recovery_launch_config(&request, &source).expect("Docker launch config");
        let handoff = config
            .recovery_continuation
            .as_ref()
            .expect("continuation provenance");
        assert_eq!(handoff.source_recovery_id, "docker-attachment");
        assert_eq!(
            handoff.source_checkpoint_revision,
            record.checkpoint_revision
        );
        assert!(!config
            .initial_prompt
            .as_deref()
            .unwrap()
            .contains(&host_blob));
    }

    #[test]
    fn live_recovery_window_prefers_source_session_then_same_worktree() {
        let temp = tempfile::tempdir().expect("tempdir");
        let worktree = temp.path().join("repo");
        fs::create_dir_all(&worktree).expect("worktree");
        let mut sessions = HashMap::new();
        sessions.insert(
            "window-worktree".to_string(),
            active_session("window-worktree", "different-source", &worktree),
        );
        sessions.insert(
            "window-source".to_string(),
            active_session("window-source", "source-session", &worktree),
        );

        assert_eq!(
            select_live_recovery_window(&sessions, "source-session", &worktree, |_| true)
                .as_deref(),
            Some("window-source")
        );
        assert_eq!(
            select_live_recovery_window(&sessions, "missing-source", &worktree, |window_id| {
                window_id == "window-worktree"
            })
            .as_deref(),
            Some("window-worktree")
        );
        assert!(
            select_live_recovery_window(&sessions, "source-session", &worktree, |_| false)
                .is_none()
        );
    }

    #[test]
    fn app_runtime_focuses_only_a_live_recovery_window() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("repo");
        fs::create_dir_all(&project_root).expect("project root");
        let store = RecoveryStore::for_project_dir(temp.path().join("project-store"));
        let repo_id = gwt_core::paths::project_scope_hash(&project_root).to_string();
        create_record(&store, "focus", &project_root, &repo_id);
        let generation = store.load("focus").unwrap().unwrap().generation;
        let (mut runtime, window_id) = test_runtime(temp.path(), &project_root, true);
        let window_id = window_id.expect("agent window");
        runtime.active_agent_sessions.insert(
            window_id.clone(),
            active_session(&window_id, "session-focus", &project_root),
        );
        runtime
            .window_pty_statuses
            .insert(window_id.clone(), gwt::WindowProcessStatus::Running);

        let mut candidate = load_recovery_center_from_store(&project_root, &store)
            .expect("view")
            .candidates
            .remove(0);
        runtime.post_process_recovery_candidate(&mut candidate);
        assert!(candidate
            .available_actions
            .contains(&RecoveryCenterAction::Focus));
        assert!(!candidate
            .available_actions
            .contains(&RecoveryCenterAction::StartFresh));
        assert!(!candidate
            .available_actions
            .contains(&RecoveryCenterAction::ConfirmResume));

        let result = handle_recovery_center_action(
            &store,
            action_request("focus", generation, RecoveryCenterAction::Focus),
        )
        .expect("focus action");
        let events = runtime.apply_recovery_center_action(
            "client-1",
            &store,
            result,
            Some(canvas_bounds()),
            recovery_action_handle("focus", generation),
        );
        assert!(events.iter().any(|event| matches!(
            event.event,
            BackendEvent::RecoveryCenterActionResult {
                result: PublicRecoveryCenterActionResult::Focus { .. }
            }
        )));
        assert!(events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. })));

        let generation = store.load("focus").unwrap().unwrap().generation;
        let launch_result = handle_recovery_center_action(
            &store,
            action_request("focus", generation, RecoveryCenterAction::StartFresh),
        )
        .expect("claim duplicate launch attempt");
        let duplicate = runtime.apply_recovery_center_action(
            "client-1",
            &store,
            launch_result,
            Some(canvas_bounds()),
            recovery_action_handle("focus", generation),
        );
        assert!(matches!(
            duplicate.as_slice(),
            [event]
                if matches!(
                    &event.event,
                    BackendEvent::RecoveryCenterError { error }
                        if error.code == RecoveryCenterErrorCode::ActionUnavailable
                )
        ));
        let after_duplicate = store.load("focus").unwrap().unwrap();
        assert_eq!(after_duplicate.lifecycle, RecoveryLifecycle::Running);
        assert!(after_duplicate.recovery_lease.is_none());
        let generation = after_duplicate.generation;

        runtime.active_agent_sessions.clear();
        runtime.post_process_recovery_candidate(&mut candidate);
        assert!(!candidate
            .available_actions
            .contains(&RecoveryCenterAction::Focus));
        let result = handle_recovery_center_action(
            &store,
            action_request("focus", generation, RecoveryCenterAction::Focus),
        )
        .expect("validated focus action");
        let unavailable = runtime.apply_recovery_center_action(
            "client-1",
            &store,
            result,
            Some(canvas_bounds()),
            recovery_action_handle("focus", generation),
        );
        assert!(matches!(
            unavailable.as_slice(),
            [event]
                if matches!(
                    &event.event,
                    BackendEvent::RecoveryCenterError { error }
                        if error.code == RecoveryCenterErrorCode::ActionUnavailable
                )
        ));
    }

    #[test]
    fn app_runtime_launch_registers_source_and_marks_sync_failure_attention() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("repo");
        fs::create_dir_all(&project_root).expect("project root");
        let store = RecoveryStore::for_project_dir(temp.path().join("project-store"));
        let repo_id = gwt_core::paths::project_scope_hash(&project_root).to_string();
        create_record(&store, "launch", &project_root, &repo_id);
        let generation = store.load("launch").unwrap().unwrap().generation;
        let (mut runtime, _) = test_runtime(temp.path(), &project_root, false);
        recovery_source_session(&project_root, "session-launch")
            .save(&runtime.sessions_dir)
            .expect("save source Session");

        let result = handle_recovery_center_action(
            &store,
            action_request("launch", generation, RecoveryCenterAction::StartFresh),
        )
        .expect("fresh action");
        let events = runtime.apply_recovery_center_action(
            "client-1",
            &store,
            result,
            Some(canvas_bounds()),
            recovery_action_handle("launch", generation),
        );
        assert!(events.iter().any(|event| matches!(
            event.event,
            BackendEvent::RecoveryCenterActionResult {
                result: PublicRecoveryCenterActionResult::LaunchRequested { .. }
            }
        )));
        assert!(runtime
            .pending_auto_resume_sources
            .values()
            .any(|source_session_id| source_session_id == "session-launch"));

        create_record(&store, "failure", &project_root, &repo_id);
        recovery_source_session(&project_root, "session-failure")
            .save(&runtime.sessions_dir)
            .expect("save failure source Session");
        let failure_generation = store.load("failure").unwrap().unwrap().generation;
        let result = handle_recovery_center_action(
            &store,
            action_request(
                "failure",
                failure_generation,
                RecoveryCenterAction::StartFresh,
            ),
        )
        .expect("fresh failure action");
        let failed = runtime.apply_recovery_center_action(
            "client-1",
            &store,
            result,
            None,
            recovery_action_handle("failure", failure_generation),
        );
        assert!(matches!(
            failed.as_slice(),
            [event]
                if matches!(
                    &event.event,
                    BackendEvent::RecoveryCenterError { error }
                        if error.code == RecoveryCenterErrorCode::ActionUnavailable
                )
        ));
        assert_eq!(
            store.load("failure").unwrap().unwrap().lifecycle,
            RecoveryLifecycle::Attention
        );
    }

    #[test]
    fn exact_launch_failure_releases_only_its_provider_root_claim() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("repo");
        fs::create_dir_all(&project_root).expect("project root");
        let store = RecoveryStore::for_project_dir(temp.path().join("project-store"));
        let repo_id = gwt_core::paths::project_scope_hash(&project_root).to_string();
        create_record(&store, "exact-failure", &project_root, &repo_id);
        bind_root(&store, "exact-failure");
        interrupt_after_supervisor_stop(&store, "exact-failure");
        let generation = store.load("exact-failure").unwrap().unwrap().generation;
        let (mut runtime, _) = test_runtime(temp.path(), &project_root, false);
        recovery_source_session(&project_root, "session-exact-failure")
            .save(&runtime.sessions_dir)
            .expect("save exact failure source");
        let result = handle_recovery_center_action(
            &store,
            action_request(
                "exact-failure",
                generation,
                RecoveryCenterAction::ConfirmResume,
            ),
        )
        .expect("claim exact failure root");
        assert!(store
            .active_provider_root_claim("codex", "provider-exact-failure", Utc::now(),)
            .unwrap()
            .is_some());

        let events = runtime.apply_recovery_center_action(
            "client-1",
            &store,
            result,
            None,
            recovery_action_handle("exact-failure", generation),
        );
        assert!(matches!(
            events.as_slice(),
            [event]
                if matches!(
                    &event.event,
                    BackendEvent::RecoveryCenterError { error }
                        if error.code == RecoveryCenterErrorCode::ActionUnavailable
                )
        ));
        assert!(store
            .active_provider_root_claim("codex", "provider-exact-failure", Utc::now(),)
            .unwrap()
            .is_none());
        assert_eq!(
            store.load("exact-failure").unwrap().unwrap().lifecycle,
            RecoveryLifecycle::Attention
        );
    }

    #[test]
    fn provider_root_claim_renewal_failure_aborts_the_attempt_and_preserves_attention() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("repo");
        fs::create_dir_all(&project_root).expect("project root");
        let store = RecoveryStore::for_project_dir(gwt_core::paths::gwt_project_dir_for_repo_path(
            &project_root,
        ));
        let repo_id = gwt_core::paths::project_scope_hash(&project_root).to_string();
        for recovery_id in ["renew-source", "renew-takeover"] {
            create_record(&store, recovery_id, &project_root, &repo_id);
            store
                .bind_root(
                    recovery_id,
                    ProviderRootBinding {
                        root_id: "renew-shared-root".to_string(),
                        session_tree_id: None,
                        quality: BindingQuality::Verified,
                        bound_at: Utc::now(),
                    },
                    format!("{recovery_id}:shared-root"),
                )
                .expect("bind shared root");
        }
        let now = Utc::now();
        let source = store.load("renew-source").unwrap().unwrap();
        store
            .claim_recovery_with_provider_root(
                "renew-source",
                source.generation,
                "renew-shared-root",
                false,
                gwt_core::recovery::RecoveryLease {
                    lease_id: "renew-stale-token".to_string(),
                    holder_id: "renew-source-window".to_string(),
                    acquired_at: now - Duration::minutes(3),
                    expires_at: now - Duration::minutes(1),
                },
                "renew-source-window",
                "expired slow launch",
                "renew-source:claim",
            )
            .expect("claim source");
        let takeover = store.load("renew-takeover").unwrap().unwrap();
        store
            .claim_recovery_with_provider_root(
                "renew-takeover",
                takeover.generation,
                "renew-shared-root",
                false,
                gwt_core::recovery::RecoveryLease {
                    lease_id: "renew-takeover-token".to_string(),
                    holder_id: "renew-takeover-window".to_string(),
                    acquired_at: now,
                    expires_at: now + Duration::minutes(2),
                },
                "renew-takeover-window",
                "take over expired launch",
                "renew-takeover:claim",
            )
            .expect("take over source claim");

        let (mut runtime, window_id) = test_runtime(temp.path(), &project_root, true);
        let window_id = window_id.expect("recovery attempt window");
        let mut source_session = recovery_source_session(&project_root, "session-renew-source");
        source_session.recovery_id = Some("renew-source".to_string());
        source_session.project_state_root = Some(project_root.clone());
        source_session
            .save(&runtime.sessions_dir)
            .expect("save source Session");
        runtime
            .pending_auto_resume_sources
            .insert(window_id.clone(), "session-renew-source".to_string());
        runtime.pending_provider_root_claims.insert(
            window_id.clone(),
            PendingProviderRootClaim {
                recovery_id: "renew-source".to_string(),
                claim_token: "renew-stale-token".to_string(),
                project_dir: store.root().to_path_buf(),
                claim_ttl: Duration::minutes(2),
            },
        );
        runtime.active_agent_sessions.insert(
            window_id.clone(),
            active_session(&window_id, "renew-attempt-session", &project_root),
        );

        let events = runtime.handle_pending_provider_root_claim_renewal(
            window_id.clone(),
            "renew-source".to_string(),
            "renew-stale-token".to_string(),
        );
        assert!(!events.is_empty());
        assert!(!runtime.window_lookup.contains_key(&window_id));
        assert!(!runtime
            .pending_provider_root_claims
            .contains_key(&window_id));
        assert!(!runtime.pending_auto_resume_sources.contains_key(&window_id));
        assert!(!runtime.active_agent_sessions.contains_key(&window_id));
        let source = store.load("renew-source").unwrap().unwrap();
        assert_eq!(source.lifecycle, RecoveryLifecycle::Attention);
        assert!(source
            .lifecycle_reason
            .as_deref()
            .is_some_and(|reason| reason.starts_with(
                "Recovery provider stopped before readiness after provider-root claim loss"
            )));
        assert_eq!(
            store
                .active_provider_root_claim("codex", "renew-shared-root", Utc::now())
                .unwrap()
                .unwrap()
                .claim_token,
            "renew-takeover-token"
        );
    }

    #[test]
    fn ready_barrier_abort_reconstructs_a_lost_exact_claim_mapping() {
        fn tree_contains_bytes(path: &Path, needle: &[u8]) -> bool {
            if path.is_file() {
                return fs::read(path)
                    .map(|bytes| bytes.windows(needle.len()).any(|window| window == needle))
                    .unwrap_or(false);
            }
            fs::read_dir(path)
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .any(|entry| tree_contains_bytes(&entry.path(), needle))
        }

        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("repo");
        fs::create_dir_all(&project_root).expect("project root");
        let store = RecoveryStore::for_project_dir(gwt_core::paths::gwt_project_dir_for_repo_path(
            &project_root,
        ));
        let repo_id = gwt_core::paths::project_scope_hash(&project_root).to_string();
        create_record(&store, "lost-map-source", &project_root, &repo_id);
        bind_root(&store, "lost-map-source");
        let source_record = store.load("lost-map-source").unwrap().unwrap();
        let claimed_at = Utc::now();
        store
            .claim_recovery_with_provider_root(
                "lost-map-source",
                source_record.generation,
                "provider-lost-map-source",
                false,
                gwt_core::recovery::RecoveryLease {
                    lease_id: "lost-map-token".to_string(),
                    holder_id: "lost-map-window".to_string(),
                    acquired_at: claimed_at,
                    expires_at: claimed_at + Duration::minutes(2),
                },
                "lost-map-window",
                "exact recovery with lost runtime mapping",
                "lost-map-source:claim",
            )
            .expect("claim exact source");
        let claim_token_sentinel = store
            .active_provider_root_claim("codex", "provider-lost-map-source", Utc::now())
            .expect("load provider-root claim")
            .expect("active provider-root claim")
            .claim_token;
        assert!(!claim_token_sentinel.is_empty());

        let (mut runtime, window_id) = test_runtime(temp.path(), &project_root, true);
        let window_id = window_id.expect("exact attempt window");
        let mut source_session = recovery_source_session(&project_root, "session-lost-map-source");
        source_session.project_state_root = Some(project_root.clone());
        source_session.recovery_id = Some("lost-map-source".to_string());
        source_session
            .save(&runtime.sessions_dir)
            .expect("save source Session");
        let mut target_session = recovery_source_session(&project_root, "session-lost-map-target");
        target_session.project_state_root = Some(project_root.clone());
        target_session.recovery_id = Some("lost-map-target".to_string());
        target_session.session_mode = gwt_agent::SessionMode::Resume;
        target_session.recovery_continuation = Some(gwt_agent::RecoveryContinuationHandoff {
            source_session_id: source_session.id.clone(),
            source_recovery_id: "lost-map-source".to_string(),
            target_recovery_id: "lost-map-target".to_string(),
            source_checkpoint_revision: 0,
            reason: "Startup requested exact provider resume".to_string(),
            inherit_checkpoint: false,
        });
        target_session
            .save(&runtime.sessions_dir)
            .expect("save target Session");
        runtime.active_agent_sessions.insert(
            window_id.clone(),
            active_session(&window_id, &target_session.id, &project_root),
        );
        assert!(!runtime
            .pending_provider_root_claims
            .contains_key(&window_id));
        assert!(!runtime.pending_auto_resume_sources.contains_key(&window_id));

        assert!(runtime.abort_pending_provider_root_claim_attempt(
            &window_id,
            "Provider Ready claim barrier rejected the recovery launch",
        ));

        assert!(!runtime.active_agent_sessions.contains_key(&window_id));
        assert!(!runtime.window_lookup.contains_key(&window_id));
        let source = store.load("lost-map-source").unwrap().unwrap();
        assert_eq!(source.lifecycle, RecoveryLifecycle::Attention);
        assert!(source.recovery_lease.is_none());
        assert!(source
            .lifecycle_reason
            .as_deref()
            .is_some_and(|reason| reason.starts_with(
                "Recovery provider stopped before readiness after provider-root claim loss"
            )));
        assert!(store
            .active_provider_root_claim("codex", "provider-lost-map-source", Utc::now(),)
            .unwrap()
            .is_none());
        let durable_recovery_ledger = store.root().join("recoveries").join("lost-map-source");
        assert!(durable_recovery_ledger.is_dir());
        assert!(tree_contains_bytes(
            &durable_recovery_ledger,
            b"provider-claim-lost:lost-map-source"
        ));
        assert!(
            !tree_contains_bytes(&durable_recovery_ledger, claim_token_sentinel.as_bytes()),
            "provider-root claim token leaked into the durable Recovery ledger"
        );
    }

    #[test]
    fn app_runtime_hides_launch_actions_without_a_source_session_ledger() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("repo");
        fs::create_dir_all(&project_root).expect("project root");
        let store = RecoveryStore::for_project_dir(temp.path().join("project-store"));
        let repo_id = gwt_core::paths::project_scope_hash(&project_root).to_string();
        create_record(&store, "sessionless", &project_root, &repo_id);
        bind_root(&store, "sessionless");
        interrupt_after_supervisor_stop(&store, "sessionless");
        let (runtime, _) = test_runtime(temp.path(), &project_root, false);

        let view = load_recovery_center_from_store(&project_root, &store).expect("load center");
        let mut sessionless = candidate(&view, "sessionless").clone();
        runtime.post_process_recovery_candidate(&mut sessionless);
        for action in [
            RecoveryCenterAction::ConfirmResume,
            RecoveryCenterAction::ContinueCheckpoint,
            RecoveryCenterAction::StartFresh,
        ] {
            assert!(!sessionless.available_actions.contains(&action));
        }
        assert_eq!(
            sessionless.details.get("Launch source").map(String::as_str),
            Some("Source Session ledger is unavailable")
        );

        recovery_source_session(&project_root, "session-sessionless")
            .save(&runtime.sessions_dir)
            .expect("save source Session");
        let view = load_recovery_center_from_store(&project_root, &store).expect("reload center");
        let mut recoverable = candidate(&view, "sessionless").clone();
        runtime.post_process_recovery_candidate(&mut recoverable);
        assert!(recoverable
            .available_actions
            .contains(&RecoveryCenterAction::ConfirmResume));
        assert!(recoverable
            .available_actions
            .contains(&RecoveryCenterAction::StartFresh));
        assert!(!recoverable.details.contains_key("Launch source"));
    }

    #[test]
    fn app_runtime_recreates_only_a_pinned_missing_intake_before_launch() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("repo");
        init_git_repo(&project_root);
        let launch_base_oid = git_stdout(&project_root, &["rev-parse", "HEAD"]);
        let worktree = temp.path().join(".intake-5");
        let store = RecoveryStore::for_project_dir(gwt_core::paths::gwt_project_dir_for_repo_path(
            &project_root,
        ));
        let repo_id = gwt_core::paths::project_scope_hash(&project_root).to_string();
        store
            .create(
                CreateRecovery {
                    recovery_id: "missing-intake".to_string(),
                    session_id: "session-missing-intake".to_string(),
                    repo_id,
                    session_kind: RecoverySessionKind::Intake,
                    worktree_path: worktree.clone(),
                    launch_base_ref: Some("develop".to_string()),
                    launch_base_oid: launch_base_oid.clone(),
                    launch_head_oid: launch_base_oid.clone(),
                    provider: "Codex".to_string(),
                    model: Some("gpt-5".to_string()),
                    runtime: "host".to_string(),
                    initial_prompt: "Continue the Intake".to_string(),
                    created_at: Utc::now(),
                },
                "missing-intake:create",
            )
            .expect("create missing Intake recovery");
        gwt_git::recovery::ensure_recovery_base_pin(
            &project_root,
            "missing-intake",
            &launch_base_oid,
        )
        .expect("pin recovery base");
        bind_root(&store, "missing-intake");
        interrupt_after_supervisor_stop(&store, "missing-intake");
        let generation = store.load("missing-intake").unwrap().unwrap().generation;
        let (mut runtime, _) = test_runtime(temp.path(), &project_root, false);
        let mut source = recovery_source_session(&worktree, "session-missing-intake");
        source.project_state_root = Some(project_root.clone());
        source.repo_hash = Some(gwt_core::paths::project_scope_hash(&project_root).to_string());
        source.launch_base_oid = Some(launch_base_oid.clone());
        source
            .save(&runtime.sessions_dir)
            .expect("save recovery source Session");

        let view = runtime.recovery_center_events("client-1");
        let BackendEvent::RecoveryCenterState { center } = &view[0].event else {
            panic!("expected Recovery Center state");
        };
        let candidate = center
            .candidates
            .iter()
            .find(|candidate| {
                candidate.action_handle == recovery_action_handle("missing-intake", generation)
            })
            .expect("public missing Intake candidate");
        assert!(!candidate.worktree_available);
        assert!(candidate
            .available_actions
            .contains(&RecoveryCenterAction::ConfirmResume));
        assert!(candidate
            .available_actions
            .contains(&RecoveryCenterAction::StartFresh));
        assert_eq!(
            candidate
                .details
                .get("Worktree restoration")
                .map(String::as_str),
            Some("Pinned Intake base is available")
        );

        let events = runtime.recovery_center_action_events(
            "client-1",
            public_action_request(
                "missing-intake",
                generation,
                RecoveryCenterAction::StartFresh,
            ),
            Some(canvas_bounds()),
        );
        assert!(
            events.iter().any(|event| matches!(
                event.event,
                BackendEvent::RecoveryCenterActionResult {
                    result: PublicRecoveryCenterActionResult::LaunchRequested { .. }
                }
            )),
            "unexpected missing-Intake recovery events: {events:#?}"
        );
        assert!(worktree.is_dir());
        assert_eq!(
            git_stdout(&worktree, &["rev-parse", "HEAD"]),
            launch_base_oid
        );
        assert!(git_stdout(&worktree, &["branch", "--show-current"]).is_empty());
    }

    #[test]
    fn continue_checkpoint_rejects_candidate_without_user_visible_context() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("repo");
        fs::create_dir_all(&project_root).expect("project root");
        let store = RecoveryStore::for_project_dir(temp.path().join("project-store"));
        let repo_id = gwt_core::paths::project_scope_hash(&project_root).to_string();
        create_record_with_initial_prompt(&store, "empty", &project_root, &repo_id, "");
        let generation = store.load("empty").unwrap().unwrap().generation;

        let error = handle_recovery_center_action(
            &store,
            action_request(
                "empty",
                generation,
                RecoveryCenterAction::ContinueCheckpoint,
            ),
        )
        .expect_err("missing continuation context must fail");

        assert_eq!(error.code, RecoveryCenterErrorCode::ActionUnavailable);
        let view = load_recovery_center_from_store(&project_root, &store).expect("load center");
        assert!(!candidate(&view, "empty")
            .available_actions
            .contains(&RecoveryCenterAction::ContinueCheckpoint));
        assert!(!candidate(&view, "empty")
            .available_actions
            .contains(&RecoveryCenterAction::StartFresh));

        let error = handle_recovery_center_action(
            &store,
            action_request("empty", generation, RecoveryCenterAction::StartFresh),
        )
        .expect_err("missing original request must not start a blank session");

        assert_eq!(error.code, RecoveryCenterErrorCode::ActionUnavailable);
    }

    #[test]
    fn discard_purges_only_gwt_recovery_payload_and_is_idempotent() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("repo");
        fs::create_dir_all(&project_root).expect("project root");
        let store = RecoveryStore::for_project_dir(temp.path().join("project-store"));
        let repo_id = gwt_core::paths::project_scope_hash(&project_root).to_string();
        create_record(&store, "discard", &project_root, &repo_id);
        bind_root(&store, "discard");
        let generation = store
            .set_lifecycle(
                "discard",
                RecoveryLifecycle::Interrupted,
                Some("provider stopped".to_string()),
                "discard:interrupted",
            )
            .unwrap()
            .generation;
        let provider_history = temp.path().join("provider-history.jsonl");
        fs::write(&provider_history, "provider-owned history\n").expect("provider history");
        let request = action_request("discard", generation, RecoveryCenterAction::Discard);

        let result = handle_recovery_center_action(&store, request.clone()).expect("discard");
        assert!(matches!(
            result,
            RecoveryCenterActionResult::Discarded { ref recovery_id, .. }
                if recovery_id == "discard"
        ));
        assert!(store.load("discard").unwrap().is_none());
        assert_eq!(
            store.load_tombstone("discard").unwrap().unwrap().lifecycle,
            RecoveryLifecycle::Discarded
        );
        assert_eq!(
            fs::read_to_string(&provider_history).expect("provider history remains"),
            "provider-owned history\n"
        );

        let repeated = handle_recovery_center_action(&store, request).expect("repeat discard");
        assert!(matches!(
            repeated,
            RecoveryCenterActionResult::Discarded { ref recovery_id, .. }
                if recovery_id == "discard"
        ));
    }

    #[test]
    fn discard_rejects_running_and_indeterminate_attention_candidates() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("repo");
        fs::create_dir_all(&project_root).expect("project root");
        let store = RecoveryStore::for_project_dir(temp.path().join("project-store"));
        let repo_id = gwt_core::paths::project_scope_hash(&project_root).to_string();

        create_record(&store, "running-discard", &project_root, &repo_id);
        bind_root(&store, "running-discard");
        let running = store
            .set_lifecycle(
                "running-discard",
                RecoveryLifecycle::Running,
                None,
                "running-discard:running",
            )
            .expect("mark running");
        let error = handle_recovery_center_action(
            &store,
            action_request(
                "running-discard",
                running.generation,
                RecoveryCenterAction::Discard,
            ),
        )
        .expect_err("Running recovery must not be discarded");
        assert_eq!(error.code, RecoveryCenterErrorCode::ActionUnavailable);
        assert!(store.load("running-discard").unwrap().is_some());

        create_record(&store, "indeterminate-discard", &project_root, &repo_id);
        store
            .advance_launch_stage(
                "indeterminate-discard",
                RecoveryLaunchStage::SpawnRequested,
                None,
                "indeterminate-discard:spawn-requested",
            )
            .expect("persist indeterminate spawn");
        let indeterminate = store
            .set_lifecycle(
                "indeterminate-discard",
                RecoveryLifecycle::Attention,
                Some("provider spawn outcome is unknown".to_string()),
                "indeterminate-discard:attention",
            )
            .expect("mark indeterminate Attention");
        let error = handle_recovery_center_action(
            &store,
            action_request(
                "indeterminate-discard",
                indeterminate.generation,
                RecoveryCenterAction::Discard,
            ),
        )
        .expect_err("indeterminate spawn must not be discarded");
        assert_eq!(error.code, RecoveryCenterErrorCode::ActionUnavailable);

        let view = load_recovery_center_from_store(&project_root, &store).expect("load center");
        assert!(!candidate(&view, "running-discard")
            .available_actions
            .contains(&RecoveryCenterAction::Discard));
        assert!(!candidate(&view, "indeterminate-discard")
            .available_actions
            .contains(&RecoveryCenterAction::Discard));
    }

    #[test]
    fn indeterminate_spawn_stages_block_relaunch_until_provider_stop_is_durable() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("repo");
        fs::create_dir_all(&project_root).expect("project root");
        let store = RecoveryStore::for_project_dir(temp.path().join("project-store"));
        let repo_id = gwt_core::paths::project_scope_hash(&project_root).to_string();

        for (recovery_id, stage) in [
            (
                "spawn-requested-relaunch",
                RecoveryLaunchStage::SpawnRequested,
            ),
            (
                "process-spawned-relaunch",
                RecoveryLaunchStage::ProcessSpawned,
            ),
        ] {
            create_record(&store, recovery_id, &project_root, &repo_id);
            bind_root(&store, recovery_id);
            store
                .advance_launch_stage(
                    recovery_id,
                    stage,
                    Some(gwt_core::recovery::ProviderRootRole::Root),
                    format!("{recovery_id}:indeterminate-stage"),
                )
                .expect("persist indeterminate provider boundary");
            let attention = store
                .set_lifecycle(
                    recovery_id,
                    RecoveryLifecycle::Attention,
                    Some("previous provider spawn outcome is unknown".to_string()),
                    format!("{recovery_id}:indeterminate-attention"),
                )
                .expect("mark indeterminate Attention");

            let view = load_recovery_center_from_store(&project_root, &store).expect("load center");
            let indeterminate_candidate = candidate(&view, recovery_id);
            assert!(indeterminate_candidate
                .available_actions
                .contains(&RecoveryCenterAction::Focus));
            assert!(indeterminate_candidate
                .available_actions
                .contains(&RecoveryCenterAction::OpenBoard));
            assert!(indeterminate_candidate
                .available_actions
                .contains(&RecoveryCenterAction::Details));
            for action in [
                RecoveryCenterAction::ConfirmResume,
                RecoveryCenterAction::ContinueCheckpoint,
                RecoveryCenterAction::StartFresh,
                RecoveryCenterAction::Discard,
            ] {
                assert!(
                    !indeterminate_candidate.available_actions.contains(&action),
                    "{action:?} must stay unavailable at {stage:?} without stop evidence"
                );
            }
            for action in [
                RecoveryCenterAction::ConfirmResume,
                RecoveryCenterAction::ContinueCheckpoint,
                RecoveryCenterAction::StartFresh,
            ] {
                let error = handle_recovery_center_action(
                    &store,
                    action_request(recovery_id, attention.generation, action),
                )
                .expect_err("backend must reject an indeterminate provider relaunch");
                assert_eq!(error.code, RecoveryCenterErrorCode::ActionUnavailable);
                assert!(error.message.contains("not confirmed stopped"));
            }

            let stopped = store
                .set_lifecycle(
                    recovery_id,
                    RecoveryLifecycle::Attention,
                    Some(super::RECOVERY_PROVIDER_STOPPED_REASON.to_string()),
                    format!("{recovery_id}:provider-stopped"),
                )
                .expect("persist provider stop evidence");
            let stopped_view =
                load_recovery_center_from_store(&project_root, &store).expect("reload center");
            let stopped_candidate = candidate(&stopped_view, recovery_id);
            assert!(stopped_candidate
                .available_actions
                .contains(&RecoveryCenterAction::ConfirmResume));
            assert!(stopped_candidate
                .available_actions
                .contains(&RecoveryCenterAction::StartFresh));
            handle_recovery_center_action(
                &store,
                action_request(
                    recovery_id,
                    stopped.generation,
                    RecoveryCenterAction::StartFresh,
                ),
            )
            .expect("durable stop evidence permits a replacement launch");
        }
    }

    #[test]
    fn discard_keeps_pre_spawn_legacy_attention_available() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("repo");
        fs::create_dir_all(&project_root).expect("project root");
        let store = RecoveryStore::for_project_dir(temp.path().join("project-store"));
        let repo_id = gwt_core::paths::project_scope_hash(&project_root).to_string();
        create_record(&store, "legacy-attention-discard", &project_root, &repo_id);
        let attention = store
            .set_lifecycle(
                "legacy-attention-discard",
                RecoveryLifecycle::Attention,
                Some("legacy provider root is ambiguous".to_string()),
                "legacy-attention-discard:attention",
            )
            .expect("mark legacy Attention");

        let view = load_recovery_center_from_store(&project_root, &store).expect("load center");
        assert!(candidate(&view, "legacy-attention-discard")
            .available_actions
            .contains(&RecoveryCenterAction::Discard));
        let discarded = handle_recovery_center_action(
            &store,
            action_request(
                "legacy-attention-discard",
                attention.generation,
                RecoveryCenterAction::Discard,
            ),
        )
        .expect("pre-spawn legacy Attention is safe to discard");
        assert!(matches!(
            discarded,
            RecoveryCenterActionResult::Discarded { .. }
        ));
    }

    #[test]
    fn app_runtime_rejects_discard_while_a_live_window_owns_the_recovery() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("repo");
        fs::create_dir_all(&project_root).expect("project root");
        let store = RecoveryStore::for_project_dir(gwt_core::paths::gwt_project_dir_for_repo_path(
            &project_root,
        ));
        let repo_id = gwt_core::paths::project_scope_hash(&project_root).to_string();
        create_record(&store, "live-discard", &project_root, &repo_id);
        let interrupted = store
            .set_lifecycle(
                "live-discard",
                RecoveryLifecycle::Interrupted,
                Some("stale durable state".to_string()),
                "live-discard:interrupted",
            )
            .expect("mark interrupted");
        let (mut runtime, window_id) = test_runtime(temp.path(), &project_root, true);
        let window_id = window_id.expect("agent window");
        runtime.active_agent_sessions.insert(
            window_id.clone(),
            active_session(&window_id, "session-live-discard", &project_root),
        );

        let events = runtime.recovery_center_action_events(
            "client-1",
            public_action_request(
                "live-discard",
                interrupted.generation,
                RecoveryCenterAction::Discard,
            ),
            None,
        );
        assert!(events.iter().any(|event| matches!(
            &event.event,
            BackendEvent::RecoveryCenterError { error }
                if error.code == RecoveryCenterErrorCode::ActionUnavailable
        )));
        assert!(store.load("live-discard").unwrap().is_some());
    }

    #[test]
    fn app_runtime_discard_keeps_tombstone_and_retries_owned_base_pin_cleanup() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("repo");
        init_git_repo(&project_root);
        let launch_base_oid = git_stdout(&project_root, &["rev-parse", "HEAD"]);
        let store = RecoveryStore::for_project_dir(gwt_core::paths::gwt_project_dir_for_repo_path(
            &project_root,
        ));
        let repo_id = gwt_core::paths::project_scope_hash(&project_root).to_string();
        store
            .create(
                CreateRecovery {
                    recovery_id: "discard-pin".to_string(),
                    session_id: "session-discard-pin".to_string(),
                    repo_id,
                    session_kind: RecoverySessionKind::Intake,
                    worktree_path: project_root.clone(),
                    launch_base_ref: Some("develop".to_string()),
                    launch_base_oid: launch_base_oid.clone(),
                    launch_head_oid: launch_base_oid.clone(),
                    provider: "codex".to_string(),
                    model: None,
                    runtime: "host".to_string(),
                    initial_prompt: "Discard recovery".to_string(),
                    created_at: Utc::now(),
                },
                "discard-pin:create",
            )
            .expect("create recovery");
        store
            .set_lifecycle(
                "discard-pin",
                RecoveryLifecycle::Interrupted,
                Some("provider stopped".to_string()),
                "discard-pin:interrupted",
            )
            .expect("mark discard source interrupted");
        gwt_git::recovery::ensure_recovery_base_pin(&project_root, "discard-pin", &launch_base_oid)
            .expect("pin recovery base");
        fs::write(project_root.join("README.md"), "second\n").expect("write second commit");
        git(&project_root, &["add", "README.md"]);
        git(&project_root, &["commit", "-qm", "second"]);
        let conflicting_oid = git_stdout(&project_root, &["rev-parse", "HEAD"]);
        let recovery_ref =
            gwt_git::recovery::recovery_base_ref_name("discard-pin").expect("recovery ref");
        git(
            &project_root,
            &["update-ref", &recovery_ref, &conflicting_oid],
        );
        let generation = store.load("discard-pin").unwrap().unwrap().generation;
        let (mut runtime, _) = test_runtime(temp.path(), &project_root, false);

        let first = runtime.recovery_center_action_events(
            "client-1",
            public_action_request("discard-pin", generation, RecoveryCenterAction::Discard),
            None,
        );
        assert!(first.iter().any(|event| matches!(
            &event.event,
            BackendEvent::RecoveryCenterError { error }
                if error.code == RecoveryCenterErrorCode::Store
        )));
        assert_eq!(
            store
                .load_tombstone("discard-pin")
                .unwrap()
                .expect("durable tombstone")
                .lifecycle,
            RecoveryLifecycle::Discarded
        );
        assert_eq!(
            git_stdout(&project_root, &["rev-parse", &recovery_ref]),
            conflicting_oid,
            "cleanup must never delete a ref that no longer points at the recorded OID"
        );

        // Simulate the operator repairing the owned ref. The payload is
        // already purged, so the repeated Discard must recover its expected
        // OID from the durable tombstone and finish cleanup idempotently.
        git(
            &project_root,
            &["update-ref", &recovery_ref, &launch_base_oid],
        );

        let repeated = runtime.recovery_center_action_events(
            "client-1",
            public_action_request("discard-pin", generation, RecoveryCenterAction::Discard),
            None,
        );
        assert!(repeated.iter().any(|event| matches!(
            event.event,
            BackendEvent::RecoveryCenterActionResult {
                result: PublicRecoveryCenterActionResult::Discarded { .. }
            }
        )));
        assert!(gwt_git::recovery::verify_recovery_base_pin(
            &project_root,
            "discard-pin",
            &launch_base_oid,
        )
        .is_err());
    }

    fn create_record(store: &RecoveryStore, recovery_id: &str, worktree: &Path, repo_id: &str) {
        create_record_with_initial_prompt(
            store,
            recovery_id,
            worktree,
            repo_id,
            "Investigate interrupted Intake",
        );
    }

    fn create_record_with_initial_prompt(
        store: &RecoveryStore,
        recovery_id: &str,
        worktree: &Path,
        repo_id: &str,
        initial_prompt: &str,
    ) {
        store
            .create(
                CreateRecovery {
                    recovery_id: recovery_id.to_string(),
                    session_id: format!("session-{recovery_id}"),
                    repo_id: repo_id.to_string(),
                    session_kind: RecoverySessionKind::Intake,
                    worktree_path: worktree.to_path_buf(),
                    launch_base_ref: Some("origin/develop".to_string()),
                    launch_base_oid: "1".repeat(40),
                    launch_head_oid: "2".repeat(40),
                    provider: "Codex".to_string(),
                    model: Some("gpt-5".to_string()),
                    runtime: "host".to_string(),
                    initial_prompt: initial_prompt.to_string(),
                    created_at: Utc::now(),
                },
                format!("{recovery_id}:create"),
            )
            .expect("create recovery");
    }

    fn bind_root(store: &RecoveryStore, recovery_id: &str) {
        store
            .bind_root(
                recovery_id,
                ProviderRootBinding {
                    root_id: format!("provider-{recovery_id}"),
                    session_tree_id: None,
                    quality: BindingQuality::Verified,
                    bound_at: Utc::now(),
                },
                format!("{recovery_id}:bind"),
            )
            .expect("bind root");
    }

    fn interrupt_after_supervisor_stop(store: &RecoveryStore, recovery_id: &str) {
        let record = store
            .load(recovery_id)
            .expect("load bound recovery")
            .expect("bound recovery");
        store
            .interrupt_after_supervisor_stop(
                recovery_id,
                record.generation,
                &record.session_id,
                Utc::now(),
                "test supervisor confirmed the provider stopped",
                format!("{recovery_id}:supervisor-stopped"),
            )
            .expect("record durable supervisor-stop proof");
    }

    fn recovery_source_session(worktree: &Path, id: &str) -> gwt_agent::Session {
        let mut session = gwt_agent::Session::new(worktree, "", gwt_agent::AgentId::Codex);
        session.id = id.to_string();
        session.is_ephemeral = true;
        session.session_kind = Some(gwt_skills::SessionKind::Intake);
        session.ephemeral_base_ref = Some("origin/develop".to_string());
        session.agent_session_id = Some("stale-provider-root".to_string());
        session.model = Some("gpt-5".to_string());
        session.runtime_target = gwt_agent::LaunchRuntimeTarget::Host;
        session
    }

    fn assert_recovery_config_preserves_source(
        config: &gwt_agent::LaunchConfig,
        source: &gwt_agent::Session,
    ) {
        assert_eq!(config.agent_id, source.agent_id);
        assert_eq!(
            config.working_dir.as_deref(),
            Some(source.worktree_path.as_path())
        );
        assert_eq!(config.model, source.model);
        assert_eq!(config.runtime_target, source.runtime_target);
        assert_eq!(config.is_ephemeral, source.is_ephemeral);
        assert_eq!(config.ephemeral_base_ref, source.ephemeral_base_ref);
    }

    fn active_session(
        window_id: &str,
        session_id: &str,
        worktree_path: &Path,
    ) -> ActiveAgentSession {
        ActiveAgentSession {
            window_id: window_id.to_string(),
            session_id: session_id.to_string(),
            agent_id: "codex".to_string(),
            branch_name: String::new(),
            display_name: "Codex".to_string(),
            worktree_path: worktree_path.to_path_buf(),
            agent_project_root: worktree_path.to_string_lossy().into_owned(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        }
    }

    fn canvas_bounds() -> gwt::WindowGeometry {
        gwt::WindowGeometry {
            x: 0.0,
            y: 0.0,
            width: 1280.0,
            height: 800.0,
        }
    }

    fn test_runtime(
        temp_root: &Path,
        project_root: &Path,
        with_agent_window: bool,
    ) -> (AppRuntime, Option<String>) {
        let mut workspace = gwt::WindowCanvasState::from_persisted(gwt::empty_workspace_state());
        let raw_window_id = with_agent_window.then(|| {
            workspace
                .add_window(gwt::WindowPreset::Agent, canvas_bounds())
                .id
        });
        let tab = ProjectTabRuntime {
            id: "tab-1".to_string(),
            title: "Repo".to_string(),
            project_root: project_root.to_path_buf(),
            kind: gwt::ProjectKind::Git,
            workspace,
            migration_pending: false,
            main_worktree_root_cache: Arc::new(std::sync::OnceLock::new()),
        };
        let (proxy, _) = AppEventProxy::stub();
        let blocking_tasks = BlockingTaskSpawner::thread();
        let sessions_dir = temp_root.join("sessions");
        let log_dir = temp_root.join("logs");
        fs::create_dir_all(&sessions_dir).expect("sessions dir");
        fs::create_dir_all(&log_dir).expect("logs dir");
        let persist_dispatcher = PersistDispatcher::new(&blocking_tasks);
        let mut runtime = AppRuntime {
            tabs: vec![tab],
            active_tab_id: Some("tab-1".to_string()),
            recent_projects: Vec::new(),
            profile_selections: HashMap::new(),
            profile_config_path: Some(temp_root.join("profile-config.toml")),
            runtimes: HashMap::new(),
            codex_bridge_routes: HashMap::new(),
            window_details: HashMap::new(),
            launch_error_terminal_details: HashMap::new(),
            window_lookup: HashMap::new(),
            board_all_view_windows: std::collections::HashSet::new(),
            session_state_path: temp_root.join("session-state.json"),
            log_dir,
            proxy,
            blocking_tasks,
            sessions_dir: sessions_dir.clone(),
            launch_wizard_cache: LaunchWizardMemoryCache::load(&sessions_dir),
            launch_wizard: None,
            pending_workspace_resume_contexts: HashMap::new(),
            pending_launch_feedback_contexts: HashMap::new(),
            inflight_launches: HashMap::new(),
            pending_auto_resume_sources: HashMap::new(),
            pending_provider_root_claims: HashMap::new(),
            pending_startup_auto_resume_sessions: Vec::new(),
            active_agent_sessions: HashMap::new(),
            work_merged_branches: HashMap::new(),
            work_cleanup_ready_branches: HashMap::new(),
            work_tip_subjects: HashMap::new(),
            work_pr_titles: HashMap::new(),
            work_ai_summaries: HashMap::new(),
            session_ledger_cache: std::cell::RefCell::new(
                crate::session_ledger_cache::SessionLedgerCache::new(),
            ),
            work_items_cache: std::cell::RefCell::new(
                gwt_core::workspace_projection::WorkItemsCache::new(),
            ),
            last_work_events_ingest: std::cell::RefCell::new(HashMap::new()),
            local_worktree_branches: std::cell::RefCell::new(HashMap::new()),
            window_pty_statuses: HashMap::new(),
            window_hook_states: HashMap::new(),
            recoverable_agent_error_windows: std::collections::HashSet::new(),
            hook_forward_target: None,
            issue_link_cache_dir: temp_root.join("cache"),
            issue_client_factory: crate::app_runtime::default_issue_client_factory(),
            pending_update: None,
            pty_writers: Arc::new(RwLock::new(HashMap::new())),
            attachment_uploads: AttachmentUploadStore::new(temp_root.join("uploads")),
            persist_dispatcher,
            file_tree_worktree_roots: HashMap::new(),
            server_url: None,
            usage_refresh: None,
            image_paste_sequence: std::sync::atomic::AtomicU64::new(0),
            agent_launch_stage_counter: std::sync::atomic::AtomicU64::new(1),
        };
        runtime.rebuild_window_lookup();
        runtime.seed_window_pty_statuses();
        let combined_window_id = raw_window_id
            .as_deref()
            .map(|raw_id| crate::combined_window_id("tab-1", raw_id));
        (runtime, combined_window_id)
    }

    fn checkpoint() -> SemanticCheckpoint {
        SemanticCheckpoint {
            summary: "Recovered discussion".to_string(),
            confirmed_decisions: vec!["Keep provider history".to_string()],
            open_questions: vec!["Which recovery action?".to_string()],
            next_action: Some("Open Recovery Center".to_string()),
            as_of_turn_id: Some("turn-7".to_string()),
            visible_items: Vec::new(),
            attachment_refs: Vec::new(),
            board_intents: vec![BoardMilestoneIntent {
                entry_id: "board-7".to_string(),
                title: "Recovery checkpoint".to_string(),
                body: "Recovered discussion".to_string(),
                queued_at: Utc::now(),
            }],
        }
    }

    fn init_git_repo(path: &Path) {
        fs::create_dir_all(path).expect("repo directory");
        git(path, &["init", "-q", "-b", "develop"]);
        git(path, &["config", "user.name", "Gwt Test"]);
        git(path, &["config", "user.email", "gwt@example.com"]);
        fs::write(path.join("README.md"), "fixture\n").expect("fixture");
        git(path, &["add", "README.md"]);
        git(path, &["commit", "-qm", "initial"]);
    }

    fn git(path: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(path)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_stdout(path: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(path)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .expect("utf8 git output")
            .trim()
            .to_string()
    }

    fn action_request(
        recovery_id: &str,
        expected_generation: u64,
        action: RecoveryCenterAction,
    ) -> RecoveryCenterActionRequest {
        RecoveryCenterActionRequest {
            action_handle: recovery_action_handle(recovery_id, expected_generation),
            recovery_id: recovery_id.to_string(),
            expected_generation,
            action,
            provider_root_id: None,
        }
    }

    fn action_request_with_root(
        recovery_id: &str,
        expected_generation: u64,
        action: RecoveryCenterAction,
        provider_root_id: &str,
    ) -> RecoveryCenterActionRequest {
        RecoveryCenterActionRequest {
            action_handle: recovery_action_handle(recovery_id, expected_generation),
            recovery_id: recovery_id.to_string(),
            expected_generation,
            action,
            provider_root_id: Some(provider_root_id.to_string()),
        }
    }

    fn public_action_request(
        recovery_id: &str,
        expected_generation: u64,
        action: RecoveryCenterAction,
    ) -> PublicRecoveryCenterActionRequest {
        PublicRecoveryCenterActionRequest {
            action_handle: recovery_action_handle(recovery_id, expected_generation),
            action,
            provider_choice_handle: None,
        }
    }

    fn candidate<'a>(
        view: &'a RecoveryCenterView,
        recovery_id: &str,
    ) -> &'a RecoveryCenterCandidate {
        view.candidates
            .iter()
            .find(|candidate| candidate.recovery_id == recovery_id)
            .expect("candidate")
    }

    fn launch_request(result: RecoveryCenterActionResult) -> Box<RecoveryCenterLaunchRequest> {
        let RecoveryCenterActionResult::LaunchRequested { request } = result else {
            panic!("expected launch request");
        };
        request
    }
}
