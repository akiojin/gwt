//! Launch wizard handler split out of `app_runtime/mod.rs` for SPEC-2077
//! Phase F1 / F2a-1 (arch-review handoff, 2026-05-01).
//!
//! Phase F is split into multiple sub-phases to keep blast radius small:
//! - F1 (merged): wizard state broadcast / clear helpers
//! - F2a-1 (this PR): branch-level open helpers (open_launch_wizard,
//!   open_launch_wizard_for_branch, refresh_open_launch_wizard_from_cache)
//! - F2a-2 (follow-up): issue-level open + prepared dispatch handlers
//! - F2b (follow-up): handle_launch_wizard_action (~600 lines)
//! - F3a/F3b (follow-up): spawn_wizard_shell_window* (~525 lines)
//!
//! [`LaunchWizardSession`] still lives in `mod.rs` because the larger wizard
//! handlers (Phase F2b / F3 scope) construct and mutate it; once those
//! phases land the struct can move here too.

use std::{
    path::{Path, PathBuf},
    thread,
};

use chrono::Utc;
use gwt::{
    knowledge_launch_target_branch_name, KnowledgeKind, LaunchWizardCompletion,
    LaunchWizardContext, LaunchWizardHydration, LaunchWizardLaunchPath, LaunchWizardLaunchRequest,
    LaunchWizardState, LaunchWizardView, LinkedIssueKind, WindowGeometry,
};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::continuation::{
    provider_conversation_availability, provider_conversation_availability_with_grok_home,
    session_matches_project_state, ProviderConversationAvailability,
};
use crate::{ShellLaunchConfig, UserEvent};

/// `Pr => None` because Launch Agent is not exposed for PR bridges
/// (`KnowledgeDetailView::launch_issue_number` stays `None` for PR entries).
fn linked_issue_kind_from_knowledge(kind: KnowledgeKind) -> Option<LinkedIssueKind> {
    match kind {
        KnowledgeKind::Issue => Some(LinkedIssueKind::Issue),
        KnowledgeKind::Spec => Some(LinkedIssueKind::Spec),
        KnowledgeKind::Pr => None,
    }
}

fn launch_wizard_open_error(
    client_id: &str,
    title: &str,
    message: impl Into<String>,
) -> OutboundEvent {
    OutboundEvent::reply(
        client_id.to_string(),
        BackendEvent::LaunchWizardOpenError {
            title: title.to_string(),
            message: message.into(),
        },
    )
}

fn launch_agent_open_error(client_id: &str, message: impl Into<String>) -> Vec<OutboundEvent> {
    vec![launch_wizard_open_error(client_id, "Launch Agent", message)]
}

fn start_work_open_error(client_id: &str, message: impl Into<String>) -> Vec<OutboundEvent> {
    vec![launch_wizard_open_error(client_id, "Start Work", message)]
}

fn manual_holder_fingerprint(
    owner: gwt::cli::execution_state::ExecutionOwnerKey,
    identity: &gwt_agent::SessionExecutionIdentity,
    runtime_incarnation: Option<u64>,
) -> String {
    format!(
        "{}:{}:{}:{}:{}:{}:{}",
        owner.kind.as_str(),
        owner.number,
        identity.execution_binding.identity.generation_id,
        identity.execution_binding.identity.binding_id,
        identity.session_id,
        identity.execution_binding.capability_generation,
        runtime_incarnation.map_or_else(|| "remote".to_string(), |value| value.to_string()),
    )
}

fn manual_generation_operation_id(
    owner: gwt::cli::execution_state::ExecutionOwnerKey,
    binding: &gwt_agent::ExecutionBindingIdentity,
    predecessor_kind: gwt_agent::ManualLaunchSuccessorPredecessor,
) -> String {
    manual_holder_operation_id(
        &format!(
            "{}:{}:{}:{}",
            owner.kind.as_str(),
            owner.number,
            binding.generation_id,
            binding.ledger_head_hash
        ),
        predecessor_kind,
    )
}

fn manual_successor_stable_component(prefix: &str, operation_id: &str) -> String {
    let digest = Sha256::digest(operation_id.as_bytes());
    format!("{prefix}-{}", hex::encode(digest))
}

fn manual_holder_operation_id(
    fingerprint: &str,
    predecessor_kind: gwt_agent::ManualLaunchSuccessorPredecessor,
) -> String {
    let kind = match predecessor_kind {
        gwt_agent::ManualLaunchSuccessorPredecessor::Blocked => "blocked",
        gwt_agent::ManualLaunchSuccessorPredecessor::Completed => "completed",
        gwt_agent::ManualLaunchSuccessorPredecessor::ExactTerminalActive => "active-terminal",
    };
    format!("manual-launch-successor:{kind}:{fingerprint}")
}

fn issue_monitor_auto_launch_geometry(index: usize) -> WindowGeometry {
    let offset = ((index % 8) as f64) * 24.0;
    WindowGeometry {
        x: 96.0 + offset,
        y: 96.0 + offset,
        width: 860.0,
        height: 520.0,
    }
}

/// SPEC #3914 FR-007: the profile chosen for one silent launch, plus the
/// candidates ranked ahead of it that were passed over (empty for the head).
#[derive(Debug, Clone)]
struct IssueMonitorLaunchProfileChoice {
    profiles: gwt::LaunchWizardPreviousProfiles,
    selected_agent_id: Option<String>,
    skipped: Vec<gwt::LaunchProfileSkip>,
    tier: Option<u8>,
}

/// SPEC #3914 FR-007: make a non-head selection visible, with the reason each
/// earlier candidate was passed over. `None` when the pool head launched, so
/// both the fresh-launch and the exact-Resume paths can append it verbatim.
fn issue_monitor_non_head_selection_toast(
    context: &super::ProjectContext,
    issue_number: u64,
    selected_agent_id: Option<&str>,
    skipped_candidates: &[gwt::LaunchProfileSkip],
) -> Option<OutboundEvent> {
    let selected_agent_id = selected_agent_id?;
    if skipped_candidates.is_empty() {
        return None;
    }
    let reasons = skipped_candidates
        .iter()
        .map(|skip| skip.reason.as_str())
        .collect::<Vec<_>>()
        .join("; ");
    tracing::info!(
        issue = issue_number,
        agent = %selected_agent_id,
        reasons = %reasons,
        "Issue Monitor selected a non-head launch candidate"
    );
    Some(OutboundEvent::project(
        context.project_key.clone(),
        BackendEvent::IssueMonitorToast {
            notification_transition: None,
            level: "info".to_string(),
            message: format!("Issue #{issue_number} launches with {selected_agent_id}: {reasons}"),
            issue_number: Some(issue_number),
        },
    ))
}

struct SilentIssueMonitorLaunchRequest {
    issue_number: u64,
    linked_issue_kind: gwt::LinkedIssueKind,
    review_prompt: Option<String>,
    review_model: Option<String>,
    delivery_id: Option<String>,
    launch_session_strategy: gwt::IssueMonitorLaunchSessionStrategy,
}

type IssueMonitorResumeHandoff = Result<Option<gwt::AutonomousHandoffResumption>, String>;

#[derive(Debug, Clone)]
struct PreparedIssueMonitorResume {
    session: gwt_agent::Session,
    autonomous_handoff: Option<gwt::AutonomousHandoffResumption>,
    session_record: Result<Vec<u8>, std::io::ErrorKind>,
    config: gwt_agent::LaunchConfig,
    workspace_resume_context: super::WorkspaceResumeContext,
}

#[derive(Debug, Clone)]
struct IssueMonitorLaunchFacts {
    base_branch: String,
    choice: IssueMonitorLaunchProfileChoice,
    hydration: Option<LaunchWizardHydration>,
    resume: Result<Option<PreparedIssueMonitorResume>, String>,
}

#[derive(Debug, Clone)]
enum IssueMonitorLaunchPreparation {
    Launch(Box<IssueMonitorLaunchFacts>),
    Answer(Box<PreparedIssueMonitorResume>),
}

type IssueMonitorSelectionSnapshot =
    Result<Option<(gwt::LaunchWizardPreviousProfiles, Option<u8>)>, String>;

#[derive(Debug, Clone)]
pub(crate) struct IssueMonitorLaunchPrepared {
    context: super::ProjectContext,
    request: super::DeferredIssueMonitorLaunch,
    handoff: IssueMonitorResumeHandoff,
    selection: IssueMonitorSelectionSnapshot,
    result: Result<IssueMonitorLaunchPreparation, String>,
}

pub(crate) type IssueMonitorLaunchPreparationKey = (String, u64, u64, Option<String>, bool);

fn issue_monitor_launch_preparation_key(
    context: &super::ProjectContext,
    request: &super::DeferredIssueMonitorLaunch,
) -> IssueMonitorLaunchPreparationKey {
    (
        context.tab_id.clone(),
        context.generation,
        request.issue_number,
        request.delivery_id.clone(),
        request.launch_session_strategy == gwt::IssueMonitorLaunchSessionStrategy::FreshRequired,
    )
}

fn issue_monitor_resume_handoff(
    project_root: &Path,
    issue_number: u64,
) -> IssueMonitorResumeHandoff {
    gwt::pending_autonomous_handoff_resumption_from_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(project_root),
        issue_number,
    )
    .map_err(|error| format!("failed to read the autonomous handoff answer: {error}"))
}

// None means an automatic answer must return to its asking Session, before
// considering the current tier pool or its provider eligibility.
fn issue_monitor_preparation_choice(
    cache: &LaunchWizardMemoryCache,
    provider_usage_accounts: &[gwt_core::usage::ProviderUsage],
    request: &super::DeferredIssueMonitorLaunch,
) -> Result<Option<IssueMonitorLaunchProfileChoice>, String> {
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&request.project_root);
    let prefs = gwt::load_issue_monitor_prefs(&prefs_path).map_err(|error| error.to_string())?;
    if prefs.launch_auto
        && gwt::preserve_answered_handoff_resume_strategy_from_prefs(
            &prefs_path,
            request.issue_number,
        )
        .map_err(|error| error.to_string())?
    {
        return Ok(None);
    }
    issue_monitor_owner_launch_profile_choice(
        cache,
        provider_usage_accounts,
        &request.project_root,
        request.issue_number,
        request.linked_issue_kind,
    )
    .map(Some)
}

fn prepare_issue_monitor_answer(
    request: &super::DeferredIssueMonitorLaunch,
    cache: &LaunchWizardMemoryCache,
    sessions_dir: &Path,
    profile_config_path: &Result<PathBuf, String>,
    auth_probe: fn(&str) -> gwt::issue_monitor::ProviderAuthState,
    handoff: IssueMonitorResumeHandoff,
) -> Result<PreparedIssueMonitorResume, String> {
    let handoff = handoff?
        .ok_or_else(|| "answered autonomous handoff is not ready for exact delivery".to_string())?;
    let source =
        gwt_agent::Session::load(&sessions_dir.join(format!("{}.toml", handoff.session_id)))
            .ok()
            .or_else(|| cache.session_by_id(&handoff.session_id).cloned())
            .ok_or_else(|| {
                format!(
                    "answered autonomous handoff Session {} is unavailable",
                    handoff.session_id
                )
            })?;
    let agent_id = source.agent_id.command();
    if auth_probe(agent_id) == gwt::issue_monitor::ProviderAuthState::Unauthenticated {
        return Err(gwt::issue_monitor::provider_unauthenticated_message(
            agent_id,
        ));
    }
    let branch =
        knowledge_launch_target_branch_name(request.linked_issue_kind, request.issue_number);
    prepare_issue_monitor_resume(
        &request.project_root,
        &branch,
        request.issue_number,
        agent_id,
        sessions_dir,
        cache,
        profile_config_path,
        Ok(Some(handoff)),
    )?
    .ok_or_else(|| "answered autonomous handoff requires its exact Session".to_string())
}

fn prepare_issue_monitor_launch(
    request: &super::DeferredIssueMonitorLaunch,
    cache: &LaunchWizardMemoryCache,
    sessions_dir: &Path,
    profile_config_path: &Result<PathBuf, String>,
    choice: IssueMonitorLaunchProfileChoice,
    auth_probe: fn(&str) -> gwt::issue_monitor::ProviderAuthState,
    handoff: IssueMonitorResumeHandoff,
) -> Result<IssueMonitorLaunchFacts, String> {
    let project_root = &request.project_root;
    let base_branch = gwt::start_work::resolve_launch_agent_base_branch(project_root)?;
    let Some(profile) = choice.profiles.preferred_profile() else {
        return Ok(IssueMonitorLaunchFacts {
            base_branch,
            choice,
            hydration: None,
            resume: Ok(None),
        });
    };
    if auth_probe(&profile.agent_id) == gwt::issue_monitor::ProviderAuthState::Unauthenticated {
        return Err(gwt::issue_monitor::provider_unauthenticated_message(
            &profile.agent_id,
        ));
    }
    let branch =
        knowledge_launch_target_branch_name(request.linked_issue_kind, request.issue_number);
    let resume = if choice.tier.is_none()
        && (request.launch_session_strategy == gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe
            || handoff.as_ref().is_ok_and(|handoff| handoff.is_some()))
    {
        prepare_issue_monitor_resume(
            project_root,
            &branch,
            request.issue_number,
            &profile.agent_id,
            sessions_dir,
            cache,
            profile_config_path,
            handoff,
        )
    } else {
        Ok(None)
    };
    let hydration =
        resolve_launch_wizard_runtime_context_hydration(project_root, branch, cache.clone())?;
    Ok(IssueMonitorLaunchFacts {
        base_branch,
        choice,
        hydration: Some(hydration),
        resume,
    })
}

/// SPEC #3200 Option A: build the independent-review agent's prompt from a
/// dispatch — the adversarial review prompt (criteria + diff as untrusted data,
/// bound to the reviewed SHA) plus the instruction to report the verdict back to
/// the Issue Monitor daemon via the `ReviewVerdict` control for this exact SHA.
fn build_review_dispatch_prompt(dispatch: &gwt::AutonomousReviewDispatch) -> String {
    let base = gwt::issue_monitor_review::build_review_prompt(
        &dispatch.required_criteria,
        &dispatch.reviewed_sha,
        &dispatch.diff,
    );
    format!(
        "{base}\n\nAfter producing the verdict JSON, report it to the Issue Monitor \
         daemon by running the gwtd JSON operation `issue.monitor.review_verdict` \
         with params {{\"issue_number\": {issue}, \"reviewed_sha\": \"{sha}\", \
         \"verdict_raw\": <the verdict JSON as a string>}}. The daemon re-judges the \
         verdict against the launch-time criteria; a verdict for any other SHA is \
         rejected.",
        issue = dispatch.issue_number,
        sha = dispatch.reviewed_sha,
    )
}

use super::{
    build_shell_process_launch, combined_window_id, detect_wizard_docker_context_and_status,
    knowledge_error_event, knowledge_kind_for_preset, linked_issue_workspace_context,
    list_branch_entries_with_active_sessions, normalize_branch_name, preferred_issue_launch_branch,
    resolve_launch_worktree, resolve_shell_launch_worktree, save_shell_work_projection,
    session_exact_resume_materializable, synthetic_branch_entry,
    workspace_projection_for_current_resume, workspace_resume_branch_exists,
    workspace_resume_branch_from_journal_project_root, workspace_resume_context_for_work_item,
    workspace_resume_context_from_journal, workspace_resume_context_from_projection,
    workspace_resume_owner_issue_number, AgentKanbanLaunchTarget, AppEventProxy, AppRuntime,
    BackendEvent, DispatchTarget, IssueLaunchWizardPrepared, IssueMonitorAgentSettingsSets,
    IssueMonitorProfileSaveContext, LaunchFeedbackContext, LaunchWizardMemoryCache,
    LaunchWizardSession, OutboundEvent, WindowPreset, WindowProcessStatus, WorkspaceResumeContext,
    WORKSPACE_OVERVIEW_JOURNAL_LIMIT,
};
use crate::usable_worktree_path_for_branch;

/// Issue #4079 AC-2: describe what an Agent Settings save does to the saved
/// candidate pool.
///
/// The save always writes index 0 (the PM ruling: the form is a switch), so the
/// operator has to see which candidate is being replaced before committing.
/// The resulting summary is produced by applying the save to a copy of the
/// pool, so it is the same string `issue.monitor.profiles` reports afterwards.
fn issue_monitor_pool_impact_view(
    pool: &[gwt::IssueMonitorLaunchProfile],
    profile: gwt::IssueMonitorLaunchProfile,
) -> gwt::LaunchWizardIssueMonitorPoolImpactView {
    let agent_id = profile.agent_id.clone();
    let replaced_agent_id = pool
        .first()
        .map(|head| head.agent_id.clone())
        .filter(|head| !head.eq_ignore_ascii_case(&agent_id));
    let mut prefs = gwt::IssueMonitorPrefs::default();
    prefs.set_launch_profile_pool(pool.to_vec());
    prefs.set_head_launch_profile(profile);
    let resulting_pool = prefs.launch_profile_pool();
    let resulting_summary = gwt::issue_monitor_launch_profile_pool_summary(&resulting_pool);
    let title = match (&replaced_agent_id, pool.is_empty()) {
        (_, true) => "Saves the first launch candidate".to_string(),
        (Some(replaced), false) => format!("Replaces candidate 1 ({replaced})"),
        (None, false) => format!("Updates candidate 1 ({agent_id})"),
    };
    let detail = if pool.len() > 1 {
        format!(
            "Agent Settings always writes candidate 1, the agent Issue Monitor launches first. \
             The remaining {} candidate(s) are left as they are; add or reorder candidates with \
             issue.monitor.profiles.set. After saving: {resulting_summary}",
            resulting_pool.len().saturating_sub(1)
        )
    } else {
        format!(
            "Agent Settings always writes candidate 1, the agent Issue Monitor launches first. \
             After saving: {resulting_summary}"
        )
    };
    gwt::LaunchWizardIssueMonitorPoolImpactView {
        action: "replace_head".to_string(),
        agent_id,
        replaced_agent_id,
        title,
        detail,
        resulting_summary,
    }
}

/// Issue #4911 AC-6: shown next to `−` and returned when the last set is
/// removed anyway.
const AGENT_SETTINGS_LAST_SET_REASON: &str =
    "At least one Agent Settings set is required: Issue Monitor needs an agent to launch.";
const AGENT_SETTINGS_ALL_AGENTS_USED_REASON: &str =
    "Every available agent already has an Agent Settings set.";
/// Issue #4911 AC-8: the form writes the whole pool, so it refuses to write
/// over a pool it did not open.
const AGENT_SETTINGS_CHANGED_ELSEWHERE_REASON: &str =
    "Issue Monitor settings changed in another window or through issue.monitor.profiles.set \
     while this form was open. Nothing was saved. Cancel and reopen the settings to edit the \
     current Agent Settings sets.";

/// The provider a set launches, keyed the way the saved pool dedupes it.
fn agent_settings_provider_key(agent_id: &str) -> String {
    gwt_agent::resolve_agent_id(agent_id)
        .map(|id| id.command().to_ascii_lowercase())
        .unwrap_or_else(|| agent_id.trim().to_ascii_lowercase())
}

/// Issue #4911 AC-7: the saved pool holds one candidate per provider and
/// silently drops a later duplicate, so the form refuses one instead.
fn agent_settings_duplicate_error(profiles: &[gwt::IssueMonitorLaunchProfile]) -> Option<String> {
    profiles.iter().enumerate().find_map(|(index, profile)| {
        let key = agent_settings_provider_key(&profile.agent_id);
        profiles[..index]
            .iter()
            .position(|earlier| agent_settings_provider_key(&earlier.agent_id) == key)
            .map(|earlier| {
                format!(
                    "Agent Settings {} and {} both use {}. An agent can appear in one set only; \
                     change or remove one of them.",
                    earlier + 1,
                    index + 1,
                    profile.agent_id
                )
            })
    })
}

/// A set for an agent the pool does not hold yet, resolved by the sparse rule
/// `issue.monitor.profiles.set` applies to `{agent_id}`: it inherits how the
/// first set is run, never its model.
fn new_agent_settings_profile(
    profiles: &[gwt::IssueMonitorLaunchProfile],
    agent_id: &str,
) -> Option<gwt::IssueMonitorLaunchProfile> {
    let patch = serde_json::from_value(serde_json::json!({ "agent_id": agent_id })).ok()?;
    gwt::merge_issue_monitor_profiles_set(profiles, profiles.first(), &[patch])
        .0
        .into_iter()
        .next()
}

/// The read-only rows of a set the form is not editing, in the vocabulary of
/// `issue.monitor.profiles` (`default` model, `auto` reasoning, `host`).
fn agent_settings_set_summary(
    profile: &gwt::IssueMonitorLaunchProfile,
    agents: &[gwt::AgentOption],
) -> Vec<gwt::LaunchWizardSummaryView> {
    let row = |label: &str, value: String| gwt::LaunchWizardSummaryView {
        label: label.to_string(),
        value,
    };
    let key = agent_settings_provider_key(&profile.agent_id);
    let detected_agent = agents
        .iter()
        .find(|agent| agent_settings_provider_key(&agent.id) == key);
    let agent = detected_agent.map_or_else(|| profile.agent_id.clone(), |agent| agent.name.clone());
    let mut rows = vec![
        row("Agent", agent),
        row(
            "Model",
            profile
                .model
                .clone()
                .unwrap_or_else(|| "default".to_string()),
        ),
        row(
            "Reasoning",
            profile
                .reasoning
                .clone()
                .unwrap_or_else(|| "auto".to_string()),
        ),
    ];
    if let Some(version) = detected_agent.and_then(|agent| agent.installed_version.as_ref()) {
        rows.push(row("Version", version.clone()));
    }
    rows.push(row(
        "Runtime",
        match (profile.runtime_target, profile.docker_service.as_deref()) {
            (gwt_agent::LaunchRuntimeTarget::Host, _) => "host".to_string(),
            (gwt_agent::LaunchRuntimeTarget::Docker, Some(service)) => format!("docker:{service}"),
            (gwt_agent::LaunchRuntimeTarget::Docker, None) => "docker".to_string(),
        },
    ));
    rows
}

impl IssueMonitorAgentSettingsSets {
    /// The sets as a save would write them. `open` is what the form reads for
    /// the open set right now; `runtime_chosen` says whether the form has been
    /// through its Runtime step, before which it reads Host for every set.
    fn resolved(
        &self,
        open: Option<gwt::IssueMonitorLaunchProfile>,
        runtime_chosen: bool,
    ) -> Vec<gwt::IssueMonitorLaunchProfile> {
        let mut profiles = self.profiles.clone();
        let (Some(open), Some(slot)) = (open, profiles.get_mut(self.active)) else {
            return profiles;
        };
        let same_agent = agent_settings_provider_key(&slot.agent_id)
            == agent_settings_provider_key(&open.agent_id);
        // A model or reasoning level only means something for the agent it was
        // chosen for, so a set whose agent changed takes the form's as they are.
        let opened_as = self.opened_as.as_ref().filter(|_| same_agent);
        type Same = fn(&gwt::IssueMonitorLaunchProfile, &gwt::IssueMonitorLaunchProfile) -> bool;
        let edited = |same: Same| !opened_as.is_some_and(|opened_as| same(opened_as, &open));
        if edited(|a, b| a.model == b.model) {
            slot.model = open.model.clone();
        }
        if edited(|a, b| a.reasoning == b.reasoning) {
            slot.reasoning = open.reasoning.clone();
        }
        if edited(|a, b| a.version == b.version) {
            slot.version = open.version.clone();
        }
        if edited(|a, b| a.session_mode == b.session_mode) {
            slot.session_mode = open.session_mode;
        }
        if edited(|a, b| a.skip_permissions == b.skip_permissions) {
            slot.skip_permissions = open.skip_permissions;
        }
        if edited(|a, b| a.fast_mode == b.fast_mode) {
            slot.fast_mode = open.fast_mode;
        }
        if runtime_chosen {
            slot.runtime_target = open.runtime_target;
            slot.docker_service = open.docker_service.clone();
            slot.docker_lifecycle_intent = open.docker_lifecycle_intent;
            slot.windows_shell = open.windows_shell;
        }
        if !same_agent {
            // Routing tags describe a candidate rather than a position, and the
            // form has no tag input, so they do not follow a set to another
            // agent (the rule the head switch already follows).
            slot.prefer_for.clear();
        }
        slot.agent_id = open.agent_id;
        profiles
    }
}

/// What the form reads for the open set, or why it cannot be one: the launch
/// target is a shell, or the chosen agent has no way to launch here.
fn open_agent_settings_profile(
    sets: &IssueMonitorAgentSettingsSets,
    wizard: &LaunchWizardState,
) -> Result<gwt::IssueMonitorLaunchProfile, String> {
    wizard
        .build_launch_config()
        .map(|config| gwt::IssueMonitorLaunchProfile::from(&config))
        .map_err(|error| {
            format!(
                "Agent Settings {} cannot be left as it is: {error}",
                sets.active + 1
            )
        })
}

impl AppRuntime {
    /// SPEC-3864 FR-006: feed the host-global "is this agent configured?"
    /// probes into the wizard. The probes are per-agent (they read that
    /// agent's own config home); everything downstream — the setup
    /// affordance and its in-pane launcher — is descriptor-driven.
    fn apply_agent_configuration_state(wizard: &mut gwt::LaunchWizardState) {
        wizard.set_hermes_launch_choices(gwt_skills::hermes_launch_choices_global());
        wizard.set_agent_needs_configuration("hermes", !gwt_skills::hermes_is_configured_global());
        wizard.set_agent_needs_configuration(
            "opencode",
            !gwt_skills::opencode_is_configured_global(),
        );
    }

    fn launch_wizard_view_for_session(&self, session: &LaunchWizardSession) -> LaunchWizardView {
        let mut view = session.wizard.view();
        if let Some(save_context) = session.issue_monitor_profile_save.as_ref() {
            view.title = "Configure Issue Monitor".to_string();
            if view.primary_action_label == "Create and launch"
                || view.primary_action_label == "Launch"
            {
                view.primary_action_label = "Save settings".to_string();
            }
            match save_context.sets.as_ref() {
                Some(sets) => {
                    view.issue_monitor_pool =
                        Some(self.agent_settings_sets_view(sets, &session.wizard));
                }
                None => {
                    view.issue_monitor_pool_impact = session
                        .wizard
                        .preview_launch_profile()
                        .map(|profile| issue_monitor_pool_impact_view(&save_context.pool, profile));
                }
            }
        }
        view
    }

    fn agent_settings_sets_view(
        &self,
        sets: &IssueMonitorAgentSettingsSets,
        wizard: &LaunchWizardState,
    ) -> gwt::LaunchWizardIssueMonitorPoolView {
        let profiles = sets.resolved(
            wizard.preview_launch_profile(),
            wizard.runtime_context_resolved,
        );
        gwt::LaunchWizardIssueMonitorPoolView {
            active_index: sets.active,
            sets: profiles
                .iter()
                .enumerate()
                .map(
                    |(index, profile)| gwt::LaunchWizardIssueMonitorPoolSetView {
                        agent_id: profile.agent_id.clone(),
                        summary: if index == sets.active {
                            Vec::new()
                        } else {
                            agent_settings_set_summary(profile, &wizard.detected_agents)
                        },
                    },
                )
                .collect(),
            add_disabled_reason: self
                .unused_agent_settings_agent(&profiles)
                .is_none()
                .then(|| AGENT_SETTINGS_ALL_AGENTS_USED_REASON.to_string()),
            remove_disabled_reason: (profiles.len() <= 1)
                .then(|| AGENT_SETTINGS_LAST_SET_REASON.to_string()),
            resulting_summary: gwt::issue_monitor_launch_profile_pool_summary(&profiles),
        }
    }

    /// The agent a new set starts on: the first built-in no set uses yet,
    /// installed ones first. The form's own agent picker then offers the rest.
    fn unused_agent_settings_agent(
        &self,
        profiles: &[gwt::IssueMonitorLaunchProfile],
    ) -> Option<String> {
        let used = profiles
            .iter()
            .map(|profile| agent_settings_provider_key(&profile.agent_id))
            .collect::<std::collections::BTreeSet<_>>();
        let unused = self
            .launch_wizard_cache
            .agent_options()
            .into_iter()
            .filter(|agent| {
                agent.custom_agent.is_none()
                    && !used.contains(&agent_settings_provider_key(&agent.id))
            })
            .collect::<Vec<_>>();
        unused
            .iter()
            .find(|agent| agent.available)
            .or(unused.first())
            .map(|agent| agent.id.clone())
    }

