use std::collections::HashSet;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[cfg(test)]
type DurableLaunchRecoveryDirectorySyncHook = Box<dyn Fn(&Path) -> std::io::Result<()> + 'static>;

#[cfg(test)]
thread_local! {
    static DURABLE_LAUNCH_RECOVERY_DIRECTORY_SYNC_HOOK:
        std::cell::RefCell<Option<DurableLaunchRecoveryDirectorySyncHook>> =
            std::cell::RefCell::new(None);
}

#[cfg(test)]
type MissingSessionCleanupHook = Box<dyn FnOnce(&str) + 'static>;

#[cfg(test)]
thread_local! {
    static MISSING_SESSION_CLEANUP_HOOK:
        std::cell::RefCell<Option<MissingSessionCleanupHook>> =
            std::cell::RefCell::new(None);
}

#[cfg(test)]
pub(super) fn set_missing_session_cleanup_hook_for_test(hook: MissingSessionCleanupHook) {
    MISSING_SESSION_CLEANUP_HOOK.with(|slot| *slot.borrow_mut() = Some(hook));
}

fn invoke_missing_session_cleanup_hook(session_id: &str) {
    #[cfg(test)]
    MISSING_SESSION_CLEANUP_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook(session_id);
        }
    });

    #[cfg(not(test))]
    let _ = session_id;
}

#[cfg(test)]
type DurableContinueWorkPostRepairHook = Box<dyn FnOnce() + 'static>;

#[cfg(test)]
thread_local! {
    static DURABLE_CONTINUE_WORK_POST_REPAIR_HOOK:
        std::cell::RefCell<Option<DurableContinueWorkPostRepairHook>> =
            std::cell::RefCell::new(None);
    static DURABLE_CONTINUE_WORK_PRE_WORK_COMMIT_HOOK:
        std::cell::RefCell<Option<DurableContinueWorkPostRepairHook>> =
            std::cell::RefCell::new(None);
    static FRESH_EXECUTION_PRE_WORK_COMMIT_HOOK:
        std::cell::RefCell<Option<DurableContinueWorkPostRepairHook>> =
            std::cell::RefCell::new(None);
}

#[cfg(test)]
pub(super) fn set_durable_continue_work_post_repair_hook_for_test(
    hook: DurableContinueWorkPostRepairHook,
) {
    DURABLE_CONTINUE_WORK_POST_REPAIR_HOOK.with(|slot| *slot.borrow_mut() = Some(hook));
}

#[cfg(test)]
pub(super) fn set_durable_continue_work_pre_work_commit_hook_for_test(
    hook: DurableContinueWorkPostRepairHook,
) {
    DURABLE_CONTINUE_WORK_PRE_WORK_COMMIT_HOOK.with(|slot| *slot.borrow_mut() = Some(hook));
}

#[cfg(test)]
pub(super) fn set_fresh_execution_pre_work_commit_hook_for_test(
    hook: DurableContinueWorkPostRepairHook,
) {
    FRESH_EXECUTION_PRE_WORK_COMMIT_HOOK.with(|slot| *slot.borrow_mut() = Some(hook));
}

#[cfg(test)]
fn invoke_durable_continue_work_post_repair_hook() {
    DURABLE_CONTINUE_WORK_POST_REPAIR_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(not(test))]
fn invoke_durable_continue_work_post_repair_hook() {}

#[cfg(test)]
fn invoke_durable_continue_work_pre_work_commit_hook() {
    DURABLE_CONTINUE_WORK_PRE_WORK_COMMIT_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(not(test))]
fn invoke_durable_continue_work_pre_work_commit_hook() {}

#[cfg(test)]
fn invoke_fresh_execution_pre_work_commit_hook() {
    FRESH_EXECUTION_PRE_WORK_COMMIT_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(not(test))]
fn invoke_fresh_execution_pre_work_commit_hook() {}

use super::workspace::{
    active_agent_summary_from_session, apply_workspace_launch_transition,
    WorkspaceLaunchProjectionKind, WorkspaceLaunchTransition,
};
use super::{
    launch_config_from_persisted_session, non_empty_workspace_text, AppRuntime, BackendEvent,
    CachedContinueWorkOutcome, OutboundEvent, PendingContinueWork, PendingContinueWorkExecution,
    PendingFreshExecutionLaunch, WindowGeometry, WindowProcessStatus, WorkspaceResumeContext,
};
use regex::Regex;

#[derive(Debug)]
struct ContinueWorkFailure {
    outcome: gwt::ContinueWorkOutcomeKind,
    message: String,
    code: &'static str,
    retryable: bool,
}

impl ContinueWorkFailure {
    fn failed(code: &'static str, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            outcome: gwt::ContinueWorkOutcomeKind::Failed,
            message: message.into(),
            code,
            retryable,
        }
    }

    fn conflict(message: impl Into<String>) -> Self {
        Self {
            outcome: gwt::ContinueWorkOutcomeKind::ConflictUnknown,
            message: message.into(),
            code: "execution_owner_unknown",
            retryable: true,
        }
    }
}

#[derive(Debug)]
struct ContinueWorkTarget {
    tab_id: String,
    project_root: PathBuf,
    worktree_path: PathBuf,
    work_branch: Option<String>,
    work_agent_id: Option<gwt_agent::AgentId>,
    work_agent_session_id: Option<String>,
    launch_seed: ContinueWorkLaunchSeed,
    owner: gwt::cli::execution_state::ExecutionOwnerKey,
    resume_context: WorkspaceResumeContext,
}

#[derive(Debug)]
enum ContinueWorkLaunchSeed {
    DurableSession(Box<gwt_agent::Session>),
    WorkProjection {
        agent_id: gwt_agent::AgentId,
        display_name: Option<String>,
        branch: String,
    },
}

