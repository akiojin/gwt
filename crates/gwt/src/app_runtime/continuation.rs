use std::collections::HashSet;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use super::workspace::{
    active_agent_summary_from_session, apply_workspace_launch_transition,
    WorkspaceLaunchProjectionKind, WorkspaceLaunchTransition,
};
use super::{
    launch_config_from_persisted_session, non_empty_workspace_text,
    workspace_resume_owner_issue_number, AppRuntime, BackendEvent, CachedContinueWorkOutcome,
    OutboundEvent, PendingContinueWork, PendingContinueWorkExecution, PendingFreshExecutionLaunch,
    WindowGeometry, WindowProcessStatus, WorkspaceResumeContext,
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
    source_session: gwt_agent::Session,
    owner: gwt::cli::execution_state::ExecutionOwnerKey,
    resume_context: WorkspaceResumeContext,
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

    fn candidate_binding_matches(&self, binding: &gwt_agent::SessionExecutionBinding) -> bool {
        if binding.session_id != self.candidate_session_id() {
            return false;
        }
        match self {
            Self::Successor(attempt) => {
                binding.identity.generation_id == attempt.candidate_generation_id
                    && binding.identity.binding_id == attempt.request.session_binding_id
            }
            Self::Takeover(attempt) => binding.identity.generation_id == attempt.generation_id,
        }
    }

    fn abort(
        &self,
        worktree: &Path,
        owner: gwt::cli::execution_state::ExecutionOwnerKey,
        reason: &str,
    ) -> std::io::Result<()> {
        match self {
            Self::Successor(attempt) => gwt::cli::execution_state::abort_successor(
                worktree,
                owner,
                &attempt.request,
                reason,
            )
            .map(|_| ()),
            Self::Takeover(attempt) => gwt::cli::execution_state::abort_generation_takeover(
                worktree,
                owner,
                &attempt.request,
                reason,
            )
            .map(|_| ()),
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
        Ok(None) => ProviderConversationAvailability::Present,
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
    let project_root = session
        .project_state_root
        .clone()
        .filter(|root| !root.as_os_str().is_empty())
        .ok_or_else(|| {
            "persisted fresh-launch Session has no canonical Project State root".to_string()
        })?;
    Ok(Some(DurableFreshExecutionCandidate {
        project_root,
        worktree_path: session.worktree_path.clone(),
        owner,
        attempt,
        binding,
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

impl AppRuntime {
    pub(super) fn reconcile_durable_fresh_execution_launches(&mut self) {
        let Ok(entries) = std::fs::read_dir(&self.sessions_dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("toml") {
                continue;
            }
            let session = match gwt_agent::Session::load(&path) {
                Ok(session) => session,
                Err(error) => {
                    tracing::warn!(
                        path = %path.display(),
                        error = %error,
                        "failed to inspect a Session during fresh-launch recovery"
                    );
                    continue;
                }
            };
            let candidate = match durable_fresh_execution_candidate(&session) {
                Ok(Some(candidate)) => candidate,
                Ok(None) => continue,
                Err(error) => {
                    tracing::warn!(
                        session_id = %session.id,
                        error = %error,
                        "retained ambiguous fresh-launch recovery evidence"
                    );
                    continue;
                }
            };

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
                    if let Err(error) = gwt::cli::execution_state::activate_successor(
                        &candidate.worktree_path,
                        candidate.owner,
                        &candidate.attempt.request,
                    ) {
                        tracing::warn!(
                            session_id = %candidate.binding.session_id,
                            error = %error,
                            "Activated fresh-launch projection repair remains pending"
                        );
                        continue;
                    }
                    let exact_current = gwt::cli::execution_state::current_execution_binding(
                        &candidate.worktree_path,
                        candidate.owner,
                    )
                    .is_ok_and(|current| current == Some(candidate.binding.identity.clone()));
                    if !exact_current {
                        tracing::warn!(
                            session_id = %candidate.binding.session_id,
                            "Activated fresh-launch repair did not recover the exact Session binding"
                        );
                        continue;
                    }
                    match gwt_core::workspace_projection::resolve_workspace_state_external_commit(
                        &candidate.project_root,
                        &candidate.attempt.request.operation_id,
                        gwt_core::workspace_projection::ExternalWorkspaceCommitDecision::Commit,
                    ) {
                        Ok(
                            gwt_core::workspace_projection::ExternalWorkspaceCommitResolution::Committed,
                        ) => {
                            tracing::info!(
                                session_id = %candidate.binding.session_id,
                                operation_id = %candidate.attempt.request.operation_id,
                                "recovered an Activated fresh launch after Host restart"
                            );
                        }
                        Ok(resolution) => {
                            tracing::warn!(
                                session_id = %candidate.binding.session_id,
                                ?resolution,
                                "Activated fresh-launch Workspace commit remains pending"
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
                    if let Err(error) = gwt::cli::execution_state::abort_successor(
                        &candidate.worktree_path,
                        candidate.owner,
                        &candidate.attempt.request,
                        "Host restarted before fresh launch activation",
                    ) {
                        tracing::warn!(
                            session_id = %candidate.binding.session_id,
                            error = %error,
                            "Prepared fresh-launch abort remains pending"
                        );
                        continue;
                    }
                    self.finish_durable_aborted_fresh_execution_cleanup(&candidate);
                }
                gwt::cli::execution_state::ContinuationAttemptStatus::Aborted => {
                    self.finish_durable_aborted_fresh_execution_cleanup(&candidate);
                }
            }
        }
    }

    fn finish_durable_aborted_fresh_execution_cleanup(
        &mut self,
        candidate: &DurableFreshExecutionCandidate,
    ) {
        let workspace_rejected = matches!(
            gwt_core::workspace_projection::resolve_workspace_state_external_commit(
                &candidate.project_root,
                &candidate.attempt.request.operation_id,
                gwt_core::workspace_projection::ExternalWorkspaceCommitDecision::Reject,
            ),
            Ok(
                gwt_core::workspace_projection::ExternalWorkspaceCommitResolution::Rejected
                    | gwt_core::workspace_projection::ExternalWorkspaceCommitResolution::Missing
            )
        );
        if !workspace_rejected {
            tracing::warn!(
                session_id = %candidate.binding.session_id,
                "Aborted fresh-launch cleanup retained its Session until Workspace rejection succeeds"
            );
            return;
        }
        match gwt_agent::remove_session_if_execution_binding_matches(
            &self.sessions_dir,
            &candidate.binding.session_id,
            &candidate.binding,
        ) {
            Ok(true) => tracing::info!(
                session_id = %candidate.binding.session_id,
                "completed durable Aborted fresh-launch Session cleanup"
            ),
            Ok(false) => tracing::warn!(
                session_id = %candidate.binding.session_id,
                "retained Aborted fresh-launch cleanup evidence after binding mismatch"
            ),
            Err(error) => tracing::warn!(
                session_id = %candidate.binding.session_id,
                error = %error,
                "Aborted fresh-launch Session cleanup remains retryable"
            ),
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
        {
            return None;
        }
        let token = self.agent_capability_tokens.get(window_id)?;
        let issuer = self.agent_capability_issuer.as_ref()?;
        if !issuer.active_token_is_current(token, &pending.binding) {
            return None;
        }
        if gwt::cli::execution_state::current_execution_binding(
            &pending.worktree_path,
            pending.owner,
        )
        .ok()
        .flatten()
            != Some(pending.binding.identity.clone())
        {
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
        let work_is_active =
            gwt_core::workspace_projection::load_workspace_work_items(&pending.project_root)
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
        if !work_is_active {
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
        let binding = match durable.execution_binding.clone() {
            Some(binding) if binding.identity == *current => binding,
            Some(_) => return false,
            None => {
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
                    || durable.save(&self.sessions_dir).is_err()
                {
                    return false;
                }
                binding
            }
        };
        if !issuer.active_token_is_current(&token, &binding)
            && issuer.promote_inspection(&token, &binding).is_err()
        {
            return false;
        }
        issuer.active_token_is_current(&token, &binding)
            && gwt::cli::execution_state::current_active_execution_binding_matches(
                &active.worktree_path,
                owner,
                &active.session_id,
                &binding.identity,
            )
            .unwrap_or(false)
    }

    pub(super) fn classify_nonlocal_active_owner_liveness(
        &self,
        session_id: &str,
    ) -> ActiveOwnerLiveness {
        let durable_path = self.sessions_dir.join(format!("{session_id}.toml"));
        let durable = match gwt_agent::Session::load_and_migrate(&durable_path) {
            Ok(session) => Some(session),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => return ActiveOwnerLiveness::Unknown,
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
        let project_root = tab.project_root.clone();
        let works =
            gwt_core::workspace_projection::load_or_synthesize_workspace_work_items(&project_root)
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

        let source_session = item
            .agents
            .iter()
            .filter_map(|agent| {
                let path = self.sessions_dir.join(format!("{}.toml", agent.session_id));
                gwt_agent::Session::load_and_migrate(&path)
                    .ok()
                    .map(|session| (agent.updated_at, session))
            })
            .filter(|(_, session)| session_matches_project_state(session, &project_root))
            .max_by_key(|(updated_at, _)| *updated_at)
            .map(|(_, session)| session)
            .ok_or_else(|| {
                ContinueWorkFailure::failed(
                    "session_metadata_missing",
                    "No durable Session metadata is available for this Work.",
                    false,
                )
            })?;
        let worktree_path = item
            .execution_containers
            .iter()
            .rev()
            .filter_map(|container| container.worktree_path.as_ref())
            .find(|path| path_matches(path, &source_session.worktree_path))
            .cloned()
            .unwrap_or_else(|| source_session.worktree_path.clone());
        let worktree_path = dunce::canonicalize(&worktree_path).map_err(|_| {
            ContinueWorkFailure::failed(
                "worktree_unavailable",
                "This Work cannot be continued because its worktree is unavailable.",
                false,
            )
        })?;
        if !path_matches(&worktree_path, &source_session.worktree_path) {
            return Err(ContinueWorkFailure::conflict(
                "The Work container and durable Session no longer agree.",
            ));
        }
        let owner_number = source_session
            .linked_issue_number
            .or_else(|| workspace_resume_owner_issue_number(item.owner.as_deref()))
            .ok_or_else(|| {
                ContinueWorkFailure::failed(
                    "execution_owner_missing",
                    "This Work has no linked Issue or SPEC execution owner.",
                    false,
                )
            })?;
        let owner = gwt::cli::execution_state::ExecutionOwnerKey {
            kind: gwt::cli::execution_state::detect_owner_kind(&project_root, owner_number),
            number: owner_number,
        };
        let resume_context = WorkspaceResumeContext {
            title: non_empty_workspace_text(Some(&item.title)),
            owner: non_empty_workspace_text(item.owner.as_deref())
                .or_else(|| Some(format!("Issue #{owner_number}"))),
            summary: non_empty_workspace_text(item.summary.as_deref())
                .or_else(|| non_empty_workspace_text(item.intent.as_deref())),
            next_action: item.latest_next_action().map(str::to_string),
        };
        Ok(ContinueWorkTarget {
            tab_id,
            project_root,
            worktree_path,
            source_session,
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
        let project_root = tab.project_root.clone();
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
            if !session_matches_project_state(&source_session, &project_root) {
                continue;
            }
            let Some(owner_number) = source_session.linked_issue_number else {
                continue;
            };
            let Ok(worktree_path) = dunce::canonicalize(&source_session.worktree_path) else {
                continue;
            };
            let owner = gwt::cli::execution_state::ExecutionOwnerKey {
                kind: gwt::cli::execution_state::detect_owner_kind(&project_root, owner_number),
                number: owner_number,
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
            let source_matches = match (&successor, &takeover) {
                (Some(attempt), None) => {
                    attempt.predecessor.initial_session_id == source_session.id
                }
                (None, Some(attempt)) => attempt.request.from_session_id == source_session.id,
                (None, None) => false,
                (Some(_), Some(_)) => {
                    return Err(ContinueWorkFailure::conflict(
                        "The operation id is bound to conflicting durable continuation attempts.",
                    ));
                }
            };
            if !source_matches {
                continue;
            }
            matches.push(ContinueWorkTarget {
                tab_id: tab_id.clone(),
                project_root: project_root.clone(),
                worktree_path,
                source_session,
                owner,
                resume_context: WorkspaceResumeContext {
                    title: None,
                    owner: Some(format!("Issue #{owner_number}")),
                    summary: None,
                    next_action: None,
                },
            });
        }
        match matches.len() {
            0 => Ok(None),
            1 => Ok(matches.pop()),
            _ => Err(ContinueWorkFailure::conflict(
                "The durable continuation operation resolves to multiple Sessions.",
            )),
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

    fn reconcile_aborted_continue_work_attempt(
        &mut self,
        client_id: &str,
        operation_id: String,
        work_id: String,
        target: &ContinueWorkTarget,
        attempt: &DurableContinueWorkAttempt,
    ) -> Vec<OutboundEvent> {
        match gwt_core::workspace_projection::resolve_workspace_state_external_commit(
            &target.project_root,
            &operation_id,
            gwt_core::workspace_projection::ExternalWorkspaceCommitDecision::Reject,
        ) {
            Ok(
                gwt_core::workspace_projection::ExternalWorkspaceCommitResolution::Rejected
                | gwt_core::workspace_projection::ExternalWorkspaceCommitResolution::Missing,
            ) => {}
            Ok(gwt_core::workspace_projection::ExternalWorkspaceCommitResolution::Busy) => {
                return self.continue_work_uncached_failure_events(
                    client_id,
                    operation_id,
                    work_id,
                    ContinueWorkFailure::failed(
                        "continuation_reconciliation_required",
                        "The aborted continuation Work transaction is still being reconciled.",
                        true,
                    ),
                );
            }
            Ok(gwt_core::workspace_projection::ExternalWorkspaceCommitResolution::Committed) => {
                return self.continue_work_uncached_failure_events(
                    client_id,
                    operation_id,
                    work_id,
                    ContinueWorkFailure::conflict(
                        "The aborted continuation has a committed Work transaction.",
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
                            "The aborted continuation Work transaction could not be rejected: {error}"
                        ),
                        true,
                    ),
                );
            }
        }

        let candidate_session_id = attempt.candidate_session_id();
        let candidate_path = self
            .sessions_dir
            .join(format!("{candidate_session_id}.toml"));
        match gwt_agent::Session::load_and_migrate(&candidate_path) {
            Ok(session) => {
                let Some(binding) = session.execution_binding.as_ref() else {
                    return self.continue_work_uncached_failure_events(
                        client_id,
                        operation_id,
                        work_id,
                        ContinueWorkFailure::conflict(
                            "The aborted candidate Session lost its execution binding.",
                        ),
                    );
                };
                if !session_matches_project_state(&session, &target.project_root)
                    || binding.owner_kind != target.owner.kind.as_str()
                    || binding.owner_number != target.owner.number
                    || !attempt.candidate_binding_matches(binding)
                {
                    return self.continue_work_uncached_failure_events(
                        client_id,
                        operation_id,
                        work_id,
                        ContinueWorkFailure::conflict(
                            "The aborted candidate Session no longer matches its durable attempt.",
                        ),
                    );
                }
                if let Err(error) = gwt_agent::remove_session_if_execution_binding_matches(
                    &self.sessions_dir,
                    candidate_session_id,
                    binding,
                ) {
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
                    );
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
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

                if let Err(abort_error) = attempt.abort(
                    &target.worktree_path,
                    target.owner,
                    "owning Host crashed before authenticated Ready",
                ) {
                    return match self.durable_continue_work_attempt(target, &operation_id) {
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
                        Ok(Some(latest))
                            if latest.status() == DurableContinueWorkAttemptStatus::Aborted =>
                        {
                            self.reconcile_aborted_continue_work_attempt(
                                client_id,
                                operation_id,
                                work_id,
                                target,
                                &latest,
                            )
                        }
                        Ok(Some(_)) => self.continue_work_uncached_failure_events(
                            client_id,
                            operation_id,
                            work_id,
                            ContinueWorkFailure::failed(
                                "continuation_reconciliation_required",
                                format!(
                                    "The stale Prepared continuation could not be aborted yet: {abort_error}"
                                ),
                                true,
                            ),
                        ),
                        Ok(None) | Err(_) => self.continue_work_uncached_failure_events(
                            client_id,
                            operation_id,
                            work_id,
                            ContinueWorkFailure::conflict(
                                "The stale Prepared continuation changed while it was being aborted.",
                            ),
                        ),
                    };
                }
                let latest = match self.durable_continue_work_attempt(target, &operation_id) {
                    Ok(Some(latest))
                        if latest.status() == DurableContinueWorkAttemptStatus::Aborted =>
                    {
                        latest
                    }
                    _ => {
                        return self.continue_work_uncached_failure_events(
                            client_id,
                            operation_id,
                            work_id,
                            ContinueWorkFailure::failed(
                                "continuation_reconciliation_required",
                                "The continuation abort could not be read back safely.",
                                true,
                            ),
                        )
                    }
                };
                return self.reconcile_aborted_continue_work_attempt(
                    client_id,
                    operation_id,
                    work_id,
                    target,
                    &latest,
                );
            }
            DurableContinueWorkAttemptStatus::Activated => {}
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
        match gwt_core::workspace_projection::resolve_workspace_state_external_commit(
            &target.project_root,
            &operation_id,
            gwt_core::workspace_projection::ExternalWorkspaceCommitDecision::Commit,
        ) {
            Ok(gwt_core::workspace_projection::ExternalWorkspaceCommitResolution::Committed) => {}
            Ok(gwt_core::workspace_projection::ExternalWorkspaceCommitResolution::Busy) => {
                return self.continue_work_uncached_failure_events(
                    client_id,
                    operation_id,
                    work_id,
                    ContinueWorkFailure::failed(
                        "continuation_reconciliation_required",
                        "The committed continuation Work transaction is still being reconciled.",
                        true,
                    ),
                );
            }
            Ok(resolution) => {
                return self.continue_work_uncached_failure_events(
                    client_id,
                    operation_id,
                    work_id,
                    ContinueWorkFailure::conflict(format!(
                        "The committed continuation has no matching Work transaction: {resolution:?}"
                    )),
                );
            }
            Err(error) => {
                return self.continue_work_uncached_failure_events(
                    client_id,
                    operation_id,
                    work_id,
                    ContinueWorkFailure::conflict(format!(
                        "The committed continuation Work transaction could not be repaired: {error}"
                    )),
                );
            }
        }

        let candidate_session_id = attempt.candidate_session_id().to_string();
        let current = match gwt::cli::execution_state::current_execution_binding(
            &target.worktree_path,
            target.owner,
        ) {
            Ok(Some(binding)) if attempt.activated_identity_matches(&binding) => binding,
            _ => {
                return self.continue_work_uncached_failure_events(
                    client_id,
                    operation_id,
                    work_id,
                    ContinueWorkFailure::conflict(
                        "The committed continuation is no longer the current execution binding.",
                    ),
                );
            }
        };
        let durable = match gwt_agent::Session::load_and_migrate(
            &self
                .sessions_dir
                .join(format!("{candidate_session_id}.toml")),
        ) {
            Ok(session)
                if session_matches_project_state(&session, &target.project_root)
                    && session.execution_binding.as_ref().is_some_and(|binding| {
                        binding.session_id == candidate_session_id
                            && binding.owner_kind == target.owner.kind.as_str()
                            && binding.owner_number == target.owner.number
                            && binding.identity == current
                    }) =>
            {
                session
            }
            _ => {
                return self.continue_work_uncached_failure_events(
                    client_id,
                    operation_id,
                    work_id,
                    ContinueWorkFailure::conflict(
                        "The committed continuation Session binding could not be read back.",
                    ),
                );
            }
        };
        let projection_matches = gwt::cli::execution_state::load(&target.worktree_path)
            .ok()
            .flatten()
            .is_some_and(|record| {
                record.owner_kind == target.owner.kind
                    && record.owner_number == target.owner.number
                    && record.status == gwt::cli::execution_state::ExecutionControlStatus::Active
                    && record.primary_session_id == candidate_session_id
            });
        let work_matches =
            gwt_core::workspace_projection::load_workspace_work_items(&target.project_root)
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
                        && matching[0]
                            .agents
                            .iter()
                            .any(|agent| agent.session_id == candidate_session_id)
                });
        if !projection_matches || !work_matches {
            return self.continue_work_uncached_failure_events(
                client_id,
                operation_id,
                work_id,
                ContinueWorkFailure::conflict(
                    "The committed continuation generation and Work projection do not agree.",
                ),
            );
        }
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
        let mut config = launch_config_from_persisted_session(&target.source_session);
        config.working_dir = Some(target.worktree_path.clone());
        config.branch = Some(target.source_session.branch.clone());
        config.linked_issue_number = Some(target.owner.number);
        let outcome = configure_provider_continuation(&mut config, &target.source_session);
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
        let execution = if let Some((from_session_id, stale_reason)) = takeover_owner {
            PendingContinueWorkExecution::Takeover(
                gwt::cli::execution_state::GenerationTakeoverRequest {
                    operation_id: operation_id.clone(),
                    principal_id: "gwt-host-continuation".to_string(),
                    work_id: Some(work_id.clone()),
                    source: Some(match outcome {
                        gwt::ContinueWorkOutcomeKind::ContinuedConversation => {
                            "continue-work:resume".to_string()
                        }
                        _ => "continue-work:handoff".to_string(),
                    }),
                    from_session_id,
                    to_session_id: continuation_session_id.clone(),
                    reason: format!("continue-work-stale-takeover: {stale_reason}"),
                    requested_at,
                },
            )
        } else {
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
            })
        };
        let prepared = match &execution {
            PendingContinueWorkExecution::Successor(request) => {
                gwt::cli::execution_state::prepare_successor(
                    &target.worktree_path,
                    target.owner,
                    request,
                )
                .map(|_| ())
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
        let repo_hash = target
            .source_session
            .repo_hash
            .clone()
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
        let pending = PendingContinueWork {
            client_id: client_id.to_string(),
            operation_id: operation_id.clone(),
            work_id: work_id.clone(),
            project_root: target.project_root.clone(),
            worktree_path: target.worktree_path.clone(),
            owner: target.owner,
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
        let abort_result = match &pending.execution {
            PendingContinueWorkExecution::Successor(request) => {
                gwt::cli::execution_state::abort_successor(
                    &pending.worktree_path,
                    pending.owner,
                    request,
                    "continuation launch failed before SessionStart",
                )
                .map(|_| ())
            }
            PendingContinueWorkExecution::Takeover(request) => {
                gwt::cli::execution_state::abort_generation_takeover(
                    &pending.worktree_path,
                    pending.owner,
                    request,
                    "continuation launch failed before SessionStart",
                )
                .map(|_| ())
            }
        };
        if abort_result.is_err() {
            let durable = match &pending.execution {
                PendingContinueWorkExecution::Successor(request) => {
                    gwt::cli::execution_state::continuation_attempt_for_operation(
                        &pending.worktree_path,
                        pending.owner,
                        &request.operation_id,
                    )
                    .map(|attempt| {
                        attempt
                            .map(|attempt| DurableContinueWorkAttempt::Successor(Box::new(attempt)))
                    })
                }
                PendingContinueWorkExecution::Takeover(request) => {
                    gwt::cli::execution_state::generation_takeover_attempt_for_operation(
                        &pending.worktree_path,
                        pending.owner,
                        &request.operation_id,
                    )
                    .map(|attempt| {
                        attempt
                            .map(|attempt| DurableContinueWorkAttempt::Takeover(Box::new(attempt)))
                    })
                }
            };
            if durable.as_ref().is_ok_and(|attempt| {
                attempt.as_ref().is_some_and(|attempt| {
                    attempt.status() == DurableContinueWorkAttemptStatus::Activated
                })
            }) {
                return self
                    .reconcile_activated_pending_continue_work(window_id, &pending)
                    .unwrap_or_default();
            }
            if !durable.as_ref().is_ok_and(|attempt| {
                attempt.as_ref().is_some_and(|attempt| {
                    attempt.status() == DurableContinueWorkAttemptStatus::Aborted
                })
            }) {
                self.stop_window_runtime_without_session_projection(window_id);
                let mut events = self.close_window_events(window_id);
                self.pending_continue_work.remove(window_id);
                events.extend(self.continue_work_pending_uncached_failure_events(
                    &pending,
                    ContinueWorkFailure::failed(
                        "continuation_reconciliation_required",
                        "The failed continuation could not prove a durable abort; retry reconciliation.",
                        true,
                    ),
                ));
                return events;
            }
        }

        self.stop_window_runtime_without_session_projection(window_id);
        self.stop_pending_continue_work_session_without_projection(window_id);
        let mut events = self.close_window_events(window_id);
        self.pending_continue_work.remove(window_id);
        match gwt_core::workspace_projection::resolve_workspace_state_external_commit(
            &pending.project_root,
            &pending.operation_id,
            gwt_core::workspace_projection::ExternalWorkspaceCommitDecision::Reject,
        ) {
            Ok(
                gwt_core::workspace_projection::ExternalWorkspaceCommitResolution::Rejected
                | gwt_core::workspace_projection::ExternalWorkspaceCommitResolution::Missing,
            ) => {}
            Ok(gwt_core::workspace_projection::ExternalWorkspaceCommitResolution::Busy)
            | Err(_) => {
                events.extend(self.continue_work_pending_uncached_failure_events(
                    &pending,
                    ContinueWorkFailure::failed(
                        "continuation_reconciliation_required",
                        "The failed continuation Work transaction still requires reconciliation.",
                        true,
                    ),
                ));
                return events;
            }
            Ok(gwt_core::workspace_projection::ExternalWorkspaceCommitResolution::Committed) => {
                events.extend(self.continue_work_pending_uncached_failure_events(
                    &pending,
                    ContinueWorkFailure::conflict(
                        "The aborted continuation has a committed Work transaction.",
                    ),
                ));
                return events;
            }
        }
        if let Err(error) = gwt_agent::remove_session_if_execution_binding_matches(
            &self.sessions_dir,
            &pending.binding.session_id,
            &pending.binding,
        ) {
            events.extend(self.continue_work_pending_uncached_failure_events(
                &pending,
                ContinueWorkFailure::failed(
                    "continuation_reconciliation_required",
                    format!("The aborted candidate Session could not be removed safely: {error}"),
                    true,
                ),
            ));
            return events;
        }
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
        if let Some(issue_number) = pending
            .launch_feedback_context
            .as_ref()
            .and_then(|context| context.issue_monitor_issue_number)
        {
            events.extend(self.issue_monitor_launch_succeeded_events(issue_number, window_id));
        }
        events
    }

    fn reconcile_activated_fresh_execution_launch_events(
        &mut self,
        window_id: &str,
        pending: &PendingFreshExecutionLaunch,
    ) -> Vec<OutboundEvent> {
        // The ledger commit is authoritative and pointer-last publication is
        // repairable. Re-running the exact Activated operation restores a
        // projection/pointer pair lost after the ledger write without
        // appending a second generation.
        if gwt::cli::execution_state::activate_successor(
            &pending.worktree_path,
            pending.owner,
            &pending.request,
        )
        .is_err()
        {
            return Vec::new();
        }
        if gwt_core::workspace_projection::resolve_workspace_state_external_commit(
            &pending.project_root,
            &pending.operation_id,
            gwt_core::workspace_projection::ExternalWorkspaceCommitDecision::Commit,
        )
        .ok()
            != Some(gwt_core::workspace_projection::ExternalWorkspaceCommitResolution::Committed)
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
        _detail: &str,
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

        if status == gwt::cli::execution_state::ContinuationAttemptStatus::Prepared {
            let _ = gwt::cli::execution_state::abort_successor(
                &pending.worktree_path,
                pending.owner,
                &pending.request,
                "fresh linked-owner launch failed before SessionStart",
            );
        }
        let Some(status) = pending_fresh_execution_attempt_status(&pending) else {
            return Vec::new();
        };
        if status == gwt::cli::execution_state::ContinuationAttemptStatus::Activated {
            return self.reconcile_activated_fresh_execution_launch_events(window_id, &pending);
        }

        let workspace_rejected = matches!(
            gwt_core::workspace_projection::resolve_workspace_state_external_commit(
                &pending.project_root,
                &pending.operation_id,
                gwt_core::workspace_projection::ExternalWorkspaceCommitDecision::Reject,
            ),
            Ok(
                gwt_core::workspace_projection::ExternalWorkspaceCommitResolution::Rejected
                    | gwt_core::workspace_projection::ExternalWorkspaceCommitResolution::Missing
            )
        );
        if !workspace_rejected {
            return Vec::new();
        }
        if status != gwt::cli::execution_state::ContinuationAttemptStatus::Aborted {
            // A callback refusal can leave the attempt durably Prepared. The
            // Work marker is safe to reject because the owner ledger proves
            // activation did not commit, but the candidate Session remains
            // reconciliation evidence until an Aborted event is durable.
            return Vec::new();
        }

        self.stop_window_runtime_without_session_projection(window_id);
        self.stop_pending_continue_work_session_without_projection(window_id);
        match gwt_agent::remove_session_if_execution_binding_matches(
            &self.sessions_dir,
            &pending.binding.session_id,
            &pending.binding,
        ) {
            Ok(true) => {
                let events = self.close_window_events(window_id);
                self.pending_fresh_execution_launches.remove(window_id);
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

        let already_activated = pending_fresh_execution_activation_status(&pending) == Some(true);
        let transaction_result = if already_activated {
            gwt::cli::execution_state::activate_successor(
                &pending.worktree_path,
                pending.owner,
                &pending.request,
            )
            .map_err(gwt_core::error::GwtError::Io)
            .and_then(|_| {
                gwt_core::workspace_projection::resolve_workspace_state_external_commit(
                    &pending.project_root,
                    &pending.operation_id,
                    gwt_core::workspace_projection::ExternalWorkspaceCommitDecision::Commit,
                )
            })
            .and_then(|resolution| {
                if resolution
                    == gwt_core::workspace_projection::ExternalWorkspaceCommitResolution::Committed
                {
                    Ok(())
                } else {
                    Err(gwt_core::error::GwtError::Other(format!(
                        "unexpected fresh launch Work resolution: {resolution:?}"
                    )))
                }
            })
        } else {
            let live_session_ids: HashSet<String> = self
                .active_agent_sessions
                .values()
                .map(|session| session.session_id.clone())
                .collect();
            gwt_core::workspace_projection::transact_workspace_state_with_commit(
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
                        let mut summary = active_agent_summary_from_session(&active_session, now);
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
                    gwt::cli::execution_state::activate_successor(
                        &pending.worktree_path,
                        pending.owner,
                        &pending.request,
                    )
                    .map(|_| ())
                    .map_err(gwt_core::error::GwtError::Io)
                },
            )
        };
        if transaction_result.is_err() {
            if pending_fresh_execution_activation_status(&pending) != Some(true) {
                return fail(
                    self,
                    "the fresh generation activation transaction was rejected",
                );
            }
            if gwt::cli::execution_state::activate_successor(
                &pending.worktree_path,
                pending.owner,
                &pending.request,
            )
            .is_err()
            {
                return Vec::new();
            }
            if gwt_core::workspace_projection::resolve_workspace_state_external_commit(
                &pending.project_root,
                &pending.operation_id,
                gwt_core::workspace_projection::ExternalWorkspaceCommitDecision::Commit,
            )
            .ok()
                != Some(
                    gwt_core::workspace_projection::ExternalWorkspaceCommitResolution::Committed,
                )
            {
                return Vec::new();
            }
        }

        if gwt::cli::execution_state::current_execution_binding(
            &pending.worktree_path,
            pending.owner,
        )
        .ok()
        .flatten()
            != Some(pending.binding.identity.clone())
            || !issuer.active_token_is_current(&token, &pending.binding)
        {
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

        let already_activated = pending_execution_is_activated(&pending);
        let transaction_result = if already_activated {
            gwt_core::workspace_projection::resolve_workspace_state_external_commit(
                &pending.project_root,
                &pending.operation_id,
                gwt_core::workspace_projection::ExternalWorkspaceCommitDecision::Commit,
            )
            .and_then(|resolution| {
                if resolution
                    == gwt_core::workspace_projection::ExternalWorkspaceCommitResolution::Committed
                {
                    Ok(())
                } else {
                    Err(gwt_core::error::GwtError::Other(format!(
                        "unexpected continuation Work resolution: {resolution:?}"
                    )))
                }
            })
        } else {
            let live_session_ids: HashSet<String> = self
                .active_agent_sessions
                .values()
                .map(|session| session.session_id.clone())
                .collect();
            gwt_core::workspace_projection::transact_workspace_state_with_commit(
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
                    let event = apply_workspace_launch_transition(
                        projection,
                        &active_session,
                        WorkspaceLaunchTransition {
                            work_id: Some(pending.work_id.clone()),
                            base_branch: None,
                            linked_issue_number: Some(pending.owner.number),
                            resume_context: Some(&pending.resume_context),
                            kind: WorkspaceLaunchProjectionKind::Resume {
                                created_by_start_work: active_session
                                    .branch_name
                                    .starts_with("work/"),
                            },
                            live_session_ids: &live_session_ids,
                            now: chrono::Utc::now(),
                        },
                    );
                    Ok(((), vec![event]))
                },
                || {
                    match &pending.execution {
                        PendingContinueWorkExecution::Successor(request) => {
                            gwt::cli::execution_state::activate_successor(
                                &pending.worktree_path,
                                pending.owner,
                                request,
                            )
                            .map(|_| ())
                        }
                        PendingContinueWorkExecution::Takeover(request) => {
                            gwt::cli::execution_state::activate_generation_takeover(
                                &pending.worktree_path,
                                pending.owner,
                                request,
                            )
                            .map(|_| ())
                        }
                    }
                    .map_err(gwt_core::error::GwtError::Io)
                },
            )
        };
        if transaction_result.is_err() {
            let activated = pending_execution_is_activated(&pending);
            if !activated {
                return self.continue_work_launch_failed_events(
                    window_id,
                    "the generation activation transaction was rejected",
                );
            }
            if gwt_core::workspace_projection::resolve_workspace_state_external_commit(
                &pending.project_root,
                &pending.operation_id,
                gwt_core::workspace_projection::ExternalWorkspaceCommitDecision::Commit,
            )
            .ok()
                != Some(
                    gwt_core::workspace_projection::ExternalWorkspaceCommitResolution::Committed,
                )
            {
                return Vec::new();
            }
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
            gwt_core::workspace_projection::load_workspace_work_items(&pending.project_root)
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