    /// Issue #4911: apply one edit to the Agent Settings sets. `None` when
    /// `action` is not such an edit or this wizard is not the settings form,
    /// so the caller hands the action to the wizard as before.
    fn apply_agent_settings_set_action(
        &self,
        session: &mut LaunchWizardSession,
        action: &gwt::LaunchWizardAction,
    ) -> Option<Result<(), String>> {
        let mut sets = session.issue_monitor_profile_save.as_ref()?.sets.clone()?;
        let open_other_set = match action {
            gwt::LaunchWizardAction::SetAgent { agent_id } => {
                let key = agent_settings_provider_key(agent_id);
                let taken = sets
                    .profiles
                    .iter()
                    .enumerate()
                    .position(|(index, profile)| {
                        index != sets.active
                            && agent_settings_provider_key(&profile.agent_id) == key
                    })?;
                return Some(Err(format!(
                    "{agent_id} is already Agent Settings {}. An agent can appear in one set \
                     only; move that set up to launch it first.",
                    taken + 1
                )));
            }
            gwt::LaunchWizardAction::AddAgentSettingsSet
            | gwt::LaunchWizardAction::RemoveAgentSettingsSet { .. }
            | gwt::LaunchWizardAction::MoveAgentSettingsSet { .. }
            | gwt::LaunchWizardAction::SelectAgentSettingsSet { .. }
                if session.wizard.runtime_resolution_pending
                    || session.wizard.launch_materialization_pending =>
            {
                return Some(Ok(()));
            }
            gwt::LaunchWizardAction::AddAgentSettingsSet => {
                let open = match open_agent_settings_profile(&sets, &session.wizard) {
                    Ok(open) => open,
                    Err(error) => return Some(Err(error)),
                };
                let mut profiles =
                    sets.resolved(Some(open), session.wizard.runtime_context_resolved);
                if let Some(error) = agent_settings_duplicate_error(&profiles) {
                    return Some(Err(error));
                }
                let Some(added) = self
                    .unused_agent_settings_agent(&profiles)
                    .and_then(|agent_id| new_agent_settings_profile(&profiles, &agent_id))
                else {
                    return Some(Err(AGENT_SETTINGS_ALL_AGENTS_USED_REASON.to_string()));
                };
                profiles.push(added);
                sets.active = profiles.len() - 1;
                sets.profiles = profiles;
                true
            }
            gwt::LaunchWizardAction::RemoveAgentSettingsSet { index } => {
                if sets.profiles.len() <= 1 {
                    return Some(Err(AGENT_SETTINGS_LAST_SET_REASON.to_string()));
                }
                if *index >= sets.profiles.len() {
                    return Some(Ok(()));
                }
                sets.profiles.remove(*index);
                let removed_open_set = *index == sets.active;
                if *index < sets.active {
                    sets.active -= 1;
                }
                sets.active = sets.active.min(sets.profiles.len() - 1);
                removed_open_set
            }
            gwt::LaunchWizardAction::MoveAgentSettingsSet { index, to } => {
                let (index, to) = (*index, *to);
                if index >= sets.profiles.len() || to >= sets.profiles.len() {
                    return Some(Ok(()));
                }
                let moved = sets.profiles.remove(index);
                sets.profiles.insert(to, moved);
                // The open set keeps its form wherever the move puts it.
                sets.active = if sets.active == index {
                    to
                } else if index < sets.active && to >= sets.active {
                    sets.active - 1
                } else if index > sets.active && to <= sets.active {
                    sets.active + 1
                } else {
                    sets.active
                };
                false
            }
            gwt::LaunchWizardAction::SelectAgentSettingsSet { index } => {
                if *index == sets.active || *index >= sets.profiles.len() {
                    return Some(Ok(()));
                }
                let open = match open_agent_settings_profile(&sets, &session.wizard) {
                    Ok(open) => open,
                    Err(error) => return Some(Err(error)),
                };
                let profiles = sets.resolved(Some(open), session.wizard.runtime_context_resolved);
                if let Some(error) = agent_settings_duplicate_error(&profiles) {
                    return Some(Err(error));
                }
                sets.profiles = profiles;
                sets.active = *index;
                true
            }
            _ => return None,
        };
        if open_other_set {
            let project_root = self.tab(&session.tab_id)?.project_root.clone();
            let profile = sets.profiles.get(sets.active)?.clone();
            session.wizard = self.issue_monitor_settings_wizard(
                project_root,
                gwt::LaunchWizardPreviousProfiles::from_profile(Some(profile.into())),
            );
            sets.opened_as = session.wizard.preview_launch_profile();
            // A runtime resolution still in flight belongs to the form that
            // was just replaced and must not land on this one.
            session.wizard_id = Uuid::new_v4().to_string();
        }
        if let Some(save_context) = session.issue_monitor_profile_save.as_mut() {
            save_context.sets = Some(sets);
        }
        Some(Ok(()))
    }

    pub(crate) fn launch_wizard_for(
        &self,
        context: &super::ProjectContext,
    ) -> Option<&LaunchWizardSession> {
        self.project_state(context)?.launch_wizard.as_ref()
    }

    pub(crate) fn launch_wizard_for_mut(
        &mut self,
        context: &super::ProjectContext,
    ) -> Option<&mut LaunchWizardSession> {
        self.project_state_mut(context)?.launch_wizard.as_mut()
    }

    pub(crate) fn take_launch_wizard(
        &mut self,
        context: &super::ProjectContext,
    ) -> Option<LaunchWizardSession> {
        self.project_state_mut(context)?.launch_wizard.take()
    }

    pub(crate) fn store_launch_wizard(&mut self, session: LaunchWizardSession) {
        if let Some(state) = self.project_state_mut(&session.project_context) {
            state.launch_wizard = Some(session);
        }
    }

    pub(crate) fn project_context_for_wizard(
        &self,
        wizard_id: &str,
    ) -> Option<super::ProjectContext> {
        self.project_states.values().find_map(|state| {
            let session = state.launch_wizard.as_ref()?;
            (session.wizard_id == wizard_id
                && self.project_context_is_current(&session.project_context))
            .then(|| session.project_context.clone())
        })
    }

    pub(crate) fn launch_wizard_state_outbound(
        &self,
        context: &super::ProjectContext,
    ) -> OutboundEvent {
        OutboundEvent::project(
            context.project_key.clone(),
            BackendEvent::LaunchWizardState {
                wizard: self
                    .launch_wizard_for(context)
                    .map(|wizard| Box::new(self.launch_wizard_view_for_session(wizard))),
            },
        )
    }

    pub(crate) fn launch_wizard_state_broadcast(
        &self,
        context: &super::ProjectContext,
        wizard: Option<LaunchWizardView>,
    ) -> OutboundEvent {
        OutboundEvent::project(
            context.project_key.clone(),
            BackendEvent::LaunchWizardState {
                wizard: wizard.map(Box::new),
            },
        )
    }

    #[cfg(test)]
    pub(crate) fn clear_launch_wizard(
        &mut self,
        context: &super::ProjectContext,
    ) -> Option<LaunchWizardSession> {
        self.take_launch_wizard(context)
    }

    pub(crate) fn open_launch_wizard(
        &mut self,
        client_id: &str,
        id: &str,
        branch_name: &str,
        linked_issue_number: Option<u64>,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id).cloned() else {
            return launch_agent_open_error(client_id, "Window not found");
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return launch_agent_open_error(client_id, "Project tab not found");
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return launch_agent_open_error(client_id, "Window not found");
        };

        if window.preset != WindowPreset::Branches && window.preset != WindowPreset::Work {
            tracing::warn!(
                preset = ?window.preset,
                window_id = id,
                "open_launch_wizard rejected: wrong preset"
            );
            return launch_agent_open_error(
                client_id,
                format!("Window preset {:?} is not a Work surface", window.preset),
            );
        }
        // SPEC-1934 US-7 / FR-034
        if tab.migration_pending {
            return launch_agent_open_error(
                client_id,
                "Complete the project migration before launching an agent",
            );
        }