#[derive(Debug, Clone)]
enum DurableContinueWorkAttempt {
    Successor(Box<gwt::cli::execution_state::ContinuationAttempt>),
    Takeover(Box<gwt::cli::execution_state::GenerationTakeoverAttempt>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DurableContinueWorkAttemptStatus {
    Prepared,
    Aborted,
    Activated,
}

impl DurableContinueWorkAttempt {
    fn work_id(&self) -> Option<&str> {
        match self {
            Self::Successor(attempt) => attempt.request.work_id.as_deref(),
            Self::Takeover(attempt) => attempt.request.work_id.as_deref(),
        }
    }

    fn source(&self) -> Option<&str> {
        match self {
            Self::Successor(attempt) => Some(attempt.request.source.as_str()),
            Self::Takeover(attempt) => attempt.request.source.as_deref(),
        }
    }

    fn candidate_session_id(&self) -> &str {
        match self {
            Self::Successor(attempt) => &attempt.request.initial_session_id,
            Self::Takeover(attempt) => &attempt.request.to_session_id,
        }
    }

    fn status(&self) -> DurableContinueWorkAttemptStatus {
        match self {
            Self::Successor(attempt) => match attempt.status {
                gwt::cli::execution_state::ContinuationAttemptStatus::Prepared => {
                    DurableContinueWorkAttemptStatus::Prepared
                }
                gwt::cli::execution_state::ContinuationAttemptStatus::Aborted => {
                    DurableContinueWorkAttemptStatus::Aborted
                }
                gwt::cli::execution_state::ContinuationAttemptStatus::Activated => {
                    DurableContinueWorkAttemptStatus::Activated
                }
            },
            Self::Takeover(attempt) => match attempt.status {
                gwt::cli::execution_state::GenerationTakeoverAttemptStatus::Prepared => {
                    DurableContinueWorkAttemptStatus::Prepared
                }
                gwt::cli::execution_state::GenerationTakeoverAttemptStatus::Aborted => {
                    DurableContinueWorkAttemptStatus::Aborted
                }
                gwt::cli::execution_state::GenerationTakeoverAttemptStatus::Activated => {
                    DurableContinueWorkAttemptStatus::Activated
                }
            },
        }
    }

    fn outcome(&self) -> Option<gwt::ContinueWorkOutcomeKind> {
        match self.source()? {
            "continue-work:resume" => Some(gwt::ContinueWorkOutcomeKind::ContinuedConversation),
            "continue-work:handoff" => Some(gwt::ContinueWorkOutcomeKind::StartedWithHandoff),
            _ => None,
        }
    }

    fn activated_identity_matches(&self, current: &gwt_agent::ExecutionBindingIdentity) -> bool {
        match self {
            Self::Successor(attempt) => {
                attempt
                    .activated_generation
                    .as_ref()
                    .is_some_and(|generation| {
                        generation.generation_id == current.generation_id
                            && generation.session_binding_id == current.binding_id
                    })
            }
            Self::Takeover(attempt) => attempt.activated_binding.as_ref() == Some(current),
        }
    }

    fn candidate_binding_matches(
        &self,
        worktree: &Path,
        owner: gwt::cli::execution_state::ExecutionOwnerKey,
        binding: &gwt_agent::SessionExecutionBinding,
    ) -> std::io::Result<bool> {
        let repo_hash =
            gwt_core::repo_hash::detect_repo_hash(worktree).map(|value| value.to_string());
        if binding.schema_version != gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION
            || binding.session_id != self.candidate_session_id()
            || binding.owner_kind != owner.kind.as_str()
            || binding.owner_number != owner.number
            || binding.capability_generation != 1
            || repo_hash.as_deref() != Some(binding.repo_hash.as_str())
        {
            return Ok(false);
        }
        match self {
            Self::Successor(attempt) => {
                gwt::cli::execution_state::continuation_attempt_execution_binding_matches(
                    worktree,
                    owner,
                    attempt,
                    &binding.session_id,
                    &binding.identity,
                )
            }
            Self::Takeover(attempt) => {
                gwt::cli::execution_state::generation_takeover_attempt_execution_binding_matches(
                    worktree,
                    owner,
                    attempt,
                    &binding.session_id,
                    &binding.identity,
                )
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn abort_and_remove_exact_session<F>(
        &self,
        worktree: &Path,
        owner: gwt::cli::execution_state::ExecutionOwnerKey,
        reason: &str,
        sessions_dir: &Path,
        session_identity: &gwt_agent::SessionExecutionIdentity,
        after_abort: F,
    ) -> std::io::Result<bool>
    where
        F: FnOnce() -> std::io::Result<()>,
    {
        match self {
            Self::Successor(attempt) => {
                gwt::cli::execution_state::abort_successor_and_remove_exact_session(
                    worktree,
                    owner,
                    &attempt.request,
                    reason,
                    sessions_dir,
                    session_identity,
                    after_abort,
                )
            }
            Self::Takeover(attempt) => {
                gwt::cli::execution_state::abort_generation_takeover_and_remove_exact_session(
                    worktree,
                    owner,
                    &attempt.request,
                    reason,
                    sessions_dir,
                    session_identity,
                    after_abort,
                )
            }
        }
    }

    fn repair_activation(
        &self,
        worktree: &Path,
        owner: gwt::cli::execution_state::ExecutionOwnerKey,
    ) -> std::io::Result<()> {
        match self {
            Self::Successor(attempt) => {
                gwt::cli::execution_state::activate_successor(worktree, owner, &attempt.request)
                    .map(|_| ())
            }
            Self::Takeover(attempt) => gwt::cli::execution_state::activate_generation_takeover(
                worktree,
                owner,
                &attempt.request,
            )
            .map(|_| ()),
        }
    }

    fn predecessor_evidence(
        &self,
        worktree: &Path,
        owner: gwt::cli::execution_state::ExecutionOwnerKey,
    ) -> std::io::Result<(String, gwt_agent::ExecutionBindingIdentity)> {
        match self {
            Self::Successor(attempt) => Ok((
                attempt.predecessor.initial_session_id.clone(),
                gwt_agent::ExecutionBindingIdentity {
                    generation_id: attempt.predecessor.generation_id.clone(),
                    binding_id: attempt.predecessor.session_binding_id.clone(),
                    ledger_head_hash: attempt.predecessor_generation_content_hash.clone(),
                },
            )),
            Self::Takeover(attempt) => {
                let ledger = gwt::cli::execution_state::load_generation_ledger(worktree, owner)?
                    .ok_or_else(|| {
                        std::io::Error::new(
                            std::io::ErrorKind::NotFound,
                            "takeover generation ledger is missing",
                        )
                    })?;
                let generation = ledger
                    .generations
                    .iter()
                    .find(|generation| generation.identity.generation_id == attempt.generation_id)
                    .ok_or_else(|| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "takeover predecessor generation is missing",
                        )
                    })?;
                Ok((
                    attempt.request.from_session_id.clone(),
                    gwt_agent::ExecutionBindingIdentity {
                        generation_id: attempt.generation_id.clone(),
                        binding_id: generation.identity.session_binding_id.clone(),
                        ledger_head_hash: attempt.predecessor_head_hash.clone(),
                    },
                ))
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ActiveOwnerLiveness {
    Stale(&'static str),
    Unknown,
}

fn canonical_public_id(value: &str, max_len: usize) -> bool {
    value.trim() == value
        && !value.is_empty()
        && value.len() <= max_len
        && !value.chars().any(char::is_control)
}

fn continue_work_outcome(
    client_id: &str,
    operation_id: String,
    work_id: String,
    outcome: gwt::ContinueWorkOutcomeKind,
    message: Option<String>,
    error_code: Option<String>,
    retryable: bool,
) -> OutboundEvent {
    OutboundEvent::reply(
        client_id.to_string(),
        BackendEvent::ContinueWorkOutcome {
            operation_id,
            work_id,
            outcome,
            message,
            error_code,
            retryable,
        },
    )
}

fn reject_continue_work_workspace_commit(
    project_root: &Path,
    work_event_root: &Path,
    operation_id: &str,
) -> std::io::Result<()> {
    match resolve_split_workspace_state_external_commit(
        project_root,
        work_event_root,
        operation_id,
        gwt_core::workspace_projection::ExternalWorkspaceCommitDecision::Reject,
    ) {
        Ok(
            gwt_core::workspace_projection::ExternalWorkspaceCommitResolution::Rejected
            | gwt_core::workspace_projection::ExternalWorkspaceCommitResolution::Missing,
        ) => Ok(()),
        Ok(resolution) => Err(std::io::Error::other(format!(
            "continuation Work rejection returned {resolution:?}"
        ))),
        Err(error) => Err(std::io::Error::other(error.to_string())),
    }
}

fn resolve_split_workspace_state_external_commit(
    project_root: &Path,
    work_event_root: &Path,
    operation_id: &str,
    decision: gwt_core::workspace_projection::ExternalWorkspaceCommitDecision,
) -> gwt_core::error::Result<gwt_core::workspace_projection::ExternalWorkspaceCommitResolution> {
    gwt_core::workspace_projection::resolve_workspace_state_external_commit_at(
        &gwt_core::paths::gwt_workspace_projection_path_for_repo_path(project_root),
        &gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(work_event_root),
        operation_id,
        decision,
    )
}

fn split_workspace_state_external_commit_resolution(
    project_root: &Path,
    work_event_root: &Path,
    operation_id: &str,
) -> gwt_core::error::Result<gwt_core::workspace_projection::ExternalWorkspaceCommitResolution> {
    gwt_core::workspace_projection::workspace_state_external_commit_resolution_at(
        &gwt_core::paths::gwt_workspace_projection_path_for_repo_path(project_root),
        &gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(work_event_root),
        operation_id,
    )
}

fn sanitize_handoff_value(value: &str) -> Option<String> {
    static SECRET: OnceLock<Regex> = OnceLock::new();
    static URL: OnceLock<Regex> = OnceLock::new();
    static ABSOLUTE_PATH: OnceLock<Regex> = OnceLock::new();
    static RELATIVE_PATH: OnceLock<Regex> = OnceLock::new();

    let flattened = value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut sanitized = gwt_core::process_console::redact_line(&flattened);
    sanitized = SECRET
        .get_or_init(|| {
            Regex::new(r"(?i)\b(?:authorization|token|secret|password|api[_-]?key)\s*[:=]\s*\S+")
                .expect("handoff secret regex")
        })
        .replace_all(&sanitized, "[redacted-secret]")
        .into_owned();
    sanitized = URL
        .get_or_init(|| Regex::new(r"(?i)\bhttps?://\S+").expect("handoff URL regex"))
        .replace_all(&sanitized, "[redacted-url]")
        .into_owned();
    sanitized = ABSOLUTE_PATH
        .get_or_init(|| {
            Regex::new(r"(?:[A-Za-z]:[\\/]|/)(?:[^\s/\\]+[\\/])+[^\s]*")
                .expect("handoff absolute path regex")
        })
        .replace_all(&sanitized, "[redacted-path]")
        .into_owned();
    sanitized = RELATIVE_PATH
        .get_or_init(|| {
            Regex::new(r"\b(?:[A-Za-z0-9._-]+[\\/]){2,}[A-Za-z0-9._-]*")
                .expect("handoff relative path regex")
        })
        .replace_all(&sanitized, "[redacted-path]")
        .into_owned();
    let sanitized = sanitized.trim();
    (!sanitized.is_empty()).then(|| sanitized.chars().take(600).collect())
}

pub(super) fn handoff_context(
    context: &WorkspaceResumeContext,
    work_id: &str,
    predecessor: &gwt_agent::ExecutionBindingIdentity,
) -> String {
    fn clean(value: Option<&str>) -> Option<String> {
        value.and_then(sanitize_handoff_value)
    }

    let mut lines = vec![
        "Handoff Context".to_string(),
        "Continue the existing Work in a new conversation.".to_string(),
        format!("Work: {work_id}"),
        format!("Predecessor generation: {}", predecessor.generation_id),
        format!("Predecessor binding: {}", predecessor.binding_id),
    ];
    for (label, value) in [
        ("Title", clean(context.title.as_deref())),
        ("Owner", clean(context.owner.as_deref())),
        ("Summary", clean(context.summary.as_deref())),
        ("Next action", clean(context.next_action.as_deref())),
    ] {
        if let Some(value) = value {
            lines.push(format!("{label}: {value}"));
        }
    }
    lines.join("\n")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProviderConversationAvailability {
    Present,
    Missing,
    Foreign,
    Unknown,
}

fn json_string_for_key<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    match value {
        serde_json::Value::Object(values) => values
            .get(key)
            .and_then(serde_json::Value::as_str)
            .or_else(|| {
                values
                    .values()
                    .find_map(|value| json_string_for_key(value, key))
            }),
        serde_json::Value::Array(values) => values
            .iter()
            .find_map(|value| json_string_for_key(value, key)),
        _ => None,
    }
}

fn provider_conversation_cwd(path: &Path) -> std::io::Result<Option<PathBuf>> {
    let file = std::fs::File::open(path)?;
    for line in std::io::BufReader::new(file).lines().take(64) {
        let line = line?;
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if let Some(cwd) = json_string_for_key(&value, "cwd")
            .map(str::trim)
            .filter(|cwd| !cwd.is_empty())
        {
            return Ok(Some(PathBuf::from(cwd)));
        }
    }
    Ok(None)
}

pub(super) fn provider_conversation_availability(
    session: &gwt_agent::Session,
) -> ProviderConversationAvailability {
    let Some(conversation_id) = session.exact_resume_session_id() else {
        return ProviderConversationAvailability::Missing;
    };
    if session.runtime_target != gwt_agent::LaunchRuntimeTarget::Host {
        return ProviderConversationAvailability::Unknown;
    }
    let path = match session.agent_id {
        gwt_agent::AgentId::ClaudeCode => {
            let Some(home) = gwt_core::usage::claude::claude_home() else {
                return ProviderConversationAvailability::Unknown;
            };
            gwt_core::usage::claude::transcript_for_session(&home, conversation_id)
        }
        gwt_agent::AgentId::Codex => {
            let Some(home) = gwt_core::usage::codex::codex_home() else {
                return ProviderConversationAvailability::Unknown;
            };
            gwt_core::usage::codex::rollout_for_session(&home, conversation_id)
        }
        _ => return ProviderConversationAvailability::Unknown,
    };
    let Some(path) = path else {
        return ProviderConversationAvailability::Missing;
    };
    match provider_conversation_cwd(&path) {
        Ok(Some(cwd)) if path_matches(&cwd, &session.worktree_path) => {
            ProviderConversationAvailability::Present
        }
        Ok(Some(_)) => ProviderConversationAvailability::Foreign,
        Ok(None) => ProviderConversationAvailability::Foreign,
        Err(_) => ProviderConversationAvailability::Unknown,
    }
}

pub(super) fn configure_provider_continuation(
    config: &mut gwt_agent::LaunchConfig,
    source_session: &gwt_agent::Session,
) -> gwt::ContinueWorkOutcomeKind {
    let exact_resume_is_safe = source_session.exact_resume_session_id().is_some()
        && !matches!(
            provider_conversation_availability(source_session),
            ProviderConversationAvailability::Missing | ProviderConversationAvailability::Foreign
        );
    if exact_resume_is_safe {
        return gwt::ContinueWorkOutcomeKind::ContinuedConversation;
    }
    config.session_mode = gwt_agent::SessionMode::Normal;
    config.resume_session_id = None;
    gwt::ContinueWorkOutcomeKind::StartedWithHandoff
}

fn path_matches(left: &Path, right: &Path) -> bool {
    match (dunce::canonicalize(left), dunce::canonicalize(right)) {
        (Ok(left), Ok(right)) => {
            gwt_core::paths::normalize_windows_child_process_path(&left)
                == gwt_core::paths::normalize_windows_child_process_path(&right)
        }
        _ => false,
    }
}

fn session_matches_project_state(session: &gwt_agent::Session, project_root: &Path) -> bool {
    if let Some(root) = session
        .project_state_root
        .as_deref()
        .filter(|root| !root.as_os_str().is_empty())
    {
        return path_matches(root, project_root);
    }
    let Some(project_repo_hash) = gwt_core::repo_hash::detect_repo_hash(project_root) else {
        return false;
    };
    if session.repo_hash.as_deref() != Some(project_repo_hash.as_str()) {
        return false;
    }
    let Ok(project_anchor) = gwt_git::worktree::main_worktree_root(project_root) else {
        return false;
    };
    let Ok(session_anchor) = gwt_git::worktree::main_worktree_root(&session.worktree_path) else {
        return false;
    };
    path_matches(&project_anchor, &session_anchor)
}

fn worktree_matches_project_state(worktree_path: &Path, project_root: &Path) -> bool {
    if path_matches(worktree_path, project_root) {
        return true;
    }
    if path_matches(
        &crate::runtime_support::normalize_recent_project_path(worktree_path),
        project_root,
    ) {
        return true;
    }
    let Ok(project_anchor) = gwt_git::worktree::main_worktree_root(project_root) else {
        return false;
    };
    let Ok(worktree_anchor) = gwt_git::worktree::main_worktree_root(worktree_path) else {
        return false;
    };
    path_matches(&project_anchor, &worktree_anchor)
}

fn canonical_continue_work_branch(worktree_path: &Path) -> Result<String, ContinueWorkFailure> {
    crate::runtime_support::current_git_branch(worktree_path)
        .map(|branch| crate::runtime_support::normalize_branch_name(branch.trim()))
        .map_err(|error| {
            ContinueWorkFailure::conflict(format!(
                "The Work branch could not be verified against its current worktree: {error}"
            ))
        })
        .and_then(|branch| {
            if branch.is_empty() {
                Err(ContinueWorkFailure::conflict(
                    "The Work branch is detached or empty and cannot be continued safely.",
                ))
            } else {
                Ok(branch)
            }
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProjectionOwnerRef {
    declared_kind: Option<gwt::cli::execution_state::ExecutionOwnerKind>,
    number: u64,
}

fn strict_projection_owner(raw_owner: &str) -> Option<ProjectionOwnerRef> {
    let owner = raw_owner.trim();
    if owner.is_empty() {
        return None;
    }
    let lower = owner.to_ascii_lowercase();
    let (declared_kind, digits) = if let Some(rest) = owner.strip_prefix('#') {
        (None, rest.trim())
    } else if lower.starts_with("issue") || lower.starts_with("spec") {
        let (prefix_len, declared_kind) = if lower.starts_with("issue") {
            (5, gwt::cli::execution_state::ExecutionOwnerKind::Issue)
        } else {
            (4, gwt::cli::execution_state::ExecutionOwnerKind::Spec)
        };
        let rest = &owner[prefix_len..];
        let first = rest.chars().next()?;
        if !(first.is_ascii_whitespace() || first == '#' || first == '-') {
            return None;
        }
        (
            Some(declared_kind),
            rest.trim_start()
                .strip_prefix('#')
                .or_else(|| rest.trim_start().strip_prefix('-'))
                .unwrap_or(rest.trim_start())
                .trim(),
        )
    } else {
        (None, owner)
    };
    if digits.is_empty() || !digits.chars().all(|character| character.is_ascii_digit()) {
        return None;
    }
    digits
        .parse::<u64>()
        .ok()
        .filter(|number| *number > 0)
        .map(|number| ProjectionOwnerRef {
            declared_kind,
            number,
        })
}

fn projection_only_continue_owner(
    item: &gwt_core::workspace_projection::WorkItem,
) -> Result<ProjectionOwnerRef, ContinueWorkFailure> {
    let owner = item
        .owner
        .as_deref()
        .map(str::trim)
        .filter(|owner| !owner.is_empty())
        .ok_or_else(|| {
            ContinueWorkFailure::failed(
                "execution_owner_missing",
                "This Work has no linked Issue or SPEC execution owner.",
                false,
            )
        })?;
    strict_projection_owner(owner).ok_or_else(|| {
        ContinueWorkFailure::failed(
            "execution_owner_ambiguous",
            "The Work has an ambiguous execution owner and cannot be continued safely.",
            false,
        )
    })
}

fn canonical_continue_work_owner(
    project_root: &Path,
    worktree_path: &Path,
    projected: ProjectionOwnerRef,
) -> Result<gwt::cli::execution_state::ExecutionOwnerKey, ContinueWorkFailure> {
    let validate = |owner: gwt::cli::execution_state::ExecutionOwnerKey| {
        if owner.number != projected.number
            || projected
                .declared_kind
                .is_some_and(|kind| kind != owner.kind)
        {
            Err(ContinueWorkFailure::failed(
                "execution_owner_ambiguous",
                "The Work owner does not match its current execution authority.",
                false,
            ))
        } else {
            Ok(owner)
        }
    };
    match gwt::cli::execution_state::current_generation_owner(worktree_path) {
        Ok(Some(owner)) => validate(owner),
        Ok(None) => {
            match gwt::cli::execution_state::recovery_projection_owner_hint(worktree_path) {
                Ok(Some(owner)) => validate(owner),
                Ok(None) => validate(gwt::cli::execution_state::ExecutionOwnerKey {
                    kind: gwt::cli::execution_state::detect_owner_kind(
                        project_root,
                        projected.number,
                    ),
                    number: projected.number,
                }),
                Err(_) => Err(ContinueWorkFailure::conflict(
                    "The Work execution authority could not be read safely.",
                )),
            }
        }
        Err(_) => Err(ContinueWorkFailure::conflict(
            "The Work execution authority could not be read safely.",
        )),
    }
}

fn projection_only_continue_container(
    item: &gwt_core::workspace_projection::WorkItem,
    project_root: &Path,
) -> Result<(PathBuf, String), ContinueWorkFailure> {
    let mut selected_path: Option<PathBuf> = None;
    let mut selected_branch: Option<String> = None;
    for container in item
        .events
        .iter()
        .filter_map(|event| event.execution_container.as_ref())
        .chain(item.execution_containers.iter())
    {
        if let Some(path) = container.worktree_path.as_deref() {
            let canonical = dunce::canonicalize(path).map_err(|_| {
                ContinueWorkFailure::failed(
                    "worktree_unavailable",
                    "This Work cannot be continued because its worktree is unavailable.",
                    false,
                )
            })?;
            if selected_path
                .as_deref()
                .is_some_and(|selected| !path_matches(selected, &canonical))
            {
                return Err(ContinueWorkFailure::failed(
                    "execution_container_ambiguous",
                    "The Work has multiple execution containers and cannot be continued safely.",
                    false,
                ));
            }
            if !worktree_matches_project_state(&canonical, project_root) {
                return Err(ContinueWorkFailure::failed(
                    "execution_container_foreign",
                    "The Work execution container does not belong to the active project.",
                    false,
                ));
            }
            selected_path.get_or_insert(canonical);
        }

        let branch = container
            .branch
            .as_deref()
            .map(str::trim)
            .filter(|branch| !branch.is_empty())
            .map(crate::runtime_support::normalize_branch_name);
        if let Some(branch) = branch {
            if selected_branch
                .as_deref()
                .is_some_and(|selected| selected != branch.as_str())
            {
                return Err(ContinueWorkFailure::failed(
                    "execution_container_ambiguous",
                    "The Work has conflicting execution branches and cannot be continued safely.",
                    false,
                ));
            }
            selected_branch.get_or_insert(branch);
        }
    }
    let worktree_path = selected_path.ok_or_else(|| {
        ContinueWorkFailure::failed(
            "execution_container_missing",
            "This Work has no execution container to continue.",
            false,
        )
    })?;
    let branch = selected_branch.ok_or_else(|| {
        ContinueWorkFailure::failed(
            "execution_branch_missing",
            "This Work has no execution branch to continue.",
            false,
        )
    })?;
    Ok((worktree_path, branch))
}

fn projection_only_continue_agent(
    item: &gwt_core::workspace_projection::WorkItem,
) -> Result<(gwt_agent::AgentId, Option<String>), ContinueWorkFailure> {
    let latest_at = item
        .agents
        .iter()
        .filter(|agent| {
            agent.agent_id.as_deref().is_some_and(|agent_id| {
                let agent_id = agent_id.trim();
                !agent_id.is_empty()
                    && !agent_id
                        .eq_ignore_ascii_case(gwt_core::workspace_projection::SHELL_WORK_AGENT_ID)
            })
        })
        .map(|agent| agent.updated_at)
        .max()
        .ok_or_else(|| {
            ContinueWorkFailure::failed(
                "agent_identity_missing",
                "This Work has no Agent identity for a new conversation.",
                false,
            )
        })?;
    let mut selected: Option<(gwt_agent::AgentId, Option<String>)> = None;
    for agent in item
        .agents
        .iter()
        .filter(|agent| agent.updated_at == latest_at)
    {
        let Some(raw_agent_id) = agent.agent_id.as_deref().map(str::trim).filter(|agent_id| {
            !agent_id.is_empty()
                && !agent_id
                    .eq_ignore_ascii_case(gwt_core::workspace_projection::SHELL_WORK_AGENT_ID)
        }) else {
            continue;
        };
        let Some(agent_id) = gwt_agent::resolve_agent_id(raw_agent_id) else {
            continue;
        };
        if matches!(agent_id, gwt_agent::AgentId::Custom(_)) {
            return Err(ContinueWorkFailure::failed(
                "agent_identity_unsupported",
                "This Work requires durable Session metadata to relaunch its custom Agent.",
                false,
            ));
        }
        if selected
            .as_ref()
            .is_some_and(|(selected, _)| selected != &agent_id)
        {
            return Err(ContinueWorkFailure::failed(
                "agent_identity_ambiguous",
                "The Work has conflicting latest Agent identities and cannot be continued safely.",
                false,
            ));
        }
        let display_name = agent
            .display_name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string);
        selected.get_or_insert((agent_id, display_name));
    }
    selected.ok_or_else(|| {
        ContinueWorkFailure::failed(
            "agent_identity_missing",
            "This Work has no Agent identity for a new conversation.",
            false,
        )
    })
}

fn work_agent_ref_authenticates_agent(
    agent: &gwt_core::workspace_projection::WorkAgentRef,
    agent_id: &gwt_agent::AgentId,
) -> bool {
    match agent.agent_id.as_deref() {
        None => false,
        Some(raw_agent_id) => {
            let raw_agent_id = raw_agent_id.trim();
            !raw_agent_id.is_empty()
                && !raw_agent_id
                    .eq_ignore_ascii_case(gwt_core::workspace_projection::SHELL_WORK_AGENT_ID)
                && gwt_agent::resolve_agent_id(raw_agent_id).as_ref() == Some(agent_id)
        }
    }
}

fn work_agent_ref_authenticates_session(
    agent: &gwt_core::workspace_projection::WorkAgentRef,
    session: &gwt_agent::Session,
) -> bool {
    agent.session_id == session.id && work_agent_ref_authenticates_agent(agent, &session.agent_id)
}

fn projection_continue_authority_matches(
    item: &gwt_core::workspace_projection::WorkItem,
    project_root: &Path,
    owner: gwt::cli::execution_state::ExecutionOwnerKey,
    worktree_path: &Path,
    branch: &str,
    agent_id: &gwt_agent::AgentId,
    agent_session_id: Option<&str>,
) -> bool {
    let Ok(projected_owner) = projection_only_continue_owner(item) else {
        return false;
    };
    if projected_owner.number != owner.number
        || projected_owner
            .declared_kind
            .is_some_and(|kind| kind != owner.kind)
    {
        return false;
    }
    let Ok((current_worktree, current_branch)) =
        projection_only_continue_container(item, project_root)
    else {
        return false;
    };
    if !path_matches(&current_worktree, worktree_path) || current_branch != branch {
        return false;
    }
    if let Some(agent_session_id) = agent_session_id {
        let mut matching = item
            .agents
            .iter()
            .filter(|agent| agent.session_id == agent_session_id);
        return matching
            .next()
            .is_some_and(|agent| work_agent_ref_authenticates_agent(agent, agent_id))
            && matching.next().is_none();
    }
    match projection_only_continue_agent(item) {
        Ok((current_agent, _)) => current_agent == *agent_id,
        Err(_) => false,
    }
}

pub(super) fn pending_execution_activation_status(pending: &PendingContinueWork) -> Option<bool> {
    match &pending.execution {
        PendingContinueWorkExecution::Successor(_) => {
            gwt::cli::execution_state::continuation_attempt_for_operation(
                &pending.worktree_path,
                pending.owner,
                &pending.operation_id,
            )
            .ok()
            .map(|attempt| {
                attempt.is_some_and(|attempt| {
                    attempt.status
                        == gwt::cli::execution_state::ContinuationAttemptStatus::Activated
                })
            })
        }
        PendingContinueWorkExecution::Takeover(_) => {
            gwt::cli::execution_state::generation_takeover_attempt_for_operation(
                &pending.worktree_path,
                pending.owner,
                &pending.operation_id,
            )
            .ok()
            .map(|attempt| {
                attempt.is_some_and(|attempt| {
                    attempt.status
                        == gwt::cli::execution_state::GenerationTakeoverAttemptStatus::Activated
                })
            })
        }
    }
}

pub(super) fn pending_fresh_execution_activation_status(
    pending: &PendingFreshExecutionLaunch,
) -> Option<bool> {
    pending_fresh_execution_attempt_status(pending)
        .map(|status| status == gwt::cli::execution_state::ContinuationAttemptStatus::Activated)
}

fn pending_fresh_execution_attempt_status(
    pending: &PendingFreshExecutionLaunch,
) -> Option<gwt::cli::execution_state::ContinuationAttemptStatus> {
    gwt::cli::execution_state::continuation_attempt_for_operation(
        &pending.worktree_path,
        pending.owner,
        &pending.operation_id,
    )
    .ok()
    .flatten()
    .map(|attempt| attempt.status)
}

#[derive(Debug)]
struct DurableFreshExecutionCandidate {
    project_root: PathBuf,
    worktree_path: PathBuf,
    owner: gwt::cli::execution_state::ExecutionOwnerKey,
    attempt: gwt::cli::execution_state::ContinuationAttempt,
    binding: gwt_agent::SessionExecutionBinding,
    session_identity: gwt_agent::SessionExecutionIdentity,
    agent_id: gwt_agent::AgentId,
}

const DURABLE_LAUNCH_RECOVERY_DIR: &str = "execution-launch-recovery";
const DURABLE_LAUNCH_RECOVERY_SCHEMA_VERSION: u32 = 3;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub(super) enum DurableLaunchRecoveryKind {
    Genesis,
    FreshSuccessor { operation_id: String },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct DurableLaunchRecoveryRecord {
    schema_version: u32,
    kind: DurableLaunchRecoveryKind,
    session_id: String,
    project_root: PathBuf,
    worktree_path: PathBuf,
    repo_hash: String,
    owner_kind: String,
    owner_number: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expected_binding: Option<gwt_agent::SessionExecutionBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expected_agent_id: Option<gwt_agent::AgentId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expected_session_identity: Option<gwt_agent::SessionExecutionIdentity>,
}

impl DurableLaunchRecoveryRecord {
    fn owner(&self) -> Result<gwt::cli::execution_state::ExecutionOwnerKey, String> {
        let kind = match self.owner_kind.as_str() {
            "spec" => gwt::cli::execution_state::ExecutionOwnerKind::Spec,
            "issue" => gwt::cli::execution_state::ExecutionOwnerKind::Issue,
            _ => return Err("launch recovery owner kind is not canonical".to_string()),
        };
        if self.schema_version != DURABLE_LAUNCH_RECOVERY_SCHEMA_VERSION
            || self.owner_number == 0
            || self.repo_hash.trim().is_empty()
        {
            return Err("launch recovery record schema or owner is invalid".to_string());
        }
        gwt_agent::validate_session_id_path_component(&self.session_id)?;
        if let DurableLaunchRecoveryKind::FreshSuccessor { operation_id } = &self.kind {
            if !canonical_public_id(operation_id, 256) {
                return Err("launch recovery operation id is not canonical".to_string());
            }
        }
        if let Some(binding) = self.expected_binding.as_ref() {
            if binding.schema_version != gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION
                || binding.session_id != self.session_id
                || binding.repo_hash != self.repo_hash
                || binding.owner_kind != self.owner_kind
                || binding.owner_number != self.owner_number
                || binding.capability_generation == 0
                || !canonical_public_id(&binding.identity.generation_id, 512)
                || !canonical_public_id(&binding.identity.binding_id, 512)
                || !canonical_public_id(&binding.identity.ledger_head_hash, 512)
            {
                return Err("launch recovery expected Session binding is not canonical".to_string());
            }
        }
        if self.expected_binding.is_some() != self.expected_agent_id.is_some()
            || self.expected_binding.is_some() != self.expected_session_identity.is_some()
        {
            return Err("launch recovery exact Session identity proof is incomplete".to_string());
        }
        if let Some(identity) = self.expected_session_identity.as_ref() {
            if identity.session_id != self.session_id
                || identity.worktree_path != self.worktree_path
                || identity.project_state_root.as_deref() != Some(self.project_root.as_path())
                || identity.repo_hash.as_deref() != Some(self.repo_hash.as_str())
                || identity.linked_issue_number != Some(self.owner_number)
                || self.expected_binding.as_ref() != Some(&identity.execution_binding)
                || self.expected_agent_id.as_ref() != Some(&identity.agent_id)
            {
                return Err(
                    "launch recovery expected Session identity is not canonical".to_string()
                );
            }
        }
        Ok(gwt::cli::execution_state::ExecutionOwnerKey {
            kind,
            number: self.owner_number,
        })
    }
}

fn durable_launch_recovery_dir(sessions_dir: &Path) -> PathBuf {
    sessions_dir.join(DURABLE_LAUNCH_RECOVERY_DIR)
}

fn durable_launch_recovery_path(sessions_dir: &Path, session_id: &str) -> Result<PathBuf, String> {
    gwt_agent::validate_session_id_path_component(session_id)?;
    Ok(durable_launch_recovery_dir(sessions_dir).join(format!("{session_id}.json")))
}

#[cfg(test)]
// Only the `#[cfg(unix)]` directory-sync tests install this hook.
#[cfg_attr(not(unix), allow(dead_code))]
pub(super) fn set_durable_launch_recovery_directory_sync_test_hook(
    hook: Option<DurableLaunchRecoveryDirectorySyncHook>,
) {
    DURABLE_LAUNCH_RECOVERY_DIRECTORY_SYNC_HOOK.with(|slot| {
        *slot.borrow_mut() = hook;
    });
}

// `directory` is only read by the test hook and the `#[cfg(unix)]` fsync.
#[cfg_attr(not(any(test, unix)), allow(unused_variables))]
fn sync_durable_launch_recovery_directory(directory: &Path) -> std::io::Result<()> {
    #[cfg(test)]
    if let Some(result) = DURABLE_LAUNCH_RECOVERY_DIRECTORY_SYNC_HOOK
        .with(|slot| slot.borrow().as_ref().map(|hook| hook(directory)))
    {
        return result;
    }

    #[cfg(unix)]
    {
        std::fs::File::open(directory)?.sync_all()?;
    }
    Ok(())
}

fn create_durable_launch_recovery_directory(directory: &Path) -> std::io::Result<()> {
    let mut missing = Vec::new();
    let mut cursor = directory;
    while !cursor.as_os_str().is_empty() && !cursor.exists() {
        missing.push(cursor.to_path_buf());
        let Some(parent) = cursor.parent() else {
            break;
        };
        cursor = parent;
    }
    std::fs::create_dir_all(directory)?;

    if missing.is_empty() {
        sync_durable_launch_recovery_directory(directory)?;
        if let Some(parent) = directory
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            sync_durable_launch_recovery_directory(parent)?;
        }
        return Ok(());
    }

    for created in missing.iter().rev() {
        sync_durable_launch_recovery_directory(created)?;
        if let Some(parent) = created
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            sync_durable_launch_recovery_directory(parent)?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn persist_durable_launch_recovery(
    sessions_dir: &Path,
    kind: DurableLaunchRecoveryKind,
    session_id: &str,
    project_root: &Path,
    worktree_path: &Path,
    owner: gwt::cli::execution_state::ExecutionOwnerKey,
    expected_binding: Option<&gwt_agent::SessionExecutionBinding>,
    expected_agent_id: Option<&gwt_agent::AgentId>,
) -> Result<(), String> {
    let expected_session_identity = match (expected_binding, expected_agent_id) {
        (None, None) => None,
        (Some(binding), Some(agent_id)) => {
            let session_path = sessions_dir.join(format!("{session_id}.toml"));
            let session = match std::fs::symlink_metadata(&session_path) {
                Ok(_) => gwt_agent::Session::load(&session_path).map_err(|error| {
                    format!("launch recovery Session identity could not be read: {error}")
                })?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    let branch = gwt_git::Repository::discover(worktree_path)
                        .and_then(|repository| repository.current_branch())
                        .map_err(|error| error.to_string())?
                        .ok_or_else(|| {
                            "launch recovery Session identity has no canonical branch".to_string()
                        })?;
                    let mut session =
                        gwt_agent::Session::new(worktree_path, branch, agent_id.clone());
                    session.id = session_id.to_string();
                    session.project_state_root = Some(project_root.to_path_buf());
                    session.repo_hash = Some(binding.repo_hash.clone());
                    session.linked_issue_number = Some(owner.number);
                    session
                }
                Err(error) => return Err(error.to_string()),
            };
            if &session.agent_id != agent_id {
                return Err("launch recovery Session Agent identity changed".to_string());
            }
            Some(gwt_agent::SessionExecutionIdentity::for_binding(
                &session, binding,
            )?)
        }
        _ => return Err("launch recovery exact Session identity proof is incomplete".to_string()),
    };
    persist_durable_launch_recovery_with_identity(
        sessions_dir,
        kind,
        session_id,
        project_root,
        worktree_path,
        owner,
        expected_binding,
        expected_agent_id,
        expected_session_identity.as_ref(),
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn persist_durable_launch_recovery_with_identity(
    sessions_dir: &Path,
    kind: DurableLaunchRecoveryKind,
    session_id: &str,
    project_root: &Path,
    worktree_path: &Path,
    owner: gwt::cli::execution_state::ExecutionOwnerKey,
    expected_binding: Option<&gwt_agent::SessionExecutionBinding>,
    expected_agent_id: Option<&gwt_agent::AgentId>,
    expected_session_identity: Option<&gwt_agent::SessionExecutionIdentity>,
) -> Result<(), String> {
    let record = DurableLaunchRecoveryRecord {
        schema_version: DURABLE_LAUNCH_RECOVERY_SCHEMA_VERSION,
        kind,
        session_id: session_id.to_string(),
        project_root: project_root.to_path_buf(),
        worktree_path: worktree_path.to_path_buf(),
        repo_hash: gwt_core::repo_hash::detect_repo_hash(worktree_path)
            .ok_or_else(|| "launch recovery worktree has no canonical repository hash".to_string())?
            .to_string(),
        owner_kind: owner.kind.as_str().to_string(),
        owner_number: owner.number,
        expected_binding: expected_binding.cloned(),
        expected_agent_id: expected_agent_id.cloned(),
        expected_session_identity: expected_session_identity.cloned(),
    };
    record.owner()?;
    let path = durable_launch_recovery_path(sessions_dir, session_id)?;
    let parent = path
        .parent()
        .ok_or_else(|| "launch recovery path has no parent".to_string())?;
    create_durable_launch_recovery_directory(parent).map_err(|error| error.to_string())?;
    let bytes = serde_json::to_vec_pretty(&record).map_err(|error| error.to_string())?;
    gwt_github::cache::write_atomic(&path, &bytes).map_err(|error| error.to_string())?;
    sync_durable_launch_recovery_directory(parent).map_err(|error| error.to_string())
}

pub(super) fn clear_durable_launch_recovery(
    sessions_dir: &Path,
    session_id: &str,
) -> Result<(), String> {
    let path = durable_launch_recovery_path(sessions_dir, session_id)?;
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    let parent = path
        .parent()
        .ok_or_else(|| "launch recovery path has no parent".to_string())?;
    let durability_barrier = if parent.exists() {
        Some(parent)
    } else {
        parent.parent().filter(|ancestor| ancestor.exists())
    };
    durability_barrier.map_or(Ok(()), |directory| {
        sync_durable_launch_recovery_directory(directory).map_err(|error| error.to_string())
    })
}

pub(super) fn durable_launch_recovery_session_identity(
    sessions_dir: &Path,
    session_id: &str,
) -> Result<Option<gwt_agent::SessionExecutionIdentity>, String> {
    let path = durable_launch_recovery_path(sessions_dir, session_id)?;
    match std::fs::symlink_metadata(&path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    }
    let bytes = std::fs::read(&path).map_err(|error| error.to_string())?;
    let record: DurableLaunchRecoveryRecord =
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    record.owner()?;
    if record.session_id != session_id {
        return Err("launch recovery Session id changed".to_string());
    }
    Ok(record.expected_session_identity)
}

pub(super) fn durable_launch_recovery_exists(sessions_dir: &Path, session_id: &str) -> bool {
    durable_launch_recovery_path(sessions_dir, session_id).is_ok_and(|path| path.exists())
}

fn durable_launch_recovery_records(
    sessions_dir: &Path,
) -> Vec<(PathBuf, DurableLaunchRecoveryRecord)> {
    let Ok(entries) = std::fs::read_dir(durable_launch_recovery_dir(sessions_dir)) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                return None;
            }
            let bytes = match std::fs::read(&path) {
                Ok(bytes) => bytes,
                Err(error) => {
                    tracing::warn!(path = %path.display(), error = %error, "launch recovery receipt could not be read");
                    return None;
                }
            };
            let record = match serde_json::from_slice::<DurableLaunchRecoveryRecord>(&bytes) {
                Ok(record) => record,
                Err(error) => {
                    tracing::warn!(path = %path.display(), error = %error, "launch recovery receipt is malformed");
                    return None;
                }
            };
            if let Err(error) = record.owner() {
                tracing::warn!(path = %path.display(), error = %error, "launch recovery receipt is invalid");
                return None;
            }
            if path.file_stem().and_then(|value| value.to_str())
                != Some(record.session_id.as_str())
            {
                tracing::warn!(path = %path.display(), "launch recovery receipt filename does not match its Session identity");
                return None;
            }
            Some((path, record))
        })
        .collect()
}

fn durable_launch_recovery_repo_matches(record: &DurableLaunchRecoveryRecord) -> bool {
    if gwt_core::repo_hash::detect_repo_hash(&record.worktree_path)
        .is_none_or(|repo_hash| repo_hash.to_string() != record.repo_hash)
    {
        return false;
    }
    gwt_git::worktree::main_worktree_root(&record.worktree_path)
        .is_ok_and(|anchor| path_matches(&anchor, &record.project_root))
}

#[derive(Clone, Copy)]
enum GenesisCompensationAuthority {
    Terminalized,
    SupersededByDifferentOwner,
}

fn genesis_initial_execution_binding_matches(
    worktree_path: &Path,
    owner: gwt::cli::execution_state::ExecutionOwnerKey,
    session_id: &str,
    binding: &gwt_agent::SessionExecutionBinding,
) -> bool {
    binding.schema_version == gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION
        && binding.session_id == session_id
        && gwt_core::repo_hash::detect_repo_hash(worktree_path)
            .is_some_and(|repo_hash| repo_hash.to_string() == binding.repo_hash)
        && binding.owner_kind == owner.kind.as_str()
        && binding.owner_number == owner.number
        && binding.capability_generation > 0
        && gwt::cli::execution_state::genesis_initial_execution_binding_matches(
            worktree_path,
            owner,
            session_id,
            &binding.identity,
        )
        .unwrap_or(false)
}

fn genesis_compensation_authority_matches(
    worktree_path: &Path,
    owner: gwt::cli::execution_state::ExecutionOwnerKey,
    session_id: &str,
    expected_binding: Option<&gwt_agent::SessionExecutionBinding>,
    authority: GenesisCompensationAuthority,
) -> Result<(), String> {
    let ledger = gwt::cli::execution_state::load_owner_generation_ledger(worktree_path, owner)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "terminalized genesis generation ledger is missing".to_string())?;
    let generation = ledger
        .generations
        .iter()
        .find(|generation| {
            generation.identity.predecessor_generation_id.is_none()
                && generation.identity.initial_session_id == session_id
                && expected_binding.is_none_or(|binding| {
                    generation.identity.generation_id == binding.identity.generation_id
                        && generation.identity.session_binding_id == binding.identity.binding_id
                })
        })
        .ok_or_else(|| "terminalized genesis generation is missing".to_string())?;
    if ledger.owner != owner {
        return Err("genesis authority owner no longer matches the failed launch".to_string());
    }
    if expected_binding.is_some_and(|binding| {
        !genesis_initial_execution_binding_matches(worktree_path, owner, session_id, binding)
    }) {
        return Err("terminalized genesis binding no longer matches the failed launch".to_string());
    }
    match authority {
        GenesisCompensationAuthority::Terminalized => {
            let terminal = ledger
                .lifecycle_events
                .iter()
                .filter(|event| event.generation_id == generation.identity.generation_id)
                .max_by_key(|event| event.sequence);
            let terminalization_operation_id =
                gwt::cli::execution_state::genesis_terminalization_operation_id(
                    &generation.identity.generation_id,
                    &generation.identity.session_binding_id,
                );
            if !terminal.is_some_and(|event| {
                event.from_status == gwt::cli::execution_state::ExecutionControlStatus::Active
                    && event.to_status == gwt::cli::execution_state::ExecutionControlStatus::Blocked
                    && event.session_id == session_id
                    && event.operation_id.as_deref() == Some(terminalization_operation_id.as_str())
            }) {
                return Err(
                    "terminalized genesis authority no longer matches the failed launch"
                        .to_string(),
                );
            }
        }
        GenesisCompensationAuthority::SupersededByDifferentOwner => {
            let current_owner = gwt::cli::execution_state::recovery_generation_owner(worktree_path)
                .map_err(|error| error.to_string())?;
            if !current_owner.is_some_and(|current_owner| current_owner != owner) {
                return Err(
                    "genesis authority is not superseded by a different current owner".to_string(),
                );
            }
        }
    }
    Ok(())
}

fn failed_genesis_work_has_session_event(
    item: &gwt_core::workspace_projection::WorkItem,
    session_id: &str,
) -> bool {
    item.events
        .iter()
        .any(|event| event.agent_session_id.as_deref() == Some(session_id))
}

fn failed_genesis_work_is_new(
    item: &gwt_core::workspace_projection::WorkItem,
    session_id: &str,
) -> bool {
    item.legacy_metadata_snapshot.is_none()
        && !item.legacy_metadata_authoritative
        && item.duplicate_event_containers.is_empty()
        && item.events.len() == 1
        && item.events[0].kind == gwt_core::workspace_projection::WorkEventKind::Start
        && item.events[0].agent_session_id.as_deref() == Some(session_id)
        && item.agents.len() == 1
        && item.agents[0].session_id == session_id
}

fn failed_genesis_work_is_already_paused(
    item: &gwt_core::workspace_projection::WorkItem,
    session_id: &str,
) -> bool {
    item.events
        .iter()
        .rev()
        .find(|event| event.agent_session_id.as_deref() == Some(session_id))
        .is_some_and(|event| event.kind == gwt_core::workspace_projection::WorkEventKind::Pause)
}

fn failed_genesis_agent_matches_container(
    agent: &gwt_core::workspace_projection::WorkspaceAgentSummary,
    worktree_path: &Path,
    branch_hint: Option<&str>,
) -> Result<String, String> {
    let agent_worktree = agent
        .worktree_path
        .as_deref()
        .ok_or_else(|| "failed genesis Workspace agent has no canonical worktree".to_string())?;
    if !path_matches(agent_worktree, worktree_path) {
        return Err(
            "failed genesis Workspace agent no longer matches its launch container".to_string(),
        );
    }
    let branch = agent
        .branch
        .as_deref()
        .map(str::trim)
        .filter(|branch| !branch.is_empty())
        .ok_or_else(|| "failed genesis Workspace agent has no canonical branch".to_string())?;
    if branch_hint
        .map(str::trim)
        .filter(|hint| !hint.is_empty())
        .is_some_and(|hint| hint != branch)
    {
        return Err("failed genesis Workspace branch changed before compensation".to_string());
    }
    Ok(branch.to_string())
}

fn failed_genesis_work_container_matches(
    item: &gwt_core::workspace_projection::WorkItem,
    worktree_path: &Path,
    branch_hint: Option<&str>,
) -> bool {
    let branch_hint = branch_hint
        .map(str::trim)
        .filter(|branch| !branch.is_empty());
    item.execution_containers
        .iter()
        .filter(|container| {
            container
                .worktree_path
                .as_deref()
                .is_some_and(|path| path_matches(path, worktree_path))
                && container
                    .branch
                    .as_deref()
                    .map(str::trim)
                    .filter(|branch| !branch.is_empty())
                    .is_some_and(|branch| branch_hint.is_none_or(|hint| hint == branch))
        })
        .count()
        == 1
}

pub(super) fn compensate_terminalized_genesis_workspace_projection(
    project_root: &Path,
    worktree_path: &Path,
    owner: gwt::cli::execution_state::ExecutionOwnerKey,
    session_id: &str,
    expected_binding: Option<&gwt_agent::SessionExecutionBinding>,
    branch_hint: Option<&str>,
) -> Result<(), String> {
    compensate_genesis_workspace_projection(
        project_root,
        worktree_path,
        owner,
        session_id,
        expected_binding,
        branch_hint,
        GenesisCompensationAuthority::Terminalized,
    )
}

fn compensate_genesis_workspace_projection(
    project_root: &Path,
    worktree_path: &Path,
    owner: gwt::cli::execution_state::ExecutionOwnerKey,
    session_id: &str,
    expected_binding: Option<&gwt_agent::SessionExecutionBinding>,
    branch_hint: Option<&str>,
    authority: GenesisCompensationAuthority,
) -> Result<(), String> {
    genesis_compensation_authority_matches(
        worktree_path,
        owner,
        session_id,
        expected_binding,
        authority,
    )?;
    gwt_core::workspace_projection::recover_pending_workspace_state_transaction(project_root)
        .map_err(|error| error.to_string())?;
    genesis_compensation_authority_matches(
        worktree_path,
        owner,
        session_id,
        expected_binding,
        authority,
    )?;

    let projection = gwt_core::workspace_projection::load_workspace_projection(project_root)
        .map_err(|error| error.to_string())?;
    let work_items = gwt_core::workspace_projection::load_workspace_work_items(project_root)
        .map_err(|error| error.to_string())?;
    if work_items.is_none() {
        if let Some(agent) = projection
            .as_ref()
            .and_then(|projection| projection.latest_agent_for_session(session_id))
        {
            failed_genesis_agent_matches_container(agent, worktree_path, branch_hint)?;
            if agent.affiliation_status
                == gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned
            {
                return Err(
                    "failed genesis Workspace assignment has no exact Work event evidence"
                        .to_string(),
                );
            }
        } else {
            return Ok(());
        }
        gwt_core::workspace_projection::mark_workspace_agent_stopped(
            project_root,
            session_id,
            None,
        )
        .map_err(|error| error.to_string())?;
        return Ok(());
    }

    let snapshot_work_items = work_items.as_ref().expect("checked WorkItems presence");
    let snapshot_targets = snapshot_work_items
        .work_items
        .iter()
        .filter(|item| failed_genesis_work_has_session_event(item, session_id))
        .collect::<Vec<_>>();
    if snapshot_targets.len() > 1 {
        return Err("failed genesis Session has multiple exact Work event targets".to_string());
    }
    if snapshot_targets.is_empty() {
        let snapshot_agent = projection
            .as_ref()
            .and_then(|projection| projection.latest_agent_for_session(session_id));
        if let Some(agent) = snapshot_agent {
            failed_genesis_agent_matches_container(agent, worktree_path, branch_hint)?;
            if agent.affiliation_status
                == gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned
            {
                return Err(
                    "failed genesis Workspace assignment has no exact Work event evidence"
                        .to_string(),
                );
            }
            gwt_core::workspace_projection::mark_workspace_agent_stopped(
                project_root,
                session_id,
                None,
            )
            .map_err(|error| error.to_string())?;
        }
        return Ok(());
    }
    let snapshot_target = snapshot_targets[0];
    let snapshot_agent = projection
        .as_ref()
        .and_then(|projection| projection.latest_agent_for_session(session_id));
    if snapshot_agent.is_none() {
        if expected_binding.is_none()
            || !failed_genesis_work_container_matches(snapshot_target, worktree_path, branch_hint)
        {
            return Err(
                "failed genesis compensated Work has no exact recovery identity".to_string(),
            );
        }
        let another_assigned_agent = projection.as_ref().is_some_and(|projection| {
            projection.agents.iter().any(|candidate| {
                candidate.affiliation_status
                    == gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned
                    && candidate.workspace_id.as_deref() == Some(snapshot_target.id.as_str())
            })
        });
        let exact_discard = snapshot_target.discarded
            && snapshot_target.events.iter().rev().any(|event| {
                event.kind == gwt_core::workspace_projection::WorkEventKind::Discard
                    && event.agent_session_id.as_deref() == Some(session_id)
            });
        if exact_discard
            || failed_genesis_work_is_already_paused(snapshot_target, session_id)
            || another_assigned_agent
        {
            return Ok(());
        }
        return Err(
            "failed genesis Work is not durably compensated after agent cleanup".to_string(),
        );
    }

    gwt_core::workspace_projection::transact_workspace_state(
        project_root,
        |projection, work_items, _persisted| {
            genesis_compensation_authority_matches(
                worktree_path,
                owner,
                session_id,
                expected_binding,
                authority,
            )
            .map_err(gwt_core::GwtError::Other)?;
            let matching_agents = projection
                .agents
                .iter()
                .filter(|agent| agent.session_id == session_id)
                .collect::<Vec<_>>();
            if matching_agents.len() > 1 {
                return Err(gwt_core::GwtError::Other(
                    "failed genesis Workspace Session is duplicated in current state".to_string(),
                ));
            }
            let agent = matching_agents.first().copied();
            let mut matching_items = work_items.work_items.iter().filter(|item| {
                item.is_incomplete() && failed_genesis_work_has_session_event(item, session_id)
            });
            let item = matching_items.next();
            if matching_items.next().is_some() {
                return Err(gwt_core::GwtError::Other(
                    "failed genesis Session has multiple incomplete Work event targets".to_string(),
                ));
            }
            let Some(item) = item else {
                let mut terminal_items = work_items.work_items.iter().filter(|item| {
                    item.discarded
                        && failed_genesis_work_has_session_event(item, session_id)
                        && item.events.iter().rev().any(|event| {
                            event.kind == gwt_core::workspace_projection::WorkEventKind::Discard
                                && event.agent_session_id.as_deref() == Some(session_id)
                        })
                });
                let terminal_item = terminal_items.next();
                if terminal_items.next().is_some() {
                    return Err(gwt_core::GwtError::Other(
                        "failed genesis Session has multiple terminal Work event targets"
                            .to_string(),
                    ));
                }
                if let Some(terminal_item) = terminal_item {
                    if expected_binding.is_none() {
                        return Err(gwt_core::GwtError::Other(
                            "failed genesis terminal Work has no exact recovery binding"
                                .to_string(),
                        ));
                    }
                    let agent = agent.ok_or_else(|| {
                        gwt_core::GwtError::Other(
                            "failed genesis terminal Work has no exact current Session assignment"
                                .to_string(),
                        )
                    })?;
                    let branch =
                        failed_genesis_agent_matches_container(agent, worktree_path, branch_hint)
                            .map_err(gwt_core::GwtError::Other)?;
                    if agent.affiliation_status
                        != gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned
                        || agent.workspace_id.as_deref() != Some(terminal_item.id.as_str())
                        || terminal_item
                            .execution_containers
                            .iter()
                            .filter(|container| {
                                container.branch.as_deref() == Some(branch.as_str())
                                    && container
                                        .worktree_path
                                        .as_deref()
                                        .is_some_and(|path| path_matches(path, worktree_path))
                            })
                            .count()
                            != 1
                    {
                        return Err(gwt_core::GwtError::Other(
                            "failed genesis terminal Work assignment changed before cleanup"
                                .to_string(),
                        ));
                    }
                    return Ok(((), Vec::new()));
                }
                if agent.is_some_and(|agent| {
                    agent.affiliation_status
                        == gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned
                }) {
                    return Err(gwt_core::GwtError::Other(
                        "failed genesis Workspace assignment has no exact Work event evidence"
                            .to_string(),
                    ));
                }
                return Ok(((), Vec::new()));
            };
            if expected_binding.is_none() {
                return Err(gwt_core::GwtError::Other(
                    "failed genesis Work evidence has no exact recovery binding".to_string(),
                ));
            }
            let agent = agent.ok_or_else(|| {
                gwt_core::GwtError::Other(
                    "failed genesis Work has no exact current Session assignment".to_string(),
                )
            })?;
            let branch = failed_genesis_agent_matches_container(agent, worktree_path, branch_hint)
                .map_err(gwt_core::GwtError::Other)?;
            if agent.affiliation_status
                != gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned
                || agent.workspace_id.as_deref() != Some(item.id.as_str())
            {
                return Err(gwt_core::GwtError::Other(
                    "failed genesis Workspace assignment changed before compensation".to_string(),
                ));
            }
            let matching_containers = item
                .execution_containers
                .iter()
                .filter(|container| {
                    container.branch.as_deref() == Some(branch.as_str())
                        && container
                            .worktree_path
                            .as_deref()
                            .is_some_and(|path| path_matches(path, worktree_path))
                })
                .cloned()
                .collect::<Vec<_>>();
            if matching_containers.len() != 1 {
                return Err(gwt_core::GwtError::Other(
                    "failed genesis Work container changed or became ambiguous".to_string(),
                ));
            }
            let another_assigned_agent = projection.agents.iter().any(|candidate| {
                candidate.session_id != session_id
                    && candidate.affiliation_status
                        == gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned
                    && candidate.workspace_id.as_deref() == Some(item.id.as_str())
            });
            let now = chrono::Utc::now();
            let event = if failed_genesis_work_is_new(item, session_id) && !another_assigned_agent {
                let mut event = gwt_core::workspace_projection::WorkEvent::new(
                    gwt_core::workspace_projection::WorkEventKind::Discard,
                    &item.id,
                    now,
                );
                event.agent_session_id = Some(session_id.to_string());
                Some(event)
            } else if !another_assigned_agent
                && !failed_genesis_work_is_already_paused(item, session_id)
            {
                let mut event = gwt_core::workspace_projection::WorkEvent::new(
                    gwt_core::workspace_projection::WorkEventKind::Pause,
                    &item.id,
                    now,
                );
                event.title = Some(item.title.clone());
                event.summary = item.summary.clone();
                event.owner = item.owner.clone();
                event.agent_session_id = Some(session_id.to_string());
                event.execution_container = matching_containers.into_iter().next();
                Some(event)
            } else {
                None
            };
            genesis_compensation_authority_matches(
                worktree_path,
                owner,
                session_id,
                expected_binding,
                authority,
            )
            .map_err(gwt_core::GwtError::Other)?;
            Ok(((), event.into_iter().collect()))
        },
    )
    .map_err(|error| error.to_string())?;

    genesis_compensation_authority_matches(
        worktree_path,
        owner,
        session_id,
        expected_binding,
        authority,
    )?;
    gwt_core::workspace_projection::mark_workspace_agent_stopped(project_root, session_id, None)
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn durable_fresh_execution_candidate(
    session: &gwt_agent::Session,
) -> Result<Option<DurableFreshExecutionCandidate>, String> {
    let Some(binding) = session.execution_binding.clone() else {
        return Ok(None);
    };
    if binding.schema_version != gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION
        || binding.session_id != session.id
        || session.linked_issue_number != Some(binding.owner_number)
        || session.repo_hash.as_deref() != Some(binding.repo_hash.as_str())
    {
        return Err("persisted fresh-launch Session binding is not canonical".to_string());
    }
    let owner_kind = match binding.owner_kind.as_str() {
        "spec" => gwt::cli::execution_state::ExecutionOwnerKind::Spec,
        "issue" => gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        _ => return Err("persisted fresh-launch owner kind is not canonical".to_string()),
    };
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: owner_kind,
        number: binding.owner_number,
    };
    let project_root = gwt::validated_project_state_root_for_session_recovery(session)
        .map_err(|error| error.to_string())?;
    let Some(attempt) = gwt::cli::execution_state::fresh_linked_owner_launch_for_session(
        &session.worktree_path,
        owner,
        &session.id,
    )
    .map_err(|error| error.to_string())?
    else {
        return Ok(None);
    };
    if binding.identity.generation_id != attempt.candidate_generation_id
        || binding.identity.binding_id != attempt.request.session_binding_id
    {
        return Err(
            "persisted fresh-launch Session does not match its durable candidate generation"
                .to_string(),
        );
    }
    Ok(Some(DurableFreshExecutionCandidate {
        project_root,
        worktree_path: session.worktree_path.clone(),
        owner,
        attempt,
        binding,
        session_identity: gwt_agent::SessionExecutionIdentity::from_session(session)
            .map_err(|error| {
                format!("persisted fresh-launch Session identity is invalid: {error}")
            })?
            .ok_or_else(|| "persisted fresh-launch Session identity is missing".to_string())?,
        agent_id: session.agent_id.clone(),
    }))
}

fn pending_execution_is_activated(pending: &PendingContinueWork) -> bool {
    pending_execution_activation_status(pending) == Some(true)
}

pub(super) fn abort_prepared_execution(
    worktree: &Path,
    owner: gwt::cli::execution_state::ExecutionOwnerKey,
    execution: &PendingContinueWorkExecution,
    reason: &str,
) -> std::io::Result<()> {
    match execution {
        PendingContinueWorkExecution::Successor(request) => {
            gwt::cli::execution_state::abort_successor(worktree, owner, request, reason).map(|_| ())
        }
        PendingContinueWorkExecution::Takeover(request) => {
            gwt::cli::execution_state::abort_generation_takeover(worktree, owner, request, reason)
                .map(|_| ())
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn abort_prepared_execution_and_remove_exact_session<F>(
    worktree: &Path,
    owner: gwt::cli::execution_state::ExecutionOwnerKey,
    execution: &PendingContinueWorkExecution,
    reason: &str,
    sessions_dir: &Path,
    session_identity: &gwt_agent::SessionExecutionIdentity,
    after_abort: F,
) -> std::io::Result<bool>
where
    F: FnOnce() -> std::io::Result<()>,
{
    match execution {
        PendingContinueWorkExecution::Successor(request) => {
            gwt::cli::execution_state::abort_successor_and_remove_exact_session(
                worktree,
                owner,
                request,
                reason,
                sessions_dir,
                session_identity,
                after_abort,
            )
        }
        PendingContinueWorkExecution::Takeover(request) => {
            gwt::cli::execution_state::abort_generation_takeover_and_remove_exact_session(
                worktree,
                owner,
                request,
                reason,
                sessions_dir,
                session_identity,
                after_abort,
            )
        }
    }
}

fn pending_continue_work_session_identity(
    pending: &PendingContinueWork,
) -> Result<gwt_agent::SessionExecutionIdentity, String> {
    let mut session = gwt_agent::Session::new(
        &pending.worktree_path,
        pending.work_branch.clone(),
        pending.work_agent_id.clone(),
    );
    session.id = pending.binding.session_id.clone();
    session.project_state_root = Some(pending.project_root.clone());
    session.repo_hash = Some(pending.binding.repo_hash.clone());
    session.linked_issue_number = Some(pending.owner.number);
    gwt_agent::SessionExecutionIdentity::for_binding(&session, &pending.binding)
}

fn continue_work_commit_readback_matches(pending: &PendingContinueWork) -> bool {
    if gwt::cli::execution_state::current_execution_binding(&pending.worktree_path, pending.owner)
        .ok()
        .flatten()
        != Some(pending.binding.identity.clone())
    {
        return false;
    }
    gwt_core::workspace_projection::load_workspace_work_items(&pending.worktree_path)
        .ok()
        .flatten()
        .is_some_and(|projection| {
            let matching = projection
                .work_items
                .iter()
                .filter(|item| item.id == pending.work_id)
                .collect::<Vec<_>>();
            matching.len() == 1
                && !matching[0].discarded
                && matching[0].status_category
                    == gwt_core::workspace_projection::WorkspaceStatusCategory::Active
                && projection_continue_authority_matches(
                    matching[0],
                    &pending.project_root,
                    pending.owner,
                    &pending.worktree_path,
                    &pending.work_branch,
                    &pending.work_agent_id,
                    Some(&pending.binding.session_id),
                )
        })
}

fn transact_pending_continue_work_with_activation(
    pending: &PendingContinueWork,
    active_session: &super::ActiveAgentSession,
    live_session_ids: &HashSet<String>,
    activate: &mut dyn FnMut() -> std::io::Result<()>,
) -> std::io::Result<()> {
    gwt_core::workspace_projection::transact_workspace_state_at_with_commit(
        &gwt_core::paths::gwt_workspace_projection_path_for_repo_path(&pending.project_root),
        &gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&pending.worktree_path),
        &gwt_core::paths::gwt_repo_local_work_events_path(&pending.worktree_path),
        &pending.project_root,
        &pending.operation_id,
        |projection, work_items, _| {
            let matching = work_items
                .work_items
                .iter()
                .filter(|item| item.id == pending.work_id)
                .collect::<Vec<_>>();
            if matching.len() != 1 || matching[0].discarded {
                return Err(gwt_core::error::GwtError::Other(
                    "Continue work target changed before activation".to_string(),
                ));
            }
            if !projection_continue_authority_matches(
                matching[0],
                &pending.project_root,
                pending.owner,
                &pending.worktree_path,
                &pending.work_branch,
                &pending.work_agent_id,
                pending.work_agent_session_id.as_deref(),
            ) {
                return Err(gwt_core::error::GwtError::Other(
                    "Continue work authority changed before activation".to_string(),
                ));
            }
            let event = apply_workspace_launch_transition(
                projection,
                active_session,
                WorkspaceLaunchTransition {
                    work_id: Some(pending.work_id.clone()),
                    base_branch: None,
                    linked_issue_number: Some(pending.owner.number),
                    resume_context: Some(&pending.resume_context),
                    kind: WorkspaceLaunchProjectionKind::Resume {
                        created_by_start_work: active_session.branch_name.starts_with("work/"),
                    },
                    live_session_ids,
                    now: chrono::Utc::now(),
                },
            );
            Ok(((), vec![event]))
        },
        || activate().map_err(gwt_core::error::GwtError::Io),
    )
    .map_err(|error| std::io::Error::other(error.to_string()))?;
    if !continue_work_commit_readback_matches(pending) {
        return Err(std::io::Error::other(
            "Continue work commit readback does not match exact authority",
        ));
    }
    Ok(())
}

fn fresh_execution_commit_readback_matches(
    worktree_path: &Path,
    owner: gwt::cli::execution_state::ExecutionOwnerKey,
    session: &gwt_agent::SessionExecutionIdentity,
) -> bool {
    gwt::cli::execution_state::current_execution_binding(worktree_path, owner)
        .ok()
        .flatten()
        == Some(session.execution_binding.identity.clone())
}

fn resolve_activated_fresh_execution_commit(
    project_root: &Path,
    worktree_path: &Path,
    owner: gwt::cli::execution_state::ExecutionOwnerKey,
    operation_id: &str,
    request: &gwt::cli::execution_state::SuccessorRequest,
    sessions_dir: &Path,
    session: &gwt_agent::SessionExecutionIdentity,
) -> std::io::Result<bool> {
    gwt::cli::execution_state::with_activated_successor_exact_session_repair(
        worktree_path,
        owner,
        request,
        sessions_dir,
        session,
        |repair| {
            repair()?;
            let resolution = resolve_split_workspace_state_external_commit(
                project_root,
                worktree_path,
                operation_id,
                gwt_core::workspace_projection::ExternalWorkspaceCommitDecision::Commit,
            )
            .map_err(|error| std::io::Error::other(error.to_string()))?;
            Ok(resolution
                == gwt_core::workspace_projection::ExternalWorkspaceCommitResolution::Committed
                && fresh_execution_commit_readback_matches(worktree_path, owner, session))
        },
    )
    .map(|result| result == Some(true))
}

pub(crate) use gwt::cli::execution_state::{
    BindingRepairFailureCause, BindingRepairOutcome, BindingRepairOutcomeRecord,
};
#[cfg(test)]
pub(crate) const BINDING_REPAIR_OUTCOME_FILE: &str =
    gwt::cli::execution_state::BINDING_REPAIR_OUTCOME_FILE;
const BINDING_REPAIR_OPERATION_ID: &str = "continue-work-local-repair";

fn new_binding_repair_outcome_record(
    binding: &gwt_agent::SessionExecutionBinding,
    owner: gwt::cli::execution_state::ExecutionOwnerKey,
    outcome: BindingRepairOutcome,
) -> BindingRepairOutcomeRecord {
    gwt::cli::execution_state::new_binding_repair_outcome_record(binding, owner, outcome)
}

#[cfg(test)]
pub(crate) fn load_binding_repair_outcome(
    worktree: &Path,
) -> std::io::Result<Option<BindingRepairOutcomeRecord>> {
    gwt::cli::execution_state::load_binding_repair_outcome(worktree)
}

#[cfg(test)]
fn persist_binding_repair_outcome(
    worktree: &Path,
    record: BindingRepairOutcomeRecord,
) -> std::io::Result<BindingRepairOutcomeRecord> {
    gwt::cli::execution_state::persist_binding_repair_outcome(worktree, record)
}

fn record_binding_repair_outcome(worktree: &Path, record: BindingRepairOutcomeRecord) -> bool {
    gwt::cli::execution_state::record_binding_repair_outcome(worktree, record)
}

fn repaired_binding_probe_receipt_matches(
    expected_session_id: &str,
    binding: &gwt_agent::SessionExecutionBinding,
    request: &gwt::AgentExecutionBindingProbeRequest,
    receipt: Option<&gwt::AgentExecutionBindingProbeReceipt>,
) -> bool {
    let Some(receipt) = receipt else {
        return false;
    };
    binding.session_id == expected_session_id
        && receipt.schema_version == gwt::AGENT_EXECUTION_BINDING_PROBE_SCHEMA_VERSION
        && receipt.operation_id == request.operation_id
        && receipt.nonce == request.nonce
        && !receipt.host_instance_id.trim().is_empty()
        && receipt.execution_binding == binding.identity
        && receipt.capability_generation == binding.capability_generation
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)] // Kept beside the private D3-R codec it exercises.
mod repaired_binding_probe_tests {
    use super::*;

    fn init_repo(path: &Path) {
        std::fs::create_dir_all(path).expect("create repository directory");
        let status = gwt_core::process::hidden_command("git")
            .args(["init", "--quiet"])
            .current_dir(path)
            .status()
            .expect("git init");
        assert!(status.success(), "git init failed");
        let status = gwt_core::process::hidden_command("git")
            .args([
                "remote",
                "add",
                "origin",
                "https://example.com/owner/binding-repair.git",
            ])
            .current_dir(path)
            .status()
            .expect("git remote add origin");
        assert!(status.success(), "git remote add origin failed");
    }

    fn binding() -> gwt_agent::SessionExecutionBinding {
        gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: "session-current".to_string(),
            repo_hash: "repo-current".to_string(),
            owner_kind: "issue".to_string(),
            owner_number: 3394,
            identity: gwt_agent::ExecutionBindingIdentity {
                generation_id: "generation-current".to_string(),
                binding_id: "binding-current".to_string(),
                ledger_head_hash: "head-current".to_string(),
            },
            capability_generation: 3,
        }
    }

    fn request() -> gwt::AgentExecutionBindingProbeRequest {
        gwt::AgentExecutionBindingProbeRequest {
            schema_version: gwt::AGENT_EXECUTION_BINDING_PROBE_SCHEMA_VERSION,
            operation_id: "operation-current".to_string(),
            nonce: "nonce-current".to_string(),
        }
    }

    fn exact_receipt(
        binding: &gwt_agent::SessionExecutionBinding,
        request: &gwt::AgentExecutionBindingProbeRequest,
    ) -> gwt::AgentExecutionBindingProbeReceipt {
        gwt::AgentExecutionBindingProbeReceipt {
            schema_version: gwt::AGENT_EXECUTION_BINDING_PROBE_SCHEMA_VERSION,
            operation_id: request.operation_id.clone(),
            nonce: request.nonce.clone(),
            host_instance_id: "host-current".to_string(),
            execution_binding: binding.identity.clone(),
            capability_generation: binding.capability_generation,
        }
    }

    #[test]
    fn repaired_authority_requires_an_exact_read_after_write_receipt() {
        let binding = binding();
        let request = request();
        let exact = exact_receipt(&binding, &request);
        assert!(repaired_binding_probe_receipt_matches(
            "session-current",
            &binding,
            &request,
            Some(&exact),
        ));
        assert!(!repaired_binding_probe_receipt_matches(
            "session-current",
            &binding,
            &request,
            None,
        ));

        let mut old_generation = exact.clone();
        old_generation.execution_binding.generation_id = "generation-old".to_string();
        assert!(!repaired_binding_probe_receipt_matches(
            "session-current",
            &binding,
            &request,
            Some(&old_generation),
        ));

        assert!(!repaired_binding_probe_receipt_matches(
            "session-foreign",
            &binding,
            &request,
            Some(&exact),
        ));
    }

    #[test]
    fn binding_repair_outcome_roundtrips_success_and_typed_failure() {
        let _env_guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        init_repo(&repo);
        let binding = binding();
        let owner = gwt::cli::execution_state::ExecutionOwnerKey {
            kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
            number: binding.owner_number,
        };

        let stored_success = persist_binding_repair_outcome(
            &repo,
            new_binding_repair_outcome_record(
                &binding,
                owner,
                BindingRepairOutcome::Succeeded {
                    host_instance_id: "host-current".to_string(),
                    receipt_generation_id: binding.identity.generation_id.clone(),
                },
            ),
        )
        .expect("persist successful repair outcome");
        assert_eq!(
            load_binding_repair_outcome(&repo)
                .expect("load successful repair outcome")
                .expect("successful repair outcome"),
            stored_success
        );

        let stored_failure = persist_binding_repair_outcome(
            &repo,
            new_binding_repair_outcome_record(
                &binding,
                owner,
                BindingRepairOutcome::Failed {
                    cause: BindingRepairFailureCause::ProbeReceiptMismatch,
                    observed_generation_id: Some("generation-stale".to_string()),
                },
            ),
        )
        .expect("persist failed repair outcome");
        assert_eq!(
            load_binding_repair_outcome(&repo)
                .expect("load failed repair outcome")
                .expect("failed repair outcome"),
            stored_failure
        );
    }

    #[test]
    fn binding_repair_outcome_rejects_tampering_and_store_failure_cannot_report_success() {
        let _env_guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        init_repo(&repo);
        let binding = binding();
        let owner = gwt::cli::execution_state::ExecutionOwnerKey {
            kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
            number: binding.owner_number,
        };
        let success = new_binding_repair_outcome_record(
            &binding,
            owner,
            BindingRepairOutcome::Succeeded {
                host_instance_id: "host-current".to_string(),
                receipt_generation_id: binding.identity.generation_id.clone(),
            },
        );
        persist_binding_repair_outcome(&repo, success.clone()).expect("persist outcome");

        let trusted_dir = gwt::cli::trusted_store::trusted_dir_for_worktree(&repo)
            .expect("trusted worktree directory");
        let path = trusted_dir.join(BINDING_REPAIR_OUTCOME_FILE);
        let mut stored: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).expect("read outcome"))
                .expect("parse outcome");
        stored["outcome"]["receipt_generation_id"] =
            serde_json::Value::String("generation-forged".to_string());
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&stored).expect("serialize forged outcome"),
        )
        .expect("forge outcome");
        assert_eq!(
            load_binding_repair_outcome(&repo)
                .expect_err("tampered outcome must fail closed")
                .kind(),
            std::io::ErrorKind::InvalidData
        );

        std::fs::remove_file(&path).expect("remove forged outcome");
        std::fs::create_dir(&path).expect("block outcome writer with directory");
        assert!(
            !record_binding_repair_outcome(&repo, success),
            "a verified repair must not report success when its outcome cannot be persisted"
        );
    }
}

impl AppRuntime {
    pub(super) fn reconcile_durable_fresh_execution_launches(&mut self) {
        for (_receipt_path, receipt) in durable_launch_recovery_records(&self.sessions_dir) {
            let owner = match receipt.owner() {
                Ok(owner) => owner,
                Err(error) => {
                    tracing::warn!(session_id = %receipt.session_id, error = %error, "retained invalid launch recovery receipt");
                    continue;
                }
            };
            if !durable_launch_recovery_repo_matches(&receipt) {
                tracing::warn!(session_id = %receipt.session_id, "retained launch recovery receipt after repository identity mismatch");
                continue;
            }
            let operation_id = match &receipt.kind {
                DurableLaunchRecoveryKind::Genesis => {
                    self.reconcile_durable_genesis_launch(&receipt, owner);
                    continue;
                }
                DurableLaunchRecoveryKind::FreshSuccessor { operation_id } => operation_id,
            };
            let path = self
                .sessions_dir
                .join(format!("{}.toml", receipt.session_id));
            let session = match gwt_agent::inspect_session_path(&path) {
                gwt_agent::SessionPathState::Present(session) => session,
                gwt_agent::SessionPathState::Missing => {
                    self.reconcile_fresh_launch_without_session(&receipt, owner, operation_id);
                    continue;
                }
                gwt_agent::SessionPathState::Error(error) => {
                    tracing::warn!(path = %path.display(), error = %error, "retained unreadable pending launch Session");
                    continue;
                }
            };
            let candidate = match durable_fresh_execution_candidate(&session) {
                Ok(Some(candidate)) => candidate,
                Ok(None) => {
                    tracing::warn!(session_id = %session.id, "retained pending launch receipt without a durable candidate");
                    continue;
                }
                Err(error) => {
                    tracing::warn!(
                        session_id = %session.id,
                        error = %error,
                        "retained ambiguous fresh-launch recovery evidence"
                    );
                    continue;
                }
            };
            if candidate.owner != owner
                || candidate.binding.session_id != receipt.session_id
                || receipt.expected_binding.as_ref() != Some(&candidate.binding)
                || receipt.expected_agent_id.as_ref() != Some(&candidate.agent_id)
                || receipt.expected_session_identity.as_ref() != Some(&candidate.session_identity)
                || candidate.attempt.request.operation_id != *operation_id
                || !path_matches(&candidate.worktree_path, &receipt.worktree_path)
                || !path_matches(&candidate.project_root, &receipt.project_root)
            {
                tracing::warn!(session_id = %receipt.session_id, "retained launch recovery receipt after exact identity mismatch");
                continue;
            }

            match candidate.attempt.status {
                gwt::cli::execution_state::ContinuationAttemptStatus::Activated => {
                    let activated_matches = candidate
                        .attempt
                        .activated_generation
                        .as_ref()
                        .is_some_and(|identity| {
                            identity.generation_id == candidate.binding.identity.generation_id
                                && identity.session_binding_id
                                    == candidate.binding.identity.binding_id
                        });
                    if !activated_matches {
                        tracing::warn!(
                            session_id = %candidate.binding.session_id,
                            "retained Activated fresh-launch evidence with a mismatched Session binding"
                        );
                        continue;
                    }
                    invoke_fresh_execution_pre_work_commit_hook();
                    let recovery_owner = gwt::cli::execution_state::recovery_generation_owner(
                        &candidate.worktree_path,
                    )
                    .ok()
                    .flatten();
                    let superseding_authority_is_readable = match recovery_owner {
                        Some(current_owner) if current_owner != candidate.owner => {
                            gwt::cli::execution_state::current_execution_binding(
                                &candidate.worktree_path,
                                current_owner,
                            )
                            .is_ok_and(|binding| binding.is_some())
                        }
                        Some(_) => gwt::cli::execution_state::current_execution_binding(
                            &candidate.worktree_path,
                            candidate.owner,
                        )
                        .is_ok_and(|binding| {
                            binding.is_some_and(|binding| binding != candidate.binding.identity)
                        }),
                        None => false,
                    };
                    if superseding_authority_is_readable {
                        let terminal_resolution = split_workspace_state_external_commit_resolution(
                            &candidate.project_root,
                            &candidate.worktree_path,
                            &candidate.attempt.request.operation_id,
                        );
                        let resolution_is_terminal = terminal_resolution.is_ok_and(|resolution| {
                            matches!(
                                resolution,
                                gwt_core::workspace_projection::
                                    ExternalWorkspaceCommitResolution::Committed
                                    | gwt_core::workspace_projection::
                                        ExternalWorkspaceCommitResolution::Rejected
                            )
                        }) || resolve_split_workspace_state_external_commit(
                                &candidate.project_root,
                                &candidate.worktree_path,
                                &candidate.attempt.request.operation_id,
                                gwt_core::workspace_projection::
                                    ExternalWorkspaceCommitDecision::Reject,
                            )
                            .is_ok_and(|resolution| {
                                resolution
                                    == gwt_core::workspace_projection::
                                        ExternalWorkspaceCommitResolution::Rejected
                            });
                        if resolution_is_terminal {
                            if let Err(error) = clear_durable_launch_recovery(
                                &self.sessions_dir,
                                &candidate.binding.session_id,
                            ) {
                                tracing::warn!(session_id = %candidate.binding.session_id, error = %error, "superseded launch recovery receipt cleanup remains pending");
                            }
                        } else {
                            tracing::warn!(
                                session_id = %candidate.binding.session_id,
                                "superseded fresh-launch Work rejection remains pending"
                            );
                        }
                        continue;
                    }
                    match resolve_activated_fresh_execution_commit(
                        &candidate.project_root,
                        &candidate.worktree_path,
                        candidate.owner,
                        &candidate.attempt.request.operation_id,
                        &candidate.attempt.request,
                        &self.sessions_dir,
                        &candidate.session_identity,
                    ) {
                        Ok(true) => {
                            if let Err(error) = clear_durable_launch_recovery(
                                &self.sessions_dir,
                                &candidate.binding.session_id,
                            ) {
                                tracing::warn!(session_id = %candidate.binding.session_id, error = %error, "settled launch recovery receipt cleanup remains pending");
                                continue;
                            }
                            tracing::info!(
                                session_id = %candidate.binding.session_id,
                                operation_id = %candidate.attempt.request.operation_id,
                                "recovered an Activated fresh launch after Host restart"
                            );
                        }
                        Ok(false) => {
                            let terminal_resolution =
                                split_workspace_state_external_commit_resolution(
                                    &candidate.project_root,
                                    &candidate.worktree_path,
                                    &candidate.attempt.request.operation_id,
                                );
                            if terminal_resolution.is_ok_and(|resolution| {
                                matches!(
                                    resolution,
                                    gwt_core::workspace_projection::
                                        ExternalWorkspaceCommitResolution::Committed
                                        | gwt_core::workspace_projection::
                                            ExternalWorkspaceCommitResolution::Rejected
                                )
                            }) {
                                if let Err(error) = clear_durable_launch_recovery(
                                    &self.sessions_dir,
                                    &candidate.binding.session_id,
                                ) {
                                    tracing::warn!(session_id = %candidate.binding.session_id, error = %error, "settled launch recovery receipt cleanup remains pending");
                                }
                                continue;
                            }
                            tracing::warn!(
                                session_id = %candidate.binding.session_id,
                                "Activated fresh-launch exact Session lease or Work readback remains pending"
                            );
                        }
                        Err(error) => {
                            tracing::warn!(
                                session_id = %candidate.binding.session_id,
                                error = %error,
                                "Activated fresh-launch Workspace commit could not be recovered"
                            );
                        }
                    }
                }
                gwt::cli::execution_state::ContinuationAttemptStatus::Prepared => {
                    let prepared_matches =
                        gwt::cli::execution_state::prepared_execution_binding_matches(
                            &candidate.worktree_path,
                            candidate.owner,
                            &candidate.binding.session_id,
                            &candidate.binding.identity,
                        )
                        .unwrap_or(false);
                    if !prepared_matches {
                        tracing::warn!(
                            session_id = %candidate.binding.session_id,
                            "retained a Prepared fresh launch whose exact binding is unreadable"
                        );
                        continue;
                    }
                    if self.finish_durable_aborted_fresh_execution_cleanup(
                        &candidate,
                        receipt
                            .expected_binding
                            .as_ref()
                            .expect("validated fresh recovery binding"),
                    ) {
                        let _ = clear_durable_launch_recovery(
                            &self.sessions_dir,
                            &candidate.binding.session_id,
                        );
                    }
                }
                gwt::cli::execution_state::ContinuationAttemptStatus::Aborted => {
                    if self.finish_durable_aborted_fresh_execution_cleanup(
                        &candidate,
                        receipt
                            .expected_binding
                            .as_ref()
                            .expect("validated fresh recovery binding"),
                    ) {
                        let _ = clear_durable_launch_recovery(
                            &self.sessions_dir,
                            &candidate.binding.session_id,
                        );
                    }
                }
            }
        }
    }

    fn reconcile_fresh_launch_without_session(
        &mut self,
        receipt: &DurableLaunchRecoveryRecord,
        owner: gwt::cli::execution_state::ExecutionOwnerKey,
        operation_id: &str,
    ) {
        let attempt = match gwt::cli::execution_state::continuation_attempt_for_operation(
            &receipt.worktree_path,
            owner,
            operation_id,
        ) {
            Ok(attempt) => attempt,
            Err(error) => {
                tracing::warn!(session_id = %receipt.session_id, error = %error, "pending launch attempt remains unreadable");
                return;
            }
        };
        let Some(attempt) = attempt else {
            if receipt.expected_binding.is_none() {
                let _ = clear_durable_launch_recovery(&self.sessions_dir, &receipt.session_id);
            } else {
                tracing::warn!(session_id = %receipt.session_id, "retained bound fresh-launch receipt whose owner attempt is missing");
            }
            return;
        };
        if attempt.predecessor_status
            != gwt::cli::execution_state::SuccessorPredecessorStatus::Blocked
            || attempt.request.source != gwt::cli::execution_state::FRESH_LINKED_OWNER_LAUNCH_SOURCE
            || attempt.request.work_id.is_some()
            || attempt.request.initial_session_id != receipt.session_id
            || attempt.request.operation_id != operation_id
        {
            tracing::warn!(session_id = %receipt.session_id, "retained launch recovery receipt that does not identify an exact fresh successor");
            return;
        }
        if let Some(expected_binding) = receipt.expected_binding.as_ref() {
            let binding_envelope_matches = expected_binding.schema_version
                == gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION
                && expected_binding.session_id == receipt.session_id
                && expected_binding.repo_hash == receipt.repo_hash
                && expected_binding.owner_kind == owner.kind.as_str()
                && expected_binding.owner_number == owner.number
                && expected_binding.capability_generation > 0;
            let exact_attempt_matches = binding_envelope_matches
                && gwt::cli::execution_state::continuation_attempt_execution_binding_matches(
                    &receipt.worktree_path,
                    owner,
                    &attempt,
                    &receipt.session_id,
                    &expected_binding.identity,
                )
                .unwrap_or(false);
            if !exact_attempt_matches {
                tracing::warn!(session_id = %receipt.session_id, "retained missing-Session fresh-launch receipt after exact binding mismatch");
                return;
            }
        }
        invoke_missing_session_cleanup_hook(&receipt.session_id);
        let cleanup = match attempt.status {
            gwt::cli::execution_state::ContinuationAttemptStatus::Prepared => {
                if let Some(expected_session_identity) = receipt.expected_session_identity.as_ref()
                {
                    gwt::cli::execution_state::abort_successor_and_remove_exact_session(
                        &receipt.worktree_path,
                        owner,
                        &attempt.request,
                        "Host restarted before fresh launch Session persistence",
                        &self.sessions_dir,
                        expected_session_identity,
                        || {
                            reject_continue_work_workspace_commit(
                                &receipt.project_root,
                                &receipt.worktree_path,
                                operation_id,
                            )
                        },
                    )
                } else {
                    gwt::cli::execution_state::abort_successor_if_session_missing_with(
                        &receipt.worktree_path,
                        owner,
                        &attempt.request,
                        "Host restarted before fresh launch Session persistence",
                        &self.sessions_dir,
                        &receipt.session_id,
                        || {
                            reject_continue_work_workspace_commit(
                                &receipt.project_root,
                                &receipt.worktree_path,
                                operation_id,
                            )
                        },
                    )
                }
            }
            gwt::cli::execution_state::ContinuationAttemptStatus::Aborted => {
                if let Some(expected_session_identity) = receipt.expected_session_identity.as_ref()
                {
                    gwt::cli::execution_state::remove_exact_session_with_owner_lease(
                        &receipt.worktree_path,
                        owner,
                        &self.sessions_dir,
                        expected_session_identity,
                        || {
                            reject_continue_work_workspace_commit(
                                &receipt.project_root,
                                &receipt.worktree_path,
                                operation_id,
                            )
                        },
                    )
                } else {
                    gwt::cli::execution_state::commit_if_session_missing_with_owner_lease(
                        &receipt.worktree_path,
                        owner,
                        &self.sessions_dir,
                        &receipt.session_id,
                        || {
                            reject_continue_work_workspace_commit(
                                &receipt.project_root,
                                &receipt.worktree_path,
                                operation_id,
                            )
                        },
                    )
                }
            }
            gwt::cli::execution_state::ContinuationAttemptStatus::Activated => {
                tracing::warn!(session_id = %receipt.session_id, "retained Activated launch receipt whose exact Session is missing");
                return;
            }
        };
        match cleanup {
            Ok(true) => {
                let _ = clear_durable_launch_recovery(&self.sessions_dir, &receipt.session_id);
            }
            Ok(false) => {
                tracing::warn!(session_id = %receipt.session_id, "retained missing-Session fresh-launch recovery after same-id Session materialization");
            }
            Err(error) => {
                tracing::warn!(session_id = %receipt.session_id, error = %error, "missing-Session fresh-launch atomic cleanup remains pending");
            }
        }
    }

    fn reconcile_durable_genesis_launch(
        &mut self,
        receipt: &DurableLaunchRecoveryRecord,
        owner: gwt::cli::execution_state::ExecutionOwnerKey,
    ) {
        let ledger = match gwt::cli::execution_state::load_owner_generation_ledger(
            &receipt.worktree_path,
            owner,
        ) {
            Ok(Some(ledger)) => ledger,
            Ok(None) => {
                if receipt.expected_binding.is_some() {
                    tracing::warn!(session_id = %receipt.session_id, "retained bound genesis receipt whose owner ledger is missing");
                    return;
                }
                let flat = match gwt::cli::execution_state::load(&receipt.worktree_path) {
                    Ok(flat) => flat,
                    Err(_) => return,
                };
                let Some(flat) = flat else {
                    if receipt.expected_binding.is_none() {
                        let _ =
                            clear_durable_launch_recovery(&self.sessions_dir, &receipt.session_id);
                    } else {
                        tracing::warn!(session_id = %receipt.session_id, "retained bound genesis receipt whose execution authority is missing");
                    }
                    return;
                };
                if flat.owner_kind != owner.kind
                    || flat.owner_number != owner.number
                    || flat.primary_session_id != receipt.session_id
                    || flat.status != gwt::cli::execution_state::ExecutionControlStatus::Active
                {
                    return;
                }
                if gwt::cli::execution_state::ensure_generation_ledger(
                    &receipt.worktree_path,
                    owner,
                    gwt::cli::execution_state::LegacyActiveDisposition::Live,
                )
                .is_err()
                {
                    return;
                }
                match gwt::cli::execution_state::load_owner_generation_ledger(
                    &receipt.worktree_path,
                    owner,
                ) {
                    Ok(Some(ledger)) => ledger,
                    _ => return,
                }
            }
            Err(error) => {
                tracing::warn!(session_id = %receipt.session_id, error = %error, "genesis launch recovery authority remains unreadable");
                return;
            }
        };
        let expected_binding = receipt.expected_binding.as_ref();
        let target = ledger.generations.iter().find(|generation| {
            generation.identity.predecessor_generation_id.is_none()
                && generation.identity.initial_session_id == receipt.session_id
                && expected_binding.is_none_or(|binding| {
                    generation.identity.generation_id == binding.identity.generation_id
                        && generation.identity.session_binding_id == binding.identity.binding_id
                })
        });
        let Some(target) = target.cloned() else {
            tracing::warn!(session_id = %receipt.session_id, "retained genesis receipt after generation identity mismatch");
            return;
        };
        if expected_binding.is_some_and(|binding| {
            !genesis_initial_execution_binding_matches(
                &receipt.worktree_path,
                owner,
                &receipt.session_id,
                binding,
            )
        }) {
            tracing::warn!(session_id = %receipt.session_id, "retained genesis receipt after full initial binding mismatch");
            return;
        }
        let binding = if let Some(binding) = expected_binding {
            binding.identity.clone()
        } else {
            if ledger.current_generation_id != target.identity.generation_id {
                return;
            }
            match gwt::cli::execution_state::current_owner_execution_binding(
                &receipt.worktree_path,
                owner,
            ) {
                Ok(Some(binding)) => binding,
                _ => return,
            }
        };
        let session_path = self
            .sessions_dir
            .join(format!("{}.toml", receipt.session_id));
        let persisted_session = match std::fs::symlink_metadata(&session_path) {
            Ok(_) => match gwt_agent::Session::load(&session_path) {
                Ok(session) => Some(session),
                Err(_) => return,
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => return,
        };
        let persisted_binding = persisted_session
            .as_ref()
            .and_then(|session| session.execution_binding.clone());
        if persisted_session.is_some()
            && (expected_binding.is_none() || persisted_binding.as_ref() != expected_binding)
        {
            tracing::warn!(session_id = %receipt.session_id, "retained genesis receipt after full Session binding mismatch");
            return;
        }
        if let Some(session) = persisted_session.as_ref() {
            let actual_identity = match gwt_agent::SessionExecutionIdentity::from_session(session) {
                Ok(Some(identity)) => identity,
                _ => return,
            };
            if receipt.expected_session_identity.as_ref() != Some(&actual_identity) {
                tracing::warn!(session_id = %receipt.session_id, "retained genesis receipt after exact Session anchor mismatch");
                return;
            }
        }
        let superseded_by_different_owner = matches!(
            gwt::cli::execution_state::recovery_generation_owner(&receipt.worktree_path),
            Ok(Some(current_owner)) if current_owner != owner
        );
        let foreign_owner_write_incomplete = !superseded_by_different_owner
            && matches!(
                gwt::cli::execution_state::recovery_projection_owner_hint(&receipt.worktree_path),
                Ok(Some(projection_owner)) if projection_owner != owner
            );
        if foreign_owner_write_incomplete {
            tracing::warn!(
                "genesis launch recovery retained behind an incomplete foreign-owner authority write"
            );
            return;
        }
        let terminal_event = ledger
            .lifecycle_events
            .iter()
            .filter(|event| event.generation_id == target.identity.generation_id)
            .max_by_key(|event| event.sequence);
        let target_status = terminal_event.map_or(target.status, |event| event.to_status);
        let terminal_reason = match target_status {
            gwt::cli::execution_state::ExecutionControlStatus::Active => {
                if ledger.current_generation_id != target.identity.generation_id {
                    return;
                }
                if expected_binding
                    .is_some_and(|session_binding| session_binding.identity != binding)
                {
                    return;
                }
                (!superseded_by_different_owner)
                    .then(|| "Host restarted before genesis launch readiness".to_string())
            }
            gwt::cli::execution_state::ExecutionControlStatus::Blocked => {
                let terminalization_operation_id =
                    gwt::cli::execution_state::genesis_terminalization_operation_id(
                        &target.identity.generation_id,
                        &target.identity.session_binding_id,
                    );
                let terminal_reason = terminal_event.and_then(|event| {
                    (event.from_status == gwt::cli::execution_state::ExecutionControlStatus::Active
                        && event.to_status
                            == gwt::cli::execution_state::ExecutionControlStatus::Blocked
                        && event.session_id == receipt.session_id
                        && event.operation_id.as_deref()
                            == Some(terminalization_operation_id.as_str()))
                    .then(|| event.reason.clone())
                });
                let Some(terminal_reason) = terminal_reason else {
                    return;
                };
                (!superseded_by_different_owner
                    && ledger.current_generation_id == target.identity.generation_id)
                    .then_some(terminal_reason)
            }
            _ => return,
        };
        if persisted_session.is_some() {
            let (Some(session_binding), Some(_expected_session_identity)) =
                (expected_binding, receipt.expected_session_identity.as_ref())
            else {
                return;
            };
            if session_binding.identity.generation_id != target.identity.generation_id
                || session_binding.identity.binding_id != target.identity.session_binding_id
            {
                return;
            }
        }
        let compensation_authority = if superseded_by_different_owner {
            GenesisCompensationAuthority::SupersededByDifferentOwner
        } else {
            GenesisCompensationAuthority::Terminalized
        };
        let persisted_branch = persisted_session
            .as_ref()
            .map(|session| session.branch.clone());
        let compensate = || -> std::io::Result<()> {
            compensate_genesis_workspace_projection(
                &receipt.project_root,
                &receipt.worktree_path,
                owner,
                &receipt.session_id,
                receipt.expected_binding.as_ref(),
                persisted_branch.as_deref(),
                compensation_authority,
            )
            .map_err(std::io::Error::other)
        };
        if persisted_session.is_none() {
            invoke_missing_session_cleanup_hook(&receipt.session_id);
        }
        let cleanup = if expected_binding.is_some() {
            let Some(expected_session_identity) = receipt.expected_session_identity.as_ref() else {
                return;
            };
            if let Some(reason) = terminal_reason.as_deref() {
                gwt::cli::execution_state::block_genesis_and_remove_exact_session(
                    &receipt.worktree_path,
                    owner,
                    &receipt.session_id,
                    &binding,
                    reason,
                    &self.sessions_dir,
                    expected_session_identity,
                    compensate,
                )
            } else {
                gwt::cli::execution_state::remove_exact_session_with_owner_lease(
                    &receipt.worktree_path,
                    owner,
                    &self.sessions_dir,
                    expected_session_identity,
                    compensate,
                )
            }
        } else {
            if let Some(reason) = terminal_reason.as_deref() {
                gwt::cli::execution_state::block_genesis_if_session_missing_with(
                    &receipt.worktree_path,
                    owner,
                    &receipt.session_id,
                    &binding,
                    reason,
                    &self.sessions_dir,
                    compensate,
                )
            } else {
                gwt::cli::execution_state::commit_if_session_missing_with_owner_lease(
                    &receipt.worktree_path,
                    owner,
                    &self.sessions_dir,
                    &receipt.session_id,
                    compensate,
                )
            }
        };
        match cleanup {
            Ok(true) => {}
            Ok(false) => {
                tracing::warn!(session_id = %receipt.session_id, "retained genesis recovery after same-id Session materialization");
                return;
            }
            Err(error) => {
                tracing::warn!(session_id = %receipt.session_id, error = %error, "genesis atomic cleanup remains pending");
                return;
            }
        }
        self.launch_wizard_cache.forget_session(&receipt.session_id);
        let _ = clear_durable_launch_recovery(&self.sessions_dir, &receipt.session_id);
    }

    fn finish_durable_aborted_fresh_execution_cleanup(
        &mut self,
        candidate: &DurableFreshExecutionCandidate,
        _expected_binding: &gwt_agent::SessionExecutionBinding,
    ) -> bool {
        let cleanup = if candidate.attempt.status
            == gwt::cli::execution_state::ContinuationAttemptStatus::Prepared
        {
            gwt::cli::execution_state::abort_successor_and_remove_exact_session(
                &candidate.worktree_path,
                candidate.owner,
                &candidate.attempt.request,
                "Host restarted before fresh launch activation",
                &self.sessions_dir,
                &candidate.session_identity,
                || {
                    reject_continue_work_workspace_commit(
                        &candidate.project_root,
                        &candidate.worktree_path,
                        &candidate.attempt.request.operation_id,
                    )
                },
            )
        } else {
            gwt::cli::execution_state::remove_exact_session_with_owner_lease(
                &candidate.worktree_path,
                candidate.owner,
                &self.sessions_dir,
                &candidate.session_identity,
                || {
                    reject_continue_work_workspace_commit(
                        &candidate.project_root,
                        &candidate.worktree_path,
                        &candidate.attempt.request.operation_id,
                    )
                },
            )
        };
        match cleanup {
            Ok(true) => {
                self.launch_wizard_cache
                    .forget_session(&candidate.binding.session_id);
                tracing::info!(session_id = %candidate.binding.session_id, "completed durable Aborted fresh-launch Session cleanup");
                true
            }
            Ok(false) => {
                tracing::warn!(session_id = %candidate.binding.session_id, "retained Aborted fresh-launch cleanup evidence after binding mismatch");
                false
            }
            Err(error) => {
                tracing::warn!(session_id = %candidate.binding.session_id, error = %error, "Aborted fresh-launch Session cleanup remains retryable");
                false
            }
        }
    }

    fn continue_work_correlated_outcome_events(
        &mut self,
        client_id: &str,
        operation_id: String,
        outcome: CachedContinueWorkOutcome,
    ) -> Vec<OutboundEvent> {
        let mut clients = vec![client_id.to_string()];
        if let Some(waiters) = self.continue_work_waiters.remove(&operation_id) {
            clients.extend(waiters);
        }
        clients.sort();
        clients.dedup();
        clients
            .into_iter()
            .map(|client_id| {
                continue_work_outcome(
                    &client_id,
                    operation_id.clone(),
                    outcome.work_id.clone(),
                    outcome.outcome,
                    outcome.message.clone(),
                    outcome.error_code.clone(),
                    outcome.retryable,
                )
            })
            .collect()
    }

    fn reconcile_activated_pending_continue_work(
        &mut self,
        window_id: &str,
        pending: &PendingContinueWork,
    ) -> Option<Vec<OutboundEvent>> {
        if !self.window_lookup.contains_key(window_id)
            || matches!(
                self.window_status(window_id),
                None | Some(WindowProcessStatus::Stopped | WindowProcessStatus::Error)
            )
        {
            return None;
        }
        let active = self.active_agent_sessions.get(window_id)?;
        if active.session_id != pending.binding.session_id
            || !path_matches(&active.worktree_path, &pending.worktree_path)
            || crate::runtime_support::normalize_branch_name(active.branch_name.trim())
                != pending.work_branch
            || gwt_agent::resolve_agent_id(active.agent_id.trim()).as_ref()
                != Some(&pending.work_agent_id)
        {
            return None;
        }
        let exact_session_identity = pending_continue_work_session_identity(pending).ok()?;
        if gwt::cli::execution_state::with_current_active_session_execution_identity_lease(
            &self.sessions_dir,
            &exact_session_identity,
            || continue_work_commit_readback_matches(pending),
        )
        .ok()
        .flatten()
            != Some(true)
        {
            return None;
        }
        let token = self.agent_capability_tokens.get(window_id)?;
        let issuer = self.agent_capability_issuer.as_ref()?;
        if !issuer.active_token_is_current(token, &pending.binding) {
            return None;
        }
        let probe = gwt::probe_authenticated_execution_binding(
            &pending.project_root,
            &pending.binding.session_id,
            &pending.binding,
            "continue-work-coordinator",
            gwt::AgentExecutionBindingProbeRequest {
                schema_version: gwt::AGENT_EXECUTION_BINDING_PROBE_SCHEMA_VERSION,
                operation_id: pending.operation_id.clone(),
                nonce: uuid::Uuid::new_v4().to_string(),
            },
        )
        .ok()?;
        if probe.execution_binding != pending.binding.identity
            || gwt::cli::execution_state::current_active_execution_binding_matches(
                &pending.worktree_path,
                pending.owner,
                &pending.predecessor_session_id,
                &pending.predecessor_binding,
            )
            .unwrap_or(true)
        {
            return None;
        }
        self.pending_continue_work.remove(window_id);
        let outcome = CachedContinueWorkOutcome {
            work_id: pending.work_id.clone(),
            outcome: pending.outcome,
            message: None,
            error_code: None,
            retryable: false,
        };
        self.continue_work_outcomes
            .insert(pending.operation_id.clone(), outcome.clone());
        let mut events = self.continue_work_correlated_outcome_events(
            &pending.client_id,
            pending.operation_id.clone(),
            outcome,
        );
        events.push(self.workspace_state_broadcast());
        if let Some(projection) = self.active_work_projection_broadcast_for_active_tab() {
            events.push(projection);
        }
        Some(events)
    }

    fn ensure_local_active_execution_authority(
        &mut self,
        window_id: &str,
        active: &super::ActiveAgentSession,
        owner: gwt::cli::execution_state::ExecutionOwnerKey,
        current: &gwt_agent::ExecutionBindingIdentity,
        mut durable: gwt_agent::Session,
    ) -> bool {
        let Some(issuer) = self.agent_capability_issuer.clone() else {
            return false;
        };
        let Some(token) = self.agent_capability_tokens.get(window_id).cloned() else {
            return false;
        };
        let (binding, repaired) = match durable.execution_binding.clone() {
            Some(binding) if binding.identity == *current => (binding, false),
            Some(_) => return false,
            None => {
                let unbound_snapshot = durable.clone();
                let repo_hash = durable
                    .repo_hash
                    .clone()
                    .filter(|value| !value.trim().is_empty())
                    .or_else(|| {
                        gwt_core::repo_hash::detect_repo_hash(&active.worktree_path)
                            .map(|value| value.to_string())
                    });
                let Some(repo_hash) = repo_hash else {
                    return false;
                };
                let binding = gwt_agent::SessionExecutionBinding {
                    schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
                    session_id: active.session_id.clone(),
                    repo_hash,
                    owner_kind: owner.kind.as_str().to_string(),
                    owner_number: owner.number,
                    identity: current.clone(),
                    capability_generation: 1,
                };
                if durable
                    .set_execution_binding(Some(binding.clone()))
                    .is_err()
                    || durable
                        .save_if_unchanged(&self.sessions_dir, &unbound_snapshot)
                        .ok()
                        != Some(true)
                {
                    record_binding_repair_outcome(
                        &active.worktree_path,
                        new_binding_repair_outcome_record(
                            &binding,
                            owner,
                            BindingRepairOutcome::Failed {
                                cause: BindingRepairFailureCause::SessionPersistenceFailed,
                                observed_generation_id: None,
                            },
                        ),
                    );
                    return false;
                }
                (binding, true)
            }
        };
        if !issuer.active_token_is_current(&token, &binding)
            && issuer.promote_inspection(&token, &binding).is_err()
        {
            if repaired {
                record_binding_repair_outcome(
                    &active.worktree_path,
                    new_binding_repair_outcome_record(
                        &binding,
                        owner,
                        BindingRepairOutcome::Failed {
                            cause: BindingRepairFailureCause::CapabilityPromotionFailed,
                            observed_generation_id: None,
                        },
                    ),
                );
            }
            return false;
        }
        if !repaired {
            return issuer.active_token_is_current(&token, &binding)
                && gwt::cli::execution_state::current_active_execution_binding_matches(
                    &active.worktree_path,
                    owner,
                    &active.session_id,
                    &binding.identity,
                )
                .unwrap_or(false);
        }

        let project_root = match gwt::validated_project_state_root_for_session_recovery(&durable) {
            Ok(project_root) => project_root,
            Err(error) => {
                tracing::warn!(
                    session_id = %active.session_id,
                    error = %error,
                    "repaired execution binding has no valid repository Project State anchor"
                );
                record_binding_repair_outcome(
                    &active.worktree_path,
                    new_binding_repair_outcome_record(
                        &binding,
                        owner,
                        BindingRepairOutcome::Failed {
                            cause: BindingRepairFailureCause::ProjectStateAnchorInvalid,
                            observed_generation_id: None,
                        },
                    ),
                );
                return false;
            }
        };
        let request = gwt::AgentExecutionBindingProbeRequest {
            schema_version: gwt::AGENT_EXECUTION_BINDING_PROBE_SCHEMA_VERSION,
            operation_id: BINDING_REPAIR_OPERATION_ID.to_string(),
            nonce: uuid::Uuid::new_v4().to_string(),
        };
        let receipt = match gwt::probe_authenticated_execution_binding(
            &project_root,
            &active.session_id,
            &binding,
            BINDING_REPAIR_OPERATION_ID,
            request.clone(),
        ) {
            Ok(receipt) => receipt,
            Err(error) => {
                tracing::warn!(
                    session_id = %active.session_id,
                    error = %error,
                    "repaired execution binding Host probe failed"
                );
                record_binding_repair_outcome(
                    &active.worktree_path,
                    new_binding_repair_outcome_record(
                        &binding,
                        owner,
                        BindingRepairOutcome::Failed {
                            cause: BindingRepairFailureCause::ProbeTransportFailed,
                            observed_generation_id: None,
                        },
                    ),
                );
                return false;
            }
        };
        if !repaired_binding_probe_receipt_matches(
            &active.session_id,
            &binding,
            &request,
            Some(&receipt),
        ) {
            tracing::warn!(
                session_id = %active.session_id,
                "repaired execution binding failed exact read-after-write authority probe"
            );
            record_binding_repair_outcome(
                &active.worktree_path,
                new_binding_repair_outcome_record(
                    &binding,
                    owner,
                    BindingRepairOutcome::Failed {
                        cause: BindingRepairFailureCause::ProbeReceiptMismatch,
                        observed_generation_id: Some(
                            receipt.execution_binding.generation_id.clone(),
                        ),
                    },
                ),
            );
            return false;
        }
        let authority_matches = issuer.active_token_is_current(&token, &binding)
            && gwt::cli::execution_state::current_active_execution_binding_matches(
                &active.worktree_path,
                owner,
                &active.session_id,
                &binding.identity,
            )
            .unwrap_or(false);
        if !authority_matches {
            return record_binding_repair_outcome(
                &active.worktree_path,
                new_binding_repair_outcome_record(
                    &binding,
                    owner,
                    BindingRepairOutcome::Failed {
                        cause: BindingRepairFailureCause::ActiveAuthorityMismatch,
                        observed_generation_id: Some(
                            receipt.execution_binding.generation_id.clone(),
                        ),
                    },
                ),
            );
        }
        record_binding_repair_outcome(
            &active.worktree_path,
            new_binding_repair_outcome_record(
                &binding,
                owner,
                BindingRepairOutcome::Succeeded {
                    host_instance_id: receipt.host_instance_id,
                    receipt_generation_id: receipt.execution_binding.generation_id,
                },
            ),
        )
    }

    pub(super) fn classify_nonlocal_active_owner_liveness(
        &self,
        session_id: &str,
    ) -> ActiveOwnerLiveness {
        let durable_path = self.sessions_dir.join(format!("{session_id}.toml"));
        let durable = match gwt_agent::inspect_session_path(&durable_path) {
            gwt_agent::SessionPathState::Present(session) => Some(session),
            gwt_agent::SessionPathState::Missing => None,
            gwt_agent::SessionPathState::Error(_) => return ActiveOwnerLiveness::Unknown,
        };
        let runtime_root = self.sessions_dir.join("runtime");
        let entries = match std::fs::read_dir(&runtime_root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if durable.is_none() {
                    return ActiveOwnerLiveness::Stale("durable Session is missing");
                }
                return if durable.as_ref().is_some_and(|session| {
                    matches!(
                        session.status,
                        gwt_agent::AgentStatus::Stopped | gwt_agent::AgentStatus::Interrupted
                    )
                }) {
                    ActiveOwnerLiveness::Stale("durable Session is stopped")
                } else {
                    ActiveOwnerLiveness::Unknown
                };
            }
            Err(_) => return ActiveOwnerLiveness::Unknown,
        };
        let mut saw_dead_runtime = false;
        let mut saw_stopped_runtime = false;
        for entry in entries {
            let Ok(entry) = entry else {
                return ActiveOwnerLiveness::Unknown;
            };
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|value| value.parse::<u32>().ok())
            else {
                continue;
            };
            let sidecar = entry.path().join(format!("{session_id}.json"));
            match sidecar.try_exists() {
                Ok(false) => continue,
                Ok(true) => {}
                Err(_) => return ActiveOwnerLiveness::Unknown,
            }
            if gwt::process::is_process_alive(pid) {
                match gwt_agent::SessionRuntimeState::load(&sidecar) {
                    Ok(state)
                        if matches!(
                            state.status,
                            gwt_agent::AgentStatus::Stopped | gwt_agent::AgentStatus::Interrupted
                        ) =>
                    {
                        saw_stopped_runtime = true;
                        continue;
                    }
                    Ok(_) | Err(_) => return ActiveOwnerLiveness::Unknown,
                }
            }
            saw_dead_runtime = true;
        }
        if saw_stopped_runtime {
            return ActiveOwnerLiveness::Stale("all owning Host runtimes are stopped");
        }
        if saw_dead_runtime {
            return ActiveOwnerLiveness::Stale("all owning Host runtimes are dead");
        }
        if durable.is_none() {
            return ActiveOwnerLiveness::Stale("durable Session is missing");
        }
        if durable.as_ref().is_some_and(|session| {
            matches!(
                session.status,
                gwt_agent::AgentStatus::Stopped | gwt_agent::AgentStatus::Interrupted
            )
        }) {
            return ActiveOwnerLiveness::Stale("durable Session is stopped");
        }
        ActiveOwnerLiveness::Unknown
    }

    pub(crate) fn stop_pending_continue_work_session_without_projection(
        &mut self,
        window_id: &str,
    ) -> bool {
        if !self.pending_continue_work.contains_key(window_id)
            && !self
                .pending_fresh_execution_launches
                .contains_key(window_id)
        {
            return false;
        }
        self.inspection_agent_windows.remove(window_id);
        if let Some(session) = self.active_agent_sessions.remove(window_id) {
            self.revoke_agent_capability_for_window(window_id);
            let _ = gwt_agent::persist_session_status(
                &self.sessions_dir,
                &session.session_id,
                gwt_agent::AgentStatus::Stopped,
            );
            self.launch_wizard_cache.mark_stopped(&session.session_id);
        } else {
            self.revoke_agent_capability_for_window(window_id);
        }
        true
    }

    fn resolve_continue_work_target(
        &self,
        work_id: &str,
    ) -> Result<ContinueWorkTarget, ContinueWorkFailure> {
        let tab_id = self.active_tab_id.clone().ok_or_else(|| {
            ContinueWorkFailure::failed(
                "no_active_project",
                "Open a project before continuing Work.",
                false,
            )
        })?;
        let tab = self.tab(&tab_id).ok_or_else(|| {
            ContinueWorkFailure::failed(
                "project_not_found",
                "The active project is no longer available.",
                true,
            )
        })?;
        if tab.kind != gwt::ProjectKind::Git {
            return Err(ContinueWorkFailure::failed(
                "git_project_required",
                "Continue work requires a Git project.",
                false,
            ));
        }
        if tab.migration_pending {
            return Err(ContinueWorkFailure::failed(
                "project_migration_pending",
                "Complete the project migration before continuing Work.",
                true,
            ));
        }
        let work_state_root = tab.project_root.clone();
        let project_root = crate::runtime_support::normalize_recent_project_path(&work_state_root);
        let works = gwt_core::workspace_projection::load_or_synthesize_workspace_work_items(
            &work_state_root,
        )
        .map_err(|_| {
            ContinueWorkFailure::failed(
                "work_state_unavailable",
                "The Work state could not be read safely.",
                true,
            )
        })?;
        let mut matching = works.work_items.iter().filter(|item| item.id == work_id);
        let item = matching.next().ok_or_else(|| {
            ContinueWorkFailure::failed(
                "work_not_found",
                "The selected Work no longer exists.",
                false,
            )
        })?;
        if matching.next().is_some() {
            return Err(ContinueWorkFailure::conflict(
                "The selected Work identity is ambiguous.",
            ));
        }
        if item.discarded {
            return Err(ContinueWorkFailure::failed(
                "work_discarded",
                "Discarded Work cannot be continued.",
                false,
            ));
        }

        let (worktree_path, branch) = projection_only_continue_container(item, &work_state_root)?;
        let actual_branch = canonical_continue_work_branch(&worktree_path)?;
        if actual_branch != branch {
            return Err(ContinueWorkFailure::conflict(
                "The Work branch no longer matches its current worktree.",
            ));
        }
        let projected_owner = projection_only_continue_owner(item)?;
        let owner = canonical_continue_work_owner(&project_root, &worktree_path, projected_owner)?;
        let source_session = item
            .agents
            .iter()
            .filter_map(|agent| {
                if gwt_agent::validate_session_id_path_component(&agent.session_id).is_err() {
                    return None;
                }
                let path = self.sessions_dir.join(format!("{}.toml", agent.session_id));
                gwt_agent::Session::load_and_migrate(&path)
                    .ok()
                    .filter(|session| session.id == agent.session_id)
                    .map(|session| (agent, session))
            })
            .filter(|(agent, session)| {
                work_agent_ref_authenticates_session(agent, session)
                    && session_matches_project_state(session, &project_root)
                    && path_matches(&session.worktree_path, &worktree_path)
                    && crate::runtime_support::normalize_branch_name(session.branch.trim())
                        == branch
                    && session.linked_issue_number == Some(owner.number)
                    && session.execution_binding.as_ref().is_none_or(|binding| {
                        binding.schema_version
                            == gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION
                            && binding.session_id == session.id
                            && gwt_core::repo_hash::detect_repo_hash(&worktree_path)
                                .is_some_and(|repo_hash| repo_hash.as_str() == binding.repo_hash)
                            && binding.owner_number == owner.number
                            && binding.owner_kind == owner.kind.as_str()
                    })
            })
            .max_by_key(|(agent, _)| agent.updated_at)
            .map(|(_, session)| session);
        let (projected_agent_id, work_agent_session_id, launch_seed) =
            if let Some(source_session) = source_session {
                let projected_agent_id = source_session.agent_id.clone();
                let work_agent_session_id = Some(source_session.id.clone());
                (
                    projected_agent_id,
                    work_agent_session_id,
                    ContinueWorkLaunchSeed::DurableSession(Box::new(source_session)),
                )
            } else {
                let (projected_agent_id, projected_display_name) =
                    projection_only_continue_agent(item)?;
                (
                    projected_agent_id.clone(),
                    None,
                    ContinueWorkLaunchSeed::WorkProjection {
                        agent_id: projected_agent_id,
                        display_name: projected_display_name,
                        branch: branch.clone(),
                    },
                )
            };
        let resume_context = WorkspaceResumeContext {
            title: non_empty_workspace_text(Some(&item.title)),
            owner: non_empty_workspace_text(item.owner.as_deref())
                .or_else(|| Some(format!("Issue #{}", owner.number))),
            summary: non_empty_workspace_text(item.summary.as_deref())
                .or_else(|| non_empty_workspace_text(item.intent.as_deref())),
            next_action: item.latest_next_action().map(str::to_string),
        };
        Ok(ContinueWorkTarget {
            tab_id,
            project_root,
            worktree_path,
            work_branch: Some(branch),
            work_agent_id: Some(projected_agent_id),
            work_agent_session_id,
            launch_seed,
            owner,
            resume_context,
        })
    }

    fn resolve_continue_work_target_for_durable_operation(
        &self,
        operation_id: &str,
    ) -> Result<Option<ContinueWorkTarget>, ContinueWorkFailure> {
        let tab_id = self.active_tab_id.clone().ok_or_else(|| {
            ContinueWorkFailure::failed(
                "no_active_project",
                "Open a project before continuing Work.",
                false,
            )
        })?;
        let tab = self.tab(&tab_id).ok_or_else(|| {
            ContinueWorkFailure::failed(
                "project_not_found",
                "The active project is no longer available.",
                true,
            )
        })?;
        if tab.kind != gwt::ProjectKind::Git || tab.migration_pending {
            return Ok(None);
        }
        let project_root = crate::runtime_support::normalize_recent_project_path(&tab.project_root);
        let entries = std::fs::read_dir(&self.sessions_dir).map_err(|_| {
            ContinueWorkFailure::failed(
                "session_state_unavailable",
                "Durable Session metadata could not be read for reconciliation.",
                true,
            )
        })?;
        let mut matches = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("toml") {
                continue;
            }
            let Ok(source_session) = gwt_agent::Session::load_and_migrate(&path) else {
                continue;
            };
            let Some(file_session_id) = path.file_stem().and_then(|value| value.to_str()) else {
                continue;
            };
            if gwt_agent::validate_session_id_path_component(file_session_id).is_err()
                || source_session.id != file_session_id
            {
                continue;
            }
            if !session_matches_project_state(&source_session, &project_root) {
                continue;
            }
            let Some(owner_number) = source_session.linked_issue_number else {
                continue;
            };
            let Ok(worktree_path) = dunce::canonicalize(&source_session.worktree_path) else {
                continue;
            };
            if !worktree_matches_project_state(&worktree_path, &project_root) {
                continue;
            }
            let binding_owner = source_session
                .execution_binding
                .as_ref()
                .and_then(|binding| {
                    if binding.schema_version
                        != gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION
                        || binding.session_id != source_session.id
                        || binding.owner_number != owner_number
                    {
                        return None;
                    }
                    let kind = match binding.owner_kind.as_str() {
                        "spec" => gwt::cli::execution_state::ExecutionOwnerKind::Spec,
                        "issue" => gwt::cli::execution_state::ExecutionOwnerKind::Issue,
                        _ => return None,
                    };
                    Some(gwt::cli::execution_state::ExecutionOwnerKey {
                        kind,
                        number: owner_number,
                    })
                });
            let authority_owner =
                match gwt::cli::execution_state::current_generation_owner(&worktree_path) {
                    Ok(Some(owner)) => Some(owner),
                    Ok(None) => match gwt::cli::execution_state::recovery_projection_owner_hint(
                        &worktree_path,
                    ) {
                        Ok(owner) => owner,
                        Err(_) => continue,
                    },
                    Err(_) => continue,
                };
            let owner = match (binding_owner, authority_owner) {
                (Some(binding), Some(authority)) if binding == authority => binding,
                (Some(_), Some(_)) => continue,
                (Some(binding), None) => binding,
                (None, Some(authority)) if authority.number == owner_number => authority,
                (None, Some(_)) => continue,
                (None, None) => gwt::cli::execution_state::ExecutionOwnerKey {
                    kind: gwt::cli::execution_state::detect_owner_kind(&project_root, owner_number),
                    number: owner_number,
                },
            };
            let successor = gwt::cli::execution_state::continuation_attempt_for_operation(
                &worktree_path,
                owner,
                operation_id,
            )
            .map_err(|error| {
                ContinueWorkFailure::conflict(format!(
                    "The continuation operation could not be read safely: {error}"
                ))
            })?;
            let takeover = gwt::cli::execution_state::generation_takeover_attempt_for_operation(
                &worktree_path,
                owner,
                operation_id,
            )
            .map_err(|error| {
                ContinueWorkFailure::conflict(format!(
                    "The takeover operation could not be read safely: {error}"
                ))
            })?;
            let (correlated_public_id, is_predecessor, is_candidate) = match (&successor, &takeover)
            {
                (Some(attempt), None) => (
                    attempt.predecessor.initial_session_id == source_session.id
                        || attempt.request.initial_session_id == source_session.id,
                    attempt.predecessor.initial_session_id == source_session.id,
                    attempt.request.initial_session_id == source_session.id,
                ),
                (None, Some(attempt)) => (
                    attempt.request.from_session_id == source_session.id
                        || attempt.request.to_session_id == source_session.id,
                    attempt.request.from_session_id == source_session.id,
                    attempt.request.to_session_id == source_session.id,
                ),
                (None, None) => (false, false, false),
                (Some(_), Some(_)) => {
                    return Err(ContinueWorkFailure::conflict(
                        "The operation id is bound to conflicting durable continuation attempts.",
                    ));
                }
            };
            if !correlated_public_id {
                continue;
            }
            let current_branch = canonical_continue_work_branch(&worktree_path)?;
            let session_branch =
                crate::runtime_support::normalize_branch_name(source_session.branch.trim());
            if current_branch != session_branch {
                return Err(ContinueWorkFailure::conflict(
                    "A correlated continuation Session belongs to a different branch.",
                ));
            }
            let live_candidate_agent_id = if is_candidate {
                let mut live_agent_id = None;
                for active in self.active_agent_sessions.values().filter(|active| {
                    active.session_id == source_session.id
                        && active.tab_id == tab_id
                        && path_matches(&active.worktree_path, &worktree_path)
                        && self.window_lookup.contains_key(&active.window_id)
                        && self.window_status(&active.window_id).is_some_and(|status| {
                            !matches!(
                                status,
                                WindowProcessStatus::Stopped | WindowProcessStatus::Error
                            )
                        })
                }) {
                    let Some(agent_id) = gwt_agent::resolve_agent_id(active.agent_id.trim()) else {
                        return Err(ContinueWorkFailure::conflict(
                            "A live continuation candidate has no canonical Agent identity.",
                        ));
                    };
                    if live_agent_id
                        .as_ref()
                        .is_some_and(|expected| expected != &agent_id)
                    {
                        return Err(ContinueWorkFailure::conflict(
                            "Live continuation candidates disagree on Agent identity.",
                        ));
                    }
                    live_agent_id = Some(agent_id);
                }
                live_agent_id
            } else {
                None
            };
            let trusted_agent_id = if is_predecessor {
                Some(source_session.agent_id.clone())
            } else {
                live_candidate_agent_id
            };
            matches.push((
                ContinueWorkTarget {
                    tab_id: tab_id.clone(),
                    project_root: project_root.clone(),
                    worktree_path,
                    work_branch: Some(current_branch),
                    work_agent_id: Some(source_session.agent_id.clone()),
                    work_agent_session_id: Some(source_session.id.clone()),
                    launch_seed: ContinueWorkLaunchSeed::DurableSession(Box::new(source_session)),
                    owner,
                    resume_context: WorkspaceResumeContext {
                        title: None,
                        owner: Some(format!("{} #{owner_number}", owner.kind.as_str())),
                        summary: None,
                        next_action: None,
                    },
                },
                trusted_agent_id,
                is_predecessor,
            ));
        }
        if matches.is_empty() {
            return Ok(None);
        }
        let mut trusted_agent_ids = matches
            .iter()
            .filter_map(|(_, trusted_agent_id, _)| trusted_agent_id.as_ref());
        let Some(expected_agent_id) = trusted_agent_ids.next().cloned() else {
            return Err(ContinueWorkFailure::conflict(
                "The durable continuation candidate has no independent Agent identity evidence.",
            ));
        };
        if trusted_agent_ids.any(|agent_id| agent_id != &expected_agent_id)
            || matches.iter().any(|(candidate, _, _)| {
                candidate.work_agent_id.as_ref() != Some(&expected_agent_id)
            })
        {
            return Err(ContinueWorkFailure::conflict(
                "The durable continuation Sessions disagree on Agent identity.",
            ));
        }
        for (candidate, _, _) in &mut matches {
            candidate.work_agent_id = Some(expected_agent_id.clone());
        }
        let target_index = matches
            .iter()
            .position(|(_, _, is_predecessor)| *is_predecessor)
            .or_else(|| {
                matches
                    .iter()
                    .position(|(_, trusted_agent_id, _)| trusted_agent_id.is_some())
            })
            .expect("trusted durable continuation target");
        let (target, _, _) = matches.swap_remove(target_index);
        // A successor attempt intentionally names both its predecessor and candidate Sessions.
        // They are one recovery target when they converge on the same durable authority.
        if matches.iter().all(|(candidate, _, _)| {
            candidate.owner == target.owner
                && path_matches(&candidate.project_root, &target.project_root)
                && path_matches(&candidate.worktree_path, &target.worktree_path)
                && candidate.work_branch == target.work_branch
                && candidate.work_agent_id == target.work_agent_id
        }) {
            Ok(Some(target))
        } else {
            Err(ContinueWorkFailure::conflict(
                "The durable continuation operation resolves to multiple execution authorities.",
            ))
        }
    }

    fn continue_work_failure_events(
        &mut self,
        client_id: &str,
        operation_id: String,
        work_id: String,
        failure: ContinueWorkFailure,
    ) -> Vec<OutboundEvent> {
        let outcome = CachedContinueWorkOutcome {
            work_id,
            outcome: failure.outcome,
            message: Some(failure.message),
            error_code: Some(failure.code.to_string()),
            retryable: failure.retryable,
        };
        self.continue_work_outcomes
            .insert(operation_id.clone(), outcome.clone());
        self.continue_work_correlated_outcome_events(client_id, operation_id, outcome)
    }

    fn continue_work_uncached_failure_events(
        &self,
        client_id: &str,
        operation_id: String,
        work_id: String,
        failure: ContinueWorkFailure,
    ) -> Vec<OutboundEvent> {
        vec![continue_work_outcome(
            client_id,
            operation_id,
            work_id,
            failure.outcome,
            Some(failure.message),
            Some(failure.code.to_string()),
            failure.retryable,
        )]
    }

    fn continue_work_pending_outcome_events(
        &mut self,
        pending: &PendingContinueWork,
        outcome: gwt::ContinueWorkOutcomeKind,
        message: Option<String>,
        error_code: Option<String>,
        retryable: bool,
    ) -> Vec<OutboundEvent> {
        self.continue_work_correlated_outcome_events(
            &pending.client_id,
            pending.operation_id.clone(),
            CachedContinueWorkOutcome {
                work_id: pending.work_id.clone(),
                outcome,
                message,
                error_code,
                retryable,
            },
        )
    }

    fn continue_work_pending_uncached_failure_events(
        &mut self,
        pending: &PendingContinueWork,
        failure: ContinueWorkFailure,
    ) -> Vec<OutboundEvent> {
        self.continue_work_pending_outcome_events(
            pending,
            failure.outcome,
            Some(failure.message),
            Some(failure.code.to_string()),
            failure.retryable,
        )
    }

    fn durable_continue_work_attempt(
        &self,
        target: &ContinueWorkTarget,
        operation_id: &str,
    ) -> Result<Option<DurableContinueWorkAttempt>, ContinueWorkFailure> {
        let successor = gwt::cli::execution_state::continuation_attempt_for_operation(
            &target.worktree_path,
            target.owner,
            operation_id,
        )
        .map_err(|error| {
            ContinueWorkFailure::conflict(format!(
                "The continuation operation could not be read safely: {error}"
            ))
        })?;
        let takeover = gwt::cli::execution_state::generation_takeover_attempt_for_operation(
            &target.worktree_path,
            target.owner,
            operation_id,
        )
        .map_err(|error| {
            ContinueWorkFailure::conflict(format!(
                "The takeover operation could not be read safely: {error}"
            ))
        })?;
        match (successor, takeover) {
            (Some(_), Some(_)) => Err(ContinueWorkFailure::conflict(
                "The operation id is bound to conflicting durable continuation attempts.",
            )),
            (Some(attempt), None) => Ok(Some(DurableContinueWorkAttempt::Successor(Box::new(
                attempt,
            )))),
            (None, Some(attempt)) => Ok(Some(DurableContinueWorkAttempt::Takeover(Box::new(
                attempt,
            )))),
            (None, None) => Ok(None),
        }
    }

    fn exact_durable_continue_work_candidate(
        &self,
        target: &ContinueWorkTarget,
        attempt: &DurableContinueWorkAttempt,
    ) -> Result<gwt_agent::Session, ContinueWorkFailure> {
        let candidate_session_id = attempt.candidate_session_id();
        gwt_agent::validate_session_id_path_component(candidate_session_id).map_err(|_| {
            ContinueWorkFailure::conflict(
                "The committed continuation candidate Session id is not canonical.",
            )
        })?;
        let candidate_path = self
            .sessions_dir
            .join(format!("{candidate_session_id}.toml"));
        if candidate_path.file_stem().and_then(|value| value.to_str()) != Some(candidate_session_id)
        {
            return Err(ContinueWorkFailure::conflict(
                "The committed continuation candidate filename is not canonical.",
            ));
        }
        let session = gwt_agent::Session::load_and_migrate(&candidate_path).map_err(|error| {
            ContinueWorkFailure::conflict(format!(
                "The committed continuation candidate Session could not be read exactly: {error}"
            ))
        })?;
        let binding = session.execution_binding.as_ref().ok_or_else(|| {
            ContinueWorkFailure::conflict(
                "The committed continuation candidate Session has no execution binding.",
            )
        })?;
        let expected_agent_id = target.work_agent_id.as_ref().ok_or_else(|| {
            ContinueWorkFailure::conflict(
                "The committed continuation target has no exact Agent identity.",
            )
        })?;
        let current_branch = canonical_continue_work_branch(&target.worktree_path)?;
        let target_branch = target.work_branch.as_deref().ok_or_else(|| {
            ContinueWorkFailure::conflict(
                "The committed continuation target has no canonical branch.",
            )
        })?;
        let session_branch = crate::runtime_support::normalize_branch_name(session.branch.trim());
        let repo_hash = gwt_core::repo_hash::detect_repo_hash(&target.worktree_path)
            .map(|value| value.to_string());
        let binding_matches = attempt
            .candidate_binding_matches(&target.worktree_path, target.owner, binding)
            .map_err(|error| {
                ContinueWorkFailure::conflict(format!(
                    "The committed continuation candidate binding could not be verified: {error}"
                ))
            })?;
        let live_agent_matches = self
            .active_agent_sessions
            .values()
            .filter(|active| {
                active.session_id == candidate_session_id
                    && active.tab_id == target.tab_id
                    && path_matches(&active.worktree_path, &target.worktree_path)
                    && self.window_lookup.contains_key(&active.window_id)
                    && self.window_status(&active.window_id).is_some_and(|status| {
                        !matches!(
                            status,
                            WindowProcessStatus::Stopped | WindowProcessStatus::Error
                        )
                    })
            })
            .all(|active| {
                gwt_agent::resolve_agent_id(active.agent_id.trim()).as_ref()
                    == Some(expected_agent_id)
            });
        if session.id != candidate_session_id
            || !session_matches_project_state(&session, &target.project_root)
            || !path_matches(&session.worktree_path, &target.worktree_path)
            || session.linked_issue_number != Some(target.owner.number)
            || repo_hash.as_deref() != session.repo_hash.as_deref()
            || repo_hash.as_deref() != Some(binding.repo_hash.as_str())
            || current_branch != target_branch
            || session_branch != current_branch
            || &session.agent_id != expected_agent_id
            || !live_agent_matches
            || binding.owner_kind != target.owner.kind.as_str()
            || binding.owner_number != target.owner.number
            || !binding_matches
        {
            return Err(ContinueWorkFailure::conflict(
                "The committed continuation candidate no longer matches its exact authority.",
            ));
        }
        Ok(session)
    }

    fn validate_durable_continue_work_candidate_if_present(
        &self,
        target: &ContinueWorkTarget,
        attempt: &DurableContinueWorkAttempt,
    ) -> Result<(), ContinueWorkFailure> {
        let candidate_session_id = attempt.candidate_session_id();
        gwt_agent::validate_session_id_path_component(candidate_session_id).map_err(|_| {
            ContinueWorkFailure::conflict(
                "The durable continuation candidate Session id is not canonical.",
            )
        })?;
        let candidate_path = self
            .sessions_dir
            .join(format!("{candidate_session_id}.toml"));
        match gwt_agent::inspect_session_path(&candidate_path) {
            gwt_agent::SessionPathState::Missing => Ok(()),
            gwt_agent::SessionPathState::Present(_) => self
                .exact_durable_continue_work_candidate(target, attempt)
                .map(|_| ()),
            gwt_agent::SessionPathState::Error(error) => {
                Err(ContinueWorkFailure::conflict(format!(
                "The durable continuation candidate Session could not be inspected safely: {error}"
            )))
            }
        }
    }

    fn reconcile_aborted_continue_work_attempt(
        &mut self,
        client_id: &str,
        operation_id: String,
        work_id: String,
        target: &ContinueWorkTarget,
        attempt: &DurableContinueWorkAttempt,
    ) -> Vec<OutboundEvent> {
        if let Err(failure) =
            self.validate_durable_continue_work_candidate_if_present(target, attempt)
        {
            return self.continue_work_uncached_failure_events(
                client_id,
                operation_id,
                work_id,
                failure,
            );
        }
        let candidate_session_id = attempt.candidate_session_id();
        let candidate_path = self
            .sessions_dir
            .join(format!("{candidate_session_id}.toml"));
        match gwt_agent::inspect_session_path(&candidate_path) {
            gwt_agent::SessionPathState::Present(_) => {
                let session = match self.exact_durable_continue_work_candidate(target, attempt) {
                    Ok(session) => session,
                    Err(failure) => {
                        return self.continue_work_uncached_failure_events(
                            client_id,
                            operation_id,
                            work_id,
                            failure,
                        )
                    }
                };
                let session_identity =
                    match gwt_agent::SessionExecutionIdentity::from_session(&session) {
                        Ok(Some(identity)) => identity,
                        _ => {
                            return self.continue_work_uncached_failure_events(
                                client_id,
                                operation_id,
                                work_id,
                                ContinueWorkFailure::conflict(
                                    "The aborted candidate Session identity is not canonical.",
                                ),
                            )
                        }
                    };
                let cleanup = gwt::cli::execution_state::remove_exact_session_with_owner_lease(
                    &target.worktree_path,
                    target.owner,
                    &self.sessions_dir,
                    &session_identity,
                    || {
                        reject_continue_work_workspace_commit(
                            &target.project_root,
                            &target.worktree_path,
                            &operation_id,
                        )
                    },
                );
                match cleanup {
                    Ok(true) => {}
                    Ok(false) => {
                        return self.continue_work_uncached_failure_events(
                            client_id,
                            operation_id,
                            work_id,
                            ContinueWorkFailure::conflict(
                                "The aborted candidate Session changed before exact cleanup.",
                            ),
                        )
                    }
                    Err(error) => {
                        return self.continue_work_uncached_failure_events(
                            client_id,
                            operation_id,
                            work_id,
                            ContinueWorkFailure::failed(
                                "continuation_reconciliation_required",
                                format!(
                                    "The aborted candidate Session could not be removed safely: {error}"
                                ),
                                true,
                            ),
                        )
                    }
                }
            }
            gwt_agent::SessionPathState::Missing => {
                invoke_missing_session_cleanup_hook(candidate_session_id);
                let cleanup = gwt::cli::execution_state::commit_if_session_missing_with_owner_lease(
                    &target.worktree_path,
                    target.owner,
                    &self.sessions_dir,
                    candidate_session_id,
                    || {
                        reject_continue_work_workspace_commit(
                            &target.project_root,
                            &target.worktree_path,
                            &operation_id,
                        )
                    },
                );
                match cleanup {
                    Ok(true) => {}
                    Ok(false) => {
                        return self.continue_work_uncached_failure_events(
                            client_id,
                            operation_id,
                            work_id,
                            ContinueWorkFailure::conflict(
                                "The aborted candidate Session materialized before missing-state cleanup.",
                            ),
                        );
                    }
                    Err(error) => {
                        return self.continue_work_uncached_failure_events(
                            client_id,
                            operation_id,
                            work_id,
                            ContinueWorkFailure::failed(
                                "continuation_reconciliation_required",
                                format!(
                                    "The aborted continuation Work transaction could not be rejected safely: {error}"
                                ),
                                true,
                            ),
                        );
                    }
                }
            }
            gwt_agent::SessionPathState::Error(error) => {
                return self.continue_work_uncached_failure_events(
                    client_id,
                    operation_id,
                    work_id,
                    ContinueWorkFailure::failed(
                        "continuation_reconciliation_required",
                        format!("The aborted candidate Session could not be read safely: {error}"),
                        true,
                    ),
                );
            }
        }

        self.continue_work_failure_events(
            client_id,
            operation_id,
            work_id,
            ContinueWorkFailure::failed(
                "continuation_aborted",
                "This continuation attempt was aborted. Start Continue work again.",
                false,
            ),
        )
    }

    fn reconcile_durable_continue_work_attempt(
        &mut self,
        client_id: &str,
        operation_id: String,
        work_id: String,
        bounds: WindowGeometry,
        target: &ContinueWorkTarget,
        attempt: DurableContinueWorkAttempt,
    ) -> Vec<OutboundEvent> {
        if attempt.work_id() != Some(work_id.as_str()) {
            return self.continue_work_uncached_failure_events(
                client_id,
                operation_id,
                work_id,
                ContinueWorkFailure::failed(
                    "operation_conflict",
                    "This operation id is already bound to another Work.",
                    false,
                ),
            );
        }
        if attempt.source().is_none_or(|source| {
            !matches!(source, "continue-work:resume" | "continue-work:handoff")
        }) {
            return self.continue_work_uncached_failure_events(
                client_id,
                operation_id,
                work_id,
                ContinueWorkFailure::conflict(
                    "The durable continuation source is missing or unsupported.",
                ),
            );
        }

        match attempt.status() {
            DurableContinueWorkAttemptStatus::Aborted => {
                return self.reconcile_aborted_continue_work_attempt(
                    client_id,
                    operation_id,
                    work_id,
                    target,
                    &attempt,
                );
            }
            DurableContinueWorkAttemptStatus::Prepared => {
                if let Err(failure) =
                    self.validate_durable_continue_work_candidate_if_present(target, &attempt)
                {
                    return self.continue_work_uncached_failure_events(
                        client_id,
                        operation_id,
                        work_id,
                        failure,
                    );
                }
                let candidate_session_id = attempt.candidate_session_id().to_string();
                match self.classify_nonlocal_active_owner_liveness(&candidate_session_id) {
                    ActiveOwnerLiveness::Unknown => {
                        return self.continue_work_uncached_failure_events(
                            client_id,
                            operation_id,
                            work_id,
                            ContinueWorkFailure::failed(
                                "continuation_reconciliation_required",
                                "A prior continuation candidate may still be live; retry after its launch is reconciled.",
                                true,
                            ),
                        );
                    }
                    ActiveOwnerLiveness::Stale(_) => {}
                }

                let exact_candidate =
                    match self.exact_durable_continue_work_candidate(target, &attempt) {
                        Ok(session) => session,
                        Err(failure) => {
                            return self.continue_work_uncached_failure_events(
                                client_id,
                                operation_id,
                                work_id,
                                failure,
                            )
                        }
                    };
                let session_identity =
                    match gwt_agent::SessionExecutionIdentity::from_session(&exact_candidate) {
                        Ok(Some(identity)) => identity,
                        _ => {
                            return self.continue_work_uncached_failure_events(
                                client_id,
                                operation_id,
                                work_id,
                                ContinueWorkFailure::conflict(
                                    "The Prepared continuation Session identity is not canonical.",
                                ),
                            )
                        }
                    };
                let cleanup = attempt.abort_and_remove_exact_session(
                    &target.worktree_path,
                    target.owner,
                    "owning Host crashed before authenticated Ready",
                    &self.sessions_dir,
                    &session_identity,
                    || {
                        reject_continue_work_workspace_commit(
                            &target.project_root,
                            &target.worktree_path,
                            &operation_id,
                        )
                    },
                );
                return match cleanup {
                    Ok(true) => self.continue_work_failure_events(
                        client_id,
                        operation_id,
                        work_id,
                        ContinueWorkFailure::failed(
                            "continuation_aborted",
                            "This continuation attempt was aborted. Start Continue work again.",
                            false,
                        ),
                    ),
                    Ok(false) => self.continue_work_uncached_failure_events(
                        client_id,
                        operation_id,
                        work_id,
                        ContinueWorkFailure::conflict(
                            "The Prepared continuation candidate changed before exact cleanup.",
                        ),
                    ),
                    Err(error) => {
                        match self.durable_continue_work_attempt(target, &operation_id) {
                            Ok(Some(latest))
                                if latest.status()
                                    == DurableContinueWorkAttemptStatus::Activated =>
                            {
                                self.reconcile_durable_continue_work_attempt(
                                    client_id,
                                    operation_id,
                                    work_id,
                                    bounds,
                                    target,
                                    latest,
                                )
                            }
                            _ => self.continue_work_uncached_failure_events(
                                client_id,
                                operation_id,
                                work_id,
                                ContinueWorkFailure::failed(
                                    "continuation_reconciliation_required",
                                    format!(
                                        "The stale Prepared continuation could not commit exact cleanup: {error}"
                                    ),
                                    true,
                                ),
                            ),
                        }
                    }
                };
            }
            DurableContinueWorkAttemptStatus::Activated => {}
        }

        if let Err(failure) = self.exact_durable_continue_work_candidate(target, &attempt) {
            return self.continue_work_uncached_failure_events(
                client_id,
                operation_id,
                work_id,
                failure,
            );
        }
        if let Err(error) = attempt.repair_activation(&target.worktree_path, target.owner) {
            return self.continue_work_uncached_failure_events(
                client_id,
                operation_id,
                work_id,
                ContinueWorkFailure::conflict(format!(
                    "The committed continuation generation could not be repaired: {error}"
                )),
            );
        }
        invoke_durable_continue_work_post_repair_hook();
        let exact_candidate = match self.exact_durable_continue_work_candidate(target, &attempt) {
            Ok(session) => session,
            Err(failure) => {
                return self.continue_work_uncached_failure_events(
                    client_id,
                    operation_id,
                    work_id,
                    failure,
                )
            }
        };
        let exact_session_identity =
            match gwt_agent::SessionExecutionIdentity::from_session(&exact_candidate) {
                Ok(Some(identity)) => identity,
                _ => {
                    return self.continue_work_uncached_failure_events(
                        client_id,
                        operation_id,
                        work_id,
                        ContinueWorkFailure::conflict(
                            "The committed continuation Session identity is not canonical.",
                        ),
                    )
                }
            };
        invoke_durable_continue_work_pre_work_commit_hook();
        let candidate_session_id = attempt.candidate_session_id().to_string();
        let leased_readback =
            gwt::cli::execution_state::with_current_active_session_execution_identity_lease(
                &self.sessions_dir,
                &exact_session_identity,
                || {
                    match resolve_split_workspace_state_external_commit(
                        &target.project_root,
                        &target.worktree_path,
                        &operation_id,
                        gwt_core::workspace_projection::ExternalWorkspaceCommitDecision::Commit,
                    ) {
                        Ok(
                            gwt_core::workspace_projection::ExternalWorkspaceCommitResolution::Committed,
                        ) => {}
                        Ok(
                            gwt_core::workspace_projection::ExternalWorkspaceCommitResolution::Busy,
                        ) => {
                            return Err(ContinueWorkFailure::failed(
                                "continuation_reconciliation_required",
                                "The committed continuation Work transaction is still being reconciled.",
                                true,
                            ));
                        }
                        Ok(resolution) => {
                            return Err(ContinueWorkFailure::conflict(format!(
                                "The committed continuation has no matching Work transaction: {resolution:?}"
                            )));
                        }
                        Err(error) => {
                            return Err(ContinueWorkFailure::conflict(format!(
                                "The committed continuation Work transaction could not be repaired: {error}"
                            )));
                        }
                    }

                    let current = match gwt::cli::execution_state::current_execution_binding(
                        &target.worktree_path,
                        target.owner,
                    ) {
                        Ok(Some(binding)) if attempt.activated_identity_matches(&binding) => {
                            binding
                        }
                        _ => {
                            return Err(ContinueWorkFailure::conflict(
                                "The committed continuation is no longer the current execution binding.",
                            ));
                        }
                    };
                    if !exact_candidate
                        .execution_binding
                        .as_ref()
                        .is_some_and(|binding| binding.identity == current)
                    {
                        return Err(ContinueWorkFailure::conflict(
                            "The committed continuation Session binding could not be read back.",
                        ));
                    }
                    let projection_matches = gwt::cli::execution_state::load(&target.worktree_path)
                        .ok()
                        .flatten()
                        .is_some_and(|record| {
                            record.owner_kind == target.owner.kind
                                && record.owner_number == target.owner.number
                                && record.status
                                    == gwt::cli::execution_state::ExecutionControlStatus::Active
                                && record.primary_session_id == candidate_session_id
                        });
                    let work_matches = gwt_core::workspace_projection::load_workspace_work_items(
                        &target.worktree_path,
                    )
                    .ok()
                    .flatten()
                    .is_some_and(|projection| {
                        let matching = projection
                            .work_items
                            .iter()
                            .filter(|item| item.id == work_id)
                            .collect::<Vec<_>>();
                        matching.len() == 1
                            && !matching[0].discarded
                            && matching[0].status_category
                                == gwt_core::workspace_projection::WorkspaceStatusCategory::Active
                            && projection_continue_authority_matches(
                                matching[0],
                                &target.project_root,
                                target.owner,
                                &target.worktree_path,
                                &exact_candidate.branch,
                                &exact_candidate.agent_id,
                                Some(&candidate_session_id),
                            )
                    });
                    if !projection_matches || !work_matches {
                        return Err(ContinueWorkFailure::conflict(
                            "The committed continuation generation and Work projection do not agree.",
                        ));
                    }
                    Ok((current, exact_candidate))
                },
            );
        let (current, durable) = match leased_readback {
            Ok(Some(Ok(readback))) => readback,
            Ok(Some(Err(failure))) => {
                return self.continue_work_uncached_failure_events(
                    client_id,
                    operation_id,
                    work_id,
                    failure,
                )
            }
            Ok(None) => {
                return self.continue_work_uncached_failure_events(
                    client_id,
                    operation_id,
                    work_id,
                    ContinueWorkFailure::conflict(
                        "The committed continuation Session changed before Work publication.",
                    ),
                )
            }
            Err(error) => {
                return self.continue_work_uncached_failure_events(
                    client_id,
                    operation_id,
                    work_id,
                    ContinueWorkFailure::conflict(format!(
                        "The committed continuation lease could not be acquired: {error}"
                    )),
                )
            }
        };
        let outcome = attempt
            .outcome()
            .expect("validated continuation source has one typed outcome");
        let local = self
            .active_agent_sessions
            .iter()
            .find(|(window_id, active)| {
                active.session_id == candidate_session_id
                    && active.tab_id == target.tab_id
                    && self.window_lookup.contains_key(window_id.as_str())
                    && self.window_status(window_id).is_some_and(|status| {
                        !matches!(
                            status,
                            WindowProcessStatus::Stopped | WindowProcessStatus::Error
                        )
                    })
            })
            .map(|(window_id, active)| (window_id.clone(), active.clone()));
        let Some((window_id, active)) = local else {
            return self.continue_work_uncached_failure_events(
                client_id,
                operation_id,
                work_id,
                ContinueWorkFailure::failed(
                    "continuation_reconciliation_required",
                    "The committed continuation has no live exact pane on this Host.",
                    true,
                ),
            );
        };
        let durable_binding = durable.execution_binding.clone();
        if !self.ensure_local_active_execution_authority(
            &window_id,
            &active,
            target.owner,
            &current,
            durable,
        ) {
            return self.continue_work_uncached_failure_events(
                client_id,
                operation_id,
                work_id,
                ContinueWorkFailure::failed(
                    "continuation_reconciliation_required",
                    "The live continuation pane could not restore exact active Host authority.",
                    true,
                ),
            );
        }
        let Some(durable_binding) = durable_binding else {
            return self.continue_work_uncached_failure_events(
                client_id,
                operation_id,
                work_id,
                ContinueWorkFailure::failed(
                    "continuation_reconciliation_required",
                    "The live continuation pane has no durable execution binding.",
                    true,
                ),
            );
        };
        let predecessor_evidence =
            attempt.predecessor_evidence(&target.worktree_path, target.owner);
        let Ok((predecessor_session_id, predecessor_binding)) = predecessor_evidence else {
            return self.continue_work_uncached_failure_events(
                client_id,
                operation_id,
                work_id,
                ContinueWorkFailure::failed(
                    "continuation_reconciliation_required",
                    "The predecessor fence evidence could not be reconstructed.",
                    true,
                ),
            );
        };
        let active_probe = gwt::probe_authenticated_execution_binding(
            &target.project_root,
            &candidate_session_id,
            &durable_binding,
            "continue-work-reconciler",
            gwt::AgentExecutionBindingProbeRequest {
                schema_version: gwt::AGENT_EXECUTION_BINDING_PROBE_SCHEMA_VERSION,
                operation_id: operation_id.clone(),
                nonce: uuid::Uuid::new_v4().to_string(),
            },
        );
        if active_probe.as_ref().is_err()
            || active_probe
                .as_ref()
                .is_ok_and(|receipt| receipt.execution_binding != current)
            || gwt::cli::execution_state::current_active_execution_binding_matches(
                &target.worktree_path,
                target.owner,
                &predecessor_session_id,
                &predecessor_binding,
            )
            .unwrap_or(true)
        {
            return self.continue_work_uncached_failure_events(
                client_id,
                operation_id,
                work_id,
                ContinueWorkFailure::failed(
                    "continuation_reconciliation_required",
                    "The live continuation pane could not prove active authority and predecessor fencing.",
                    true,
                ),
            );
        }
        let mut events = self.focus_existing_live_work_agent_events(&window_id, Some(bounds));
        let cached_outcome = CachedContinueWorkOutcome {
            work_id,
            outcome,
            message: None,
            error_code: None,
            retryable: false,
        };
        self.continue_work_outcomes
            .insert(operation_id.clone(), cached_outcome.clone());
        self.pending_continue_work
            .retain(|_, pending| pending.operation_id != operation_id);
        events.extend(self.continue_work_correlated_outcome_events(
            client_id,
            operation_id,
            cached_outcome,
        ));
        events.push(self.workspace_state_broadcast());
        if let Some(projection) = self.active_work_projection_broadcast_for_active_tab() {
            events.push(projection);
        }
        events
    }

    pub(crate) fn continue_work_events(
        &mut self,
        client_id: &str,
        operation_id: String,
        work_id: String,
        bounds: WindowGeometry,
    ) -> Vec<OutboundEvent> {
        if !canonical_public_id(&operation_id, 256) || !canonical_public_id(&work_id, 512) {
            return self.continue_work_failure_events(
                client_id,
                operation_id,
                work_id,
                ContinueWorkFailure::failed(
                    "invalid_request",
                    "Continue work request identity is invalid.",
                    false,
                ),
            );
        }
        if let Some(cached) = self.continue_work_outcomes.get(&operation_id).cloned() {
            if cached.work_id != work_id {
                return self.continue_work_uncached_failure_events(
                    client_id,
                    operation_id,
                    work_id,
                    ContinueWorkFailure::failed(
                        "operation_conflict",
                        "This operation id is already bound to another Work.",
                        false,
                    ),
                );
            }
            return self.continue_work_correlated_outcome_events(client_id, operation_id, cached);
        }
        if let Some((window_id, pending)) = self
            .pending_continue_work
            .iter()
            .find(|(_, pending)| pending.operation_id == operation_id)
            .map(|(window_id, pending)| (window_id.clone(), pending.clone()))
        {
            if pending.work_id != work_id {
                return self.continue_work_uncached_failure_events(
                    client_id,
                    operation_id,
                    work_id,
                    ContinueWorkFailure::failed(
                        "operation_conflict",
                        "This operation id is already bound to another Work.",
                        false,
                    ),
                );
            }
            if !pending_execution_is_activated(&pending) {
                self.continue_work_waiters
                    .entry(operation_id.clone())
                    .or_default()
                    .insert(client_id.to_string());
                return self.focus_existing_live_work_agent_events(&window_id, Some(bounds));
            }
        }
        if self
            .pending_continue_work
            .values()
            .any(|pending| pending.work_id == work_id && pending.operation_id != operation_id)
        {
            return self.continue_work_failure_events(
                client_id,
                operation_id,
                work_id,
                ContinueWorkFailure::failed(
                    "continue_work_in_progress",
                    "Continue work is already in progress for this Work.",
                    true,
                ),
            );
        }

        let target = match self.resolve_continue_work_target(&work_id) {
            Ok(target) => target,
            Err(failure) if failure.code == "work_state_unavailable" => {
                match self.resolve_continue_work_target_for_durable_operation(&operation_id) {
                    Ok(Some(target)) => target,
                    Ok(None) => {
                        return self.continue_work_uncached_failure_events(
                            client_id,
                            operation_id,
                            work_id,
                            failure,
                        )
                    }
                    Err(reconciliation_failure) => {
                        return self.continue_work_uncached_failure_events(
                            client_id,
                            operation_id,
                            work_id,
                            reconciliation_failure,
                        )
                    }
                }
            }
            Err(failure) => {
                return self.continue_work_failure_events(client_id, operation_id, work_id, failure)
            }
        };
        let generation_was_missing = match gwt::cli::execution_state::load_generation_ledger(
            &target.worktree_path,
            target.owner,
        ) {
            Ok(Some(_)) => false,
            Ok(None) => true,
            Err(error) => {
                return self.continue_work_failure_events(
                    client_id,
                    operation_id,
                    work_id,
                    ContinueWorkFailure::conflict(format!(
                        "The execution generation could not be verified: {error}"
                    )),
                )
            }
        };
        let legacy_disposition = if generation_was_missing {
            match gwt::cli::execution_state::load(&target.worktree_path) {
                Ok(Some(record))
                    if record.status
                        == gwt::cli::execution_state::ExecutionControlStatus::Active =>
                {
                    let local_live =
                        self.active_agent_sessions
                            .iter()
                            .any(|(window_id, active)| {
                                active.session_id == record.primary_session_id
                                    && active.tab_id == target.tab_id
                                    && self.window_lookup.contains_key(window_id.as_str())
                                    && self.window_status(window_id).is_some_and(|status| {
                                        !matches!(
                                            status,
                                            WindowProcessStatus::Stopped
                                                | WindowProcessStatus::Error
                                        )
                                    })
                            });
                    if local_live
                        || matches!(
                            self.classify_nonlocal_active_owner_liveness(
                                &record.primary_session_id,
                            ),
                            ActiveOwnerLiveness::Stale(_)
                        )
                    {
                        gwt::cli::execution_state::LegacyActiveDisposition::Live
                    } else {
                        gwt::cli::execution_state::LegacyActiveDisposition::Unknown
                    }
                }
                _ => gwt::cli::execution_state::LegacyActiveDisposition::Unknown,
            }
        } else {
            gwt::cli::execution_state::LegacyActiveDisposition::Unknown
        };
        if let Err(error) = gwt::cli::execution_state::ensure_generation_ledger(
            &target.worktree_path,
            target.owner,
            legacy_disposition,
        ) {
            return self.continue_work_failure_events(
                client_id,
                operation_id,
                work_id,
                ContinueWorkFailure::conflict(format!(
                    "The current execution owner could not be classified safely: {error}"
                )),
            );
        }
        let ledger = match gwt::cli::execution_state::load_generation_ledger(
            &target.worktree_path,
            target.owner,
        ) {
            Ok(Some(ledger)) => ledger,
            Ok(None) => {
                return self.continue_work_failure_events(
                    client_id,
                    operation_id,
                    work_id,
                    ContinueWorkFailure::failed(
                        "execution_owner_missing",
                        "No execution generation exists for this Work.",
                        false,
                    ),
                )
            }
            Err(error) => {
                return self.continue_work_failure_events(
                    client_id,
                    operation_id,
                    work_id,
                    ContinueWorkFailure::conflict(format!(
                        "The execution generation could not be verified: {error}"
                    )),
                )
            }
        };
        let durable_attempt = match self.durable_continue_work_attempt(&target, &operation_id) {
            Ok(attempt) => attempt,
            Err(failure) => {
                return self.continue_work_failure_events(
                    client_id,
                    operation_id,
                    work_id,
                    failure,
                );
            }
        };
        if let Some(attempt) = durable_attempt {
            return self.reconcile_durable_continue_work_attempt(
                client_id,
                operation_id,
                work_id,
                bounds,
                &target,
                attempt,
            );
        }
        let takeover_owner = match ledger.current_effective_status() {
            Some(gwt::cli::execution_state::ExecutionControlStatus::Active) => {
                let Some(record) = gwt::cli::execution_state::load(&target.worktree_path)
                    .ok()
                    .flatten()
                else {
                    return self.continue_work_failure_events(
                        client_id,
                        operation_id,
                        work_id,
                        ContinueWorkFailure::conflict(
                            "The active execution projection could not be verified.",
                        ),
                    );
                };
                if let Some((window_id, active)) = self
                    .active_agent_sessions
                    .iter()
                    .find(|(window_id, active)| {
                        active.session_id == record.primary_session_id
                            && active.tab_id == target.tab_id
                            && self.window_lookup.contains_key(window_id.as_str())
                            && self.window_status(window_id).is_some_and(|status| {
                                !matches!(
                                    status,
                                    WindowProcessStatus::Stopped | WindowProcessStatus::Error
                                )
                            })
                    })
                    .map(|(window_id, active)| (window_id.clone(), active.clone()))
                {
                    let current = gwt::cli::execution_state::current_execution_binding(
                        &target.worktree_path,
                        target.owner,
                    )
                    .ok()
                    .flatten();
                    let durable = gwt_agent::Session::load_and_migrate(
                        &self
                            .sessions_dir
                            .join(format!("{}.toml", active.session_id)),
                    )
                    .ok();
                    if let (Some(current), Some(durable)) = (current, durable) {
                        if self.ensure_local_active_execution_authority(
                            &window_id,
                            &active,
                            target.owner,
                            &current,
                            durable,
                        ) {
                            let mut events = self
                                .focus_existing_live_work_agent_events(&window_id, Some(bounds));
                            self.continue_work_outcomes.insert(
                                operation_id.clone(),
                                CachedContinueWorkOutcome {
                                    work_id: work_id.clone(),
                                    outcome: gwt::ContinueWorkOutcomeKind::FocusedExisting,
                                    message: None,
                                    error_code: None,
                                    retryable: false,
                                },
                            );
                            events.push(continue_work_outcome(
                                client_id,
                                operation_id,
                                work_id,
                                gwt::ContinueWorkOutcomeKind::FocusedExisting,
                                None,
                                None,
                                false,
                            ));
                            return events;
                        }
                    }
                }
                match self.classify_nonlocal_active_owner_liveness(
                    &record.primary_session_id,
                ) {
                    ActiveOwnerLiveness::Stale(reason) => {
                        Some((record.primary_session_id, reason))
                    }
                    ActiveOwnerLiveness::Unknown => {
                        return self.continue_work_failure_events(
                            client_id,
                            operation_id,
                            work_id,
                            ContinueWorkFailure::conflict(
                                "Another Host may own the active execution; its liveness could not be determined.",
                            ),
                        )
                    }
                }
            }
            Some(gwt::cli::execution_state::ExecutionControlStatus::Blocked) => {
                return self.continue_work_failure_events(
                    client_id,
                    operation_id,
                    work_id,
                    ContinueWorkFailure::failed(
                        "execution_blocked",
                        "Resolve and reopen the blocked execution before continuing Work.",
                        false,
                    ),
                );
            }
            Some(gwt::cli::execution_state::ExecutionControlStatus::Completed) => None,
            None => {
                return self.continue_work_failure_events(
                    client_id,
                    operation_id,
                    work_id,
                    ContinueWorkFailure::conflict(
                        "The current execution generation is incomplete.",
                    ),
                );
            }
        };
        let predecessor_session_id = match (
            takeover_owner.as_ref(),
            gwt::cli::execution_state::load(&target.worktree_path),
        ) {
            (Some((session_id, _)), _) => session_id.clone(),
            (None, Ok(Some(record)))
                if record.owner_kind == target.owner.kind
                    && record.owner_number == target.owner.number
                    && record.status
                        == gwt::cli::execution_state::ExecutionControlStatus::Completed =>
            {
                record.primary_session_id
            }
            _ => {
                return self.continue_work_failure_events(
                    client_id,
                    operation_id,
                    work_id,
                    ContinueWorkFailure::conflict(
                        "The current execution projection changed before continuation.",
                    ),
                )
            }
        };
        let (mut config, outcome) = match &target.launch_seed {
            ContinueWorkLaunchSeed::DurableSession(source_session) => {
                let mut config = launch_config_from_persisted_session(source_session);
                config.working_dir = Some(target.worktree_path.clone());
                config.branch = Some(source_session.branch.clone());
                let outcome = configure_provider_continuation(&mut config, source_session);
                (config, outcome)
            }
            ContinueWorkLaunchSeed::WorkProjection {
                agent_id,
                display_name,
                branch,
            } => {
                let mut config = gwt_agent::AgentLaunchBuilder::new(agent_id.clone())
                    .working_dir(target.worktree_path.clone())
                    .branch(branch.clone())
                    .linked_issue_number(target.owner.number)
                    .session_mode(gwt_agent::SessionMode::Normal)
                    .build();
                if let Some(display_name) = display_name {
                    config.display_name = display_name.clone();
                }
                (config, gwt::ContinueWorkOutcomeKind::StartedWithHandoff)
            }
        };
        config.linked_issue_number = Some(target.owner.number);
        let continuation_session_id = uuid::Uuid::new_v4().to_string();
        let predecessor_binding = match gwt::cli::execution_state::current_execution_binding(
            &target.worktree_path,
            target.owner,
        ) {
            Ok(Some(binding)) => binding,
            _ => {
                return self.continue_work_failure_events(
                    client_id,
                    operation_id,
                    work_id,
                    ContinueWorkFailure::conflict(
                        "The predecessor execution binding disappeared before launch.",
                    ),
                );
            }
        };
        if outcome == gwt::ContinueWorkOutcomeKind::StartedWithHandoff {
            config.args.push(handoff_context(
                &target.resume_context,
                &work_id,
                &predecessor_binding,
            ));
        }
        let requested_at = chrono::Utc::now();
        let replaces_active_generation = takeover_owner.is_some();
        let execution =
            PendingContinueWorkExecution::Successor(gwt::cli::execution_state::SuccessorRequest {
                operation_id: operation_id.clone(),
                principal_id: "gwt-host-continuation".to_string(),
                work_id: Some(work_id.clone()),
                source: match outcome {
                    gwt::ContinueWorkOutcomeKind::ContinuedConversation => {
                        "continue-work:resume".to_string()
                    }
                    _ => "continue-work:handoff".to_string(),
                },
                session_binding_id: uuid::Uuid::new_v4().to_string(),
                initial_session_id: continuation_session_id.clone(),
                entrypoint: gwt::cli::execution_state::entrypoint_from_launch(
                    &config.args,
                    config.session_mode == gwt_agent::SessionMode::Resume,
                ),
                requested_at,
            });
        let prepared = match &execution {
            PendingContinueWorkExecution::Successor(request) => {
                if replaces_active_generation {
                    gwt::cli::execution_state::prepare_active_continuation_successor(
                        &target.worktree_path,
                        target.owner,
                        request,
                    )
                    .map(|_| ())
                } else {
                    gwt::cli::execution_state::prepare_successor(
                        &target.worktree_path,
                        target.owner,
                        request,
                    )
                    .map(|_| ())
                }
            }
            PendingContinueWorkExecution::Takeover(request) => {
                gwt::cli::execution_state::prepare_generation_takeover(
                    &target.worktree_path,
                    target.owner,
                    request,
                )
                .map(|_| ())
            }
        };
        if let Err(error) = prepared {
            return self.continue_work_failure_events(
                client_id,
                operation_id,
                work_id,
                ContinueWorkFailure::conflict(format!(
                    "The execution continuation could not be prepared: {error}"
                )),
            );
        }
        let planned_identity = match &execution {
            PendingContinueWorkExecution::Successor(request) => {
                gwt::cli::execution_state::prepared_successor_execution_binding(
                    &target.worktree_path,
                    target.owner,
                    request,
                )
            }
            PendingContinueWorkExecution::Takeover(request) => {
                gwt::cli::execution_state::prepared_generation_takeover_execution_binding(
                    &target.worktree_path,
                    target.owner,
                    request,
                )
            }
        };
        let planned_identity = match planned_identity {
            Ok(identity) => identity,
            Err(error) => {
                if let Err(abort_error) = abort_prepared_execution(
                    &target.worktree_path,
                    target.owner,
                    &execution,
                    "prepared binding derivation failed",
                ) {
                    tracing::warn!(
                        error_kind = ?abort_error.kind(),
                        "prepared continuation binding failure could not be durably aborted"
                    );
                    return self.continue_work_uncached_failure_events(
                        client_id,
                        operation_id,
                        work_id,
                        ContinueWorkFailure::failed(
                            "continuation_reconciliation_required",
                            "The Prepared continuation could not prove a durable abort; retry reconciliation.",
                            true,
                        ),
                    );
                }
                return self.continue_work_failure_events(
                    client_id,
                    operation_id,
                    work_id,
                    ContinueWorkFailure::failed(
                        "prepared_binding_failed",
                        format!("The Prepared execution binding could not be verified: {error}"),
                        true,
                    ),
                );
            }
        };
        let repo_hash = match &target.launch_seed {
            ContinueWorkLaunchSeed::DurableSession(source_session) => {
                source_session.repo_hash.clone()
            }
            ContinueWorkLaunchSeed::WorkProjection { .. } => None,
        }
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            gwt_core::repo_hash::detect_repo_hash(&target.worktree_path)
                .map(|repo_hash| repo_hash.to_string())
        });
        let Some(repo_hash) = repo_hash else {
            if let Err(abort_error) = abort_prepared_execution(
                &target.worktree_path,
                target.owner,
                &execution,
                "repository identity unavailable",
            ) {
                tracing::warn!(
                    error_kind = ?abort_error.kind(),
                    "repository identity failure could not durably abort continuation"
                );
                return self.continue_work_uncached_failure_events(
                    client_id,
                    operation_id,
                    work_id,
                    ContinueWorkFailure::failed(
                        "continuation_reconciliation_required",
                        "The Prepared continuation could not prove a durable abort; retry reconciliation.",
                        true,
                    ),
                );
            }
            return self.continue_work_failure_events(
                client_id,
                operation_id,
                work_id,
                ContinueWorkFailure::failed(
                    "repository_identity_unavailable",
                    "The repository identity could not be verified for continuation.",
                    false,
                ),
            );
        };
        let binding = gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: continuation_session_id,
            repo_hash,
            owner_kind: target.owner.kind.as_str().to_string(),
            owner_number: target.owner.number,
            identity: planned_identity,
            capability_generation: 1,
        };
        let readiness_nonce = uuid::Uuid::new_v4().to_string();
        config.env_vars.insert(
            gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV.to_string(),
            readiness_nonce.clone(),
        );
        config.execution_intent =
            gwt_agent::ExecutionLaunchIntent::PreparedContinuation(binding.clone());
        let dispatch_execution = execution.clone();
        let (Some(work_branch), Some(work_agent_id)) =
            (target.work_branch.clone(), target.work_agent_id.clone())
        else {
            let _ = abort_prepared_execution(
                &target.worktree_path,
                target.owner,
                &execution,
                "Work authority disappeared before continuation dispatch",
            );
            return self.continue_work_failure_events(
                client_id,
                operation_id,
                work_id,
                ContinueWorkFailure::conflict(
                    "The Work authority disappeared before continuation dispatch.",
                ),
            );
        };
        let pending = PendingContinueWork {
            client_id: client_id.to_string(),
            operation_id: operation_id.clone(),
            work_id: work_id.clone(),
            project_root: target.project_root.clone(),
            worktree_path: target.worktree_path.clone(),
            owner: target.owner,
            work_branch,
            work_agent_id,
            work_agent_session_id: target.work_agent_session_id.clone(),
            execution,
            binding,
            readiness_nonce,
            outcome,
            resume_context: target.resume_context.clone(),
            predecessor_session_id,
            predecessor_binding,
        };
        match self.spawn_continue_work_window(
            &target.tab_id,
            config,
            bounds,
            target.resume_context,
            pending,
        ) {
            Ok(events) => events,
            Err(error) => {
                if let Err(abort_error) = abort_prepared_execution(
                    &target.worktree_path,
                    target.owner,
                    &dispatch_execution,
                    "continuation window dispatch failed",
                ) {
                    tracing::warn!(
                        error_kind = ?abort_error.kind(),
                        "window dispatch failure could not durably abort continuation"
                    );
                    return self.continue_work_uncached_failure_events(
                        client_id,
                        operation_id,
                        work_id,
                        ContinueWorkFailure::failed(
                            "continuation_reconciliation_required",
                            "The Prepared continuation could not prove a durable abort; retry reconciliation.",
                            true,
                        ),
                    );
                }
                self.continue_work_failure_events(
                    client_id,
                    operation_id,
                    work_id,
                    ContinueWorkFailure::failed("launch_dispatch_failed", error, true),
                )
            }
        }
    }

    pub(crate) fn continue_work_launch_failed_events(
        &mut self,
        window_id: &str,
        detail: &str,
    ) -> Vec<OutboundEvent> {
        let Some(pending) = self.pending_continue_work.get(window_id).cloned() else {
            return Vec::new();
        };
        // Activated is the irreversible logical commit point. A late PTY
        // error or lost success delivery must be reconciled by operation
        // readback; it must never tear down or report failure for a committed
        // generation.
        let Some(is_activated) = pending_execution_activation_status(&pending) else {
            // Fail closed on an unreadable durable attempt. Teardown or abort
            // could destroy the exact live evidence needed by a retry.
            return Vec::new();
        };
        if is_activated {
            return self
                .reconcile_activated_pending_continue_work(window_id, &pending)
                .unwrap_or_default();
        }
        let session_identity = match pending_continue_work_session_identity(&pending) {
            Ok(identity) => identity,
            Err(error) => {
                tracing::warn!(error = %error, "retained continuation after invalid expected Session identity");
                return Vec::new();
            }
        };
        let cleanup = abort_prepared_execution_and_remove_exact_session(
            &pending.worktree_path,
            pending.owner,
            &pending.execution,
            "continuation launch failed before SessionStart",
            &self.sessions_dir,
            &session_identity,
            || match resolve_split_workspace_state_external_commit(
                &pending.project_root,
                &pending.worktree_path,
                &pending.operation_id,
                gwt_core::workspace_projection::ExternalWorkspaceCommitDecision::Reject,
            ) {
                Ok(
                    gwt_core::workspace_projection::ExternalWorkspaceCommitResolution::Rejected
                    | gwt_core::workspace_projection::ExternalWorkspaceCommitResolution::Missing,
                ) => Ok(()),
                Ok(resolution) => Err(std::io::Error::other(format!(
                    "continuation Work rejection returned {resolution:?}"
                ))),
                Err(error) => Err(std::io::Error::other(error.to_string())),
            },
        );
        match cleanup {
            Ok(true) => {}
            Ok(false) => return self.continue_work_pending_uncached_failure_events(
                &pending,
                ContinueWorkFailure::conflict(
                    "The failed continuation candidate no longer matches its exact Agent identity.",
                ),
            ),
            Err(_) => {
                if pending_execution_activation_status(&pending) == Some(true) {
                    return self
                        .reconcile_activated_pending_continue_work(window_id, &pending)
                        .unwrap_or_default();
                }
                return self.continue_work_pending_uncached_failure_events(
                    &pending,
                    ContinueWorkFailure::failed(
                        "continuation_reconciliation_required",
                        "The failed continuation could not commit its exact cleanup; retry reconciliation.",
                        true,
                    ),
                );
            }
        }
        self.stop_window_runtime_without_session_projection(window_id);
        let mut events = self.close_window_events(window_id);
        self.pending_continue_work.remove(window_id);
        let message = format!("Continue work launch failed before activation: {detail}");
        self.continue_work_outcomes.insert(
            pending.operation_id.clone(),
            CachedContinueWorkOutcome {
                work_id: pending.work_id.clone(),
                outcome: gwt::ContinueWorkOutcomeKind::Failed,
                message: Some(message.clone()),
                error_code: Some("launch_failed".to_string()),
                retryable: true,
            },
        );
        events.extend(self.continue_work_pending_outcome_events(
            &pending,
            gwt::ContinueWorkOutcomeKind::Failed,
            Some(message),
            Some("launch_failed".to_string()),
            true,
        ));
        events
    }