        let project_root = tab.project_root.clone();
        let tab_id = address.tab_id.clone();
        let Some(context) = self.project_context(&tab_id) else {
            return Vec::new();
        };
        match self.open_launch_wizard_for_branch(
            &tab_id,
            &project_root,
            branch_name,
            linked_issue_number,
            None,
        ) {
            Ok(()) => vec![self.launch_wizard_state_outbound(&context)],
            Err(error) => launch_agent_open_error(client_id, error),
        }
    }

    pub(crate) fn open_launch_wizard_for_branch(
        &mut self,
        tab_id: &str,
        project_root: &Path,
        branch_name: &str,
        linked_issue_number: Option<u64>,
        linked_issue_kind: Option<LinkedIssueKind>,
    ) -> Result<(), String> {
        self.open_launch_wizard_for_branch_with_context(
            tab_id,
            project_root,
            branch_name,
            linked_issue_number,
            linked_issue_kind,
            None,
        )
    }

    pub(crate) fn open_launch_wizard_for_branch_with_context(
        &mut self,
        tab_id: &str,
        project_root: &Path,
        branch_name: &str,
        linked_issue_number: Option<u64>,
        linked_issue_kind: Option<LinkedIssueKind>,
        workspace_resume_context: Option<WorkspaceResumeContext>,
    ) -> Result<(), String> {
        let context = self
            .project_context(tab_id)
            .ok_or_else(|| "Project tab not found".to_string())?;
        let normalized_branch_name = normalize_branch_name(branch_name);
        let live_sessions = self.live_sessions_for_branch(tab_id, &normalized_branch_name);
        let worktree_path = None;
        let quick_start_root = project_root.to_path_buf();
        // SPEC-2014 US-27: Branches > Launch Agent must expose all
        // resumable sessions in Quick Start so users can choose a specific
        // prior conversation. The cache is already in memory, so this stays
        // off the GUI hot path's filesystem scan.
        let mut quick_start_entries = self
            .launch_wizard_cache
            .quick_start_entries(&quick_start_root, &normalized_branch_name);
        if workspace_resume_context.is_none() {
            quick_start_entries.retain(|entry| entry.resume_session_id.is_some());
        }
        let previous_profiles = self.launch_wizard_cache.agent_preferences();
        let agent_options = self.launch_wizard_cache.agent_options();
        let docker_context = None;
        let docker_service_status = gwt_docker::ComposeServiceStatus::NotFound;
        let wizard_id = Uuid::new_v4().to_string();
        let mut wizard = LaunchWizardState::open_with_previous_profiles(
            LaunchWizardContext {
                selected_branch: synthetic_branch_entry(branch_name),
                normalized_branch_name,
                worktree_path,
                quick_start_root,
                live_sessions,
                docker_context,
                docker_service_status,
                linked_issue_number,
                linked_issue_kind,
                ultracode_supported: self.launch_wizard_cache.claude_ultracode_supported(),
                claude_workflows_enabled: self.launch_wizard_cache.claude_workflows_enabled(),
            },
            agent_options,
            quick_start_entries,
            previous_profiles,
        );
        Self::apply_agent_configuration_state(&mut wizard);
        wizard.mark_runtime_context_unresolved();
        let origin = if workspace_resume_context.is_some() {
            super::LaunchWizardOrigin::WorkspaceResume
        } else {
            super::LaunchWizardOrigin::ManualLaunchAgent
        };
        self.store_launch_wizard(LaunchWizardSession {
            project_context: context.clone(),
            tab_id: tab_id.to_string(),
            wizard_id,
            wizard,
            workspace_resume_context,
            agent_kanban_target: None,
            auto_submit_after_runtime_resolution: None,
            issue_monitor_profile_save: None,
            issue_monitor_launch_issue_number: None,
            origin,
            manual_holder_intent: None,
        });

        Ok(())
    }

    pub(crate) fn open_knowledge_launch_wizard_for_base_branch(
        &mut self,
        tab_id: &str,
        project_root: &Path,
        base_branch_name: &str,
        issue_number: u64,
        linked_issue_kind: LinkedIssueKind,
    ) -> Result<(), String> {
        let previous_profiles = self.launch_wizard_cache.agent_preferences();
        self.open_knowledge_launch_wizard_for_base_branch_with_previous_profiles(
            tab_id,
            project_root,
            base_branch_name,
            issue_number,
            linked_issue_kind,
            previous_profiles,
        )
    }

    fn open_knowledge_launch_wizard_for_base_branch_with_previous_profiles(
        &mut self,
        tab_id: &str,
        project_root: &Path,
        base_branch_name: &str,
        issue_number: u64,
        linked_issue_kind: LinkedIssueKind,
        previous_profiles: gwt::LaunchWizardPreviousProfiles,
    ) -> Result<(), String> {
        self.project_context(tab_id)
            .ok_or_else(|| "Project tab not found".to_string())?;
        self.store_launch_wizard(self.build_knowledge_launch_wizard_session(
            tab_id,
            project_root,
            base_branch_name,
            issue_number,
            linked_issue_kind,
            previous_profiles,
        ));

        Ok(())
    }

    fn build_knowledge_launch_wizard_session(
        &self,
        tab_id: &str,
        project_root: &Path,
        base_branch_name: &str,
        issue_number: u64,
        linked_issue_kind: LinkedIssueKind,
        previous_profiles: gwt::LaunchWizardPreviousProfiles,
    ) -> LaunchWizardSession {
        // #3426: the unified Issue surface preset collapses SPEC entries to
        // LinkedIssueKind::Issue, so re-canonicalize the kind from the cached
        // label evidence before it seeds the Work owner label. Absent evidence
        // keeps the caller-declared kind. This stays scoped to the owner label:
        // `linked_issue_kind` also drives `show_linked_issue` and the manual
        // branch suffix, and flipping those from a cache label would hide the
        // wizard's Linked issue section and seed `spec-N` instead of the
        // unified `issue-N` branch convention.
        let canonical_owner_kind =
            match gwt::cli::execution_state::detect_owner_kind_evidence(project_root, issue_number)
            {
                Some(gwt::cli::execution_state::ExecutionOwnerKind::Spec) => LinkedIssueKind::Spec,
                Some(gwt::cli::execution_state::ExecutionOwnerKind::Issue) => {
                    LinkedIssueKind::Issue
                }
                None => linked_issue_kind,
            };
        let base_branch_name = normalize_branch_name(base_branch_name);
        let target_branch_name =
            knowledge_launch_target_branch_name(linked_issue_kind, issue_number);
        let live_sessions = self.live_sessions_for_branch(tab_id, &target_branch_name);
        let quick_start_root = project_root.to_path_buf();
        let quick_start_entries = Vec::new();
        let agent_options = self.launch_wizard_cache.agent_options();
        let docker_context = None;
        let docker_service_status = gwt_docker::ComposeServiceStatus::NotFound;
        let wizard_id = Uuid::new_v4().to_string();
        // SPEC #3431 FR-070: this must be the spelling the durable execution
        // binding produces, not a display label. `workspace.ensure` compares
        // the two verbatim, so `SPEC #<n>` here wedges the Work forever.
        let owner_label = match canonical_owner_kind {
            LinkedIssueKind::Issue => format!("Issue #{issue_number}"),
            LinkedIssueKind::Spec => format!("SPEC-{issue_number}"),
        };
        let workspace_resume_context = Some(linked_issue_workspace_context(
            project_root,
            issue_number,
            owner_label,
        ));
        let mut wizard = LaunchWizardState::open_knowledge_launch_with_previous_profiles(
            LaunchWizardContext {
                selected_branch: synthetic_branch_entry(&base_branch_name),
                normalized_branch_name: target_branch_name,
                worktree_path: None,
                quick_start_root,
                live_sessions,
                docker_context,
                docker_service_status,
                linked_issue_number: Some(issue_number),
                linked_issue_kind: Some(linked_issue_kind),
                ultracode_supported: self.launch_wizard_cache.claude_ultracode_supported(),
                claude_workflows_enabled: self.launch_wizard_cache.claude_workflows_enabled(),
            },
            base_branch_name,
            agent_options,
            quick_start_entries,
            previous_profiles,
        );
        Self::apply_agent_configuration_state(&mut wizard);
        wizard.mark_runtime_context_unresolved();
        LaunchWizardSession {
            project_context: self
                .project_context(tab_id)
                .expect("wizard project is open"),
            tab_id: tab_id.to_string(),
            wizard_id,
            wizard,
            workspace_resume_context,
            agent_kanban_target: None,
            auto_submit_after_runtime_resolution: None,
            issue_monitor_profile_save: None,
            issue_monitor_launch_issue_number: None,
            origin: super::LaunchWizardOrigin::Knowledge,
            manual_holder_intent: None,
        }
    }

    pub(crate) fn open_active_work_launch_wizard(
        &mut self,
        context: &super::ProjectContext,
        client_id: &str,
        branch_name: &str,
        linked_issue_number: Option<u64>,
    ) -> Vec<OutboundEvent> {
        if !self.project_context_is_current(context) {
            return Vec::new();
        }
        let tab_id = context.tab_id.clone();
        let Some(tab) = self.tab(&tab_id) else {
            return launch_agent_open_error(client_id, "Project tab not found");
        };
        if tab.kind != gwt::ProjectKind::Git {
            return launch_agent_open_error(client_id, "Add Agent requires a Git project");
        }
        // SPEC-1934 US-7 / FR-034
        if tab.migration_pending {
            return launch_agent_open_error(
                client_id,
                "Complete the project migration before adding an agent",
            );
        }

        let project_root = tab.project_root.clone();
        if let Some(window_id) = self.live_agent_window_for_work(&tab_id, Some(branch_name), None) {
            return self.focus_existing_live_work_agent_events(&window_id, None);
        }
        match self.open_launch_wizard_for_branch(
            &tab_id,
            &project_root,
            branch_name,
            linked_issue_number,
            None,
        ) {
            Ok(()) => {
                if let Some(session) = self.launch_wizard_for_mut(context) {
                    session.wizard.launch_path = LaunchWizardLaunchPath::ManualSetup;
                }
                vec![self.launch_wizard_state_outbound(context)]
            }
            Err(error) => launch_agent_open_error(client_id, error),
        }
    }

    pub(crate) fn open_start_work_in_agent_kanban(
        &mut self,
        client_id: &str,
        board_id: &str,
        lane_id: gwt::AgentKanbanLane,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(board_id).cloned() else {
            return start_work_open_error(client_id, "Window not found");
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return start_work_open_error(client_id, "Project tab not found");
        };
        if tab.kind != gwt::ProjectKind::Git {
            return start_work_open_error(client_id, "Start Work requires a Git project");
        }
        if tab.migration_pending {
            return start_work_open_error(
                client_id,
                "Complete the project migration before starting work",
            );
        }
        let Some(board_window) = tab.workspace.window(&address.raw_id) else {
            return start_work_open_error(client_id, "Window not found");
        };
        if board_window.preset != gwt::WindowPreset::AgentKanban {
            return start_work_open_error(
                client_id,
                format!(
                    "Window preset {:?} is not an Agent Kanban surface",
                    board_window.preset
                ),
            );
        }

        let tab_id = address.tab_id.clone();
        let Some(context) = self.project_context(&tab_id) else {
            return Vec::new();
        };
        let project_root = tab.project_root.clone();
        match self.open_start_work_for_project(&tab_id, &project_root) {
            Ok(()) => {
                if let Some(session) = self.launch_wizard_for_mut(&context) {
                    session.agent_kanban_target = Some(AgentKanbanLaunchTarget {
                        board_id: address.raw_id,
                        lane_id,
                    });
                }
                self.activate_tab_for_launch_wizard_events(tab_id)
            }
            Err(error) => start_work_open_error(client_id, error),
        }
    }

    pub(crate) fn open_agent_kanban_launch_wizard(
        &mut self,
        client_id: &str,
        board_id: &str,
        lane_id: gwt::AgentKanbanLane,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(board_id).cloned() else {
            return launch_agent_open_error(client_id, "Window not found");
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return launch_agent_open_error(client_id, "Project tab not found");
        };
        if tab.kind != gwt::ProjectKind::Git {
            return launch_agent_open_error(client_id, "Launch Agent requires a Git project");
        }
        if tab.migration_pending {
            return launch_agent_open_error(
                client_id,
                "Complete the project migration before launching an agent",
            );
        }
        let Some(board_window) = tab.workspace.window(&address.raw_id) else {
            return launch_agent_open_error(client_id, "Window not found");
        };
        if board_window.preset != gwt::WindowPreset::AgentKanban {
            return launch_agent_open_error(
                client_id,
                format!(
                    "Window preset {:?} is not an Agent Kanban surface",
                    board_window.preset
                ),
            );
        }

        let tab_id = address.tab_id.clone();
        let Some(context) = self.project_context(&tab_id) else {
            return Vec::new();
        };
        let project_root = tab.project_root.clone();
        let branch_name = match gwt::start_work::resolve_launch_agent_base_branch(&project_root) {
            Ok(branch_name) => branch_name,
            Err(error) => return launch_agent_open_error(client_id, error),
        };
        match self.open_launch_wizard_for_branch(&tab_id, &project_root, &branch_name, None, None) {
            Ok(()) => {
                if let Some(session) = self.launch_wizard_for_mut(&context) {
                    session.agent_kanban_target = Some(AgentKanbanLaunchTarget {
                        board_id: address.raw_id,
                        lane_id,
                    });
                    session.wizard.launch_path = LaunchWizardLaunchPath::ManualSetup;
                }
                self.activate_tab_for_launch_wizard_events(tab_id)
            }
            Err(error) => launch_agent_open_error(client_id, error),
        }
    }

    fn activate_tab_for_launch_wizard_events(&mut self, tab_id: String) -> Vec<OutboundEvent> {
        let Some(context) = self.project_context(&tab_id) else {
            return Vec::new();
        };
        vec![self.launch_wizard_state_outbound(&context)]
    }

    pub(crate) fn resume_workspace_events(
        &mut self,
        context: &super::ProjectContext,
        client_id: &str,
        source: gwt::WorkspaceResumeSource,
        journal_id: Option<String>,
    ) -> Vec<OutboundEvent> {
        if !self.project_context_is_current(context) {
            return Vec::new();
        }
        // SPEC-2359 / Issue #2757: Resume click failures must surface through
        // `LaunchWizardOpenError` (a client-scoped reply) instead of the
        // legacy `ProjectOpenError` broadcast, which the frontend renders only
        // on the project picker overlay and is therefore invisible while a
        // project tab is already open.
        let error_event =
            |message: &str| vec![launch_wizard_open_error(client_id, "Resume Work", message)];

        let tab_id = context.tab_id.clone();
        let Some(tab) = self.tab(&tab_id) else {
            return error_event("Project tab not found");
        };
        if tab.kind != gwt::ProjectKind::Git {
            return error_event("Resume Work requires a Git project");
        }
        // SPEC-1934 US-7 / FR-034
        if tab.migration_pending {
            return error_event("Complete the project migration before resuming work");
        }
        let project_root = tab.project_root.clone();
        let tab_title = tab.title.clone();
        let current_sessions = self
            .active_agent_sessions
            .values()
            .filter(|session| session.tab_id == tab_id)
            .collect::<Vec<_>>();

        let (branch_candidate, resume_context) = match source {
            gwt::WorkspaceResumeSource::Current => {
                let projection =
                    gwt_core::workspace_projection::load_workspace_projection(&project_root)
                        .ok()
                        .flatten()
                        .map(|projection| {
                            workspace_projection_for_current_resume(
                                projection,
                                &current_sessions,
                                &tab_title,
                                Utc::now(),
                            )
                        });
                let branch = projection
                    .as_ref()
                    .and_then(|projection| projection.git_details.as_ref())
                    .and_then(|details| details.branch.clone());
                let context = projection
                    .as_ref()
                    .map(workspace_resume_context_from_projection)
                    .unwrap_or_else(|| WorkspaceResumeContext {
                        title: Some(format!("{tab_title} Work")),
                        owner: None,
                        summary: None,
                        next_action: None,
                    });
                (branch, context)
            }
            gwt::WorkspaceResumeSource::Journal => {
                let Some(journal_id) = journal_id else {
                    return error_event("Work journal id is required");
                };
                let Ok(entries) =
                    gwt_core::workspace_projection::load_recent_workspace_journal_entries(
                        &project_root,
                        WORKSPACE_OVERVIEW_JOURNAL_LIMIT,
                    )
                else {
                    return error_event("Work journal could not be loaded");
                };
                let Some(entry) = entries.into_iter().find(|entry| entry.id == journal_id) else {
                    return error_event("Work journal entry not found");
                };
                (
                    workspace_resume_branch_from_journal_project_root(
                        &entry.project_root,
                        &project_root,
                    ),
                    workspace_resume_context_from_journal(&entry),
                )
            }
        };

        if let Some(branch_name) = branch_candidate
            .as_deref()
            .map(normalize_branch_name)
            .filter(|branch| !branch.trim().is_empty())
        {
            if workspace_resume_branch_exists(&project_root, &branch_name) {
                if let Some(window_id) =
                    self.live_agent_window_for_work(&tab_id, Some(&branch_name), None)
                {
                    return self.focus_existing_live_work_agent_events(&window_id, None);
                }
                let linked_issue_number =
                    workspace_resume_owner_issue_number(resume_context.owner.as_deref());
                return match self.open_launch_wizard_for_branch_with_context(
                    &tab_id,
                    &project_root,
                    &branch_name,
                    linked_issue_number,
                    None,
                    Some(resume_context),
                ) {
                    Ok(()) => vec![self.launch_wizard_state_outbound(context)],
                    Err(error) => error_event(&error),
                };
            }
        }

        match self.open_start_work_for_project_with_context(
            &tab_id,
            &project_root,
            Some(resume_context),
        ) {
            Ok(()) => vec![self.launch_wizard_state_outbound(context)],
            Err(error) => error_event(&error),
        }
    }

    // SPEC-2359 US-42: list / resume entries for the Workspace Resume
    // picker. These bypass the Launch Wizard entirely so the Resume
    // button can restart a previously-assigned agent in-place.

    pub(crate) fn list_resumable_agents_events(
        &mut self,
        context: &super::ProjectContext,
        client_id: &str,
        operation_id: String,
        workspace_id: Option<String>,
    ) -> Vec<OutboundEvent> {
        if !self.project_context_is_current(context) {
            return Vec::new();
        }
        let agents = self.collect_resumable_agents(context, workspace_id.as_deref());
        vec![OutboundEvent::reply(
            client_id.to_string(),
            BackendEvent::WorkspaceResumableAgents {
                operation_id,
                agents,
                workspace_id,
            },
        )]
    }

    pub(crate) fn resume_workspace_agent_events(
        &mut self,
        context: &super::ProjectContext,
        client_id: &str,
        operation_id: String,
        session_id: String,
        agent_session_id: Option<String>,
        bounds: WindowGeometry,
    ) -> Vec<OutboundEvent> {
        if !self.project_context_is_current(context) {
            return Vec::new();
        }
        let reply_error = |message: String| {
            vec![OutboundEvent::reply(
                client_id.to_string(),
                BackendEvent::WorkspaceResumeAgentError {
                    operation_id: operation_id.clone(),
                    session_id: session_id.clone(),
                    message,
                },
            )]
        };
        // SPEC-2359 W-17 (FR-398): client-scoped ack so the requesting
        // frontend can settle its pending Resume UI deterministically.
        let started_ack = |session_id: &str, branch: Option<String>| {
            OutboundEvent::reply(
                client_id.to_string(),
                BackendEvent::WorkspaceResumeAgentStarted {
                    operation_id: operation_id.clone(),
                    session_id: session_id.to_string(),
                    branch,
                },
            )
        };

        let tab_id = context.tab_id.clone();
        let Some(tab) = self.tab(&tab_id) else {
            return reply_error("Project tab not found".to_string());
        };
        if tab.kind != gwt::ProjectKind::Git {
            return reply_error("Resume requires a Git project".to_string());
        }
        if tab.migration_pending {
            return reply_error(
                "Complete the project migration before resuming an agent".to_string(),
            );
        }

        // Linked Resume is an authority-producing continuation. Route it
        // through the same coordinator as the Workspace Continue action so
        // the resumed pane is either rebound to the current generation or
        // launched with one Prepared successor. Legacy unlinked Sessions keep
        // the observation-only resume path below.
        let linked_session = gwt_agent::Session::load_and_migrate(
            &self.sessions_dir.join(format!("{session_id}.toml")),
        )
        .ok()
        .filter(|session| session.linked_issue_number.is_some());
        if let Some(linked_session) = linked_session {
            if agent_session_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_some_and(|requested| {
                    linked_session
                        .resume_session_id_for(Some(requested))
                        .as_deref()
                        != Some(requested)
                })
            {
                return reply_error(
                    "The requested Session id is not resumable; use the Work Resume picker or start a new Work.".to_string(),
                );
            }
            let work_id =
                gwt_core::workspace_projection::load_workspace_work_items(&tab.project_root)
                    .ok()
                    .flatten()
                    .and_then(|projection| {
                        projection
                            .work_items
                            .into_iter()
                            .find(|work| {
                                work.agents
                                    .iter()
                                    .any(|agent| agent.session_id == linked_session.id)
                            })
                            .map(|work| work.id)
                    })
                    .unwrap_or_else(|| format!("work-session-{}", linked_session.id));
            let branch =
                (!linked_session.branch.trim().is_empty()).then(|| linked_session.branch.clone());
            let mut events = self.continue_work_events(
                context,
                client_id,
                operation_id.clone(),
                work_id,
                bounds,
            );
            let failure = events.iter().find_map(|outbound| match &outbound.event {
                BackendEvent::ContinueWorkOutcome {
                    outcome: gwt::ContinueWorkOutcomeKind::Failed,
                    message,
                    ..
                } => Some(
                    message
                        .clone()
                        .unwrap_or_else(|| "Resume continuation failed".to_string()),
                ),
                _ => None,
            });
            if let Some(message) = failure {
                return reply_error(message);
            }
            events.push(started_ack(&session_id, branch));
            return events;
        }

        if let Some((window_id, live_gwt_session)) = self
            .active_agent_sessions
            .iter()
            .find(|(window_id, session)| {
                session.session_id == session_id
                    && session.tab_id == tab_id
                    && self.window_lookup.contains_key(window_id.as_str())
                    && self
                        .window_status(window_id.as_str())
                        .is_some_and(|status| {
                            !matches!(
                                status,
                                WindowProcessStatus::Stopped | WindowProcessStatus::Error
                            )
                        })
            })
            .map(|(window_id, session)| (window_id.clone(), session.session_id.clone()))
        {
            // SPEC-2359 D1: focusing a live window resumes the conversation it is
            // already running. If the user clicked Resume on a *different* (older)
            // Session, focusing would silently drop that intent — surface a
            // visible error instead of pretending the request succeeded.
            if self.resume_conversation_conflicts(agent_session_id.as_deref(), &live_gwt_session) {
                return reply_error(
                    "This Work is currently running a different conversation; stop it before resuming an older Session.".to_string(),
                );
            }
            let mut events = self.focus_existing_live_work_agent_events(&window_id, Some(bounds));
            events.push(started_ack(&session_id, None));
            return events;
        }

        let project_root = tab.project_root.clone();
        let sessions_dir = self.sessions_dir.clone();
        let session_path = sessions_dir.join(format!("{session_id}.toml"));
        let session = match gwt_agent::Session::load_and_migrate(&session_path) {
            Ok(session) => session,
            Err(_) => {
                return reply_error(
                    "Session metadata is missing; restart via Start Work or Launch Agent."
                        .to_string(),
                );
            }
        };
        if let Some(window_id) = self.live_agent_window_for_work(
            &tab_id,
            (!session.branch.trim().is_empty()).then_some(session.branch.as_str()),
            Some(session.worktree_path.as_path()),
        ) {
            // D1: the matched live window may belong to a *different* Work on the
            // same branch/worktree. Resolve its gwt session id and apply the same
            // conversation-conflict guard before focusing.
            let live_gwt_session = self
                .active_agent_sessions
                .iter()
                .find(|(candidate, _)| candidate.as_str() == window_id.as_str())
                .map(|(_, live)| live.session_id.clone());
            if let Some(live_gwt_session) = live_gwt_session {
                if self
                    .resume_conversation_conflicts(agent_session_id.as_deref(), &live_gwt_session)
                {
                    return reply_error(
                    "This Work is currently running a different conversation; stop it before resuming an older Session.".to_string(),
                );
                }
            }
            let mut events = self.focus_existing_live_work_agent_events(&window_id, Some(bounds));
            events.push(started_ack(
                &session_id,
                (!session.branch.trim().is_empty()).then(|| session.branch.clone()),
            ));
            return events;
        }

        let session_worktree_exists = session.worktree_path.as_path().exists();
        if !session_exact_resume_materializable(&project_root, &session) {
            return reply_error(
                "This Session cannot be resumed on this machine because its branch is no longer available; use Workspace Continue or Launch Agent to start a new Work.".to_string(),
            );
        }

        // Build a fresh LaunchConfig from the persisted Session and add the
        // resume_session_id only when the agent CLI captured a previous
        // conversation handle (Claude / Codex / opt-in custom agents).
        let agent_id = session.agent_id.clone();
        let mut builder = gwt_agent::AgentLaunchBuilder::new(agent_id.clone());
        if session_worktree_exists {
            builder = builder.working_dir(session.worktree_path.clone());
        }
        if !session.branch.is_empty() {
            builder = builder.branch(session.branch.clone());
        }
        if let Some(model) = session.model.clone() {
            builder = builder.model(model);
        }
        if let Some(level) = session.reasoning_level.clone() {
            builder = builder.reasoning_level(level);
        }
        if session.fast_mode_enabled() {
            builder = builder.fast_mode(true);
        }
        builder = builder.runtime_target(session.runtime_target);
        if let Some(service) = session.docker_service.clone() {
            builder = builder.docker_service(service);
        }
        builder = builder.docker_lifecycle_intent(session.docker_lifecycle_intent);
        if let Some(shell) = session.windows_shell {
            builder = builder.windows_shell(shell);
        }
        if let Some(linked) = session.linked_issue_number {
            builder = builder.linked_issue_number(linked);
        }

        // Resume the specific Session (conversation UUID) the user clicked when
        // one was requested; otherwise resume the Work's latest conversation.
        if let Some(resume_id) = session.resume_session_id_for(agent_session_id.as_deref()) {
            builder = builder
                .session_mode(gwt_agent::SessionMode::Resume)
                .resume_session_id(resume_id);
        } else if agent_session_id
            .as_deref()
            .is_some_and(|id| !id.trim().is_empty())
        {
            return reply_error(
                "The requested Session id is not resumable; use the Work Resume picker or start a new Work.".to_string(),
            );
        } else if session.agent_id.supports_resume_picker() {
            builder = builder.session_mode(gwt_agent::SessionMode::Resume);
        } else if session.agent_id.supports_continue_latest() {
            builder = builder.session_mode(gwt_agent::SessionMode::Continue);
        } else {
            return reply_error(
                "No saved conversation is available for this Session; use Continue work to start a linked execution with handoff context.".to_string(),
            );
        }

        let mut config = builder.build();
        // Preserve the display name when resuming.
        if !session.display_name.is_empty() {
            config.display_name = session.display_name.clone();
        }

        // Build a Workspace Resume context so the spawned window's title
        // and the Workspace projection summary keep the prior identity
        // instead of falling back to the agent's default display name.
        // #3065: the context comes from the resumed branch's own Work item,
        // never from the repo-shared current projection.
        let workspace_resume_context = Some(workspace_resume_context_for_work_item(
            &project_root,
            Some(session.branch.as_str()),
            &session.worktree_path,
        ));

        match self.spawn_agent_window(&tab_id, config, bounds, workspace_resume_context) {
            Ok(mut events) => {
                events.push(started_ack(
                    &session_id,
                    (!session.branch.trim().is_empty()).then(|| session.branch.clone()),
                ));
                events
            }
            Err(error) => reply_error(error),
        }
    }

    /// SPEC-2359 D1: true when a specific conversation was requested for resume
    /// but the live window a focus would land on is running a *different*
    /// conversation. `live_gwt_session_id` is the gwt session id (Work) owning
    /// that live window; its Session TOML's latest `agent_session_id` is the
    /// conversation it is currently running. Returns false when no specific
    /// conversation was requested (a plain Work resume is satisfied by focus).
    fn resume_conversation_conflicts(
        &self,
        requested: Option<&str>,
        live_gwt_session_id: &str,
    ) -> bool {
        let Some(requested) = requested.map(str::trim).filter(|value| !value.is_empty()) else {
            return false;
        };
        let live_conversation = {
            let path = self
                .sessions_dir
                .join(format!("{live_gwt_session_id}.toml"));
            gwt_agent::Session::load_and_migrate(&path)
                .ok()
                .and_then(|session| session.agent_session_id)
        };
        live_conversation.as_deref().map(str::trim) != Some(requested)
    }

    pub(crate) fn resume_branch_latest_agent_events(
        &mut self,
        client_id: &str,
        id: &str,
        branch_name: &str,
        bounds: WindowGeometry,
    ) -> Vec<OutboundEvent> {
        let branch_error = |message: String| {
            vec![OutboundEvent::reply(
                client_id.to_string(),
                BackendEvent::BranchError {
                    id: id.to_string(),
                    message,
                },
            )]
        };

        let Some(address) = self.window_lookup.get(id).cloned() else {
            return branch_error("Window not found".to_string());
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return branch_error("Project tab not found".to_string());
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return branch_error("Window not found".to_string());
        };
        if window.preset != WindowPreset::Branches && window.preset != WindowPreset::Work {
            tracing::warn!(
                preset = ?window.preset,
                window_id = id,
                "resume_branch_latest_agent rejected: wrong preset"
            );
            return branch_error(format!(
                "Window preset {:?} is not a Work surface",
                window.preset
            ));
        }
        if tab.kind != gwt::ProjectKind::Git {
            return branch_error("Resume requires a Git project".to_string());
        }
        if tab.migration_pending {
            return branch_error(
                "Complete the project migration before resuming an agent".to_string(),
            );
        }

        let tab_id = address.tab_id.clone();
        let Some(context) = self.project_context(&tab_id) else {
            return Vec::new();
        };
        let project_root = tab.project_root.clone();
        let normalized_branch_name = normalize_branch_name(branch_name);
        // SPEC-2359 W-17 (FR-398): client-scoped ack so the requesting
        // frontend can settle its pending Resume UI deterministically.
        let started_ack = |session_id: String, branch: String| {
            OutboundEvent::reply(
                client_id.to_string(),
                BackendEvent::WorkspaceResumeAgentStarted {
                    operation_id: String::new(),
                    session_id,
                    branch: Some(branch),
                },
            )
        };
        if let Some(window_id) =
            self.live_agent_window_for_work(&tab_id, Some(&normalized_branch_name), None)
        {
            let live_session_id = self
                .active_agent_sessions
                .get(&window_id)
                .map(|session| session.session_id.clone())
                .unwrap_or_default();
            let mut events =
                self.focus_existing_live_work_agent_events(&window_id, Some(bounds.clone()));
            events.push(started_ack(live_session_id, normalized_branch_name.clone()));
            return events;
        }
        let Some(session) =
            self.latest_resumable_branch_session(&project_root, &normalized_branch_name)
        else {
            return branch_error(format!(
                "No resumable session found for {normalized_branch_name}"
            ));
        };

        if let Some(window_id) = self
            .active_agent_sessions
            .iter()
            .find(|(_, active)| active.session_id == session.id)
            .map(|(window_id, _)| window_id.clone())
        {
            if !self.window_lookup.contains_key(&window_id) {
                return branch_error(format!("Agent window not found for session {}", session.id));
            }
            let mut events = self.focus_window_events(&window_id, Some(bounds));
            if events.is_empty() {
                events.push(self.workspace_state_broadcast(&context));
            }
            events.push(started_ack(
                session.id.clone(),
                normalized_branch_name.clone(),
            ));
            return events;
        }

        if !session_exact_resume_materializable(&project_root, &session) {
            return branch_error(format!(
                "No resumable session found for {normalized_branch_name}"
            ));
        }
        let mut config = super::launch_config_from_persisted_session(&session);
        config.launch_route = gwt_agent::LaunchRoute::Manual;
        if !session.worktree_path.as_path().exists() {
            config.working_dir = None;
        }
        if config.session_mode != gwt_agent::SessionMode::Resume {
            return branch_error(format!(
                "No resumable session found for {normalized_branch_name}"
            ));
        }
        // #3065: the context comes from the resumed branch's own Work item,
        // never from the repo-shared current projection.
        let workspace_resume_context = Some(workspace_resume_context_for_work_item(
            &project_root,
            Some(session.branch.as_str()),
            &session.worktree_path,
        ));

        match self.spawn_agent_window(&tab_id, config, bounds, workspace_resume_context) {
            Ok(mut events) => {
                events.push(started_ack(
                    session.id.clone(),
                    normalized_branch_name.clone(),
                ));
                events
            }
            Err(error) => branch_error(error),
        }
    }

    /// Build a list of agents that the Workspace Resume picker can offer
    /// for the currently-active Git project tab. Includes live agents with
    /// `lifecycle_status = Running` so the picker can show them and focus
    /// their window on click. Non-live entries require a backing Session
    /// toml on disk.
    fn collect_resumable_agents(
        &self,
        context: &super::ProjectContext,
        workspace_id: Option<&str>,
    ) -> Vec<gwt::ResumableAgentView> {
        let tab_id = context.tab_id.as_str();
        let Some(tab) = self.tab(tab_id) else {
            return Vec::new();
        };
        if tab.kind != gwt::ProjectKind::Git {
            return Vec::new();
        }
        let live_session_ids: std::collections::HashSet<&str> = self
            .active_agent_sessions
            .values()
            .filter(|session| session.tab_id == tab_id)
            .map(|session| session.session_id.as_str())
            .collect();

        let project_root = tab.project_root.clone();
        let Ok(Some(projection)) =
            gwt_core::workspace_projection::load_workspace_projection(&project_root)
        else {
            return Vec::new();
        };

        let sessions_dir = self.sessions_dir.clone();

        let workspace_work_item = workspace_id.and_then(|wid| {
            gwt_core::workspace_projection::load_workspace_work_items(&project_root)
                .ok()
                .flatten()
                .and_then(|items| items.work_items.into_iter().find(|item| item.id == wid))
        });
        let work_item_session_ids: Option<std::collections::HashSet<String>> =
            workspace_work_item.as_ref().map(|item| {
                item.agents
                    .iter()
                    .map(|agent| agent.session_id.clone())
                    .collect()
            });
        let work_item_branch = workspace_work_item.as_ref().and_then(|item| {
            item.execution_containers
                .iter()
                .filter_map(|container| container.branch.as_deref())
                .map(str::trim)
                .find(|branch| !branch.is_empty())
                .map(str::to_string)
        });

        let resume_kind_for_session = |session: &gwt_agent::Session| {
            if session.exact_resume_session_id().is_some() {
                gwt::ResumableAgentResumeKind::Session
            } else if session.agent_id.supports_resume_picker() {
                gwt::ResumableAgentResumeKind::NativePicker
            } else {
                gwt::ResumableAgentResumeKind::MetadataOnly
            }
        };
        let lifecycle_status_for_session = |session: &gwt_agent::Session| {
            if session.should_mark_interrupted_from_lifecycle()
                || session.status == gwt_agent::AgentStatus::Interrupted
            {
                Some(gwt::ResumableAgentLifecycleStatus::Interrupted)
            } else if session.exact_auto_resume_candidate() {
                Some(gwt::ResumableAgentLifecycleStatus::Active)
            } else {
                None
            }
        };

        let mut entries: Vec<gwt::ResumableAgentView> = projection
            .agents
            .iter()
            .filter(|agent| !agent.session_id.trim().is_empty())
            .filter(|agent| match &work_item_session_ids {
                Some(ids) => ids.contains(&agent.session_id),
                None => true,
            })
            .filter_map(|agent| {
                let is_live = live_session_ids.contains(agent.session_id.as_str());
                let (resume_kind, lifecycle_status) = if is_live {
                    (
                        gwt::ResumableAgentResumeKind::Session,
                        Some(gwt::ResumableAgentLifecycleStatus::Running),
                    )
                } else {
                    let session_path = sessions_dir.join(format!("{}.toml", agent.session_id));
                    match gwt_agent::Session::load_and_migrate(&session_path) {
                        Ok(session) => {
                            if !session_exact_resume_materializable(&project_root, &session) {
                                return None;
                            }
                            (
                                resume_kind_for_session(&session),
                                lifecycle_status_for_session(&session),
                            )
                        }
                        Err(_) => return None,
                    }
                };
                Some(gwt::ResumableAgentView {
                    session_id: agent.session_id.clone(),
                    agent_id: agent.agent_id.clone(),
                    display_name: agent.display_name.clone(),
                    branch: agent.branch.clone(),
                    worktree_path: agent
                        .worktree_path
                        .as_ref()
                        .map(|path| path.display().to_string()),
                    last_activity_at: Some(agent.updated_at.to_rfc3339()),
                    resume_kind,
                    lifecycle_status,
                })
            })
            .collect();

        if let Some(branch) = work_item_branch.as_deref() {
            let agent_sessions = self
                .session_ledger_cache
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .load(&self.sessions_dir);
            let project_repo_hash = gwt_core::repo_hash::detect_repo_hash(&project_root);
            let registry = crate::workspace_session_registry::branch_session_registry(
                &agent_sessions,
                project_repo_hash.as_ref().map(|hash| hash.as_str()),
            );
            let existing_session_ids: Vec<&str> = entries
                .iter()
                .map(|entry| entry.session_id.as_str())
                .collect();
            let (branch_sessions, _) =
                crate::workspace_session_registry::registry_sessions_for_branch(
                    &registry,
                    Some(branch),
                    &existing_session_ids,
                    crate::workspace_session_registry::REGISTRY_SESSION_CAP,
                );
            for session in branch_sessions {
                if !session_exact_resume_materializable(&project_root, session) {
                    continue;
                }
                if entries.iter().any(|entry| entry.session_id == session.id) {
                    continue;
                }
                entries.push(gwt::ResumableAgentView {
                    session_id: session.id.clone(),
                    agent_id: session.agent_id.command().to_string(),
                    display_name: session.display_name.clone(),
                    branch: (!session.branch.trim().is_empty()).then(|| session.branch.clone()),
                    worktree_path: Some(session.worktree_path.display().to_string()),
                    last_activity_at: Some(
                        session
                            .last_activity_at
                            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                    ),
                    resume_kind: resume_kind_for_session(session),
                    lifecycle_status: lifecycle_status_for_session(session),
                });
            }
        }

        entries.sort_by(|left, right| right.last_activity_at.cmp(&left.last_activity_at));
        entries
    }

    pub(crate) fn open_start_work_for_project(
        &mut self,
        tab_id: &str,
        project_root: &Path,
    ) -> Result<(), String> {
        self.open_start_work_for_project_with_context(tab_id, project_root, None)
    }

    pub(crate) fn open_start_work_for_project_with_context(
        &mut self,
        tab_id: &str,
        project_root: &Path,
        workspace_resume_context: Option<WorkspaceResumeContext>,
    ) -> Result<(), String> {
        let context = self
            .project_context(tab_id)
            .ok_or_else(|| "Project tab not found".to_string())?;
        let base_branch = gwt::start_work::START_WORK_BASE_BRANCH_CANDIDATES[0].to_string();
        let work_branch =
            gwt::start_work::reserve_start_work_branch_name_for_project(project_root, Utc::now())
                .map_err(|error| error.to_string())?;
        let quick_start_root = project_root.to_path_buf();
        let quick_start_entries = Vec::new();
        let previous_profiles = self.launch_wizard_cache.agent_preferences();
        let agent_options = self.launch_wizard_cache.agent_options();
        let docker_context = None;
        let docker_service_status = gwt_docker::ComposeServiceStatus::NotFound;
        let wizard_id = Uuid::new_v4().to_string();
        let mut wizard = LaunchWizardState::open_start_work_with_previous_profiles(
            LaunchWizardContext {
                selected_branch: synthetic_branch_entry(&base_branch),
                normalized_branch_name: work_branch,
                worktree_path: None,
                quick_start_root,
                live_sessions: Vec::new(),
                docker_context,
                docker_service_status,
                linked_issue_number: workspace_resume_context.as_ref().and_then(|context| {
                    workspace_resume_owner_issue_number(context.owner.as_deref())
                }),
                linked_issue_kind: None,
                ultracode_supported: self.launch_wizard_cache.claude_ultracode_supported(),
                claude_workflows_enabled: self.launch_wizard_cache.claude_workflows_enabled(),
            },
            base_branch,
            agent_options,
            quick_start_entries,
            previous_profiles,
        );
        Self::apply_agent_configuration_state(&mut wizard);
        wizard.mark_runtime_context_unresolved();
        self.store_launch_wizard(LaunchWizardSession {
            project_context: context.clone(),
            tab_id: tab_id.to_string(),
            wizard_id: wizard_id.clone(),
            wizard,
            workspace_resume_context,
            agent_kanban_target: None,
            auto_submit_after_runtime_resolution: None,
            issue_monitor_profile_save: None,
            issue_monitor_launch_issue_number: None,
            origin: super::LaunchWizardOrigin::StartWork,
            manual_holder_intent: None,
        });

        // SPEC-2359 US-83 / FR-444 + FR-445: populate the "open existing branch"
        // picker off the UI thread. Fetch origin first so a teammate's freshly
        // pushed branch appears even when the Branches tab was never opened, then
        // push the candidates into the live wizard via a typed event.
        let proxy = self.proxy.clone();
        let candidates_root = project_root.to_path_buf();
        let active_session_branches = self.active_session_branches_for_tab(tab_id);
        thread::spawn(move || {
            if let Ok(git_root) = gwt_git::worktree::main_worktree_root(&candidates_root) {
                let _ = gwt_git::WorktreeManager::new(&git_root).fetch_origin();
            }
            let candidates = list_branch_entries_with_active_sessions(
                &candidates_root,
                &active_session_branches,
            )
            .map(|entries| {
                gwt::branch_list::eligible_remote_start_work_branch_names(
                    &entries,
                    &active_session_branches,
                )
            })
            .unwrap_or_default();
            proxy.send(UserEvent::RefreshLaunchWizardBranchCandidates {
                wizard_id,
                candidates,
            });
        });

        Ok(())
    }

    pub(crate) fn open_issue_launch_wizard_events(
        &mut self,
        client_id: &str,
        id: &str,
        issue_number: u64,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id).cloned() else {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_error_event(id, KnowledgeKind::Issue, "Window not found", None, None),
            )];
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_error_event(
                    id,
                    KnowledgeKind::Issue,
                    "Project tab not found",
                    None,
                    None,
                ),
            )];
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_error_event(id, KnowledgeKind::Issue, "Window not found", None, None),
            )];
        };
        let Some(kind) = knowledge_kind_for_preset(window.preset) else {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_error_event(
                    id,
                    KnowledgeKind::Issue,
                    "Window is not a knowledge bridge",
                    None,
                    None,
                ),
            )];
        };
        // SPEC-1934 US-7 / FR-034
        if tab.migration_pending {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_error_event(
                    id,
                    KnowledgeKind::Issue,
                    "Complete the project migration before launching from an Issue",
                    None,
                    None,
                ),
            )];
        }

        let project_root = tab.project_root.clone();
        let tab_id = address.tab_id.clone();
        let Some(project_context) = self.project_context(&tab_id) else {
            return Vec::new();
        };
        let proxy = self.proxy.clone();
        let client_id = client_id.to_string();
        let id = id.to_string();
        let active_session_branches = self.active_session_branches_for_tab(&address.tab_id);
        thread::spawn(move || {
            let result =
                list_branch_entries_with_active_sessions(&project_root, &active_session_branches)
                    .map_err(|error| error.to_string())
                    .and_then(|entries| {
                        preferred_issue_launch_branch(&entries)
                            .ok_or_else(|| "No local branch is available for launch".to_string())
                    });
            proxy.send(UserEvent::IssueLaunchWizardPrepared(
                IssueLaunchWizardPrepared {
                    project_context,
                    client_id,
                    id,
                    knowledge_kind: kind,
                    tab_id,
                    project_root,
                    issue_number,
                    result,
                },
            ));
        });
        Vec::new()
    }

    pub(crate) fn open_issue_monitor_launch_wizard_events(
        &mut self,
        context: &super::ProjectContext,
        client_id: &str,
        issue_number: u64,
        linked_issue_kind: gwt::LinkedIssueKind,
    ) -> Vec<OutboundEvent> {
        if !self.project_context_is_current(context) {
            return Vec::new();
        }
        let project_root = context.project_root.clone();
        self.open_issue_monitor_launch_wizard_events_for_project(
            client_id,
            &project_root,
            issue_number,
            linked_issue_kind,
        )
    }

    fn open_issue_monitor_launch_wizard_events_for_project(
        &mut self,
        client_id: &str,
        project_root: &Path,
        issue_number: u64,
        linked_issue_kind: gwt::LinkedIssueKind,
    ) -> Vec<OutboundEvent> {
        let Some(tab_id) = self.issue_monitor_tab_id_for_project_root(project_root) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::IssueMonitorToast {
                    notification_transition: None,
                    level: "error".to_string(),
                    message: "Project tab not found".to_string(),
                    issue_number: Some(issue_number),
                },
            )
            .with_error_project_root(project_root)];
        };
        let Some(context) = self.project_context(&tab_id) else {
            return Vec::new();
        };
        let Some(tab) = self.tab(&tab_id) else {
            return Vec::new();
        };
        if tab.kind != gwt::ProjectKind::Git {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::IssueMonitorToast {
                    notification_transition: None,
                    level: "error".to_string(),
                    message: "Issue Monitor launch requires a Git project".to_string(),
                    issue_number: Some(issue_number),
                },
            )
            .with_error_project_root(project_root)];
        }
        if tab.migration_pending {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::IssueMonitorToast {
                    notification_transition: None,
                    level: "error".to_string(),
                    message: "Complete the project migration before launching monitored Issue work"
                        .to_string(),
                    issue_number: Some(issue_number),
                },
            )
            .with_error_project_root(project_root)];
        }

        let project_root = tab.project_root.clone();
        let base_branch_name =
            match gwt::start_work::resolve_launch_agent_base_branch(&project_root) {
                Ok(branch) => branch,
                Err(error) => {
                    return vec![OutboundEvent::reply(
                        client_id,
                        BackendEvent::IssueMonitorToast {
                            notification_transition: None,
                            level: "error".to_string(),
                            message: error,
                            issue_number: Some(issue_number),
                        },
                    )
                    .with_error_project_root(&project_root)];
                }
            };
        let previous_profiles = self.issue_monitor_previous_profiles(&project_root);
        match self.open_knowledge_launch_wizard_for_base_branch_with_previous_profiles(
            &tab_id,
            &project_root,
            &base_branch_name,
            issue_number,
            linked_issue_kind,
            previous_profiles,
        ) {
            Ok(()) => {
                if let Some(session) = self.launch_wizard_for_mut(&context) {
                    session.issue_monitor_launch_issue_number = Some(issue_number);
                    session.origin = super::LaunchWizardOrigin::IssueMonitor;
                    session
                        .wizard
                        .apply(gwt::LaunchWizardAction::SetInitialPrompt {
                            value: gwt::issue_monitor_launch_prompt(
                                linked_issue_kind,
                                issue_number,
                            ),
                        });
                }
                vec![
                    OutboundEvent::reply(
                        client_id,
                        BackendEvent::IssueMonitorToast {
                            notification_transition: None,
                            level: "info".to_string(),
                            message: "Issue Monitor launch prepared".to_string(),
                            issue_number: Some(issue_number),
                        },
                    ),
                    self.launch_wizard_state_outbound(&context),
                ]
            }
            Err(error) => vec![OutboundEvent::reply(
                client_id,
                BackendEvent::IssueMonitorToast {
                    notification_transition: None,
                    level: "error".to_string(),
                    message: error,
                    issue_number: Some(issue_number),
                },
            )
            .with_error_project_root(&project_root)],
        }
    }

    pub(crate) fn open_issue_monitor_configure_wizard_events(
        &mut self,
        context: &super::ProjectContext,
        client_id: &str,
        issue_number: u64,
        linked_issue_kind: gwt::LinkedIssueKind,
    ) -> Vec<OutboundEvent> {
        if !self.project_context_is_current(context) {
            return Vec::new();
        }
        let project_root = context.project_root.clone();
        self.open_issue_monitor_configure_wizard_events_for_project(
            client_id,
            &project_root,
            issue_number,
            linked_issue_kind,
        )
    }

    fn open_issue_monitor_configure_wizard_events_for_project(
        &mut self,
        client_id: &str,
        project_root: &Path,
        issue_number: u64,
        linked_issue_kind: gwt::LinkedIssueKind,
    ) -> Vec<OutboundEvent> {
        let Some(context) = self
            .issue_monitor_tab_id_for_project_root(project_root)
            .and_then(|id| self.project_context(&id))
        else {
            return Vec::new();
        };
        let events = self.open_issue_monitor_launch_wizard_events_for_project(
            client_id,
            project_root,
            issue_number,
            linked_issue_kind,
        );
        if !matches!(
            self.launch_wizard_for_mut(&context),
            Some(LaunchWizardSession {
                wizard: _,
                issue_monitor_profile_save: None,
                ..
            })
        ) {
            return events;
        }
        let pool = self.issue_monitor_saved_pool(project_root);
        if let Some(session) = self.launch_wizard_for_mut(&context) {
            session.issue_monitor_profile_save = Some(IssueMonitorProfileSaveContext {
                client_id: client_id.to_string(),
                issue_number: Some(issue_number),
                pool,
                sets: None,
            });
            session
                .wizard
                .apply(gwt::LaunchWizardAction::UseStartMethod {
                    method: gwt::LaunchWizardStartMethodKind::ConfigureAndStart,
                });
        }
        events
            .into_iter()
            .map(|mut event| {
                if let BackendEvent::IssueMonitorToast { message, .. } = &mut event.event {
                    if message == "Issue Monitor launch prepared" {
                        *message = "Issue Monitor settings opened".to_string();
                    }
                }
                if matches!(event.event, BackendEvent::LaunchWizardState { .. }) {
                    event = self.launch_wizard_state_outbound(&context);
                }
                event
            })
            .collect()
    }

    pub(crate) fn open_issue_monitor_configure_profile_wizard_events(
        &mut self,
        context: &super::ProjectContext,
        client_id: &str,
    ) -> Vec<OutboundEvent> {
        if !self.project_context_is_current(context) {
            return Vec::new();
        }
        let tab_id = context.tab_id.clone();
        let Some(tab) = self.tab(&tab_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::IssueMonitorToast {
                    notification_transition: None,
                    level: "error".to_string(),
                    message: "Project tab not found".to_string(),
                    issue_number: None,
                },
            )
            .with_error_project_root(&context.project_root)];
        };
        if tab.kind != gwt::ProjectKind::Git {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::IssueMonitorToast {
                    notification_transition: None,
                    level: "error".to_string(),
                    message: "Issue Monitor settings require a Git project".to_string(),
                    issue_number: None,
                },
            )
            .with_error_project_root(&context.project_root)];
        }
        if tab.migration_pending {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::IssueMonitorToast {
                    notification_transition: None,
                    level: "error".to_string(),
                    message:
                        "Complete the project migration before configuring Issue Monitor settings"
                            .to_string(),
                    issue_number: None,
                },
            )
            .with_error_project_root(&context.project_root)];
        }

        let project_root = tab.project_root.clone();
        // Issue #4366 AC-6: the settings form shows what the operator saved.
        // The launch choice skips held providers, and pre-filling from it made
        // a hold look like the saved agent had changed — and saving the form
        // unchanged wrote the fallback over the head.
        let previous_profiles = self.issue_monitor_saved_head_profiles(&project_root);
        let pool = self.issue_monitor_saved_pool(&project_root);
        let wizard_id = Uuid::new_v4().to_string();
        let wizard = self.issue_monitor_settings_wizard(project_root, previous_profiles);
        // Issue #4911: every saved candidate is one Agent Settings set and the
        // form opens on the first. A pool that was never configured starts as
        // the one set the form shows.
        let profiles = if pool.is_empty() {
            wizard
                .preview_launch_profile()
                .or_else(|| new_agent_settings_profile(&[], &wizard.agent_id))
                .into_iter()
                .collect()
        } else {
            pool.clone()
        };
        let sets = (!profiles.is_empty()).then(|| IssueMonitorAgentSettingsSets {
            profiles,
            active: 0,
            opened_as: wizard.preview_launch_profile(),
        });
        self.store_launch_wizard(LaunchWizardSession {
            project_context: context.clone(),
            tab_id: tab_id.to_string(),
            wizard_id,
            wizard,
            workspace_resume_context: None,
            agent_kanban_target: None,
            auto_submit_after_runtime_resolution: None,
            issue_monitor_profile_save: Some(IssueMonitorProfileSaveContext {
                client_id: client_id.to_string(),
                issue_number: None,
                pool,
                sets,
            }),
            issue_monitor_launch_issue_number: None,
            origin: super::LaunchWizardOrigin::IssueMonitor,
            manual_holder_intent: None,
        });

        vec![
            OutboundEvent::reply(
                client_id,
                BackendEvent::IssueMonitorToast {
                    notification_transition: None,
                    level: "info".to_string(),
                    message: "Issue Monitor settings opened".to_string(),
                    issue_number: None,
                },
            ),
            self.launch_wizard_state_outbound(context),
        ]
    }

    /// The Issue Monitor settings form, pre-filled from one saved profile. It
    /// configures a launch profile rather than a launch, so it has no branch,
    /// session or Docker context of its own.
    fn issue_monitor_settings_wizard(
        &self,
        project_root: PathBuf,
        previous_profiles: gwt::LaunchWizardPreviousProfiles,
    ) -> LaunchWizardState {
        let saved_agent_id = previous_profiles.preferred_agent_id().map(str::to_string);
        let base_branch_name = gwt::start_work::START_WORK_BASE_BRANCH_CANDIDATES[0].to_string();
        let mut wizard = LaunchWizardState::open_start_work_with_previous_profiles(
            LaunchWizardContext {
                selected_branch: synthetic_branch_entry(&base_branch_name),
                normalized_branch_name: normalize_branch_name(&base_branch_name),
                worktree_path: None,
                quick_start_root: project_root,
                live_sessions: Vec::new(),
                docker_context: None,
                docker_service_status: gwt_docker::ComposeServiceStatus::NotFound,
                linked_issue_number: None,
                linked_issue_kind: None,
                ultracode_supported: self.launch_wizard_cache.claude_ultracode_supported(),
                claude_workflows_enabled: self.launch_wizard_cache.claude_workflows_enabled(),
            },
            base_branch_name,
            self.launch_wizard_cache.agent_options(),
            Vec::new(),
            previous_profiles,
        );
        Self::apply_agent_configuration_state(&mut wizard);
        wizard.mark_runtime_context_unresolved();
        wizard.apply(gwt::LaunchWizardAction::UseStartMethod {
            method: gwt::LaunchWizardStartMethodKind::ConfigureAndStart,
        });
        // D4: an undetected saved agent must not become an implicit pool edit.
        // Keep its identity so validation requires an explicit replacement.
        if let Some(saved_agent_id) = saved_agent_id {
            wizard.agent_id = saved_agent_id;
        }
        wizard
    }

    pub(super) fn issue_monitor_previous_profiles(
        &self,
        project_root: &Path,
    ) -> gwt::LaunchWizardPreviousProfiles {
        self.issue_monitor_launch_profile_choice(project_root, None)
            .profiles
    }

    /// Issue #4366 AC-6: the saved pool head exactly as saved. Unlike the
    /// launch choice this never skips a held provider, because it is what the
    /// Agent Settings form shows and writes back.
    pub(super) fn issue_monitor_saved_head_profiles(
        &self,
        project_root: &Path,
    ) -> gwt::LaunchWizardPreviousProfiles {
        match self
            .issue_monitor_saved_pool(project_root)
            .into_iter()
            .next()
        {
            Some(head) => gwt::LaunchWizardPreviousProfiles::from_profile(Some(head.into())),
            None => self.issue_monitor_previous_profiles(project_root),
        }
    }

    /// Issue #4079 AC-2: the saved candidate pool, read once when the Agent
    /// Settings form opens so the wizard can preview the save's effect on it.
    pub(super) fn issue_monitor_saved_pool(
        &self,
        project_root: &Path,
    ) -> Vec<gwt::IssueMonitorLaunchProfile> {
        gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(project_root))
            .map(|prefs| prefs.launch_profile_pool())
            .unwrap_or_default()
    }

    /// Preview saved profiles, retaining the head when every candidate is
    /// held. Actual owner launches use the fallible owner selector below.
    fn issue_monitor_launch_profile_choice(
        &self,
        project_root: &Path,
        avoid_provider: Option<&str>,
    ) -> IssueMonitorLaunchProfileChoice {
        issue_monitor_launch_profile_choice(
            &self.launch_wizard_cache,
            &self.provider_usage_accounts,
            project_root,
            avoid_provider,
        )
    }

    /// Select an eligible candidate for an owner, rejecting all-held pools
    /// for both automatic tiers and manually configured candidates.
    fn issue_monitor_owner_launch_profile_choice(
        &self,
        project_root: &Path,
        issue_number: u64,
        linked_issue_kind: gwt::LinkedIssueKind,
    ) -> Result<IssueMonitorLaunchProfileChoice, String> {
        issue_monitor_owner_launch_profile_choice(
            &self.launch_wizard_cache,
            &self.provider_usage_accounts,
            project_root,
            issue_number,
            linked_issue_kind,
        )
    }

    #[cfg(test)]
    pub(crate) fn auto_launch_issue_monitor_request_events_for_project(
        &mut self,
        project_root: &Path,
        issue_number: u64,
        linked_issue_kind: gwt::LinkedIssueKind,
    ) -> Vec<OutboundEvent> {
        self.apply_issue_monitor_launch_delivery(
            project_root,
            issue_number,
            linked_issue_kind,
            None,
            gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
            None,
        )
    }

    #[cfg(test)]
    pub(crate) fn auto_launch_issue_monitor_delivery_events(
        &mut self,
        context: &super::ProjectContext,
        issue_number: u64,
        linked_issue_kind: gwt::LinkedIssueKind,
        delivery_id: Option<String>,
        launch_session_strategy: gwt::IssueMonitorLaunchSessionStrategy,
    ) -> Vec<OutboundEvent> {
        if !self.project_context_is_current(context) {
            return Vec::new();
        }
        let project_root = context.project_root.clone();
        self.apply_issue_monitor_launch_delivery(
            &project_root,
            issue_number,
            linked_issue_kind,
            delivery_id,
            launch_session_strategy,
            None,
        )
    }

    pub(crate) fn auto_launch_issue_monitor_delivery_events_for_project(
        &mut self,
        project_root: &Path,
        issue_number: u64,
        linked_issue_kind: gwt::LinkedIssueKind,
        delivery_id: Option<String>,
        launch_session_strategy: gwt::IssueMonitorLaunchSessionStrategy,
    ) -> Vec<OutboundEvent> {
        let Some(context) = self.project_context_for_root(project_root) else {
            return Vec::new();
        };
        let request = super::DeferredIssueMonitorLaunch {
            project_root: project_root.to_path_buf(),
            issue_number,
            linked_issue_kind,
            delivery_id,
            launch_session_strategy,
        };
        if let Some(deferred) = self.deferred_issue_monitor_launches.as_mut() {
            deferred.push(request);
            return Vec::new();
        }
        let key = issue_monitor_launch_preparation_key(&context, &request);
        if !self.issue_monitor_launch_preparations.insert(key.clone()) {
            return Vec::new();
        }
        let cache = self.launch_wizard_cache.clone();
        let sessions_dir = self.sessions_dir.clone();
        let profile_config_path = self.profile_config_path();
        let provider_usage = self.provider_usage_accounts.clone();
        let auth_probe = self.issue_monitor_provider_auth_probe;
        let proxy = self.proxy.clone();
        let failure_delivery_id = request.delivery_id.clone();
        if let Err(error) = self.blocking_tasks.try_spawn(move || {
            let handoff = issue_monitor_resume_handoff(&request.project_root, issue_number);
            let choice = issue_monitor_preparation_choice(&cache, &provider_usage, &request);
            let selection = choice
                .as_ref()
                .map(|choice| {
                    choice
                        .as_ref()
                        .map(|choice| (choice.profiles.clone(), choice.tier))
                })
                .map_err(Clone::clone);
            let result = choice.and_then(|choice| match choice {
                Some(choice) => prepare_issue_monitor_launch(
                    &request,
                    &cache,
                    &sessions_dir,
                    &profile_config_path,
                    choice,
                    auth_probe,
                    handoff.clone(),
                )
                .map(|facts| IssueMonitorLaunchPreparation::Launch(Box::new(facts))),
                None => prepare_issue_monitor_answer(
                    &request,
                    &cache,
                    &sessions_dir,
                    &profile_config_path,
                    auth_probe,
                    handoff.clone(),
                )
                .map(|resume| IssueMonitorLaunchPreparation::Answer(Box::new(resume))),
            });
            proxy.send(UserEvent::IssueMonitorLaunchPrepared(Box::new(
                IssueMonitorLaunchPrepared {
                    context,
                    request,
                    handoff,
                    selection,
                    result,
                },
            )));
        }) {
            self.issue_monitor_launch_preparations.remove(&key);
            return self.apply_issue_monitor_launch_delivery(
                project_root,
                issue_number,
                linked_issue_kind,
                failure_delivery_id,
                launch_session_strategy,
                Some(Err(error)),
            );
        }
        Vec::new()
    }

    pub(crate) fn handle_issue_monitor_launch_prepared(
        &mut self,
        prepared: IssueMonitorLaunchPrepared,
    ) -> Vec<OutboundEvent> {
        let IssueMonitorLaunchPrepared {
            context,
            request,
            handoff,
            selection,
            result,
        } = prepared;
        let key = issue_monitor_launch_preparation_key(&context, &request);
        if !self.issue_monitor_launch_preparations.remove(&key)
            || !self.project_context_is_current(&context)
        {
            return Vec::new();
        }
        let mut session_changed = false;
        if let Ok(preparation) = &result {
            let resume = match preparation {
                IssueMonitorLaunchPreparation::Launch(facts) => {
                    facts.resume.as_ref().ok().and_then(Option::as_ref)
                }
                IssueMonitorLaunchPreparation::Answer(resume) => Some(resume.as_ref()),
            };
            if let Some(resume) = resume {
                let path = self
                    .sessions_dir
                    .join(format!("{}.toml", resume.session.id));
                session_changed =
                    std::fs::read(&path).map_err(|error| error.kind()) != resume.session_record;
                if session_changed {
                    match gwt_agent::Session::load(&path) {
                        Ok(session) => self.launch_wizard_cache.record_session(session),
                        Err(_) => self.launch_wizard_cache.forget_session(&resume.session.id),
                    }
                }
            }
        }
        // Profile edits, Session changes and newly answered questions supersede worker facts.
        if session_changed
            || selection
                != issue_monitor_preparation_choice(
                    &self.launch_wizard_cache,
                    &self.provider_usage_accounts,
                    &request,
                )
                .map(|choice| choice.map(|choice| (choice.profiles, choice.tier)))
            || handoff != issue_monitor_resume_handoff(&request.project_root, request.issue_number)
        {
            return self.auto_launch_issue_monitor_delivery_events_for_project(
                &request.project_root,
                request.issue_number,
                request.linked_issue_kind,
                request.delivery_id,
                request.launch_session_strategy,
            );
        }
        self.apply_issue_monitor_launch_delivery(
            &request.project_root,
            request.issue_number,
            request.linked_issue_kind,
            request.delivery_id,
            request.launch_session_strategy,
            Some(result),
        )
    }

    fn apply_issue_monitor_launch_delivery(
        &mut self,
        project_root: &Path,
        issue_number: u64,
        linked_issue_kind: gwt::LinkedIssueKind,
        delivery_id: Option<String>,
        launch_session_strategy: gwt::IssueMonitorLaunchSessionStrategy,
        prepared: Option<Result<IssueMonitorLaunchPreparation, String>>,
    ) -> Vec<OutboundEvent> {
        let Some(context) = self
            .issue_monitor_tab_id_for_project_root(project_root)
            .and_then(|id| self.project_context(&id))
        else {
            return Vec::new();
        };
        // Issue #4378 AC-2: hold deliveries until the startup generation
        // reaper reports back, so a launch never races a stale generation.
        if let Some(deferred) = self.deferred_issue_monitor_launches.as_mut() {
            deferred.push(super::DeferredIssueMonitorLaunch {
                project_root: project_root.to_path_buf(),
                issue_number,
                linked_issue_kind,
                delivery_id,
                launch_session_strategy,
            });
            return Vec::new();
        }
        let mut recovery_events = Vec::new();
        if let Some(delivery_id) = delivery_id.as_deref() {
            match self
                .issue_monitor_launch_deliveries
                .get(delivery_id)
                .cloned()
            {
                Some(super::IssueMonitorLaunchDeliveryState::Materializing {
                    window_id,
                    started_at,
                }) => {
                    let window_exists = self.tracked_window_exists(&window_id);
                    // Issue #3851: the TTL bounds pre-PTY materialization only.
                    // Once a runtime exists, SessionStart may legitimately wait
                    // for terminal input; runtime status owns exit recovery.
                    let live_runtime = self.runtimes.contains_key(&window_id)
                        && self.window_status(&window_id).is_some_and(|status| {
                            !matches!(
                                status,
                                WindowProcessStatus::Stopped | WindowProcessStatus::Error
                            )
                        });
                    if window_exists
                        && (live_runtime
                            || started_at.elapsed() < super::ISSUE_MONITOR_MATERIALIZING_TTL)
                    {
                        return Vec::new();
                    }
                    self.issue_monitor_launch_deliveries.remove(delivery_id);
                    self.pending_launch_feedback_contexts.remove(&window_id);
                    self.inflight_launches
                        .retain(|_, (pending_window_id, _)| pending_window_id != &window_id);
                    if window_exists {
                        recovery_events.extend(
                            self.close_window_after_issue_monitor_finalize_events(&window_id),
                        );
                    }
                }
                Some(super::IssueMonitorLaunchDeliveryState::LaunchedPendingAck { window_id }) => {
                    return self.issue_monitor_launch_completed_delivery_events(
                        project_root,
                        issue_number,
                        &window_id,
                        Some(delivery_id),
                    );
                }
                Some(super::IssueMonitorLaunchDeliveryState::Launched { window_id }) => {
                    return self.issue_monitor_launch_succeeded_delivery_events(
                        project_root,
                        issue_number,
                        &window_id,
                        Some(delivery_id),
                    );
                }
                Some(super::IssueMonitorLaunchDeliveryState::LaunchFailed {
                    message,
                    session_mode,
                }) => {
                    return self.issue_monitor_launch_failed_delivery_events_with_mode(
                        Some(project_root),
                        issue_number,
                        &message,
                        Some(delivery_id),
                        session_mode,
                    );
                }
                None => {}
            }
            if let Some((window_id, was_materialized, _)) =
                self.existing_issue_monitor_delivery_window(project_root, issue_number, delivery_id)
            {
                match self.claim_issue_monitor_launch_delivery(
                    project_root,
                    issue_number,
                    delivery_id,
                    &window_id,
                ) {
                    Ok(true) => {}
                    Ok(false) => return recovery_events,
                    Err(error) => {
                        recovery_events.extend(self.issue_monitor_control_error_events(
                            Some(project_root),
                            None,
                            error,
                            "reclaim-launch-delivery-window",
                            Some(issue_number),
                        ));
                        return recovery_events;
                    }
                }
                if !was_materialized {
                    let exact_live_holder = self.active_agent_sessions.contains_key(&window_id)
                        && self.runtimes.contains_key(&window_id)
                        && self.window_status(&window_id).is_some_and(|status| {
                            !matches!(
                                status,
                                WindowProcessStatus::Stopped | WindowProcessStatus::Error
                            )
                        });
                    if !exact_live_holder {
                        recovery_events.extend(
                            self.close_window_after_issue_monitor_finalize_events(&window_id),
                        );
                    }
                } else {
                    let Some((window_id, materialized, workspace_durable)) = self
                        .existing_issue_monitor_delivery_window(
                            project_root,
                            issue_number,
                            delivery_id,
                        )
                    else {
                        return recovery_events;
                    };
                    return if materialized && workspace_durable {
                        self.issue_monitor_launch_succeeded_delivery_events(
                            project_root,
                            issue_number,
                            &window_id,
                            Some(delivery_id),
                        )
                    } else {
                        self.issue_monitor_launch_completed_delivery_events(
                            project_root,
                            issue_number,
                            &window_id,
                            Some(delivery_id),
                        )
                    };
                }
            }
        }
        match self.silent_issue_monitor_launch_events_for_project(
            project_root,
            SilentIssueMonitorLaunchRequest {
                issue_number,
                linked_issue_kind,
                review_prompt: None,
                review_model: None,
                delivery_id: delivery_id.clone(),
                launch_session_strategy,
            },
            prepared,
        ) {
            Ok(Some(events)) => {
                recovery_events.extend(events);
                recovery_events
            }
            Ok(None) => {
                if self.launch_wizard_for(&context).is_some() {
                    recovery_events.push(OutboundEvent::project(
                        context.project_key.clone(),
                        BackendEvent::IssueMonitorToast {
                            notification_transition: None,
                            level: "info".to_string(),
                            message: "Issue Monitor settings are already open".to_string(),
                            issue_number: Some(issue_number),
                        },
                    ));
                    return recovery_events;
                }
                recovery_events.extend(
                    self.open_issue_monitor_configure_wizard_events_for_project(
                        "__issue_monitor__",
                        project_root,
                        issue_number,
                        linked_issue_kind,
                    )
                    .into_iter()
                    .map(|mut event| {
                        if matches!(event.target, DispatchTarget::Client(_)) {
                            event.target = DispatchTarget::Project(context.project_key.clone());
                        }
                        event
                    })
                    .collect::<Vec<_>>(),
                );
                recovery_events
            }
            Err(error) => {
                let answered_handoff_pending = gwt::load_issue_monitor_prefs(
                    &gwt::issue_monitor_prefs_path_for_repo_path(project_root),
                )
                .ok()
                .is_some_and(|prefs| {
                    prefs.autonomous_handoffs.iter().any(|handoff| {
                        handoff.issue_number == issue_number
                            && handoff.answer.is_some()
                            && handoff.delivered_at.is_none()
                    })
                });
                if answered_handoff_pending {
                    if let Some(delivery_id) = delivery_id.as_deref() {
                        self.issue_monitor_launch_deliveries.remove(delivery_id);
                    }
                    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(project_root);
                    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
                    let settlement = gwt::prepare_autonomous_handoff_delivery_from_prefs(
                        &prefs_path,
                        issue_number,
                        &now,
                    )
                    .and_then(|prepared| match prepared {
                        Some(gwt::AutonomousHandoffDeliveryPreparation::Ready(prepared)) => {
                            gwt::record_autonomous_handoff_delivery_failure_from_prefs(
                                &prefs_path,
                                &prepared.handoff_id,
                                &prepared.session_id,
                                prepared.attempt,
                                &error,
                                &now,
                            )
                            .map(Some)
                        }
                        _ => Ok(None),
                    });
                    let disposition = match settlement {
                        Ok(Some(gwt::AutonomousHandoffDeliveryFailureOutcome::Retry {
                            retry_not_before,
                            ..
                        })) => format!(" exact-session retry after {retry_not_before}"),
                        Ok(Some(gwt::AutonomousHandoffDeliveryFailureOutcome::Escalated {
                            ..
                        })) => " parked for human review after bounded retries".to_string(),
                        Ok(Some(gwt::AutonomousHandoffDeliveryFailureOutcome::Rejected))
                        | Ok(None) => " kept in its existing durable delivery state".to_string(),
                        Err(settlement_error) => {
                            format!(" could not persist its delivery failure: {settlement_error}")
                        }
                    };
                    recovery_events.push(OutboundEvent::project(context.project_key.clone(),
                        BackendEvent::IssueMonitorToast {
                            notification_transition: None,
                            level: "error".to_string(),
                            message: format!(
                                "Issue Monitor could not continue the exact answered session;{disposition}: {error}"
                            ),
                            issue_number: Some(issue_number),
                        },
                    ).with_error_project_root(&context.project_root));
                } else {
                    recovery_events.extend(self.issue_monitor_launch_failed_delivery_events(
                        Some(project_root),
                        issue_number,
                        &error,
                        delivery_id.as_deref(),
                    ));
                }
                recovery_events
            }
        }
    }

    /// Issue #4802 AC-2: the live implementation agent pane already working
    /// `issue_number` on this tab — linked to the Issue, or running in the
    /// Issue's work branch worktree. Stopped and Error panes are finished, and
    /// an independent review window only observes the Issue.
    pub(super) fn live_issue_agent_window(
        &self,
        tab_id: &str,
        issue_number: u64,
        target_branch: &str,
    ) -> Option<String> {
        let tab = self.tab(tab_id)?;
        let target_branch = normalize_branch_name(target_branch);
        tab.workspace
            .persisted()
            .windows
            .iter()
            .filter(|window| window.preset == WindowPreset::Agent)
            .map(|window| (combined_window_id(tab_id, &window.id), window))
            .find(|(window_id, window)| {
                let pending =
                    self.issue_monitor_pending_feedback_for_window(window_id, &tab.project_root);
                let linked_issue = pending
                    .and_then(|context| context.issue_monitor_issue_number)
                    .or(window.linked_issue_number)
                    .or_else(|| {
                        let session_id = window.session_id.as_deref()?;
                        self.launch_wizard_cache
                            .session_by_id(session_id)
                            .and_then(|session| session.linked_issue_number)
                    });
                let in_issue_worktree =
                    self.active_agent_sessions
                        .get(window_id)
                        .is_some_and(|active| {
                            normalize_branch_name(&active.branch_name) == target_branch
                        });
                (linked_issue == Some(issue_number) || in_issue_worktree)
                    && !pending.is_some_and(|context| context.issue_monitor_review_dispatch)
                    && !self
                        .issue_monitor_review_dispatch_windows
                        .contains(window_id)
                    && self.window_status(window_id).is_some_and(|status| {
                        !matches!(
                            status,
                            WindowProcessStatus::Stopped | WindowProcessStatus::Error
                        )
                    })
            })
            .map(|(window_id, _)| window_id)
    }

    /// Issue #4802 AC-2: acknowledge a launch onto the pane that is already
    /// working its Issue instead of opening a second one. The delivery goes
    /// through the same claim / materialized / durable / launched sequence a
    /// freshly spawned window uses, so the Monitor binds the running pane.
    fn adopt_live_issue_agent_window_events(
        &mut self,
        context: &super::ProjectContext,
        project_root: &Path,
        issue_number: u64,
        live_window_id: &str,
        delivery_id: Option<&str>,
    ) -> Vec<OutboundEvent> {
        tracing::warn!(
            issue_number,
            window_id = %live_window_id,
            "Issue Monitor launch refused: the Issue's worktree already has a live agent pane"
        );
        let mut events = match delivery_id {
            Some(delivery_id) => match self.claim_issue_monitor_launch_delivery(
                project_root,
                issue_number,
                delivery_id,
                live_window_id,
            ) {
                Ok(true) => self.issue_monitor_launch_completed_delivery_events(
                    project_root,
                    issue_number,
                    live_window_id,
                    Some(delivery_id),
                ),
                Ok(false) => return Vec::new(),
                Err(error) => {
                    return self.issue_monitor_control_error_events(
                        Some(project_root),
                        None,
                        error,
                        "adopt-live-issue-window",
                        Some(issue_number),
                    )
                }
            },
            None => self.issue_monitor_launch_succeeded_delivery_events(
                project_root,
                issue_number,
                live_window_id,
                None,
            ),
        };
        events.push(OutboundEvent::project(
            context.project_key.clone(),
            BackendEvent::IssueMonitorToast {
                notification_transition: None,
                level: "warn".to_string(),
                message: format!(
                    "Issue Monitor did not open a second pane for #{issue_number}: window {live_window_id} is already working it"
                ),
                issue_number: Some(issue_number),
            },
        ));
        events
    }

    fn existing_issue_monitor_delivery_window(
        &self,
        project_root: &Path,
        issue_number: u64,
        delivery_id: &str,
    ) -> Option<(String, bool, bool)> {
        let tab_id = self.issue_monitor_tab_id_for_project_root(project_root)?;
        let tab = self.tab(&tab_id)?;
        let prefs = gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(
            &tab.project_root,
        ))
        .ok()?;
        let delivery = prefs.pending_launch_deliveries.iter().find(|delivery| {
            delivery.issue_number == issue_number && delivery.delivery_id == delivery_id
        })?;
        let bound_window_id = delivery.materializer_window_id.as_deref()?;
        let window_id = tab
            .workspace
            .persisted()
            .windows
            .iter()
            .map(|window| combined_window_id(&tab_id, &window.id))
            .find(|window_id| window_id == bound_window_id)?;
        Some((
            window_id,
            delivery.materialized_window_id.as_deref() == Some(bound_window_id),
            delivery.workspace_durable_window_id.as_deref() == Some(bound_window_id),
        ))
    }

    /// SPEC #3200 Option A: handle a daemon `review_dispatch` — prepare the
    /// independent-review prompt (bound to the reviewed SHA, with the criteria +
    /// diff as untrusted data) and surface a notification that review was
    /// dispatched. Spawning the review agent in an isolated worktree on a
    /// different model, and bridging its verdict back via the `ReviewVerdict`
    /// control, is the live-integration step verified against a real PR.
    pub(crate) fn auto_dispatch_issue_monitor_review_events_for_project(
        &mut self,
        project_root: &Path,
        dispatch: gwt::AutonomousReviewDispatch,
    ) -> Vec<OutboundEvent> {
        let Some(context) = self.project_context_for_root(project_root) else {
            return Vec::new();
        };
        let prompt = build_review_dispatch_prompt(&dispatch);
        tracing::info!(
            issue = dispatch.issue_number,
            pr = dispatch.pr_number,
            reviewed_sha = %dispatch.reviewed_sha,
            prompt_bytes = prompt.len(),
            "autonomous independent-review dispatch"
        );
        // Spawn a FRESH-session review agent in the implementation work-branch
        // worktree (idle by review time); it reviews the diff embedded in its
        // prompt and reports the verdict via the gwtd issue.monitor.review_verdict
        // op. skip_permissions is forced on the autonomous path so review runs
        // unattended.
        // SPEC #3200 FR-015: the configured review model (if any) is forced for
        // the review agent so it differs from the implementer's.
        let review_model = gwt::load_issue_monitor_prefs(
            &gwt::issue_monitor_prefs_path_for_repo_path(project_root),
        )
        .ok()
        .and_then(|prefs| prefs.autonomous_tuning.review_model);
        match self.silent_issue_monitor_launch_events_for_project(
            project_root,
            SilentIssueMonitorLaunchRequest {
                issue_number: dispatch.issue_number,
                linked_issue_kind: dispatch.linked_issue_kind,
                review_prompt: Some(prompt),
                review_model,
                delivery_id: None,
                launch_session_strategy: gwt::IssueMonitorLaunchSessionStrategy::FreshRequired,
            },
            None,
        ) {
            Ok(Some(events)) => events,
            Ok(None) => vec![OutboundEvent::project(
                context.project_key.clone(),
                BackendEvent::IssueMonitorToast {
                    notification_transition: None,
                    level: "warn".to_string(),
                    message: format!(
                        "Independent review for #{} could not start (launch settings unavailable)",
                        dispatch.issue_number
                    ),
                    issue_number: Some(dispatch.issue_number),
                },
            )],
            Err(error) => self.issue_monitor_launch_failed_events(
                Some(project_root),
                dispatch.issue_number,
                &error,
            ),
        }
    }

    fn silent_issue_monitor_launch_events_for_project(
        &mut self,
        requested_project_root: &Path,
        request: SilentIssueMonitorLaunchRequest,
        prepared: Option<Result<IssueMonitorLaunchPreparation, String>>,
    ) -> Result<Option<Vec<OutboundEvent>>, String> {
        let context = self
            .project_context_for_root(requested_project_root)
            .ok_or_else(|| "Project tab not found".to_string())?;
        let SilentIssueMonitorLaunchRequest {
            issue_number,
            linked_issue_kind,
            review_prompt,
            review_model,
            delivery_id,
            launch_session_strategy,
        } = request;
        let Some(tab_id) = self.issue_monitor_tab_id_for_project_root(requested_project_root)
        else {
            return Err("Project tab not found".to_string());
        };
        let (project_kind, migration_pending, project_root, proposed_window_id) = {
            let Some(tab) = self.tab(&tab_id) else {
                return Err("Project tab not found".to_string());
            };
            (
                tab.kind,
                tab.migration_pending,
                tab.project_root.clone(),
                combined_window_id(
                    &tab_id,
                    &tab.workspace.next_window_id_preview(WindowPreset::Agent),
                ),
            )
        };
        if let Some(delivery_id) = delivery_id.as_deref() {
            // Claim the existing pane when adopting it: a proposed new ID
            // would incorrectly ask for another slot at a saturated cap.
            let claim_window_id = review_prompt
                .is_none()
                .then(|| {
                    self.live_issue_agent_window(
                        &tab_id,
                        issue_number,
                        &knowledge_launch_target_branch_name(linked_issue_kind, issue_number),
                    )
                })
                .flatten()
                .unwrap_or_else(|| proposed_window_id.clone());
            match self.claim_issue_monitor_launch_delivery(
                &project_root,
                issue_number,
                delivery_id,
                &claim_window_id,
            ) {
                Ok(true) => {}
                Ok(false) => return Ok(Some(Vec::new())),
                Err(error) => {
                    return Ok(Some(self.issue_monitor_control_error_events(
                        Some(&project_root),
                        None,
                        error,
                        "claim-launch-delivery",
                        Some(issue_number),
                    )));
                }
            }
        }
        if project_kind != gwt::ProjectKind::Git {
            return Err("Issue Monitor launch requires a Git project".to_string());
        }
        if migration_pending {
            return Err(
                "Complete the project migration before launching monitored Issue work".to_string(),
            );
        }

        // Preparation failures still belong to the exact claimed delivery.
        let mut prepared = match prepared.transpose()? {
            Some(IssueMonitorLaunchPreparation::Answer(resume)) => {
                let agent_id = resume.session.agent_id.command().to_string();
                let target_branch =
                    knowledge_launch_target_branch_name(linked_issue_kind, issue_number);
                let (events, _) = self.silent_issue_monitor_resume_events(
                    &tab_id,
                    &project_root,
                    &target_branch,
                    issue_number,
                    delivery_id,
                    &agent_id,
                    Some(Ok(Some(*resume))),
                )?;
                return events.map(Some).ok_or_else(|| {
                    "answered autonomous handoff requires its exact Session".to_string()
                });
            }
            Some(IssueMonitorLaunchPreparation::Launch(facts)) => Some(*facts),
            None => None,
        };

        // An answer belongs to its asking Session. Auto settings may have
        // changed provider, model, or tier while the question was pending;
        // defer that selection until the next producing launch.
        let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&project_root);
        let auto_answer_pending = prepared.is_none()
            && review_prompt.is_none()
            && gwt::load_issue_monitor_prefs(&prefs_path)
                .map_err(|error| error.to_string())?
                .launch_auto
            && gwt::preserve_answered_handoff_resume_strategy_from_prefs(&prefs_path, issue_number)
                .map_err(|error| error.to_string())?;
        if auto_answer_pending {
            let handoff =
                gwt::pending_autonomous_handoff_resumption_from_prefs(&prefs_path, issue_number)
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| {
                        "answered autonomous handoff is not ready for exact delivery".to_string()
                    })?;
            let source = self
                .issue_monitor_session_by_id(&handoff.session_id)
                .ok_or_else(|| {
                    format!(
                        "answered autonomous handoff Session {} is unavailable",
                        handoff.session_id
                    )
                })?;
            let agent_id = source.agent_id.command();
            if (self.issue_monitor_provider_auth_probe)(agent_id)
                == gwt::issue_monitor::ProviderAuthState::Unauthenticated
            {
                return Err(gwt::issue_monitor::provider_unauthenticated_message(
                    agent_id,
                ));
            }
            let target_branch =
                knowledge_launch_target_branch_name(linked_issue_kind, issue_number);
            let (events, _) = self.silent_issue_monitor_resume_events(
                &tab_id,
                &project_root,
                &target_branch,
                issue_number,
                delivery_id,
                agent_id,
                None,
            )?;
            return events.map(Some).ok_or_else(|| {
                "answered autonomous handoff requires its exact Session".to_string()
            });
        }

        let base_branch_name = match prepared.as_ref() {
            Some(facts) => facts.base_branch.clone(),
            None => gwt::start_work::resolve_launch_agent_base_branch(&project_root)?,
        };
        let IssueMonitorLaunchProfileChoice {
            profiles: previous_profiles,
            selected_agent_id,
            skipped: skipped_candidates,
            tier,
        } = prepared
            .as_ref()
            .map(|facts| facts.choice.clone())
            .map(Ok)
            .unwrap_or_else(|| {
                self.issue_monitor_owner_launch_profile_choice(
                    &project_root,
                    issue_number,
                    linked_issue_kind,
                )
            })?;
        let non_head_selection_toast = issue_monitor_non_head_selection_toast(
            &context,
            issue_number,
            selected_agent_id.as_deref(),
            &skipped_candidates,
        );
        let Some(profile_agent_id) = previous_profiles
            .preferred_profile()
            .map(|profile| profile.agent_id.clone())
        else {
            return Ok(None);
        };
        // Issue #3676 AC-2: refuse before any terminal is spawned when the
        // profile provider's CLI is definitively unauthenticated. The error
        // funnels through the normal launch-failed path, so the active slot
        // is released instead of burning on a provider login screen.
        if prepared.is_none()
            && (self.issue_monitor_provider_auth_probe)(&profile_agent_id)
                == gwt::issue_monitor::ProviderAuthState::Unauthenticated
        {
            return Err(gwt::issue_monitor::provider_unauthenticated_message(
                &profile_agent_id,
            ));
        }
        // SPEC #3200 FR-015: for the independent review, force a different model
        // than the implementer's (when configured) so the verdict is not a
        // self-grade. `None` keeps the saved model (still a fresh session).
        let implementer_model = previous_profiles
            .preferred_profile()
            .and_then(|profile| profile.model.clone());
        let review_model_override = gwt::issue_monitor::resolve_review_model(
            implementer_model.as_deref(),
            review_model.as_deref(),
        );
        let launch_profiles = previous_profiles.clone();
        let answered_handoff_pending = review_prompt.is_none()
            && gwt::preserve_answered_handoff_resume_strategy_from_prefs(
                &gwt::issue_monitor_prefs_path_for_repo_path(&project_root),
                issue_number,
            )
            .ok()
            .unwrap_or(false);

        let record_launch_tier = || -> Result<(), String> {
            if review_prompt.is_none() {
                if let Some(tier) = tier {
                    gwt::mutate_issue_monitor_prefs(
                        &gwt::issue_monitor_prefs_path_for_repo_path(&project_root),
                        |prefs| prefs.record_tier_launch(issue_number, tier),
                    )
                    .map_err(|error| {
                        format!("Could not record Issue Monitor launch tier: {error}")
                    })?;
                }
            }
            Ok(())
        };

        // FR-022: an Issue whose agent window was previously closed without a
        // merge keeps a resumable session. Re-engage by resuming that session
        // (continuing the conversation) instead of launching a fresh agent.
        // SPEC #3200 Option A: the independent review must be a FRESH session
        // (no shared context with the implementation agent), so the review path
        // skips resume and always launches a new agent.
        let target_branch = knowledge_launch_target_branch_name(linked_issue_kind, issue_number);
        // Issue #4802 AC-2: a second implementation launch into a worktree
        // that already has a live agent pane is refused. The running pane is
        // this launch, so the delivery is acknowledged onto it rather than
        // opening another one. An answered handoff is delivered into its live
        // holder by the resume path below, and the independent review is a
        // deliberate second pane, so both keep their own routes.
        if review_prompt.is_none() && !answered_handoff_pending {
            if let Some(live_window_id) =
                self.live_issue_agent_window(&tab_id, issue_number, &target_branch)
            {
                return Ok(Some(self.adopt_live_issue_agent_window_events(
                    &context,
                    &project_root,
                    issue_number,
                    &live_window_id,
                    delivery_id.as_deref(),
                )));
            }
        }
        // Auto uses a fresh launch even at the same tier index: an override
        // may have changed that tier's model or effort since the last launch.
        let resume_holder_window_id = if review_prompt.is_none()
            && tier.is_none()
            && (launch_session_strategy == gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe
                || answered_handoff_pending)
        {
            let (events, holder_window_id) = self.silent_issue_monitor_resume_events(
                &tab_id,
                &project_root,
                &target_branch,
                issue_number,
                delivery_id.clone(),
                &profile_agent_id,
                prepared
                    .as_mut()
                    .map(|facts| std::mem::replace(&mut facts.resume, Ok(None))),
            )?;
            if let Some(mut events) = events {
                // SPEC #3914 FR-007: a resumed launch reports its skipped
                // candidates like a fresh one. An empty vector means the
                // delivery stays pending, so nothing launched to report on.
                if !events.is_empty() {
                    record_launch_tier()?;
                    events.extend(non_head_selection_toast);
                }
                return Ok(Some(events));
            }
            holder_window_id
        } else {
            None
        };

        let mut session = self.build_knowledge_launch_wizard_session(
            &tab_id,
            &project_root,
            &base_branch_name,
            issue_number,
            linked_issue_kind,
            previous_profiles,
        );
        let initial_prompt = review_prompt.clone().unwrap_or_else(|| {
            // Issue #4630: a launch that consumed an operator requeue carries
            // the operator's reason, read from the exact durable delivery.
            let requeue_reason = gwt::issue_monitor_launch_delivery_requeue_reason(
                &gwt::issue_monitor_prefs_path_for_repo_path(&project_root),
                issue_number,
                delivery_id.as_deref(),
            );
            gwt::issue_monitor_launch_prompt_with_requeue_reason(
                linked_issue_kind,
                issue_number,
                requeue_reason.as_deref(),
            )
        });
        session
            .wizard
            .apply(gwt::LaunchWizardAction::SetInitialPrompt {
                value: initial_prompt,
            });
        session
            .wizard
            .apply(gwt::LaunchWizardAction::UseStartMethod {
                method: gwt::LaunchWizardStartMethodKind::StartWithLastSettings,
            });
        let mut launch_request = self.resolve_silent_issue_monitor_launch_request(
            &mut session,
            &project_root,
            launch_profiles,
            prepared.and_then(|facts| facts.hydration),
        )?;
        // Any path that reaches this point is a raw fresh launch: either the
        // durable policy requires it, the exact-resume preflight failed closed,
        // no candidate exists, or this is an independent review. Preserve the
        // current saved provider profile while clearing conversation identity.
        if let LaunchWizardLaunchRequest::Agent(config) = &mut launch_request {
            config.session_mode = gwt_agent::SessionMode::Normal;
            config.resume_session_id = None;
        }
        // Issue #4217 (AC-2): every launch that reaches this path was started
        // by the Issue Monitor, so the route is recorded before — and
        // independently of — the `autonomous_mode` preference read below.
        launch_request.set_issue_monitor_launch_route();
        // SPEC #3200 T-040/FR-006: in unattended autonomous mode the
        // monitor-launched implementation agent must not stall on a permission
        // prompt. Default OFF leaves the SPEC #3165 human-gated launch untouched.
        let autonomous_mode = gwt::load_issue_monitor_prefs(
            &gwt::issue_monitor_prefs_path_for_repo_path(&project_root),
        )
        .map(|prefs| prefs.autonomous_mode)
        .unwrap_or(false);
        launch_request.force_skip_permissions_for_autonomous(autonomous_mode);
        // Issue #4543 AC-6: nothing else names this launch surface, and both
        // steps above changed what the already-built config means. Re-decide
        // now so the Execution Control Record describes the launch that ships.
        launch_request
            .record_permission_launch_source(gwt_agent::PermissionLaunchSource::SilentIssueMonitor);
        // Issue #4544 AC-1 / AC-5: refuse here, before the window, for the same
        // reason the unauthenticated-provider probe above refuses — and for
        // both monitor launches, not just the implementing one.
        //
        // The launch-time gate in `app_runtime::launch` only sees producing
        // owners, and the independent review agent deliberately carries no
        // Execution Control Record (`set_review_dispatch_context`). Without
        // this, a review launch on a provider that cannot skip permissions
        // would sit at a prompt with nobody watching — the same failure the
        // implementation path is protected from. The refusal funnels through
        // the normal launch-failed path, so the active slot is released.
        if let LaunchWizardLaunchRequest::Agent(config) = &launch_request {
            if let Some(record) = gwt::cli::permission_readiness::pre_launch_block(
                "issue",
                issue_number,
                &format!("monitor-launch:{issue_number}"),
                &config.permission_decision,
            ) {
                return Err(format!("launch refused: {}", record.describe()));
            }
        }
        // Issue #3478 (AC-1): the unattended agent must know it is unattended,
        // so its hooks can convert a confirmation question into a NeedsHuman
        // handoff instead of letting it hold this slot until the stuck timeout.
        launch_request.set_autonomous_execution_context(autonomous_mode, issue_number);
        // SPEC #3200 FR-015: apply the distinct review model for the review agent.
        if let (Some(model), LaunchWizardLaunchRequest::Agent(config)) =
            (&review_model_override, &mut launch_request)
        {
            config.model = Some(model.clone());
        }
        // SPEC-3248 P8a: the independent review agent is subordinate to the
        // implementing session's execution — it must not take over (or be
        // gated by) the Execution Control Record for the linked owner.
        // Issue #3984: the same decision is published into the review agent's
        // environment so its hooks apply the review contract instead of the
        // producing-session gates it can never satisfy.
        if review_prompt.is_some() {
            launch_request.set_review_dispatch_context();
        }
        let launch_index = self
            .tab(&session.tab_id)
            .map(|tab| {
                tab.workspace
                    .persisted()
                    .windows
                    .iter()
                    .filter(|window| window.preset == WindowPreset::Agent)
                    .count()
            })
            .unwrap_or(0);
        let geometry = issue_monitor_auto_launch_geometry(launch_index);
        let issue_monitor_session_mode = match &launch_request {
            LaunchWizardLaunchRequest::Agent(config) => Some(config.session_mode),
            LaunchWizardLaunchRequest::Shell(_) => None,
        };
        let feedback = LaunchFeedbackContext {
            client_id: "__issue_monitor__".to_string(),
            title: "Issue Monitor".to_string(),
            issue_monitor_issue_number: Some(issue_number),
            issue_monitor_delivery_id: delivery_id,
            issue_monitor_project_root: Some(project_root.clone()),
            issue_monitor_session_mode,
            issue_monitor_autonomous_handoff: None,
            issue_monitor_autonomous_submit_started: false,
            // Issue #4041: the review window observes the Issue; it never
            // owns the launch binding the implementation window holds.
            issue_monitor_review_dispatch: review_prompt.is_some(),
        };
        record_launch_tier()?;
        let mut events = match launch_request {
            LaunchWizardLaunchRequest::Agent(config) => self
                .spawn_agent_window_with_feedback_at_geometry(
                    &session.tab_id,
                    *config,
                    geometry,
                    session.workspace_resume_context.clone(),
                    feedback,
                )?,
            LaunchWizardLaunchRequest::Shell(_) => {
                return Err("Issue Monitor automatic launch requires an agent target".to_string());
            }
        };
        if let Some(holder_window_id) = resume_holder_window_id {
            events.push(OutboundEvent::project(context.project_key.clone(), BackendEvent::IssueMonitorToast {
                notification_transition: None,
                level: "warn".to_string(),
                message: format!(
                    "Issue Monitor started a fresh session because native conversation is already held by window {holder_window_id}"
                ),
                issue_number: Some(issue_number),
            }));
        }
        events.extend(non_head_selection_toast);
        let message = if review_prompt.is_some() {
            "Issue Monitor independent review launched".to_string()
        } else {
            "Issue Monitor launch requested".to_string()
        };
        events.push(OutboundEvent::project(
            context.project_key.clone(),
            BackendEvent::IssueMonitorToast {
                notification_transition: None,
                level: "info".to_string(),
                message,
                issue_number: Some(issue_number),
            },
        ));
        Ok(Some(events))
    }

    /// FR-022: resume an existing agent session for `target_branch` when one is
    /// available, instead of launching a fresh agent. Returns `Ok(None)` when no
    /// resumable session exists so the caller falls back to a fresh launch.
    #[allow(clippy::too_many_arguments)]
    fn silent_issue_monitor_resume_events(
        &mut self,
        tab_id: &str,
        project_root: &Path,
        target_branch: &str,
        issue_number: u64,
        delivery_id: Option<String>,
        profile_agent_id: &str,
        prepared: Option<Result<Option<PreparedIssueMonitorResume>, String>>,
    ) -> Result<(Option<Vec<OutboundEvent>>, Option<String>), String> {
        let context = self
            .project_context(tab_id)
            .ok_or_else(|| "Project tab not found".to_string())?;
        let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(project_root);
        let prepared = match prepared {
            Some(prepared) => prepared?,
            None => prepare_issue_monitor_resume(
                project_root,
                target_branch,
                issue_number,
                profile_agent_id,
                &self.sessions_dir,
                &self.launch_wizard_cache,
                &self.profile_config_path(),
                issue_monitor_resume_handoff(project_root, issue_number),
            )?,
        };
        let Some(PreparedIssueMonitorResume {
            session,
            autonomous_handoff,
            mut config,
            workspace_resume_context,
            ..
        }) = prepared
        else {
            return Ok((None, None));
        };
        if let Some(holder_window_id) = self.issue_monitor_native_conversation_holder(&session) {
            let Some(handoff) = autonomous_handoff.as_ref() else {
                return Ok((None, Some(holder_window_id)));
            };
            if !self.active_agent_sessions.contains_key(&holder_window_id) {
                // A materializing window reserves the native writer identity
                // but is not yet a conversation that can accept an answer.
                return Ok((Some(Vec::new()), None));
            }
            let Some(pane) = self
                .runtimes
                .get(&holder_window_id)
                .map(|runtime| runtime.pane.clone())
            else {
                // The exact conversation is known to be owned or currently
                // materializing, but there is no writable live pane yet. Keep
                // the durable delivery pending instead of launching a second
                // writer or consuming the answer.
                return Ok((Some(Vec::new()), None));
            };
            let local_delivery_key = delivery_id
                .clone()
                .unwrap_or_else(|| format!("handoff:{}", handoff.handoff_id));
            if matches!(
                self.issue_monitor_launch_deliveries.get(&local_delivery_key),
                Some(super::IssueMonitorLaunchDeliveryState::Materializing { window_id, .. })
                    if window_id == &holder_window_id
            ) {
                return Ok((Some(Vec::new()), None));
            }
            if let Some(delivery_id) = delivery_id.as_deref() {
                match self.claim_issue_monitor_launch_delivery(
                    project_root,
                    issue_number,
                    delivery_id,
                    &holder_window_id,
                ) {
                    Ok(true) => {}
                    Ok(false) => return Ok((Some(Vec::new()), None)),
                    Err(error) => {
                        return Err(format!(
                            "failed to bind the autonomous answer delivery to live window \
                             {holder_window_id}: {error}"
                        ));
                    }
                }
            }
            let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            let prepared = match gwt::prepare_autonomous_handoff_delivery_from_prefs(
                &prefs_path,
                issue_number,
                &now,
            )
            .map_err(|error| format!("failed to prepare the autonomous answer: {error}"))?
            {
                Some(gwt::AutonomousHandoffDeliveryPreparation::Ready(prepared)) => prepared,
                Some(gwt::AutonomousHandoffDeliveryPreparation::Backoff {
                    retry_not_before,
                    ..
                }) => {
                    return Err(format!(
                        "answered autonomous handoff delivery is in backoff until {retry_not_before}"
                    ));
                }
                Some(gwt::AutonomousHandoffDeliveryPreparation::InFlight { .. }) => {
                    return Ok((Some(Vec::new()), None));
                }
                Some(gwt::AutonomousHandoffDeliveryPreparation::Ambiguous { reason, .. }) => {
                    return Err(reason);
                }
                None => return Ok((Some(Vec::new()), None)),
            };
            let record_pre_submit_failure =
                |message: &str| match gwt::record_autonomous_handoff_delivery_failure_from_prefs(
                    &prefs_path,
                    &prepared.handoff_id,
                    &prepared.session_id,
                    prepared.attempt,
                    message,
                    &now,
                ) {
                    Ok(_) => message.to_string(),
                    Err(error) => {
                        format!("{message}; failed to record bounded retry: {error}")
                    }
                };
            let target = match super::autonomous_handoff_delivery_target_for_session(
                &session,
                issue_number,
                &holder_window_id,
                delivery_id.as_deref(),
                &self.issue_monitor_materializer_id,
            ) {
                Ok(target) => target,
                Err(error) => return Err(record_pre_submit_failure(&error)),
            };
            let bound = match gwt::bind_autonomous_handoff_delivery_target_from_prefs(
                &prefs_path,
                &prepared.handoff_id,
                &prepared.session_id,
                prepared.attempt,
                &target,
            ) {
                Ok(bound) => bound,
                Err(error) => {
                    let error = format!("failed to bind the autonomous answer target: {error}");
                    return Err(record_pre_submit_failure(&error));
                }
            };
            if !bound {
                let error = "autonomous answer target no longer matches its durable attempt";
                return Err(record_pre_submit_failure(error));
            }
            self.issue_monitor_launch_deliveries.insert(
                local_delivery_key.clone(),
                super::IssueMonitorLaunchDeliveryState::Materializing {
                    window_id: holder_window_id.clone(),
                    started_at: std::time::Instant::now(),
                },
            );
            let proxy = self.proxy.clone();
            let project_root = project_root.to_path_buf();
            let worker_holder_window_id = holder_window_id.clone();
            let worker_delivery_id = delivery_id.clone();
            let worker_local_delivery_key = local_delivery_key.clone();
            let handoff_id = prepared.handoff_id.clone();
            let handoff_session_id = prepared.session_id.clone();
            let attempt = prepared.attempt;
            let prompt = format!("{}\r", prepared.prompt);
            if let Err(error) = self.blocking_tasks.try_spawn(move || {
                let result = super::pty_io::write_pane_input_and_submit_blocking(&pane, &prompt);
                proxy.send(UserEvent::IssueMonitorAnswerDeliveryComplete(
                    super::IssueMonitorAnswerDelivery {
                        project_root,
                        issue_number,
                        holder_window_id: worker_holder_window_id,
                        delivery_id: worker_delivery_id,
                        local_delivery_key: worker_local_delivery_key,
                        handoff_id,
                        session_id: handoff_session_id,
                        attempt,
                        result,
                    },
                ));
            }) {
                self.issue_monitor_launch_deliveries
                    .remove(&local_delivery_key);
                let _ = gwt::record_autonomous_handoff_delivery_failure_from_prefs(
                    &prefs_path,
                    &prepared.handoff_id,
                    &prepared.session_id,
                    prepared.attempt,
                    &error,
                    &now,
                );
                return Err(format!(
                    "failed to schedule the autonomous answer delivery to live window \
                     {holder_window_id}: {error}"
                ));
            }
            return Ok((Some(Vec::new()), None));
        }
        // A Blocked owner cannot regain producing authority through Resume:
        // execution.continue requires execution.reopen first. Preserve native
        // writer/answer handling above, then use the normal fresh-launch path.
        if session.linked_issue_number == Some(issue_number)
            && gwt::cli::execution_state::load(&session.worktree_path)
                .map_err(|error| format!("failed to inspect Monitor resume authority: {error}"))?
                .is_some_and(|record| {
                    record.owner_number == issue_number
                        && record.status
                            == gwt::cli::execution_state::ExecutionControlStatus::Blocked
                })
        {
            if autonomous_handoff.is_some() {
                return Err(
                    "answered autonomous handoff requires execution.reopen before Resume"
                        .to_string(),
                );
            }
            return Ok((None, None));
        }
        if !session.worktree_path.as_path().exists() {
            config.working_dir = None;
        }
        if config.session_mode != gwt_agent::SessionMode::Resume {
            return Ok((None, None));
        }
        // Issue #4217 (AC-2): a resumed monitor launch is still a monitor
        // launch. The route is stamped whatever the preference below says.
        config.launch_route = gwt_agent::LaunchRoute::Autonomous;
        let autonomous_mode = gwt::load_issue_monitor_prefs(
            &gwt::issue_monitor_prefs_path_for_repo_path(project_root),
        )
        .map(|prefs| prefs.autonomous_mode)
        .unwrap_or(false);
        if autonomous_mode {
            config.env_vars.insert(
                gwt::autonomous_handoff::GWT_AUTONOMOUS_EXECUTION_ENV.to_string(),
                "1".to_string(),
            );
            config.env_vars.insert(
                gwt::autonomous_handoff::GWT_AUTONOMOUS_ISSUE_ENV.to_string(),
                issue_number.to_string(),
            );
        }
        // Write-ahead before the exact Resume is scheduled. The protected
        // prompt is consumed only by this launch; UserPromptSubmit supplies
        // the semantic receipt that marks it delivered.
        let autonomous_delivery_attempt = if autonomous_handoff.is_some() {
            let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            match gwt::prepare_autonomous_handoff_delivery_from_prefs(
                &prefs_path,
                issue_number,
                &now,
            )
            .map_err(|error| format!("failed to prepare the autonomous answer: {error}"))?
            {
                Some(gwt::AutonomousHandoffDeliveryPreparation::Ready(prepared)) => {
                    config.args.push(prepared.prompt.clone());
                    Some(prepared)
                }
                Some(gwt::AutonomousHandoffDeliveryPreparation::Backoff {
                    retry_not_before,
                    ..
                }) => {
                    return Err(format!(
                        "answered autonomous handoff delivery is in backoff until {retry_not_before}"
                    ));
                }
                Some(gwt::AutonomousHandoffDeliveryPreparation::InFlight { .. }) => {
                    return Ok((Some(Vec::new()), None));
                }
                Some(gwt::AutonomousHandoffDeliveryPreparation::Ambiguous { reason, .. }) => {
                    return Err(reason);
                }
                None => return Ok((Some(Vec::new()), None)),
            }
        } else {
            None
        };
        let workspace_resume_context = Some(workspace_resume_context);
        let launch_index = self
            .tab(tab_id)
            .map(|tab| {
                tab.workspace
                    .persisted()
                    .windows
                    .iter()
                    .filter(|window| window.preset == WindowPreset::Agent)
                    .count()
            })
            .unwrap_or(0);
        let geometry = issue_monitor_auto_launch_geometry(launch_index);
        let feedback = LaunchFeedbackContext {
            client_id: "__issue_monitor__".to_string(),
            title: "Issue Monitor".to_string(),
            issue_monitor_issue_number: Some(issue_number),
            issue_monitor_delivery_id: delivery_id,
            issue_monitor_project_root: Some(project_root.to_path_buf()),
            issue_monitor_session_mode: Some(config.session_mode),
            issue_monitor_autonomous_handoff: autonomous_delivery_attempt.clone(),
            issue_monitor_autonomous_submit_started: false,
            issue_monitor_review_dispatch: false,
        };
        let launch = self.spawn_agent_window_with_feedback_at_geometry(
            tab_id,
            config,
            geometry,
            workspace_resume_context,
            feedback,
        );
        let mut events = match launch {
            Ok(events) => events,
            Err(error) => {
                if let Some(prepared) = autonomous_delivery_attempt {
                    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
                    let _ = gwt::record_autonomous_handoff_delivery_failure_from_prefs(
                        &prefs_path,
                        &prepared.handoff_id,
                        &prepared.session_id,
                        prepared.attempt,
                        &error,
                        &now,
                    );
                }
                return Err(error);
            }
        };
        events.push(OutboundEvent::project(
            context.project_key.clone(),
            BackendEvent::IssueMonitorToast {
                notification_transition: None,
                level: "info".to_string(),
                message: "Issue Monitor resumed existing session".to_string(),
                issue_number: Some(issue_number),
            },
        ));
        Ok((Some(events), None))
    }

    fn issue_monitor_native_conversation_holder(
        &self,
        candidate: &gwt_agent::Session,
    ) -> Option<String> {
        self.issue_monitor_native_conversation_holder_excluding(candidate, None)
    }

    pub(super) fn issue_monitor_native_conversation_holder_excluding(
        &self,
        candidate: &gwt_agent::Session,
        excluded_window_id: Option<&str>,
    ) -> Option<String> {
        let candidate_conversation_id = candidate.exact_resume_session_id()?;
        let matches_candidate = |source_session_id: &str| {
            self.issue_monitor_session_by_id(source_session_id)
                .is_some_and(|source| {
                    source.agent_id == candidate.agent_id
                        && source.exact_resume_session_id() == Some(candidate_conversation_id)
                })
        };
        self.active_agent_sessions
            .iter()
            .find_map(|(window_id, active)| {
                (excluded_window_id != Some(window_id.as_str())
                    && self.window_lookup.contains_key(window_id.as_str())
                    && self
                        .window_status(window_id.as_str())
                        .is_some_and(|status| {
                            !matches!(
                                status,
                                WindowProcessStatus::Stopped | WindowProcessStatus::Error
                            )
                        })
                    && matches_candidate(&active.session_id))
                .then(|| window_id.clone())
            })
            .or_else(|| {
                self.pending_auto_resume_sources.iter().find_map(
                    |(window_id, source_session_id)| {
                        (excluded_window_id != Some(window_id.as_str())
                            && self.window_lookup.contains_key(window_id.as_str())
                            && self
                                .inflight_launches
                                .values()
                                .any(|(pending_window_id, _)| pending_window_id == window_id)
                            && matches_candidate(source_session_id))
                        .then(|| window_id.clone())
                    },
                )
            })
    }

    pub(super) fn issue_monitor_session_by_id(
        &self,
        session_id: &str,
    ) -> Option<gwt_agent::Session> {
        gwt_agent::Session::load(&self.sessions_dir.join(format!("{session_id}.toml")))
            .ok()
            .or_else(|| self.launch_wizard_cache.session_by_id(session_id).cloned())
    }

    fn resolve_silent_issue_monitor_launch_request(
        &self,
        session: &mut LaunchWizardSession,
        project_root: &Path,
        previous_profiles: gwt::LaunchWizardPreviousProfiles,
        prepared_hydration: Option<LaunchWizardHydration>,
    ) -> Result<LaunchWizardLaunchRequest, String> {
        let completion = session.wizard.completion.take().ok_or_else(|| {
            session
                .wizard
                .error
                .clone()
                .unwrap_or_else(|| "Issue Monitor launch settings are incomplete".to_string())
        })?;
        let completion = match completion {
            LaunchWizardCompletion::ResolveRuntime(_config) => {
                let branch_name = session.wizard.branch_name.clone();
                let preferred_agent_id = previous_profiles.preferred_agent_id().map(str::to_string);
                let mut hydration = match prepared_hydration {
                    Some(hydration) => hydration,
                    None => resolve_launch_wizard_runtime_context_hydration(
                        project_root,
                        branch_name,
                        self.launch_wizard_cache.clone(),
                    )?,
                };
                // A silent fresh fallback must use the current saved Monitor
                // profile, not a target-branch Quick Start Session. Leaving
                // these entries populated lets the predecessor conversation
                // overwrite the selected provider/model during hydration.
                hydration.quick_start_entries.clear();
                hydration.previous_profiles = Some(previous_profiles);
                session.wizard.apply_runtime_context(hydration);
                if let Some(agent_id) = preferred_agent_id {
                    session
                        .wizard
                        .apply(gwt::LaunchWizardAction::SetAgent { agent_id });
                }
                session
                    .wizard
                    .apply(gwt::LaunchWizardAction::UseStartMethod {
                        method: gwt::LaunchWizardStartMethodKind::StartWithLastSettings,
                    });
                session.wizard.completion.take().ok_or_else(|| {
                    session.wizard.error.clone().unwrap_or_else(|| {
                        "Issue Monitor launch settings are incomplete".to_string()
                    })
                })?
            }
            completion => completion,
        };
        match completion {
            LaunchWizardCompletion::Launch(config) => Ok(*config),
            LaunchWizardCompletion::FocusWindow { window_id } => Err(format!(
                "Issue Monitor launch resolved to existing window {window_id}"
            )),
            LaunchWizardCompletion::Cancelled => {
                Err("Issue Monitor launch was cancelled".to_string())
            }
            LaunchWizardCompletion::ResolveRuntime(_) => {
                Err("Issue Monitor launch runtime context is unresolved".to_string())
            }
        }
    }

    pub(crate) fn handle_issue_launch_wizard_prepared(
        &mut self,
        prepared: IssueLaunchWizardPrepared,
    ) -> Vec<OutboundEvent> {
        let IssueLaunchWizardPrepared {
            project_context: context,
            client_id,
            id,
            knowledge_kind,
            tab_id,
            project_root,
            issue_number,
            result,
        } = prepared;
        if !self.project_context_is_current(&context) {
            return Vec::new();
        }
        if self.tab(&tab_id).is_none() {
            return vec![OutboundEvent::reply(
                &client_id,
                knowledge_error_event(id, knowledge_kind, "Project tab not found", None, None),
            )];
        }

        match result {
            Ok(base_branch_name) => {
                let Some(linked_issue_kind) = linked_issue_kind_from_knowledge(knowledge_kind)
                else {
                    return vec![OutboundEvent::reply(
                        &client_id,
                        knowledge_error_event(
                            id,
                            knowledge_kind,
                            "Launch Agent is not available for this knowledge bridge",
                            None,
                            None,
                        ),
                    )];
                };
                match self.open_knowledge_launch_wizard_for_base_branch(
                    &tab_id,
                    &project_root,
                    &base_branch_name,
                    issue_number,
                    linked_issue_kind,
                ) {
                    Ok(()) => vec![self.launch_wizard_state_outbound(&context)],
                    Err(error) => vec![OutboundEvent::reply(
                        &client_id,
                        knowledge_error_event(id, knowledge_kind, error, None, None),
                    )],
                }
            }
            Err(error) => vec![OutboundEvent::reply(
                &client_id,
                knowledge_error_event(id, knowledge_kind, error, None, None),
            )],
        }
    }

    #[cfg(test)]
    pub(crate) fn handle_launch_wizard_action(
        &mut self,
        context: &super::ProjectContext,
        action: gwt::LaunchWizardAction,
        bounds: Option<WindowGeometry>,
    ) -> Vec<OutboundEvent> {
        self.handle_launch_wizard_action_for_client(context, None, action, bounds)
    }

    fn manual_launch_generation_disposition(
        &self,
        session: &LaunchWizardSession,
        config: &gwt_agent::LaunchConfig,
    ) -> Result<super::ManualLaunchGenerationDisposition, String> {
        if session.origin != super::LaunchWizardOrigin::ManualLaunchAgent
            || config.session_mode != gwt_agent::SessionMode::Normal
            || config.is_ephemeral
            || config.suppress_execution_control
            || !matches!(
                config.execution_intent,
                gwt_agent::ExecutionLaunchIntent::Automatic
            )
        {
            return Ok(super::ManualLaunchGenerationDisposition::NotApplicable);
        }
        let Some(owner_number) = config.linked_issue_number else {
            return Ok(super::ManualLaunchGenerationDisposition::Genesis);
        };
        let Some(worktree) = config.working_dir.as_deref().or(session
            .wizard
            .context
            .worktree_path
            .as_deref())
        else {
            return Ok(super::ManualLaunchGenerationDisposition::Genesis);
        };
        let owner = match gwt::cli::execution_state::current_generation_owner(worktree) {
            Ok(Some(owner)) => owner,
            Ok(None) => match gwt::cli::execution_state::recovery_generation_owner(worktree) {
                Ok(Some(_)) => {
                    return Ok(super::ManualLaunchGenerationDisposition::Unknown(
                        "Execution authority has an interrupted pointer commit; reconcile it before manual launch"
                            .to_string(),
                    ))
                }
                Ok(None) => return Ok(super::ManualLaunchGenerationDisposition::Genesis),
                Err(error) => {
                    return Ok(super::ManualLaunchGenerationDisposition::Unknown(format!(
                        "Execution authority is incomplete: {error}"
                    )))
                }
            },
            Err(error) => {
                return Ok(super::ManualLaunchGenerationDisposition::Unknown(format!(
                    "Execution authority is unreadable: {error}"
                )))
            }
        };
        if owner.number != owner_number {
            return Ok(super::ManualLaunchGenerationDisposition::Conflict(
                "The linked execution owner changed before manual launch".to_string(),
            ));
        }
        let Some(ledger) = gwt::cli::execution_state::load_generation_ledger(worktree, owner)
            .map_err(|error| error.to_string())?
        else {
            return Ok(super::ManualLaunchGenerationDisposition::Unknown(
                "The current execution owner ledger is missing".to_string(),
            ));
        };
        let status = ledger.current_effective_status().ok_or_else(|| {
            "The current execution generation has no effective status".to_string()
        })?;
        let current = gwt::cli::execution_state::current_execution_binding(worktree, owner)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| {
                "The current execution generation has no canonical binding".to_string()
            })?;
        if let Some(existing) =
            gwt::cli::execution_state::prepared_owner_launch_successor_for_predecessor(
                worktree, owner, &current,
            )
            .map_err(|error| error.to_string())?
        {
            let candidate_session_id = existing.request.initial_session_id.as_str();
            let candidate_path = self
                .sessions_dir
                .join(format!("{candidate_session_id}.toml"));
            match gwt_agent::inspect_session_path(&candidate_path) {
                gwt_agent::SessionPathState::Missing => {
                    let predecessor_kind = match existing.predecessor_status {
                        gwt::cli::execution_state::SuccessorPredecessorStatus::Blocked => {
                            gwt_agent::ManualLaunchSuccessorPredecessor::Blocked
                        }
                        gwt::cli::execution_state::SuccessorPredecessorStatus::Completed => {
                            gwt_agent::ManualLaunchSuccessorPredecessor::Completed
                        }
                        gwt::cli::execution_state::SuccessorPredecessorStatus::Active => {
                            return Ok(super::ManualLaunchGenerationDisposition::Unknown(
                                "A legacy Active successor requires reconciliation".to_string(),
                            ));
                        }
                    };
                    return Ok(super::ManualLaunchGenerationDisposition::Prepare(
                        super::ManualLaunchPreparation {
                            owner,
                            operation_id: existing.request.operation_id,
                            expected_binding: current,
                            expected_session: None,
                            expected_runtime: None,
                            predecessor_kind,
                        },
                    ));
                }
                gwt_agent::SessionPathState::Present(candidate) => {
                    if let Some(window_id) =
                        self.active_agent_sessions
                            .iter()
                            .find_map(|(window_id, active)| {
                                (active.session_id == candidate_session_id
                                    && self.window_lookup.contains_key(window_id))
                                .then(|| window_id.clone())
                            })
                    {
                        return Ok(
                            super::ManualLaunchGenerationDisposition::ExistingSuccessorWindow(
                                window_id,
                            ),
                        );
                    }
                    let exact_candidate =
                        gwt_agent::SessionExecutionIdentity::from_session(&candidate)
                            .map_err(|error| error.to_string())?;
                    let recovery_identity =
                        super::continuation::durable_launch_recovery_session_identity(
                            &self.sessions_dir,
                            candidate_session_id,
                        )?;
                    if exact_candidate.is_some() && exact_candidate == recovery_identity {
                        let predecessor_kind = match existing.predecessor_status {
                            gwt::cli::execution_state::SuccessorPredecessorStatus::Blocked => {
                                gwt_agent::ManualLaunchSuccessorPredecessor::Blocked
                            }
                            gwt::cli::execution_state::SuccessorPredecessorStatus::Completed => {
                                gwt_agent::ManualLaunchSuccessorPredecessor::Completed
                            }
                            gwt::cli::execution_state::SuccessorPredecessorStatus::Active => {
                                return Ok(super::ManualLaunchGenerationDisposition::Unknown(
                                    "A legacy Active successor requires reconciliation".to_string(),
                                ));
                            }
                        };
                        return Ok(super::ManualLaunchGenerationDisposition::Prepare(
                            super::ManualLaunchPreparation {
                                owner,
                                operation_id: existing.request.operation_id,
                                expected_binding: current,
                                expected_session: None,
                                expected_runtime: None,
                                predecessor_kind,
                            },
                        ));
                    }
                    return Ok(super::ManualLaunchGenerationDisposition::Conflict(
                        "A Prepared successor Session already exists outside this window; reconcile it before launching again"
                            .to_string(),
                    ));
                }
                gwt_agent::SessionPathState::Error(error) => {
                    return Ok(super::ManualLaunchGenerationDisposition::Unknown(format!(
                        "The Prepared successor Session is unreadable: {error}"
                    )))
                }
            }
        }
        let predecessor_kind = match status {
            gwt::cli::execution_state::ExecutionControlStatus::Blocked => {
                gwt_agent::ManualLaunchSuccessorPredecessor::Blocked
            }
            gwt::cli::execution_state::ExecutionControlStatus::Completed => {
                gwt_agent::ManualLaunchSuccessorPredecessor::Completed
            }
            gwt::cli::execution_state::ExecutionControlStatus::Active => {
                gwt_agent::ManualLaunchSuccessorPredecessor::ExactTerminalActive
            }
        };
        if status != gwt::cli::execution_state::ExecutionControlStatus::Active {
            return Ok(super::ManualLaunchGenerationDisposition::Prepare(
                super::ManualLaunchPreparation {
                    owner,
                    operation_id: manual_generation_operation_id(owner, &current, predecessor_kind),
                    expected_binding: current,
                    expected_session: None,
                    expected_runtime: None,
                    predecessor_kind,
                },
            ));
        }
        let record = gwt::cli::execution_state::load(worktree)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "The current execution generation has no projection".to_string())?;
        let holder_path = self
            .sessions_dir
            .join(format!("{}.toml", record.primary_session_id));
        let holder = match gwt_agent::inspect_session_path(&holder_path) {
            gwt_agent::SessionPathState::Present(holder) => *holder,
            gwt_agent::SessionPathState::Missing => {
                return Err("The current execution holder Session is missing".to_string())
            }
            gwt_agent::SessionPathState::Error(error) => {
                return Err(format!(
                    "The current execution holder Session is unreadable: {error}"
                ))
            }
        };
        let predecessor = gwt_agent::SessionExecutionIdentity::from_session(&holder)
            .map_err(|error| error.to_string())?
            .filter(|identity| identity.execution_binding.identity == current)
            .ok_or_else(|| {
                "The current execution holder Session binding is not exact".to_string()
            })?;
        let local_runtime = self
            .active_agent_sessions
            .iter()
            .find_map(|(window_id, active)| {
                let runtime_incarnation = self
                    .runtimes
                    .get(window_id)
                    .map(|runtime| runtime.incarnation);
                (active.session_id == predecessor.session_id
                    && crate::runtime_support::same_worktree_path(
                        &active.worktree_path,
                        &predecessor.worktree_path,
                    )
                    && self.window_lookup.contains_key(window_id)
                    && runtime_incarnation.is_some()
                    && self.window_status(window_id).is_some_and(|status| {
                        !matches!(
                            status,
                            WindowProcessStatus::Stopped | WindowProcessStatus::Error
                        )
                    }))
                .then(|| {
                    (
                        window_id.clone(),
                        runtime_incarnation.expect("checked above"),
                    )
                })
            });
        let (local_window_id, local_runtime_incarnation) = local_runtime
            .map(|(window_id, incarnation)| (Some(window_id), Some(incarnation)))
            .unwrap_or((None, None));
        let runtime_disposition = if local_runtime_incarnation.is_some() {
            gwt::cli::execution_state::ExactSessionRuntimeDisposition::Live
        } else {
            gwt::cli::execution_state::classify_exact_session_runtime(
                &self.sessions_dir,
                &predecessor,
            )
            .map_err(|error| error.to_string())?
        };
        let runtime_proof = match runtime_disposition {
            gwt::cli::execution_state::ExactSessionRuntimeDisposition::Terminal(proof)
            | gwt::cli::execution_state::ExactSessionRuntimeDisposition::Defunct(proof) => {
                Some(gwt_agent::ManualLaunchRuntimeEvidence::Proof(proof))
            }
            gwt::cli::execution_state::ExactSessionRuntimeDisposition::Live => {
                local_runtime_incarnation.map(|runtime_incarnation| {
                    gwt_agent::ManualLaunchRuntimeEvidence::Proof(
                        gwt_agent::ManualLaunchRuntimeProof {
                            host_pid: std::process::id(),
                            runtime_incarnation,
                        },
                    )
                })
            }
            // Issue #3457: absence is exact evidence, not a missing proof.
            gwt::cli::execution_state::ExactSessionRuntimeDisposition::Absent => {
                Some(gwt_agent::ManualLaunchRuntimeEvidence::Absent)
            }
            // Issue #3934: a dead Host leaves no fenced proof to carry.
            gwt::cli::execution_state::ExactSessionRuntimeDisposition::HostDead
            | gwt::cli::execution_state::ExactSessionRuntimeDisposition::ChildExited
            | gwt::cli::execution_state::ExactSessionRuntimeDisposition::Unknown => None,
        };
        let fingerprint = manual_holder_fingerprint(owner, &predecessor, local_runtime_incarnation);
        let intent = super::ManualLaunchHolderIntent {
            operation_id: manual_generation_operation_id(owner, &current, predecessor_kind),
            fingerprint,
            owner,
            predecessor,
            predecessor_kind,
            local_window_id,
            local_runtime_incarnation,
            runtime_proof,
        };
        match runtime_disposition {
            gwt::cli::execution_state::ExactSessionRuntimeDisposition::Live => Ok(
                super::ManualLaunchGenerationDisposition::ConfirmLive(intent),
            ),
            gwt::cli::execution_state::ExactSessionRuntimeDisposition::Terminal(_) => {
                if !gwt::cli::execution_state::holder_status_permits_generation_reclaim(
                    holder.status,
                ) {
                    return Ok(super::ManualLaunchGenerationDisposition::Unknown(
                        "The exact runtime is terminal but the holder Session is not durably stopped"
                            .to_string(),
                    ));
                }
                Ok(super::ManualLaunchGenerationDisposition::Prepare(
                    intent.preparation(),
                ))
            }
            gwt::cli::execution_state::ExactSessionRuntimeDisposition::Defunct(_) => Ok(
                super::ManualLaunchGenerationDisposition::Prepare(intent.preparation()),
            ),
            // Issue #3457: the holder published no sidecar in any namespace,
            // so no Host is running it and no proof can ever appear. Unlike a
            // Terminal holder there is no exit record to cross-check against
            // the durable status, so a `.toml` a crashed Host left behind as
            // Running must still be recoverable here.
            gwt::cli::execution_state::ExactSessionRuntimeDisposition::Absent => Ok(
                super::ManualLaunchGenerationDisposition::Prepare(intent.preparation()),
            ),
            // Issue #3934: every Host that wrote a sidecar for the holder is
            // gone, but this route needs a proof to hand the successor. The
            // scan reaper terminalizes such a generation under its own leases.
            gwt::cli::execution_state::ExactSessionRuntimeDisposition::HostDead => {
                Ok(super::ManualLaunchGenerationDisposition::Unknown(
                    "The holder's Hosts are all gone but left no runtime exit proof".to_string(),
                ))
            }
            gwt::cli::execution_state::ExactSessionRuntimeDisposition::ChildExited => {
                Ok(super::ManualLaunchGenerationDisposition::Unknown(
                    "The holder's PTY process has exited. The next Issue Monitor scan releases its generation; retry after that scan."
                        .to_string(),
                ))
            }
            gwt::cli::execution_state::ExactSessionRuntimeDisposition::Unknown => {
                Ok(super::ManualLaunchGenerationDisposition::Unknown(
                    "The holder has no exact runtime exit proof or liveness proof".to_string(),
                ))
            }
        }
    }

    fn manual_holder_durable_identity_is_exact(
        &self,
        intent: &super::ManualLaunchHolderIntent,
    ) -> bool {
        gwt_agent::Session::load(
            &self
                .sessions_dir
                .join(format!("{}.toml", intent.predecessor.session_id)),
        )
        .ok()
        .and_then(|session| gwt_agent::SessionExecutionIdentity::from_session(&session).ok())
        .flatten()
        .as_ref()
            == Some(&intent.predecessor)
    }

    pub(crate) fn handle_launch_wizard_action_for_client(
        &mut self,
        context: &super::ProjectContext,
        client_id: Option<&str>,
        action: gwt::LaunchWizardAction,
        bounds: Option<WindowGeometry>,
    ) -> Vec<OutboundEvent> {
        if !self.project_context_is_current(context) {
            return Vec::new();
        }
        let Some(mut session) = self.take_launch_wizard(context) else {
            return Vec::new();
        };
        let action_stage = Self::launch_wizard_action_error_stage(&action);
        let action_label = Self::launch_wizard_action_label(&action);
        let requested_agent_id = match &action {
            gwt::LaunchWizardAction::SetAgent { agent_id } => Some(agent_id.clone()),
            _ => None,
        };
        if session.wizard.holder_decision.is_some()
            && !matches!(
                action,
                gwt::LaunchWizardAction::Cancel
                    | gwt::LaunchWizardAction::MoveExistingPane { .. }
                    | gwt::LaunchWizardAction::StopAndStartSuccessor { .. }
            )
        {
            let error =
                "Resolve or cancel the current holder decision before changing launch settings"
                    .to_string();
            Self::log_launch_wizard_error(
                &session,
                action_stage,
                action_label,
                requested_agent_id.as_deref(),
                &error,
            );
            session.wizard.error = Some(error);
            self.store_launch_wizard(session);
            return vec![self.launch_wizard_state_outbound(context)];
        }
        if let Some(result) = self.apply_agent_settings_set_action(&mut session, &action) {
            session.wizard.error = result.err();
            if let Some(error) = session.wizard.error.as_deref() {
                Self::log_launch_wizard_error(
                    &session,
                    action_stage,
                    action_label,
                    requested_agent_id.as_deref(),
                    error,
                );
            }
            self.store_launch_wizard(session);
            return vec![self.launch_wizard_state_outbound(context)];
        }
        let mut apply_action = true;
        match &action {
            gwt::LaunchWizardAction::MoveExistingPane {
                fingerprint,
                window_id,
            } => {
                let exact = session.manual_holder_intent.as_ref().is_some_and(|intent| {
                    intent.fingerprint == *fingerprint
                        && self.manual_holder_durable_identity_is_exact(intent)
                        && intent.local_window_id.as_deref() == Some(window_id.as_str())
                        && intent.local_runtime_incarnation.is_some_and(|incarnation| {
                            self.runtimes
                                .get(window_id)
                                .is_some_and(|runtime| runtime.incarnation == incarnation)
                        })
                        && self
                            .active_agent_sessions
                            .get(window_id)
                            .is_some_and(|active| {
                                active.session_id == intent.predecessor.session_id
                                    && self.window_lookup.contains_key(window_id)
                                    && self.window_status(window_id).is_some_and(|status| {
                                        !matches!(
                                            status,
                                            WindowProcessStatus::Stopped
                                                | WindowProcessStatus::Error
                                        )
                                    })
                            })
                });
                if !exact {
                    let error = "The current holder changed; review the refreshed launch decision."
                        .to_string();
                    Self::log_launch_wizard_error(
                        &session,
                        action_stage,
                        action_label,
                        requested_agent_id.as_deref(),
                        &error,
                    );
                    session.wizard.error = Some(error);
                    self.store_launch_wizard(session);
                    return vec![self.launch_wizard_state_outbound(context)];
                }
                let mut events = self.focus_existing_live_work_agent_events(window_id, bounds);
                events.push(self.launch_wizard_state_broadcast(context, None));
                return events;
            }
            gwt::LaunchWizardAction::StopAndStartSuccessor {
                fingerprint,
                window_id,
            } => {
                if bounds.is_none() {
                    let error = "Viewport bounds are required before stopping the current holder"
                        .to_string();
                    Self::log_launch_wizard_error(
                        &session,
                        action_stage,
                        action_label,
                        requested_agent_id.as_deref(),
                        &error,
                    );
                    session.wizard.error = Some(error);
                    self.store_launch_wizard(session);
                    return vec![self.launch_wizard_state_outbound(context)];
                }
                let mut fixed_wizard = session.wizard.clone();
                fixed_wizard.holder_decision = None;
                fixed_wizard.error = None;
                fixed_wizard.apply(gwt::LaunchWizardAction::Submit);
                let Some(LaunchWizardCompletion::Launch(fixed_request)) =
                    fixed_wizard.completion.take()
                else {
                    let error = "The fixed successor launch configuration is no longer available"
                        .to_string();
                    Self::log_launch_wizard_error(
                        &session,
                        action_stage,
                        action_label,
                        requested_agent_id.as_deref(),
                        &error,
                    );
                    session.wizard.error = Some(error);
                    self.store_launch_wizard(session);
                    return vec![self.launch_wizard_state_outbound(context)];
                };
                let LaunchWizardLaunchRequest::Agent(mut fixed_config) = *fixed_request else {
                    let error =
                        "Manual holder handoff requires a fixed Agent launch target".to_string();
                    Self::log_launch_wizard_error(
                        &session,
                        action_stage,
                        action_label,
                        requested_agent_id.as_deref(),
                        &error,
                    );
                    session.wizard.error = Some(error);
                    self.store_launch_wizard(session);
                    return vec![self.launch_wizard_state_outbound(context)];
                };
                let Some(intent) = session.manual_holder_intent.clone().filter(|intent| {
                    intent.fingerprint == *fingerprint
                        && self.manual_holder_durable_identity_is_exact(intent)
                        && intent.local_window_id.as_deref() == Some(window_id.as_str())
                        && intent.local_runtime_incarnation.is_some_and(|incarnation| {
                            self.runtimes
                                .get(window_id)
                                .is_some_and(|runtime| runtime.incarnation == incarnation)
                        })
                        && self
                            .active_agent_sessions
                            .get(window_id)
                            .is_some_and(|active| {
                                active.session_id == intent.predecessor.session_id
                                    && self.window_lookup.contains_key(window_id)
                                    && self.window_status(window_id).is_some_and(|status| {
                                        !matches!(
                                            status,
                                            WindowProcessStatus::Stopped
                                                | WindowProcessStatus::Error
                                        )
                                    })
                            })
                }) else {
                    let error = "The current holder changed; review the refreshed launch decision."
                        .to_string();
                    Self::log_launch_wizard_error(
                        &session,
                        action_stage,
                        action_label,
                        requested_agent_id.as_deref(),
                        &error,
                    );
                    session.wizard.error = Some(error);
                    self.store_launch_wizard(session);
                    return vec![self.launch_wizard_state_outbound(context)];
                };
                let Some(runtime_incarnation) = intent.local_runtime_incarnation else {
                    let error =
                        "The exact holder runtime incarnation is unavailable; no successor was started."
                            .to_string();
                    Self::log_launch_wizard_error(
                        &session,
                        action_stage,
                        action_label,
                        requested_agent_id.as_deref(),
                        &error,
                    );
                    session.wizard.error = Some(error);
                    self.store_launch_wizard(session);
                    return vec![self.launch_wizard_state_outbound(context)];
                };
                if let Err(error) = self.stop_exact_manual_holder_runtime(
                    window_id,
                    runtime_incarnation,
                    &intent.predecessor,
                    true,
                ) {
                    let error = format!(
                        "The holder could not be proven stopped; no successor was started: {error}"
                    );
                    Self::log_launch_wizard_error(
                        &session,
                        action_stage,
                        action_label,
                        requested_agent_id.as_deref(),
                        &error,
                    );
                    session.wizard.error = Some(error);
                    self.store_launch_wizard(session);
                    return vec![self.launch_wizard_state_outbound(context)];
                }
                let stopped = gwt_agent::Session::load(
                    &self
                        .sessions_dir
                        .join(format!("{}.toml", intent.predecessor.session_id)),
                )
                .ok()
                .and_then(|session| {
                    gwt_agent::SessionExecutionIdentity::from_session(&session).ok()
                })
                .flatten()
                .is_some_and(|identity| {
                    identity == intent.predecessor
                        && matches!(
                            gwt_agent::Session::load(
                                &self
                                    .sessions_dir
                                    .join(format!("{}.toml", identity.session_id))
                            )
                            .map(|session| session.status),
                            Ok(gwt_agent::AgentStatus::Stopped
                                | gwt_agent::AgentStatus::Interrupted)
                        )
                });
                if !stopped {
                    let error = "The holder could not be proven stopped; no successor was started."
                        .to_string();
                    Self::log_launch_wizard_error(
                        &session,
                        action_stage,
                        action_label,
                        requested_agent_id.as_deref(),
                        &error,
                    );
                    session.wizard.error = Some(error);
                    self.store_launch_wizard(session);
                    return vec![self.launch_wizard_state_outbound(context)];
                }
                fixed_config.execution_intent = gwt_agent::ExecutionLaunchIntent::ManualSuccessor {
                    operation_id: intent.operation_id,
                    expected_binding: intent.predecessor.execution_binding.identity.clone(),
                    expected_predecessor: Some(Box::new(intent.predecessor)),
                    expected_runtime: intent.runtime_proof,
                    predecessor_kind: intent.predecessor_kind,
                };
                session.wizard.holder_decision = None;
                session.wizard.error = None;
                session.wizard.completion = Some(LaunchWizardCompletion::Launch(Box::new(
                    LaunchWizardLaunchRequest::Agent(fixed_config),
                )));
                apply_action = false;
            }
            _ => {}
        }
        if apply_action {
            session.wizard.apply(action);
        }
        let automatic_agent_launch = match session.wizard.completion.as_ref() {
            Some(LaunchWizardCompletion::Launch(request)) => match request.as_ref() {
                LaunchWizardLaunchRequest::Agent(config)
                    if matches!(
                        config.execution_intent,
                        gwt_agent::ExecutionLaunchIntent::Automatic
                    ) =>
                {
                    Some(config.as_ref().clone())
                }
                _ => None,
            },
            _ => None,
        };
        if let Some(automatic_agent_launch) = automatic_agent_launch {
            match self.manual_launch_generation_disposition(&session, &automatic_agent_launch) {
                Ok(super::ManualLaunchGenerationDisposition::NotApplicable)
                | Ok(super::ManualLaunchGenerationDisposition::Genesis) => {}
                Ok(super::ManualLaunchGenerationDisposition::Prepare(preparation)) => {
                    if let Some(LaunchWizardCompletion::Launch(request)) =
                        session.wizard.completion.as_mut()
                    {
                        if let LaunchWizardLaunchRequest::Agent(config) = request.as_mut() {
                            config.execution_intent =
                                gwt_agent::ExecutionLaunchIntent::ManualSuccessor {
                                    operation_id: preparation.operation_id,
                                    expected_binding: preparation.expected_binding,
                                    expected_predecessor: preparation
                                        .expected_session
                                        .map(Box::new),
                                    expected_runtime: preparation.expected_runtime,
                                    predecessor_kind: preparation.predecessor_kind,
                                };
                        }
                    }
                }
                Ok(super::ManualLaunchGenerationDisposition::ExistingSuccessorWindow(
                    window_id,
                )) => {
                    session.wizard.completion =
                        Some(LaunchWizardCompletion::FocusWindow { window_id });
                }
                Ok(super::ManualLaunchGenerationDisposition::ConfirmLive(intent)) => {
                    let summary = format!(
                        "{} · Session {}",
                        intent.predecessor.agent_id.display_name(),
                        intent.predecessor.session_id
                    );
                    session.wizard.holder_decision = Some(intent.decision_view(summary));
                    session.manual_holder_intent = Some(intent);
                    session.wizard.completion = None;
                }
                Ok(super::ManualLaunchGenerationDisposition::Conflict(error))
                | Ok(super::ManualLaunchGenerationDisposition::Unknown(error))
                | Err(error) => {
                    session.wizard.error = Some(error);
                    session.wizard.completion = None;
                }
            }
        }
        if let Some(error) = session.wizard.error.as_deref() {
            Self::log_launch_wizard_error(
                &session,
                action_stage,
                action_label,
                requested_agent_id.as_deref(),
                error,
            );
        }

        match session.wizard.completion.take() {
            Some(LaunchWizardCompletion::Cancelled) => {
                vec![self.launch_wizard_state_broadcast(context, None)]
            }
            Some(LaunchWizardCompletion::FocusWindow { window_id }) => {
                let Some(address) = self.window_lookup.get(&window_id).cloned() else {
                    let error = "The selected session window is no longer available".to_string();
                    Self::log_launch_wizard_error(
                        &session,
                        "focus_window",
                        action_label,
                        requested_agent_id.as_deref(),
                        &error,
                    );
                    session.wizard.error = Some(error);
                    self.store_launch_wizard(session);
                    return vec![self.launch_wizard_state_outbound(context)];
                };
                let Some(tab) = self.tab_mut(&address.tab_id) else {
                    let error = "Project tab not found".to_string();
                    Self::log_launch_wizard_error(
                        &session,
                        "focus_window",
                        action_label,
                        requested_agent_id.as_deref(),
                        &error,
                    );
                    session.wizard.error = Some(error);
                    self.store_launch_wizard(session);
                    return vec![self.launch_wizard_state_outbound(context)];
                };
                if !tab.workspace.focus_window(&address.raw_id, None) {
                    let error = "The selected session window is no longer available".to_string();
                    Self::log_launch_wizard_error(
                        &session,
                        "focus_window",
                        action_label,
                        requested_agent_id.as_deref(),
                        &error,
                    );
                    session.wizard.error = Some(error);
                    self.store_launch_wizard(session);
                    return vec![self.launch_wizard_state_outbound(context)];
                }
                let _ = self.persist();
                let mut events = vec![self.workspace_state_broadcast(context)];
                events.push(self.launch_wizard_state_broadcast(context, None));
                events
            }
            Some(LaunchWizardCompletion::ResolveRuntime(_config)) => {
                let Some(project_root) = self
                    .tab(&session.tab_id)
                    .map(|tab| tab.project_root.clone())
                else {
                    let error = "Project tab not found".to_string();
                    Self::log_launch_wizard_error(
                        &session,
                        "resolve_runtime",
                        action_label,
                        requested_agent_id.as_deref(),
                        &error,
                    );
                    session.wizard.error = Some(error);
                    self.store_launch_wizard(session);
                    return vec![self.launch_wizard_state_outbound(context)];
                };
                let wizard_id = session.wizard_id.clone();
                let branch_name = session.wizard.branch_name.clone();
                let cache = self.launch_wizard_cache.clone();
                let proxy = self.proxy.clone();
                session
                    .wizard
                    .mark_runtime_resolution_pending("Preparing runtime context...");
                thread::spawn(move || {
                    let result = resolve_launch_wizard_runtime_context_hydration(
                        &project_root,
                        branch_name,
                        cache,
                    );
                    proxy.send(UserEvent::LaunchWizardRuntimeResolved {
                        wizard_id,
                        result: Box::new(result),
                    });
                });
                self.store_launch_wizard(session);
                vec![self.launch_wizard_state_outbound(context)]
            }
            Some(LaunchWizardCompletion::Launch(config)) => {
                if let Some(save_context) = session.issue_monitor_profile_save.clone() {
                    return self.save_issue_monitor_profile_from_launch_request(
                        session,
                        save_context,
                        *config,
                    );
                }
                let Some(bounds) = bounds else {
                    let error = "Viewport bounds are required to launch a window".to_string();
                    Self::log_launch_wizard_error(
                        &session,
                        "launch_bounds",
                        action_label,
                        requested_agent_id.as_deref(),
                        &error,
                    );
                    session.wizard.error = Some(error);
                    self.store_launch_wizard(session);
                    return vec![self.launch_wizard_state_outbound(context)];
                };
                session
                    .wizard
                    .mark_launch_materialization_pending("Preparing worktree...");
                self.project_state_mut(context)
                    .expect("current wizard project")
                    .pending_launch_wizard_materializations
                    .insert(session.wizard_id.clone(), session.clone());
                self.proxy
                    .send(UserEvent::LaunchWizardLaunchMaterializationRequested {
                        wizard_id: session.wizard_id.clone(),
                        client_id: client_id.map(str::to_string),
                        config,
                        bounds,
                    });
                self.store_launch_wizard(session);
                vec![self.launch_wizard_state_outbound(context)]
            }
            None => {
                self.store_launch_wizard(session);
                vec![self.launch_wizard_state_outbound(context)]
            }
        }
    }

    pub(crate) fn handle_launch_wizard_launch_materialization_requested(
        &mut self,
        wizard_id: String,
        client_id: Option<String>,
        config: LaunchWizardLaunchRequest,
        bounds: WindowGeometry,
    ) -> Vec<OutboundEvent> {
        let Some(pending_session) = self.project_states.values_mut().find_map(|state| {
            state
                .pending_launch_wizard_materializations
                .remove(&wizard_id)
        }) else {
            return Vec::new();
        };
        let context = pending_session.project_context.clone();
        if !self.project_context_is_current(&context) {
            return Vec::new();
        }
        let owns_visible_slot = self
            .launch_wizard_for(&context)
            .is_some_and(|session| session.wizard_id == wizard_id);
        let mut session = if owns_visible_slot {
            self.take_launch_wizard(&context)
                .expect("matching visible launch wizard")
        } else {
            pending_session
        };
        let issue_monitor_project_root = session
            .issue_monitor_launch_issue_number
            .and_then(|_| self.tab(&session.tab_id))
            .map(|tab| tab.project_root.clone());

        match config {
            LaunchWizardLaunchRequest::Agent(config) => self.materialize_launch_wizard_agent_with(
                session,
                owns_visible_slot,
                config,
                move |runtime, session, config| {
                    let workspace_resume_context = session.workspace_resume_context.clone();
                    let launch_feedback_context =
                        client_id.map(|client_id| LaunchFeedbackContext {
                            client_id,
                            title: if session.wizard.wizard_mode == gwt::LaunchWizardMode::StartWork
                            {
                                "Start Work".to_string()
                            } else {
                                "Launch Agent".to_string()
                            },
                            issue_monitor_issue_number: session.issue_monitor_launch_issue_number,
                            issue_monitor_delivery_id: None,
                            issue_monitor_project_root: issue_monitor_project_root.clone(),
                            issue_monitor_session_mode: Some(config.session_mode),
                            issue_monitor_autonomous_handoff: None,
                            issue_monitor_autonomous_submit_started: false,
                            issue_monitor_review_dispatch: false,
                        });
                    if let Some(target) = session.agent_kanban_target.clone() {
                        runtime.spawn_agent_window_in_agent_kanban(
                            &session.tab_id,
                            *config,
                            bounds,
                            workspace_resume_context,
                            launch_feedback_context,
                            target,
                        )
                    } else if let Some(launch_feedback_context) = launch_feedback_context {
                        runtime.spawn_agent_window_with_feedback(
                            &session.tab_id,
                            *config,
                            bounds,
                            workspace_resume_context,
                            launch_feedback_context,
                        )
                    } else {
                        runtime.spawn_agent_window(
                            &session.tab_id,
                            *config,
                            bounds,
                            workspace_resume_context,
                        )
                    }
                },
            ),
            LaunchWizardLaunchRequest::Shell(config) => {
                match self.spawn_wizard_shell_window(&session.tab_id, *config, bounds) {
                    Ok(mut events) => {
                        if owns_visible_slot {
                            events.insert(0, self.launch_wizard_state_broadcast(&context, None));
                        }
                        events
                    }
                    Err(error) => {
                        Self::log_launch_wizard_error(
                            &session,
                            "spawn_shell_window",
                            "submit",
                            None,
                            &error,
                        );
                        session.wizard.clear_launch_materialization_pending();
                        session.wizard.error = Some(error);
                        if owns_visible_slot {
                            self.store_launch_wizard(session);
                            vec![self.launch_wizard_state_outbound(&context)]
                        } else {
                            Vec::new()
                        }
                    }
                }
            }
        }
    }

    pub(super) fn materialize_launch_wizard_agent_with<F>(
        &mut self,
        mut session: LaunchWizardSession,
        owns_visible_slot: bool,
        mut config: Box<gwt_agent::LaunchConfig>,
        spawn: F,
    ) -> Vec<OutboundEvent>
    where
        F: FnOnce(
            &mut Self,
            &LaunchWizardSession,
            Box<gwt_agent::LaunchConfig>,
        ) -> Result<Vec<OutboundEvent>, String>,
    {
        let context = session.project_context.clone();
        if !self.project_context_is_current(&context) {
            return Vec::new();
        }
        let manual_project_root = self
            .tab(&session.tab_id)
            .map(|tab| tab.project_root.clone())
            .ok_or_else(|| "Project tab not found".to_string());
        if let Err(error) = manual_project_root.and_then(|project_root| {
            self.prepare_manual_successor_before_pane(&project_root, &mut config)
        }) {
            Self::log_launch_wizard_error(
                &session,
                "manual_successor_preflight",
                "submit",
                Some(config.agent_id.command()),
                &error,
            );
            session.wizard.clear_launch_materialization_pending();
            session.wizard.error = Some(error);
            return if owns_visible_slot {
                self.store_launch_wizard(session);
                vec![self.launch_wizard_state_outbound(&context)]
            } else {
                Vec::new()
            };
        }
        let requested_agent_id = config.agent_id.command().to_string();
        let spawn_result = spawn(self, &session, config);
        self.finish_launch_wizard_agent_spawn(
            session,
            owns_visible_slot,
            requested_agent_id.as_str(),
            spawn_result,
        )
    }

    pub(super) fn finish_launch_wizard_agent_spawn(
        &mut self,
        mut session: LaunchWizardSession,
        owns_visible_slot: bool,
        requested_agent_id: &str,
        spawn_result: Result<Vec<OutboundEvent>, String>,
    ) -> Vec<OutboundEvent> {
        let context = session.project_context.clone();
        if !self.project_context_is_current(&context) {
            return Vec::new();
        }
        match spawn_result {
            Ok(mut events) => {
                if owns_visible_slot {
                    events.insert(0, self.launch_wizard_state_broadcast(&context, None));
                }
                events
            }
            Err(error) => {
                Self::log_launch_wizard_error(
                    &session,
                    "spawn_agent_window",
                    "submit",
                    Some(requested_agent_id),
                    &error,
                );
                session.wizard.clear_launch_materialization_pending();
                session.wizard.error = Some(error);
                if owns_visible_slot {
                    self.store_launch_wizard(session);
                    vec![self.launch_wizard_state_outbound(&context)]
                } else {
                    Vec::new()
                }
            }
        }
    }

    pub(crate) fn prepare_manual_successor_before_pane(
        &self,
        project_root: &Path,
        config: &mut Box<gwt_agent::LaunchConfig>,
    ) -> Result<(), String> {
        let gwt_agent::ExecutionLaunchIntent::ManualSuccessor {
            operation_id,
            expected_binding,
            expected_predecessor,
            expected_runtime,
            predecessor_kind,
        } = config.execution_intent.clone()
        else {
            return Ok(());
        };
        resolve_launch_worktree(project_root, config.as_mut())?;
        let worktree = config
            .working_dir
            .clone()
            .ok_or_else(|| "Manual successor preflight did not resolve a worktree".to_string())?;
        let owner_number = config
            .linked_issue_number
            .ok_or_else(|| "Manual successor preflight requires a linked owner".to_string())?;
        let owner = gwt::cli::execution_state::ExecutionOwnerKey {
            kind: gwt::cli::execution_state::detect_owner_kind(&worktree, owner_number),
            number: owner_number,
        };
        if expected_predecessor.as_deref().is_some_and(|identity| {
            identity.execution_binding.owner_kind != owner.kind.as_str()
                || identity.execution_binding.owner_number != owner.number
                || identity.execution_binding.identity != expected_binding
        }) {
            return Err("Manual successor predecessor changed before preflight".to_string());
        }
        let issuer = self.agent_capability_issuer.as_ref().ok_or_else(|| {
            "Manual successor preflight is missing its Host capability issuer".to_string()
        })?;
        let reservation = if predecessor_kind
            == gwt_agent::ManualLaunchSuccessorPredecessor::ExactTerminalActive
        {
            let predecessor = expected_predecessor.as_deref().ok_or_else(|| {
                "Active manual successor requires an exact predecessor Session".to_string()
            })?;
            Some(issuer.reserve_manual_execution_handoff(&predecessor.execution_binding)?)
        } else {
            None
        };
        let result = (|| {
            let candidate_session_id =
                manual_successor_stable_component("manual-successor", &operation_id);
            let source = match predecessor_kind {
                gwt_agent::ManualLaunchSuccessorPredecessor::Completed => {
                    gwt::cli::execution_state::MANUAL_COMPLETED_OWNER_LAUNCH_SOURCE
                }
                gwt_agent::ManualLaunchSuccessorPredecessor::Blocked
                | gwt_agent::ManualLaunchSuccessorPredecessor::ExactTerminalActive => {
                    gwt::cli::execution_state::FRESH_LINKED_OWNER_LAUNCH_SOURCE
                }
            };
            let predecessor_status = match predecessor_kind {
                gwt_agent::ManualLaunchSuccessorPredecessor::Completed => {
                    gwt::cli::execution_state::SuccessorPredecessorStatus::Completed
                }
                gwt_agent::ManualLaunchSuccessorPredecessor::Blocked => {
                    gwt::cli::execution_state::SuccessorPredecessorStatus::Blocked
                }
                gwt_agent::ManualLaunchSuccessorPredecessor::ExactTerminalActive => {
                    gwt::cli::execution_state::SuccessorPredecessorStatus::Active
                }
            };
            let existing = gwt::cli::execution_state::continuation_attempt_for_operation(
                &worktree,
                owner,
                &operation_id,
            )
            .map_err(|error| error.to_string())?;
            let requested_at = gwt::cli::execution_state::load_generation_ledger(&worktree, owner)
                .map_err(|error| error.to_string())?
                .and_then(|ledger| {
                    ledger
                        .current_generation()
                        .map(|generation| generation.identity.activated_at)
                })
                .ok_or_else(|| {
                    "Manual successor predecessor generation is unavailable".to_string()
                })?;
            let request = existing.map(|attempt| attempt.request).unwrap_or_else(|| {
                gwt::cli::execution_state::SuccessorRequest {
                    operation_id: operation_id.clone(),
                    principal_id: "gwt-host-manual-launch".to_string(),
                    work_id: None,
                    source: source.to_string(),
                    session_binding_id: manual_successor_stable_component(
                        "manual-binding",
                        &operation_id,
                    ),
                    initial_session_id: candidate_session_id.clone(),
                    entrypoint: gwt::cli::execution_state::entrypoint_from_launch(
                        config.entrypoint_args(),
                        false,
                    ),
                    requested_at,
                }
            });
            super::continuation::persist_durable_launch_recovery(
                &self.sessions_dir,
                super::continuation::DurableLaunchRecoveryKind::FreshSuccessor {
                    operation_id: operation_id.clone(),
                },
                &request.initial_session_id,
                project_root,
                &worktree,
                owner,
                None,
                None,
            )?;
            let attempt = gwt::cli::execution_state::prepare_exact_manual_launch_successor(
                &worktree,
                owner,
                &request,
                gwt::cli::execution_state::ExactManualLaunchPredecessor {
                    sessions_dir: &self.sessions_dir,
                    session: expected_predecessor.as_deref(),
                    runtime: expected_runtime,
                    binding: &expected_binding,
                    status: predecessor_status,
                    terminal_reason:
                        "exact producing runtime terminated before manual Launch Agent",
                },
            );
            let attempt = match attempt {
                Ok(attempt) => attempt,
                Err(error) => {
                    if gwt::cli::execution_state::continuation_attempt_for_operation(
                        &worktree,
                        owner,
                        &operation_id,
                    )
                    .is_ok_and(|attempt| attempt.is_none())
                    {
                        let _ = super::continuation::clear_durable_launch_recovery(
                            &self.sessions_dir,
                            &request.initial_session_id,
                        );
                    }
                    return Err(error.to_string());
                }
            };
            if attempt.status != gwt::cli::execution_state::ContinuationAttemptStatus::Prepared {
                return Err(format!(
                    "Manual successor operation is {:?}; reconcile before retrying",
                    attempt.status
                ));
            }
            let identity = gwt::cli::execution_state::prepared_successor_execution_binding(
                &worktree,
                owner,
                &attempt.request,
            )
            .map_err(|error| error.to_string())?;
            let repo_hash = gwt_core::repo_hash::detect_repo_hash(&worktree)
                .ok_or_else(|| "Manual successor repository hash is unavailable".to_string())?;
            config.execution_intent = gwt_agent::ExecutionLaunchIntent::PreparedManualSuccessor(
                gwt_agent::SessionExecutionBinding {
                    schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
                    session_id: attempt.request.initial_session_id,
                    repo_hash: repo_hash.as_str().to_string(),
                    owner_kind: owner.kind.as_str().to_string(),
                    owner_number: owner.number,
                    identity,
                    capability_generation: 1,
                },
            );
            Ok(())
        })();
        if let Some(reservation) = reservation.as_ref() {
            let finalized = if result.is_ok() {
                issuer.commit_manual_execution_handoff(reservation)
            } else {
                issuer.rollback_manual_execution_handoff(reservation)
            };
            if !finalized && result.is_ok() {
                return Err("Manual successor handoff reservation was lost".to_string());
            }
        }
        result
    }

    pub(super) fn save_issue_monitor_profile_from_launch_request(
        &mut self,
        mut session: LaunchWizardSession,
        save_context: IssueMonitorProfileSaveContext,
        config: LaunchWizardLaunchRequest,
    ) -> Vec<OutboundEvent> {
        let context = session.project_context.clone();
        if !self.project_context_is_current(&context) {
            return Vec::new();
        }
        let IssueMonitorProfileSaveContext {
            client_id,
            issue_number,
            pool: opened_pool,
            sets,
        } = save_context;
        let LaunchWizardLaunchRequest::Agent(config) = config else {
            session.wizard.error =
                Some("Issue Monitor settings require an agent launch target".to_string());
            self.store_launch_wizard(session);
            return vec![self.launch_wizard_state_outbound(&context)];
        };
        let Some(project_root) = self
            .tab(&session.tab_id)
            .map(|tab| tab.project_root.clone())
        else {
            session.wizard.error = Some("Project tab not found".to_string());
            self.store_launch_wizard(session);
            return vec![self.launch_wizard_state_outbound(&context)];
        };
        let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&project_root);
        let launch_profile = gwt::IssueMonitorLaunchProfile::from(config.as_ref());
        // Issue #4911: the settings form saves every Agent Settings set, in
        // order; the open one is the profile this request carries.
        let sets = sets.map(|sets| sets.resolved(Some(launch_profile.clone()), true));
        if let Some(error) = sets.as_deref().and_then(agent_settings_duplicate_error) {
            session.wizard.error = Some(error);
            self.store_launch_wizard(session);
            return vec![self.launch_wizard_state_outbound(&context)];
        }
        let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
            std::time::Instant::now() + std::time::Duration::from_millis(250),
        );
        let saved = gwt::mutate_issue_monitor_prefs_recovering(
            &prefs_path,
            &gwt::IssueMonitorPrefs::recovery_default(),
            |prefs| {
                // Issue #4911 AC-8: compared under the prefs lock, so a pool
                // written since the form opened is never replaced unseen.
                if sets.is_some() && prefs.launch_profile_pool() != opened_pool {
                    return Err(AGENT_SETTINGS_CHANGED_ELSEWHERE_REASON.to_string());
                }
                if prefs.advance_effect_authority_epoch().is_none() {
                    return Err(
                        "Failed to save Issue Monitor settings: authority epoch overflow"
                            .to_string(),
                    );
                }
                match sets {
                    Some(sets) => prefs.set_launch_profile_pool(sets),
                    // SPEC #3914 FR-003 / Issue #4079 AC-1: the per-Issue
                    // Agent Settings save is a switch, so it writes the pool
                    // head (and the `launch_profile` mirror the Monitor
                    // launches from). Upserting by provider left the head —
                    // and therefore the effective agent — untouched whenever
                    // the chosen provider already sat further down the pool.
                    None => prefs.set_head_launch_profile(launch_profile),
                }
                Ok(())
            },
        );
        match saved {
            Ok((_, Ok(()))) => {}
            Ok((_, Err(error))) => {
                session.wizard.error = Some(error);
                self.store_launch_wizard(session);
                return vec![self.launch_wizard_state_outbound(&context)];
            }
            Err(error) => {
                session.wizard.error =
                    Some(format!("Failed to save Issue Monitor settings: {error}"));
                self.store_launch_wizard(session);
                return vec![self.launch_wizard_state_outbound(&context)];
            }
        }
        let mut events = vec![
            self.launch_wizard_state_broadcast(&context, None),
            OutboundEvent::reply(
                &client_id,
                BackendEvent::IssueMonitorToast {
                    notification_transition: None,
                    level: "info".to_string(),
                    message: "Issue Monitor settings saved".to_string(),
                    issue_number,
                },
            ),
        ];
        events.extend(self.local_issue_monitor_events_for(&context, Some(&client_id), |_| {}));
        events
    }

    pub(crate) fn handle_launch_wizard_runtime_resolved(
        &mut self,
        wizard_id: String,
        result: Result<LaunchWizardHydration, String>,
    ) -> Vec<OutboundEvent> {
        let Some(context) = self.project_context_for_wizard(&wizard_id) else {
            return Vec::new();
        };
        let Some(mut session) = self.take_launch_wizard(&context) else {
            return Vec::new();
        };
        if session.wizard_id != wizard_id {
            self.store_launch_wizard(session);
            return Vec::new();
        }
        match result {
            Ok(mut hydration) => {
                // Issue #4911: the Runtime step proposes where the open Agent
                // Settings set runs, so it starts from that set's own saved
                // runtime rather than from the last launch in this repository.
                if let Some(open_set) = session
                    .issue_monitor_profile_save
                    .as_ref()
                    .and_then(|save_context| save_context.sets.as_ref())
                    .and_then(|sets| sets.profiles.get(sets.active))
                {
                    hydration.previous_profiles = hydration
                        .previous_profiles
                        .map(|profiles| profiles.with_repo_local(Some(open_set.clone().into())));
                }
                session.wizard.apply_runtime_context(hydration);
                let auto_submit_bounds = session.auto_submit_after_runtime_resolution.take();
                self.store_launch_wizard(session);
                if let Some(bounds) = auto_submit_bounds {
                    return self.handle_launch_wizard_action_for_client(
                        &context,
                        None,
                        gwt::LaunchWizardAction::Submit,
                        Some(bounds),
                    );
                }
                vec![self.launch_wizard_state_outbound(&context)]
            }
            Err(error) => {
                Self::log_launch_wizard_error(
                    &session,
                    "resolve_runtime",
                    "runtime_resolved",
                    None,
                    &error,
                );
                session.wizard.set_hydration_error(error);
                self.store_launch_wizard(session);
                vec![self.launch_wizard_state_outbound(&context)]
            }
        }
    }
}