    fn completed_fresh_execution_launch_events(
        &mut self,
        window_id: &str,
        pending: &PendingFreshExecutionLaunch,
    ) -> Vec<OutboundEvent> {
        let Some(active_session) = self.active_agent_sessions.get(window_id).cloned() else {
            return Vec::new();
        };
        if let Some(issue_number) = pending.linked_issue_number {
            if let Err(error) = super::launch::record_issue_branch_link_with_cache_dir(
                &pending.worktree_path,
                &active_session.branch_name,
                issue_number,
                &self.issue_link_cache_dir,
            ) {
                tracing::warn!(error = %error, "fresh launch issue linkage update was skipped");
            }
        }
        if let Err(error) =
            clear_durable_launch_recovery(&self.sessions_dir, &pending.binding.session_id)
        {
            tracing::warn!(
                session_id = %pending.binding.session_id,
                error = %error,
                "settled fresh-launch recovery receipt cleanup remains pending"
            );
            return Vec::new();
        }
        self.pending_fresh_execution_launches.remove(window_id);
        let _ = self.persist();
        self.launch_error_terminal_details.remove(window_id);
        let mut events = vec![self.workspace_state_broadcast()];
        if let Some(projection) = self.active_work_projection_broadcast_for_active_tab() {
            events.push(projection);
        }
        let composed_status = self
            .window_status(window_id)
            .unwrap_or(WindowProcessStatus::Running);
        events.extend(Self::status_events(
            window_id.to_string(),
            composed_status,
            None,
        ));
        // SPEC #3200 FR-052: the spawn path deliberately defers the Issue
        // Monitor completion for a fresh execution launch until this
        // SessionStart finalizer. It owns the same `launch_feedback_context`,
        // so it must ACK the durable delivery — otherwise the delivery tuple
        // stays pending forever and the daemon keeps redelivering it.
        if let Some(context) = pending.launch_feedback_context.as_ref() {
            if let Some(issue_number) = context.issue_monitor_issue_number {
                let project_root = context
                    .issue_monitor_project_root
                    .as_deref()
                    .unwrap_or(&pending.project_root);
                events.extend(self.issue_monitor_launch_completed_delivery_events(
                    project_root,
                    issue_number,
                    window_id,
                    context.issue_monitor_delivery_id.as_deref(),
                ));
            }
        }
        events
    }