fn resolve_launch_wizard_runtime_context_hydration(
    project_root: &Path,
    branch_name: String,
    cache: LaunchWizardMemoryCache,
) -> Result<LaunchWizardHydration, String> {
    let (context_path, resolved_worktree_path) =
        launch_runtime_context_paths(project_root, &branch_name);
    let quick_start_entries = cache.quick_start_entries(&context_path, &branch_name);
    let previous_profiles = cache.previous_profiles(&context_path);
    let agent_options = cache.agent_options();
    let (docker_context, docker_service_status) =
        detect_wizard_docker_context_and_status(&context_path);
    Ok(LaunchWizardHydration {
        selected_branch: None,
        normalized_branch_name: branch_name,
        worktree_path: resolved_worktree_path,
        quick_start_root: context_path,
        docker_context,
        docker_service_status,
        agent_options,
        quick_start_entries,
        previous_profiles: Some(previous_profiles),
        // Runtime re-resolution preserves picker candidates set at first hydration.
        open_branch_candidates: Vec::new(),
    })
}

fn launch_runtime_context_paths(
    project_root: &Path,
    branch_name: &str,
) -> (PathBuf, Option<PathBuf>) {
    let worktrees = launch_runtime_worktrees(project_root);
    if let Some(worktree_path) = worktrees
        .as_deref()
        .and_then(|worktrees| usable_worktree_path_for_branch(worktrees, branch_name))
    {
        return (worktree_path.clone(), Some(worktree_path));
    }
    if project_root_is_git_worktree(project_root) {
        return (project_root.to_path_buf(), None);
    }
    if let Some(default_worktree_path) = worktrees
        .as_deref()
        .and_then(default_runtime_detection_worktree_path)
    {
        return (default_worktree_path, None);
    }
    (project_root.to_path_buf(), None)
}

fn launch_runtime_worktrees(project_root: &Path) -> Option<Vec<gwt_git::WorktreeInfo>> {
    let main_repo_path = gwt_git::worktree::main_worktree_root(project_root).ok()?;
    gwt_git::WorktreeManager::new(&main_repo_path).list().ok()
}

fn project_root_is_git_worktree(project_root: &Path) -> bool {
    let output = gwt_core::process::hidden_command("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(project_root)
        .output();
    output.is_ok_and(|output| {
        output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "true"
    })
}

fn default_runtime_detection_worktree_path(worktrees: &[gwt_git::WorktreeInfo]) -> Option<PathBuf> {
    ["develop", "main"]
        .iter()
        .find_map(|branch| usable_worktree_path_for_branch(worktrees, branch))
}

impl AppRuntime {
    pub(crate) fn spawn_wizard_shell_window(
        &mut self,
        tab_id: &str,
        config: ShellLaunchConfig,
        bounds: WindowGeometry,
    ) -> Result<Vec<OutboundEvent>, String> {
        let context = self
            .project_context(tab_id)
            .ok_or_else(|| "Project tab not found".to_string())?;
        let tab = self
            .tab_mut(tab_id)
            .ok_or_else(|| "Project tab not found".to_string())?;
        let project_root = tab.project_root.display().to_string();
        let project_root_path = tab.project_root.clone();
        let title = format!(
            "{} · {}",
            config.display_name,
            config.branch.as_ref().unwrap_or(&"work".to_string())
        );
        let window = tab
            .workspace
            .add_window_with_title(WindowPreset::Shell, title, false, bounds);
        self.register_window(tab_id, &window.id);
        let window_id = combined_window_id(tab_id, &window.id);

        self.window_pty_statuses
            .insert(window_id.clone(), WindowProcessStatus::Running);
        self.window_hook_states.remove(&window_id);

        // SPEC-2359 US-80 (FR-427): register the Start-Work Shell as a
        // first-class Work so it appears in the Active Work / Workspace
        // projection like an agent. `config.branch` is set even for new
        // branches, so the branch-derived Work id is stable before the worktree
        // exists; `config.working_dir` is `None` until the async launch creates
        // a new-branch worktree.
        let live_session_ids: std::collections::HashSet<String> = self
            .active_agent_sessions
            .values()
            .map(|session| session.session_id.clone())
            .collect();
        let shell_work_registered = match save_shell_work_projection(
            &project_root_path,
            &window_id,
            config.working_dir.clone(),
            config.branch.clone(),
            &live_session_ids,
        ) {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!(
                    project_root = %project_root_path.display(),
                    window_id = %window_id,
                    error = %error,
                    "shell Work projection registration skipped"
                );
                false
            }
        };

        let mut events = vec![self.workspace_state_broadcast(&context)];
        if shell_work_registered {
            if let Some(tab) = self.tab(tab_id) {
                if let Some(projection) = self.active_work_projection_for_tab(tab_id, tab) {
                    events.push(OutboundEvent::project(
                        context.project_key.clone(),
                        BackendEvent::ActiveWorkProjection {
                            projection: Box::new(projection),
                        },
                    ));
                }
            }
        }
        events.extend(self.status_events(
            window_id.clone(),
            WindowProcessStatus::Running,
            Some("Launching...".to_string()),
        ));

        let proxy = self.proxy.for_project(context);
        let profile_config_path = self.profile_config_path()?;
        thread::spawn(move || {
            Self::spawn_wizard_shell_window_async(
                proxy,
                project_root,
                window_id,
                config,
                profile_config_path,
            );
        });

        Ok(events)
    }

    pub(crate) fn spawn_wizard_shell_window_async(
        proxy: AppEventProxy,
        project_root: String,
        window_id: String,
        mut config: ShellLaunchConfig,
        profile_config_path: PathBuf,
    ) {
        let result = (|| {
            proxy.send(UserEvent::LaunchProgress {
                window_id: window_id.clone(),
                message: "Preparing worktree...".to_string(),
            });
            resolve_shell_launch_worktree(Path::new(&project_root), &mut config)?;
            let worktree_path = config
                .working_dir
                .clone()
                .unwrap_or_else(|| PathBuf::from(&project_root));
            gwt_agent::LaunchEnvironment::from_active_profile(
                &profile_config_path,
                config.runtime_target,
            )?
            .with_project_root(&worktree_path)
            .apply_to_parts(&mut config.env_vars, &mut config.remove_env);

            if config.runtime_target == gwt_agent::LaunchRuntimeTarget::Docker {
                proxy.send(UserEvent::LaunchProgress {
                    window_id: window_id.clone(),
                    message: "Starting Docker service...".to_string(),
                });
            }

            build_shell_process_launch(Path::new(&project_root), &mut config)
        })();

        proxy.send(UserEvent::ShellLaunchComplete {
            window_id,
            result: Box::new(result),
        });
    }

    pub(super) fn refresh_open_launch_wizard_from_cache(
        &mut self,
        project: &super::ProjectContext,
    ) {
        let Some(context) = self
            .launch_wizard_for(project)
            .map(|session| session.wizard.context.clone())
        else {
            return;
        };
        let agent_options = self.launch_wizard_cache.agent_options();
        let quick_start_entries = self
            .launch_wizard_cache
            .quick_start_entries(&context.quick_start_root, &context.normalized_branch_name);
        let Some(session) = self.launch_wizard_for_mut(project) else {
            return;
        };
        session.wizard.apply_hydration(LaunchWizardHydration {
            selected_branch: Some(context.selected_branch),
            normalized_branch_name: context.normalized_branch_name,
            worktree_path: context.worktree_path,
            quick_start_root: context.quick_start_root,
            docker_context: context.docker_context,
            docker_service_status: context.docker_service_status,
            agent_options,
            quick_start_entries,
            previous_profiles: None,
            // Cache refresh preserves picker candidates set at first hydration.
            open_branch_candidates: Vec::new(),
        });
    }
}