    fn reconcile_activated_fresh_execution_launch_events(
        &mut self,
        window_id: &str,
        pending: &PendingFreshExecutionLaunch,
    ) -> Vec<OutboundEvent> {
        invoke_fresh_execution_pre_work_commit_hook();
        if !resolve_activated_fresh_execution_commit(
            &pending.project_root,
            &pending.worktree_path,
            pending.owner,
            &pending.operation_id,
            &pending.request,
            &self.sessions_dir,
            &pending.session_identity,
        )
        .unwrap_or(false)
        {
            return Vec::new();
        }
        let exact_current = gwt::cli::execution_state::current_execution_binding(
            &pending.worktree_path,
            pending.owner,
        )
        .ok()
        .flatten()
            == Some(pending.binding.identity.clone());
        let exact_live = self
            .active_agent_sessions
            .get(window_id)
            .is_some_and(|session| {
                session.session_id == pending.binding.session_id
                    && path_matches(&session.worktree_path, &pending.worktree_path)
            });
        let exact_capability = self
            .agent_capability_tokens
            .get(window_id)
            .zip(self.agent_capability_issuer.as_ref())
            .is_some_and(|(token, issuer)| issuer.active_token_is_current(token, &pending.binding));
        let exact_probe = exact_current
            && exact_live
            && exact_capability
            && gwt::probe_authenticated_execution_binding(
                &pending.project_root,
                &pending.binding.session_id,
                &pending.binding,
                "fresh-linked-owner-launch-coordinator",
                gwt::AgentExecutionBindingProbeRequest {
                    schema_version: gwt::AGENT_EXECUTION_BINDING_PROBE_SCHEMA_VERSION,
                    operation_id: pending.operation_id.clone(),
                    nonce: uuid::Uuid::new_v4().to_string(),
                },
            )
            .is_ok_and(|receipt| receipt.execution_binding == pending.binding.identity);
        if exact_probe {
            return self.completed_fresh_execution_launch_events(window_id, pending);
        }
        Vec::new()
    }

    pub(crate) fn fresh_execution_launch_failed_events(
        &mut self,
        window_id: &str,
        detail: &str,
    ) -> Vec<OutboundEvent> {
        let Some(pending) = self
            .pending_fresh_execution_launches
            .get(window_id)
            .cloned()
        else {
            return Vec::new();
        };
        let Some(status) = pending_fresh_execution_attempt_status(&pending) else {
            // Preserve the exact candidate evidence when durable authority is
            // unreadable. A later correlated retry can reconcile safely.
            return Vec::new();
        };
        if status == gwt::cli::execution_state::ContinuationAttemptStatus::Activated {
            return self.reconcile_activated_fresh_execution_launch_events(window_id, &pending);
        }

        let cleanup = if status == gwt::cli::execution_state::ContinuationAttemptStatus::Prepared {
            gwt::cli::execution_state::abort_successor_and_remove_exact_session(
                &pending.worktree_path,
                pending.owner,
                &pending.request,
                "fresh linked-owner launch failed before SessionStart",
                &self.sessions_dir,
                &pending.session_identity,
                || {
                    reject_continue_work_workspace_commit(
                        &pending.project_root,
                        &pending.worktree_path,
                        &pending.operation_id,
                    )
                },
            )
        } else {
            gwt::cli::execution_state::remove_exact_session_with_owner_lease(
                &pending.worktree_path,
                pending.owner,
                &self.sessions_dir,
                &pending.session_identity,
                || {
                    reject_continue_work_workspace_commit(
                        &pending.project_root,
                        &pending.worktree_path,
                        &pending.operation_id,
                    )
                },
            )
        };
        match cleanup {
            Ok(true) => {
                self.stop_window_runtime_without_session_projection(window_id);
                self.launch_wizard_cache
                    .forget_session(&pending.binding.session_id);
                let mut events = Self::status_events(
                    window_id.to_string(),
                    WindowProcessStatus::Error,
                    Some(detail.to_string()),
                );
                events.extend(self.close_window_events(window_id));
                self.pending_fresh_execution_launches.remove(window_id);
                let _ =
                    clear_durable_launch_recovery(&self.sessions_dir, &pending.binding.session_id);
                events
            }
            Ok(false) => {
                tracing::warn!(
                    session_id = %pending.binding.session_id,
                    "retained aborted fresh-launch pending state after binding mismatch"
                );
                Vec::new()
            }
            Err(error) => {
                if pending_fresh_execution_attempt_status(&pending)
                    == Some(gwt::cli::execution_state::ContinuationAttemptStatus::Activated)
                {
                    return self
                        .reconcile_activated_fresh_execution_launch_events(window_id, &pending);
                }
                tracing::warn!(
                session_id = %pending.binding.session_id,
                error = %error,
                    "retained aborted fresh-launch pending state after cleanup failure"
                );
                Vec::new()
            }
        }
    }