#[cfg(test)]
mod review_dispatch_tests {
    use super::build_review_dispatch_prompt;

    #[test]
    fn review_dispatch_prompt_is_adversarial_sha_bound_and_reports_back() {
        let dispatch = gwt::AutonomousReviewDispatch {
            issue_number: 42,
            pr_number: 99,
            reviewed_sha: "abc123".to_string(),
            required_criteria: vec!["AC-1".to_string()],
            diff: "diff --git a/x b/x".to_string(),
            linked_issue_kind: gwt::LinkedIssueKind::Spec,
        };
        let prompt = build_review_dispatch_prompt(&dispatch);
        assert!(
            prompt.contains("REFUTE"),
            "adversarial framing carried through"
        );
        assert!(prompt.contains("UNTRUSTED DATA"), "injection framing");
        assert!(prompt.contains("AC-1"), "required criterion");
        assert!(prompt.contains("abc123"), "bound to the reviewed SHA");
        assert!(
            prompt.contains("issue.monitor.review_verdict"),
            "instructs verdict report-back via the gwtd op"
        );
        assert!(prompt.contains("42"), "names the issue");
    }
}

#[cfg(test)]
mod manual_successor_identity_tests {
    use super::manual_generation_operation_id;

    #[test]
    fn manual_successor_operation_id_is_generation_stable_across_runtime_incarnations() {
        let owner = gwt::cli::execution_state::ExecutionOwnerKey {
            kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
            number: 3547,
        };
        let binding = gwt_agent::ExecutionBindingIdentity {
            generation_id: "generation-1".to_string(),
            binding_id: "binding-1".to_string(),
            ledger_head_hash: "head-1".to_string(),
        };

        let before_runtime_replacement = manual_generation_operation_id(
            owner,
            &binding,
            gwt_agent::ManualLaunchSuccessorPredecessor::ExactTerminalActive,
        );
        let after_runtime_replacement = manual_generation_operation_id(
            owner,
            &binding,
            gwt_agent::ManualLaunchSuccessorPredecessor::ExactTerminalActive,
        );

        assert_eq!(before_runtime_replacement, after_runtime_replacement);
        assert!(before_runtime_replacement.contains("generation-1"));
        assert!(before_runtime_replacement.contains("head-1"));
    }
}