    pub(crate) fn handle_continue_work_ready_timeout(
        &mut self,
        window_id: &str,
        operation_id: &str,
    ) -> Vec<OutboundEvent> {
        if self
            .pending_continue_work
            .get(window_id)
            .is_some_and(|pending| pending.operation_id == operation_id)
        {
            return self.continue_work_launch_failed_events(
                window_id,
                "authenticated SessionStart readiness timed out",
            );
        }
        let feedback = self
            .pending_fresh_execution_launches
            .get(window_id)
            .filter(|pending| pending.operation_id == operation_id)
            .and_then(|pending| pending.launch_feedback_context.clone());
        if feedback.is_some()
            || self
                .pending_fresh_execution_launches
                .get(window_id)
                .is_some_and(|pending| pending.operation_id == operation_id)
        {
            return self.launch_error_events_with_continue_work(
                window_id.to_string(),
                "authenticated SessionStart readiness timed out".to_string(),
                feedback,
            );
        }
        Vec::new()
    }

    pub(crate) fn finalize_fresh_execution_launch_session_start(
        &mut self,
        window_id: &str,
        readiness_nonce: Option<&str>,
    ) -> Vec<OutboundEvent> {
        let Some(pending) = self
            .pending_fresh_execution_launches
            .get(window_id)
            .cloned()
        else {
            return Vec::new();
        };
        let fail = |runtime: &mut Self, detail: &str| {
            runtime.launch_error_events_with_continue_work(
                window_id.to_string(),
                detail.to_string(),
                pending.launch_feedback_context.clone(),
            )
        };
        if readiness_nonce != Some(pending.readiness_nonce.as_str()) {
            return fail(
                self,
                "the authenticated SessionStart readiness nonce did not match",
            );
        }
        let Some(active_session) = self.active_agent_sessions.get(window_id).cloned() else {
            return fail(self, "the launched pane has no active Session");
        };
        if active_session.session_id != pending.binding.session_id
            || !path_matches(&active_session.worktree_path, &pending.worktree_path)
        {
            return fail(
                self,
                "the launched Session does not match its Prepared execution binding",
            );
        }
        let Some(token) = self.agent_capability_tokens.get(window_id).cloned() else {
            return fail(self, "the Prepared Host capability is missing");
        };
        let Some(issuer) = self.agent_capability_issuer.clone() else {
            return fail(self, "the Host capability issuer is unavailable");
        };
        if !issuer.prepared_token_is_current(&token, &pending.binding) {
            return fail(self, "the Prepared Host capability is no longer current");
        }
        let probe = gwt::probe_authenticated_prepared_execution_binding(
            &pending.project_root,
            &pending.binding.session_id,
            &pending.binding,
            "fresh-linked-owner-launch-coordinator",
            gwt::AgentExecutionBindingProbeRequest {
                schema_version: gwt::AGENT_EXECUTION_BINDING_PROBE_SCHEMA_VERSION,
                operation_id: pending.operation_id.clone(),
                nonce: uuid::Uuid::new_v4().to_string(),
            },
        );
        if probe.as_ref().is_err()
            || probe
                .as_ref()
                .is_ok_and(|receipt| receipt.execution_binding != pending.binding.identity)
        {
            return fail(
                self,
                "the Host could not prove the exact Prepared execution binding",
            );
        }
        if issuer.promote_prepared(&token, &pending.binding).is_err()
            || !issuer.active_token_is_current(&token, &pending.binding)
        {
            return fail(self, "the Prepared Host capability could not be promoted");
        }

        invoke_fresh_execution_pre_work_commit_hook();
        let already_activated = pending_fresh_execution_activation_status(&pending) == Some(true);
        let transaction_committed = if already_activated {
            resolve_activated_fresh_execution_commit(
                &pending.project_root,
                &pending.worktree_path,
                pending.owner,
                &pending.operation_id,
                &pending.request,
                &self.sessions_dir,
                &pending.session_identity,
            )
        } else {
            let live_session_ids: HashSet<String> = self
                .active_agent_sessions
                .values()
                .map(|session| session.session_id.clone())
                .collect();
            gwt::cli::execution_state::with_prepared_successor_exact_session_activation(
                &pending.worktree_path,
                pending.owner,
                &pending.request,
                &self.sessions_dir,
                &pending.session_identity,
                |activate| {
                    gwt_core::workspace_projection::transact_workspace_state_at_with_commit(
                        &gwt_core::paths::gwt_workspace_projection_path_for_repo_path(
                            &pending.project_root,
                        ),
                        &gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(
                            &pending.worktree_path,
                        ),
                        &gwt_core::paths::gwt_repo_local_work_events_path(&pending.worktree_path),
                        &pending.project_root,
                        &pending.operation_id,
                        |projection, _work_items, _| {
                            let now = chrono::Utc::now();
                            let events = if let Some(context) = pending.resume_context.as_ref() {
                                let event = apply_workspace_launch_transition(
                                    projection,
                                    &active_session,
                                    WorkspaceLaunchTransition {
                                        work_id: gwt_core::workspace_projection::canonical_work_id(
                                            &pending.project_root,
                                            Some(active_session.branch_name.as_str()),
                                            Some(active_session.worktree_path.as_path()),
                                        ),
                                        base_branch: pending.base_branch.as_deref(),
                                        linked_issue_number: pending.linked_issue_number,
                                        resume_context: Some(context),
                                        kind: if pending.base_branch.is_some() {
                                            WorkspaceLaunchProjectionKind::StartWork
                                        } else {
                                            WorkspaceLaunchProjectionKind::Resume {
                                                created_by_start_work: active_session
                                                    .branch_name
                                                    .starts_with("work/"),
                                            }
                                        },
                                        live_session_ids: &live_session_ids,
                                        now,
                                    },
                                );
                                vec![event]
                            } else {
                                projection.retain_live_agents_keep_shells(
                                    live_session_ids.iter().map(String::as_str),
                                    now,
                                );
                                let mut summary =
                                    active_agent_summary_from_session(&active_session, now);
                                summary.affiliation_status = gwt_core::workspace_projection::
                                    WorkspaceAgentAffiliationStatus::Unassigned;
                                summary.workspace_id = None;
                                projection.register_unassigned_agent(summary);
                                projection.updated_at = now;
                                Vec::new()
                            };
                            Ok(((), events))
                        },
                        || {
                            activate()
                                .map(|_| ())
                                .map_err(gwt_core::error::GwtError::Io)
                        },
                    )
                    .map_err(|error| std::io::Error::other(error.to_string()))?;
                    if fresh_execution_commit_readback_matches(
                        &pending.worktree_path,
                        pending.owner,
                        &pending.session_identity,
                    ) {
                        Ok(())
                    } else {
                        Err(std::io::Error::other(
                            "fresh launch commit readback does not match exact authority",
                        ))
                    }
                },
            )
            .map(|result| result.is_some())
        };
        if !matches!(transaction_committed, Ok(true)) {
            if pending_fresh_execution_activation_status(&pending) != Some(true) {
                return fail(
                    self,
                    "the fresh generation activation transaction was rejected",
                );
            }
            // A response-loss retry owns any partial generation+Work repair.
            // It must reacquire the exact active Session lease and must never
            // publish Work from this unclassified error path.
            return Vec::new();
        }

        if !issuer.active_token_is_current(&token, &pending.binding) {
            return Vec::new();
        }
        let active_probe = gwt::probe_authenticated_execution_binding(
            &pending.project_root,
            &pending.binding.session_id,
            &pending.binding,
            "fresh-linked-owner-launch-coordinator",
            gwt::AgentExecutionBindingProbeRequest {
                schema_version: gwt::AGENT_EXECUTION_BINDING_PROBE_SCHEMA_VERSION,
                operation_id: pending.operation_id.clone(),
                nonce: uuid::Uuid::new_v4().to_string(),
            },
        );
        if active_probe.as_ref().is_err()
            || active_probe
                .as_ref()
                .is_ok_and(|receipt| receipt.execution_binding != pending.binding.identity)
            || gwt::cli::execution_state::current_execution_binding(
                &pending.worktree_path,
                pending.owner,
            )
            .ok()
            .flatten()
                == Some(pending.predecessor_binding.clone())
        {
            return Vec::new();
        }

        self.completed_fresh_execution_launch_events(window_id, &pending)
    }