#[cfg(test)]
mod launch_agent_branch_resolution_tests {
    use std::{fs, path::Path};

    use tempfile::tempdir;

    use gwt::start_work::resolve_launch_agent_base_branch;

    const NO_BRANCHES_ERROR: &str =
        "No branches exist in this repository; create an initial commit first";

    fn run_git(cwd: &Path, args: &[&str]) {
        let output = gwt_core::process::hidden_command("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {args:?} in {} failed: {}",
            cwd.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_stdout(cwd: &Path, args: &[&str]) -> String {
        let output = gwt_core::process::hidden_command("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {args:?} in {} failed: {}",
            cwd.display(),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn init_committed_repo(repo: &Path, branch: &str) {
        fs::create_dir_all(repo).expect("create repository");
        run_git(repo, &["init", "-q", "-b", branch]);
        run_git(repo, &["config", "user.name", "Test User"]);
        run_git(repo, &["config", "user.email", "test@example.com"]);
        fs::write(repo.join("README.md"), "fixture\n").expect("write fixture");
        run_git(repo, &["add", "README.md"]);
        run_git(repo, &["commit", "-qm", "fixture"]);
    }

    fn init_bare_workspace(
        workspace: &Path,
        head_branch: &str,
        additional_branches: &[&str],
        checked_out_branch: Option<&str>,
    ) {
        fs::create_dir_all(workspace).expect("create workspace");
        let seed = workspace.join("seed");
        init_committed_repo(&seed, head_branch);
        for branch in additional_branches {
            run_git(&seed, &["branch", branch]);
        }

        let bare = workspace.join("repo.git");
        let bare_arg = bare.to_string_lossy().into_owned();
        run_git(workspace, &["init", "--bare", &bare_arg]);
        run_git(&seed, &["remote", "add", "origin", &bare_arg]);
        run_git(&seed, &["push", "-q", "origin", head_branch]);
        for branch in additional_branches {
            run_git(&seed, &["push", "-q", "origin", branch]);
        }

        let head_ref = format!("refs/heads/{head_branch}");
        run_git(&bare, &["symbolic-ref", "HEAD", &head_ref]);
        if let Some(branch) = checked_out_branch {
            let worktree = workspace.join(branch);
            let worktree_arg = worktree.to_string_lossy().into_owned();
            run_git(&bare, &["worktree", "add", "-q", &worktree_arg, branch]);
        }
    }

    fn init_empty_bare_workspace(workspace: &Path) {
        fs::create_dir_all(workspace).expect("create workspace");
        let bare = workspace.join("repo.git");
        let bare_arg = bare.to_string_lossy().into_owned();
        run_git(workspace, &["init", "--bare", &bare_arg]);
    }

    #[test]
    fn launch_agent_branch_resolution_prefers_checked_out_develop_in_container_workspace() {
        // `git` is resolved through PATH, which sibling tests replace under the env lock.
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let workspace = temp.path().join("workspace");
        init_bare_workspace(&workspace, "main", &["develop"], Some("develop"));

        assert_eq!(
            resolve_launch_agent_base_branch(&workspace),
            Ok("develop".to_string())
        );
    }

    #[test]
    fn launch_agent_branch_resolution_uses_checked_out_main_without_develop() {
        // `git` is resolved through PATH, which sibling tests replace under the env lock.
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let workspace = temp.path().join("workspace");
        init_bare_workspace(&workspace, "main", &[], Some("main"));

        assert_eq!(
            resolve_launch_agent_base_branch(&workspace),
            Ok("main".to_string())
        );
    }

    #[test]
    fn launch_agent_branch_resolution_preserves_existing_normal_current_branch() {
        // `git` is resolved through PATH, which sibling tests replace under the env lock.
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        init_committed_repo(&repo, "feature/current");

        assert_eq!(
            resolve_launch_agent_base_branch(&repo),
            Ok("feature/current".to_string())
        );
    }

    #[test]
    fn launch_agent_branch_resolution_uses_existing_bare_head_without_default_worktree() {
        // `git` is resolved through PATH, which sibling tests replace under the env lock.
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let workspace = temp.path().join("workspace");
        init_bare_workspace(&workspace, "master", &[], None);

        assert_eq!(
            resolve_launch_agent_base_branch(&workspace),
            Ok("master".to_string())
        );
    }

    #[test]
    fn launch_agent_branch_resolution_rejects_empty_bare_repository() {
        // `git` is resolved through PATH, which sibling tests replace under the env lock.
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let workspace = temp.path().join("workspace");
        init_empty_bare_workspace(&workspace);

        assert_eq!(
            resolve_launch_agent_base_branch(&workspace),
            Err(NO_BRANCHES_ERROR.to_string())
        );
    }

    #[test]
    fn launch_agent_branch_resolution_rejects_unborn_current_branch() {
        // `git` is resolved through PATH, which sibling tests replace under the env lock.
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repository");
        run_git(&repo, &["init", "-q", "-b", "future"]);

        assert_eq!(
            resolve_launch_agent_base_branch(&repo),
            Err(NO_BRANCHES_ERROR.to_string())
        );
    }

    #[test]
    fn launch_agent_branch_resolution_uses_develop_worktree_from_detached_head() {
        // `git` is resolved through PATH, which sibling tests replace under the env lock.
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        init_committed_repo(&repo, "main");
        run_git(&repo, &["branch", "develop"]);
        let develop = temp.path().join("develop");
        let develop_arg = develop.to_string_lossy().into_owned();
        run_git(&repo, &["worktree", "add", "-q", &develop_arg, "develop"]);
        run_git(&repo, &["checkout", "-q", "--detach", "HEAD"]);

        assert_eq!(
            resolve_launch_agent_base_branch(&repo),
            Ok("develop".to_string())
        );
    }

    #[test]
    fn launch_agent_branch_resolution_falls_back_from_unusable_root_git_metadata() {
        // `git` is resolved through PATH, which sibling tests replace under the env lock.
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let workspace = temp.path().join("workspace");
        init_bare_workspace(&workspace, "master", &[], None);
        fs::write(workspace.join(".git"), "gitdir: missing\n").expect("write broken gitdir");

        assert_eq!(
            resolve_launch_agent_base_branch(&workspace),
            Ok("master".to_string())
        );
    }

    #[test]
    fn launch_agent_branch_resolution_prefers_develop_when_project_root_is_bare() {
        // `git` is resolved through PATH, which sibling tests replace under the env lock.
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let workspace = temp.path().join("workspace");
        init_bare_workspace(&workspace, "main", &["develop"], Some("develop"));

        assert_eq!(
            resolve_launch_agent_base_branch(&workspace.join("repo.git")),
            Ok("develop".to_string())
        );
    }

    #[test]
    fn launch_agent_branch_resolution_rejects_local_ref_that_is_not_a_commit() {
        // `git` is resolved through PATH, which sibling tests replace under the env lock.
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let workspace = temp.path().join("workspace");
        init_bare_workspace(&workspace, "master", &["main"], Some("main"));
        let blob = git_stdout(&workspace.join("seed"), &["rev-parse", "master:README.md"]);
        fs::write(
            workspace.join("repo.git/refs/heads/main"),
            format!("{blob}\n"),
        )
        .expect("replace main ref with blob");

        assert_eq!(
            resolve_launch_agent_base_branch(&workspace),
            Ok("master".to_string())
        );
    }

    #[test]
    fn launch_agent_branch_resolution_preserves_git_error_when_fallback_is_unavailable() {
        // `git` is resolved through PATH, which sibling tests replace under the env lock.
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let workspace = temp.path().join("workspace");
        fs::create_dir_all(&workspace).expect("create workspace");
        fs::write(workspace.join(".git"), "gitdir: missing\n").expect("write broken gitdir");

        let error = resolve_launch_agent_base_branch(&workspace).expect_err("reject metadata");
        assert_ne!(error, NO_BRANCHES_ERROR);
        assert!(
            error.contains("rev-parse --git-common-dir"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn launch_agent_branch_resolution_preserves_malformed_local_ref_error() {
        // `git` is resolved through PATH, which sibling tests replace under the env lock.
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        init_committed_repo(&repo, "main");
        fs::write(repo.join(".git/refs/heads/main"), "not-an-object-id\n")
            .expect("corrupt main ref");

        let error = resolve_launch_agent_base_branch(&repo).expect_err("reject broken ref");
        assert_ne!(error, NO_BRANCHES_ERROR);
        assert!(
            error.contains("symbolic-ref") || error.contains("broken ref"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn launch_agent_branch_resolution_does_not_report_branch_zero_for_detached_nondefault_branch() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        init_committed_repo(&repo, "feature/current");
        run_git(&repo, &["checkout", "-q", "--detach", "HEAD"]);

        let error = resolve_launch_agent_base_branch(&repo).expect_err("require base branch");
        assert_ne!(error, NO_BRANCHES_ERROR);
        assert!(
            error.contains("current or checked-out develop/main"),
            "unexpected error: {error}"
        );
    }
}

fn issue_monitor_owner_launch_profile_choice(
    cache: &LaunchWizardMemoryCache,
    provider_usage_accounts: &[gwt_core::usage::ProviderUsage],
    project_root: &Path,
    issue_number: u64,
    linked_issue_kind: gwt::LinkedIssueKind,
) -> Result<IssueMonitorLaunchProfileChoice, String> {
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(project_root);
    let prefs = gwt::load_issue_monitor_prefs(&prefs_path).map_err(|error| error.to_string())?;
    if !prefs.launch_auto {
        let pool = prefs.launch_profile_pool();
        if !pool.is_empty() {
            let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            let selection = gwt::select_launch_profile(
                &pool,
                &prefs.launch_admission_provider_quota_holds(),
                &[],
                None,
                &now,
            );
            let index = selection.selected.ok_or_else(|| {
                "No eligible Issue Monitor launch candidate: every provider is held".to_string()
            })?;
            let profile = pool[index].clone();
            return Ok(IssueMonitorLaunchProfileChoice {
                profiles: gwt::LaunchWizardPreviousProfiles::from_profile(Some(
                    profile.clone().into(),
                )),
                selected_agent_id: Some(profile.agent_id),
                skipped: selection.skipped,
                tier: None,
            });
        }
        return Ok(issue_monitor_launch_profile_choice(
            cache,
            provider_usage_accounts,
            project_root,
            None,
        ));
    }
    let available = cache
        .agent_options()
        .into_iter()
        .filter(|option| option.available)
        .map(|option| option.id)
        .collect::<Vec<_>>();
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let holds = prefs.launch_admission_provider_quota_holds();
    let work_tags = vec![if linked_issue_kind == gwt::LinkedIssueKind::Spec {
        "kind:spec".to_string()
    } else {
        "kind:issue".to_string()
    }];
    let selection = prefs
        .select_auto_launch_profile(
            issue_number,
            linked_issue_kind == gwt::LinkedIssueKind::Spec,
            Some(&available),
            |pool| gwt::select_launch_profile(pool, &holds, &work_tags, None, &now),
        )
        .ok_or_else(|| "No eligible Issue Monitor automatic tier candidate".to_string())?;
    Ok(IssueMonitorLaunchProfileChoice {
        profiles: gwt::LaunchWizardPreviousProfiles::from_profile(Some(
            selection.profile.clone().into(),
        )),
        selected_agent_id: Some(selection.profile.agent_id),
        skipped: selection.skipped,
        tier: Some(selection.tier),
    })
}

fn issue_monitor_launch_profile_choice(
    cache: &LaunchWizardMemoryCache,
    _provider_usage_accounts: &[gwt_core::usage::ProviderUsage],
    project_root: &Path,
    avoid_provider: Option<&str>,
) -> IssueMonitorLaunchProfileChoice {
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(project_root);
    if let Ok(prefs) = gwt::load_issue_monitor_prefs(&prefs_path) {
        let pool = prefs.launch_profile_pool();
        if !pool.is_empty() {
            let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            let selection = gwt::select_launch_profile(
                &pool,
                &prefs.launch_admission_provider_quota_holds(),
                &[],
                avoid_provider,
                &now,
            );
            let (index, skipped) = match selection.selected {
                Some(index) => (index, selection.skipped),
                None => {
                    tracing::warn!(
                        project_root = %project_root.display(),
                        skipped = ?selection.skipped,
                        "every Issue Monitor launch candidate is held; previewing the saved pool head"
                    );
                    (0, Vec::new())
                }
            };
            let profile = pool[index].clone();
            return IssueMonitorLaunchProfileChoice {
                profiles: gwt::LaunchWizardPreviousProfiles::from_profile(Some(
                    profile.clone().into(),
                )),
                selected_agent_id: Some(profile.agent_id),
                skipped,
                tier: None,
            };
        }
    }
    let profiles = cache.previous_profiles(project_root);
    let profiles = if profiles.repo_local().is_some() {
        profiles
    } else {
        let fallback_profile = profiles.preferred_profile().cloned();
        profiles.with_repo_local(fallback_profile)
    };
    IssueMonitorLaunchProfileChoice {
        profiles,
        selected_agent_id: None,
        skipped: Vec::new(),
        tier: None,
    }
}

#[allow(clippy::too_many_arguments)]
fn prepare_issue_monitor_resume(
    project_root: &Path,
    target_branch: &str,
    issue_number: u64,
    profile_agent_id: &str,
    sessions_dir: &Path,
    cache: &LaunchWizardMemoryCache,
    profile_config_path: &Result<PathBuf, String>,
    autonomous_handoff: Result<Option<gwt::AutonomousHandoffResumption>, String>,
) -> Result<Option<PreparedIssueMonitorResume>, String> {
    let autonomous_handoff = autonomous_handoff?;
    let session = if let Some(handoff) = autonomous_handoff.as_ref() {
        let session =
            gwt_agent::Session::load(&sessions_dir.join(format!("{}.toml", handoff.session_id)))
                .ok()
                .or_else(|| cache.session_by_id(&handoff.session_id).cloned())
                .ok_or_else(|| {
                    format!(
                        "answered autonomous handoff {} references unavailable gwt Session {}",
                        handoff.handoff_id, handoff.session_id
                    )
                })?;
        if session.linked_issue_number != Some(issue_number)
            || normalize_branch_name(&session.branch) != normalize_branch_name(target_branch)
            || !session_matches_project_state(&session, project_root)
        {
            return Err(format!(
                "answered autonomous handoff {} does not match Issue #{issue_number}'s exact Session",
                handoff.handoff_id
            ));
        }
        session
    } else {
        let Some(session) = cache.latest_resumable_branch_session(project_root, target_branch)
        else {
            return Ok(None);
        };
        session
    };
    let session_record = std::fs::read(sessions_dir.join(format!("{}.toml", session.id)))
        .map_err(|error| error.kind());
    // Issue #3676 AC-1: a stored session only qualifies for resume when
    // its provider matches the Monitor's current launch profile. A
    // mismatched provider must fall through to a fresh launch on the
    // profile provider instead of re-binding the slot to the old CLI.
    if !session
        .agent_id
        .command()
        .eq_ignore_ascii_case(profile_agent_id.trim())
    {
        if autonomous_handoff.is_some() {
            return Err(
                "answered autonomous handoff provider does not match the Monitor profile"
                    .to_string(),
            );
        }
        return Ok(None);
    }
    if !session_exact_resume_materializable(project_root, &session) {
        if autonomous_handoff.is_some() {
            return Err(
                "answered autonomous handoff Session can no longer be materialized".to_string(),
            );
        }
        return Ok(None);
    }
    let provider_availability = if session.agent_id == gwt_agent::AgentId::GrokBuild {
        let (effective_env, _) = gwt_agent::LaunchEnvironment::from_active_profile(
            &profile_config_path.clone()?,
            session.runtime_target,
        )?
        .into_parts();
        let grok_home =
            gwt_core::usage::grok::grok_home_from_env(&effective_env, &session.worktree_path);
        provider_conversation_availability_with_grok_home(&session, grok_home.as_deref())
    } else {
        provider_conversation_availability(&session)
    };
    if provider_availability != ProviderConversationAvailability::Present {
        if autonomous_handoff.is_some() {
            return Err(
                "answered autonomous handoff native conversation is unavailable or foreign"
                    .to_string(),
            );
        }
        return Ok(None);
    }
    // Building a resume config resolves its working directory's Git remote and runner.
    // Keep that work with the other launch facts, before returning to the GUI.
    let config = super::launch_config_from_persisted_session(&session);
    let workspace_resume_context = workspace_resume_context_for_work_item(
        project_root,
        Some(session.branch.as_str()),
        &session.worktree_path,
    );
    Ok(Some(PreparedIssueMonitorResume {
        session,
        autonomous_handoff,
        session_record,
        config,
        workspace_resume_context,
    }))
}