    pub(crate) fn finalize_continue_work_session_start(
        &mut self,
        window_id: &str,
        readiness_nonce: Option<&str>,
    ) -> Vec<OutboundEvent> {
        let Some(pending) = self.pending_continue_work.get(window_id).cloned() else {
            return Vec::new();
        };
        if readiness_nonce != Some(pending.readiness_nonce.as_str()) {
            return self.continue_work_launch_failed_events(
                window_id,
                "the authenticated SessionStart readiness nonce did not match",
            );
        }
        let Some(active_session) = self.active_agent_sessions.get(window_id).cloned() else {
            return self.continue_work_launch_failed_events(
                window_id,
                "the launched pane has no active Session",
            );
        };
        if active_session.session_id != pending.binding.session_id
            || !path_matches(&active_session.worktree_path, &pending.worktree_path)
        {
            return self.continue_work_launch_failed_events(
                window_id,
                "the launched Session does not match its Prepared execution binding",
            );
        }
        let Some(token) = self.agent_capability_tokens.get(window_id).cloned() else {
            return self.continue_work_launch_failed_events(
                window_id,
                "the Prepared Host capability is missing",
            );
        };
        let Some(issuer) = self.agent_capability_issuer.clone() else {
            return self.continue_work_launch_failed_events(
                window_id,
                "the Host capability issuer is unavailable",
            );
        };
        if !issuer.prepared_token_is_current(&token, &pending.binding) {
            return self.continue_work_launch_failed_events(
                window_id,
                "the Prepared Host capability is no longer current",
            );
        }
        let probe = gwt::probe_authenticated_prepared_execution_binding(
            &pending.project_root,
            &pending.binding.session_id,
            &pending.binding,
            "continue-work-coordinator",
            gwt::AgentExecutionBindingProbeRequest {
                schema_version: gwt::AGENT_EXECUTION_BINDING_PROBE_SCHEMA_VERSION,
                operation_id: pending.operation_id.clone(),
                nonce: uuid::Uuid::new_v4().to_string(),
            },
        );
        if probe.as_ref().is_err()
            || probe
                .as_ref()
                .is_ok_and(|receipt| receipt.execution_binding != pending.binding.identity)
        {
            return self.continue_work_launch_failed_events(
                window_id,
                "the Host could not prove the exact Prepared execution binding",
            );
        }
        // Finish every fallible process-local authority transition before the
        // durable generation+Work commit point. If the CAS below fails, the
        // normal pre-activation cleanup revokes this bearer; after the CAS,
        // no capability promotion failure can strand a partial generation.
        if issuer.promote_prepared(&token, &pending.binding).is_err()
            || !issuer.active_token_is_current(&token, &pending.binding)
        {
            return self.continue_work_launch_failed_events(
                window_id,
                "the Prepared Host capability could not be promoted",
            );
        }

        let exact_session_identity = match pending_continue_work_session_identity(&pending) {
            Ok(identity) => identity,
            Err(_) => {
                return self.continue_work_launch_failed_events(
                    window_id,
                    "the Prepared continuation Session identity is not canonical",
                )
            }
        };
        let already_activated = pending_execution_is_activated(&pending);
        let transaction_result: Result<(), gwt_core::error::GwtError> = if already_activated {
            match gwt::cli::execution_state::with_current_active_session_execution_identity_lease(
                &self.sessions_dir,
                &exact_session_identity,
                || {
                    resolve_split_workspace_state_external_commit(
                        &pending.project_root,
                        &pending.worktree_path,
                        &pending.operation_id,
                        gwt_core::workspace_projection::ExternalWorkspaceCommitDecision::Commit,
                    )
                    .and_then(|resolution| {
                        if resolution
                            == gwt_core::workspace_projection::ExternalWorkspaceCommitResolution::Committed
                            && continue_work_commit_readback_matches(&pending)
                        {
                            Ok(())
                        } else {
                            Err(gwt_core::error::GwtError::Other(format!(
                                "unexpected continuation Work resolution or readback: {resolution:?}"
                            )))
                        }
                    })
                },
            ) {
                Ok(Some(result)) => result,
                Ok(None) => Err(gwt_core::error::GwtError::Other(
                    "the active continuation Session changed before Work commit".to_string(),
                )),
                Err(error) => Err(gwt_core::error::GwtError::Io(error)),
            }
        } else {
            let live_session_ids: HashSet<String> = self
                .active_agent_sessions
                .values()
                .map(|session| session.session_id.clone())
                .collect();
            let leased = match &pending.execution {
                PendingContinueWorkExecution::Successor(request) => {
                    gwt::cli::execution_state::with_prepared_successor_exact_session_activation(
                        &pending.worktree_path,
                        pending.owner,
                        request,
                        &self.sessions_dir,
                        &exact_session_identity,
                        |activate| {
                            let mut activate_unit = || activate().map(|_| ());
                            transact_pending_continue_work_with_activation(
                                &pending,
                                &active_session,
                                &live_session_ids,
                                &mut activate_unit,
                            )
                        },
                    )
                }
                PendingContinueWorkExecution::Takeover(request) => {
                    gwt::cli::execution_state::with_prepared_generation_takeover_exact_session_activation(
                        &pending.worktree_path,
                        pending.owner,
                        request,
                        &self.sessions_dir,
                        &exact_session_identity,
                        |activate| {
                            let mut activate_unit = || activate().map(|_| ());
                            transact_pending_continue_work_with_activation(
                                &pending,
                                &active_session,
                                &live_session_ids,
                                &mut activate_unit,
                            )
                        },
                    )
                }
            };
            match leased {
                Ok(Some(())) => Ok(()),
                Ok(None) => Err(gwt_core::error::GwtError::Other(
                    "the Prepared continuation Session changed before activation".to_string(),
                )),
                Err(error) => Err(gwt_core::error::GwtError::Io(error)),
            }
        };
        if transaction_result.is_err() {
            let activated = pending_execution_is_activated(&pending);
            if !activated {
                return self.continue_work_launch_failed_events(
                    window_id,
                    "the generation activation transaction was rejected",
                );
            }
            // The generation may have committed before a Work publication
            // error, but recovery must repair that partial commit under the
            // exact active Session lease. Never resolve it from this
            // unclassified error path after a lease mismatch.
            return Vec::new();
        }

        if gwt::cli::execution_state::current_execution_binding(
            &pending.worktree_path,
            pending.owner,
        )
        .ok()
        .flatten()
            != Some(pending.binding.identity.clone())
        {
            return Vec::new();
        }
        if !issuer.active_token_is_current(&token, &pending.binding) {
            return Vec::new();
        }
        let active_probe = gwt::probe_authenticated_execution_binding(
            &pending.project_root,
            &pending.binding.session_id,
            &pending.binding,
            "continue-work-coordinator",
            gwt::AgentExecutionBindingProbeRequest {
                schema_version: gwt::AGENT_EXECUTION_BINDING_PROBE_SCHEMA_VERSION,
                operation_id: pending.operation_id.clone(),
                nonce: uuid::Uuid::new_v4().to_string(),
            },
        );
        if active_probe.as_ref().is_err()
            || active_probe
                .as_ref()
                .is_ok_and(|receipt| receipt.execution_binding != pending.binding.identity)
        {
            return Vec::new();
        }
        if gwt::cli::execution_state::current_active_execution_binding_matches(
            &pending.worktree_path,
            pending.owner,
            &pending.predecessor_session_id,
            &pending.predecessor_binding,
        )
        .unwrap_or(true)
        {
            return Vec::new();
        }
        let work_readback =
            gwt_core::workspace_projection::load_workspace_work_items(&pending.worktree_path)
                .ok()
                .flatten()
                .is_some_and(|projection| {
                    projection.work_items.iter().any(|item| {
                        item.id == pending.work_id
                            && item.status_category
                                == gwt_core::workspace_projection::WorkspaceStatusCategory::Active
                            && item
                                .agents
                                .iter()
                                .any(|agent| agent.session_id == pending.binding.session_id)
                    })
                });
        if !work_readback {
            return Vec::new();
        }

        self.pending_continue_work.remove(window_id);
        self.continue_work_outcomes.insert(
            pending.operation_id.clone(),
            CachedContinueWorkOutcome {
                work_id: pending.work_id.clone(),
                outcome: pending.outcome,
                message: None,
                error_code: None,
                retryable: false,
            },
        );
        let mut events =
            self.continue_work_pending_outcome_events(&pending, pending.outcome, None, None, false);
        events.push(self.workspace_state_broadcast());
        if let Some(projection) = self.active_work_projection_broadcast_for_active_tab() {
            events.push(projection);
        }
        events
    }
}
