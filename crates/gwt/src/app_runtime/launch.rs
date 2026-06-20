//! Session / launch lifecycle split out of `app_runtime/mod.rs` for
//! SPEC-3064 Phase 1 (Pass 2).
//!
//! Owns:
//! - The launch payload types ([`ProcessLaunch`], [`AgentLaunchCompletion`],
//!   [`AgentLaunchResult`]) and the success dispatch bridge
//!   (`dispatch_agent_launch_success`)
//! - [`LaunchWizardMemoryCache`] (session cache backing the Launch Wizard)
//!   and `launch_config_from_persisted_session`
//! - SPEC-2809 launch stage correlation (`next_agent_launch_stage_id`
//!   fed by the `AppRuntime::agent_launch_stage_counter` field per
//!   SPEC-3064 FR-002, `emit_agent_launch_stage`) and the
//!   SPEC-2359 in-flight launch dedup (`INFLIGHT_LAUNCH_TTL`,
//!   `inflight_launch_key`)
//! - Issue<->branch link persistence (`IssueBranchLinkStore`,
//!   `record_issue_branch_link_with_cache_dir`, ...) and the codex managed
//!   hook discovery / trust registration helpers
//! - The launch / spawn / close-work method surface
//!   ([`AppRuntime::handle_launch_complete`], [`AppRuntime::start_window`],
//!   [`AppRuntime::spawn_agent_window_async`], [`AppRuntime::close_work`],
//!   [`AppRuntime::mark_agent_session_stopped`], ...)
//!
//! Behavior-preserving move: `WindowRuntime`, `LaunchWizardSession`, and
//! `AppRuntime::new` stay in `mod.rs` and are reached via `super`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;

use super::{
    active_agent_session_matches_work, agent_launch_purpose_title,
    apply_docker_runtime_to_launch_config, apply_host_package_runner_fallback_checked,
    apply_windows_host_shell_wrapper, combined_window_id, detect_shell_program,
    finalize_docker_agent_launch_config, geometry_to_pty_size, install_launch_gwt_bin_env,
    launch_output_mirror, mark_auto_resume_source_completed, normalize_branch_name,
    refresh_managed_gwt_assets_for_agent_with_codex_hook_discovery_mode,
    resolve_docker_launch_plan, resolve_launch_spec_with_fallback, resolve_launch_worktree,
    save_resumed_workspace_projection, save_start_work_workspace_projection, ActiveAgentSession,
    AgentKanbanLaunchTarget, AppEventProxy, AppRuntime, BackendEvent, HookForwardTarget,
    LaunchFeedbackContext, LiveSessionEntry, OutboundEvent, Pane, UserEvent, WindowGeometry,
    WindowPreset, WindowProcessStatus, WindowRuntime, WorkspaceResumeContext,
};

#[derive(Debug, Clone)]
pub struct ProcessLaunch {
    pub(crate) command: String,
    pub(crate) args: Vec<String>,
    pub(crate) env: HashMap<String, String>,
    pub(crate) remove_env: Vec<String>,
    pub(crate) cwd: Option<PathBuf>,
}

pub type AgentLaunchCompletion = (
    ProcessLaunch,
    String,
    String,
    String,
    PathBuf,
    gwt_agent::AgentId,
    Option<u64>,
    Option<String>,
    gwt_agent::LaunchRuntimeTarget,
    String,
);

pub type AgentLaunchResult = Result<AgentLaunchCompletion, String>;

pub(super) fn dispatch_agent_launch_success<F>(
    proxy: AppEventProxy,
    window_id: String,
    completion: AgentLaunchCompletion,
    spawn_project_index_bootstrap: F,
) where
    F: FnOnce(AppEventProxy, PathBuf),
{
    let project_index_root = completion.4.clone();
    proxy.send(UserEvent::LaunchComplete {
        window_id,
        result: Ok(completion),
    });
    spawn_project_index_bootstrap(proxy, project_index_root);
}

pub(super) fn launch_config_from_persisted_session(
    session: &gwt_agent::Session,
) -> gwt_agent::LaunchConfig {
    let agent_id = session.agent_id.clone();
    let mut builder = gwt_agent::AgentLaunchBuilder::new(agent_id);
    builder = builder.working_dir(session.worktree_path.clone());
    if !session.branch.is_empty() {
        builder = builder.branch(session.branch.clone());
    }
    if let Some(model) = session.model.clone() {
        builder = builder.model(model);
    }
    if let Some(version) = session.tool_version.clone() {
        builder = builder.version(version);
    }
    if let Some(level) = session.reasoning_level.clone() {
        builder = builder.reasoning_level(level);
    }
    if session.skip_permissions {
        builder = builder.skip_permissions(true);
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

    if let Some(resume_id) = session.exact_resume_session_id() {
        builder = builder
            .session_mode(gwt_agent::SessionMode::Resume)
            .resume_session_id(resume_id.to_string());
    } else {
        builder = builder.session_mode(gwt_agent::SessionMode::Normal);
    }

    let mut config = builder.build();
    if let Some(version) = session.tool_version.clone() {
        config.tool_version = Some(version);
    }
    if !session.display_name.is_empty() {
        config.display_name = session.display_name.clone();
    }
    config
}

#[derive(Debug, Clone)]
enum AgentWindowPlacement {
    Centered(WindowGeometry),
    Exact(WindowGeometry),
}

impl AgentWindowPlacement {
    fn bounds(&self) -> WindowGeometry {
        match self {
            Self::Centered(bounds) | Self::Exact(bounds) => bounds.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LaunchWizardMemoryCache {
    sessions: Vec<gwt_agent::Session>,
    agent_options: Vec<gwt::AgentOption>,
}

impl LaunchWizardMemoryCache {
    pub(crate) fn load(sessions_dir: &Path) -> Self {
        Self {
            sessions: Self::load_sessions(sessions_dir),
            agent_options: Self::load_agent_options(),
        }
    }

    #[cfg(test)]
    pub(crate) fn load_with_agent_options(
        sessions_dir: &Path,
        agent_options: Vec<gwt::AgentOption>,
    ) -> Self {
        Self {
            sessions: Self::load_sessions(sessions_dir),
            agent_options,
        }
    }

    fn load_sessions(sessions_dir: &Path) -> Vec<gwt_agent::Session> {
        let Ok(entries) = std::fs::read_dir(sessions_dir) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                (path.extension().and_then(|ext| ext.to_str()) == Some("toml")).then_some(path)
            })
            .filter_map(|path| gwt_agent::Session::load_and_migrate(&path).ok())
            .collect()
    }

    fn load_agent_options() -> Vec<gwt::AgentOption> {
        gwt::load_agent_options(&gwt_agent::VersionCache::load(
            &gwt::default_wizard_version_cache_path(),
        ))
    }

    pub(super) fn refresh_agent_options(&mut self) {
        self.agent_options = Self::load_agent_options();
    }

    pub(super) fn agent_options(&self) -> Vec<gwt::AgentOption> {
        self.agent_options.clone()
    }

    pub(super) fn quick_start_entries(
        &self,
        repo_path: &Path,
        branch_name: &str,
    ) -> Vec<gwt::QuickStartEntry> {
        gwt::launch_wizard::quick_start_entries_from_sessions(
            repo_path,
            branch_name,
            &self.sessions,
        )
    }

    fn latest_resumable_branch_session(
        &self,
        repo_path: &Path,
        branch_name: &str,
    ) -> Option<gwt_agent::Session> {
        let entry = self
            .quick_start_entries(repo_path, branch_name)
            .into_iter()
            .find(|entry| entry.resume_session_id.is_some())?;
        self.sessions
            .iter()
            .find(|session| session.id == entry.session_id)
            .cloned()
    }

    /// Replace all cached sessions with a freshly disk-loaded set. Called from
    /// the off-thread branch load (#2995) so resume availability and resolution
    /// observe session TOMLs the hook CLI wrote out-of-process after launch,
    /// without ever blocking the main UI thread on disk I/O.
    fn replace_sessions(&mut self, sessions: Vec<gwt_agent::Session>) {
        self.sessions = sessions;
    }

    fn session_by_id(&self, session_id: &str) -> Option<&gwt_agent::Session> {
        self.sessions
            .iter()
            .find(|session| session.id == session_id)
    }

    pub(super) fn agent_preferences(&self) -> gwt::LaunchWizardPreviousProfiles {
        gwt::launch_wizard::previous_launch_profiles_from_sessions(&self.sessions)
    }

    pub(super) fn previous_profiles(&self, repo_path: &Path) -> gwt::LaunchWizardPreviousProfiles {
        gwt::launch_wizard::previous_launch_profiles_for_repo_from_sessions(
            repo_path,
            &self.sessions,
        )
    }

    fn record_session(&mut self, session: gwt_agent::Session) {
        if let Some(existing) = self
            .sessions
            .iter_mut()
            .find(|existing| existing.id == session.id)
        {
            *existing = session;
        } else {
            self.sessions.push(session);
        }
    }

    fn mark_stopped(&mut self, session_id: &str) {
        if let Some(session) = self
            .sessions
            .iter_mut()
            .find(|session| session.id == session_id)
        {
            session.update_status(gwt_agent::AgentStatus::Stopped);
        }
    }
}

#[derive(Debug, Default, serde::Deserialize, serde::Serialize)]
pub(super) struct IssueBranchLinkStore {
    #[serde(default)]
    pub(super) branches: HashMap<String, u64>,
}

/// SPEC-2809 — per-spawn correlation id for Launch Wizard stages so the
/// Console window's `agent` tab can group multiple stage events (binary
/// resolve / env prep / worktree create / PTY handoff) under one
/// invocation header. Atomic so parallel wizard sessions do not collide.
/// SPEC-3064 FR-002: the counter is the per-instance
/// `AppRuntime::agent_launch_stage_counter` field threaded in by callers.
pub(crate) fn next_agent_launch_stage_id(counter: &std::sync::atomic::AtomicU64) -> u64 {
    counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Emit a `gwt.process.summary` event for one Launch Wizard stage so the
/// Console window's `agent` tab surfaces the pipeline that ends in the
/// PTY spawn. Stage semantics (`start`, `done`, `error`) follow the same
/// vocabulary as the `spawn_logged` summary contract.
pub(crate) fn emit_agent_launch_stage(spawn_id: u64, stage: &str, detail: &str) {
    tracing::info!(
        target: "gwt.process.summary",
        kind = "agent",
        spawn_id = spawn_id,
        stage = stage,
        detail = detail,
        "agent launch stage",
    );
    // Also push a synthetic line into the hub so the agent tab shows the
    // stage banner in real time (the summary event alone lives in
    // canonical log + Logs window only).
    let hub = gwt_core::process_console::global();
    let label = format!("[{stage}] {detail}");
    hub.push(gwt_core::process_console::ProcessLine::new(
        gwt_core::process_console::ProcessKind::AgentBootstrap,
        spawn_id,
        gwt_core::process_console::ProcessStream::Stdout,
        label,
    ));
}

/// SPEC-2359 W-17 (FR-398): dedup window for launches that are past window
/// registration but not yet live. Entries also clear on launch completion.
const INFLIGHT_LAUNCH_TTL: std::time::Duration = std::time::Duration::from_secs(60);

/// Identity of a launch for in-flight dedup. Includes the agent and the
/// resume conversation so parallel restores of *different* Sessions on the
/// same Work (startup auto-resume) and multi-agent launches on one Work stay
/// allowed — only a re-request of the *same* launch dedupes. `None` when the
/// config carries neither a branch nor a working dir: such launches have no
/// stable Work identity and must never dedup against each other.
fn inflight_launch_key(tab_id: &str, config: &gwt_agent::LaunchConfig) -> Option<String> {
    let branch = config
        .branch
        .as_deref()
        .map(normalize_branch_name)
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_default();
    let dir = config
        .working_dir
        .as_deref()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    if branch.is_empty() && dir.is_empty() {
        return None;
    }
    let agent = config.agent_id.command();
    let resume = config.resume_session_id.as_deref().unwrap_or_default();
    Some(format!(
        "{tab_id}\u{001f}{agent}\u{001f}{branch}\u{001f}{dir}\u{001f}{resume}"
    ))
}

fn record_issue_branch_link_with_cache_dir(
    repo_path: &Path,
    branch_name: &str,
    issue_number: u64,
    cache_dir: &Path,
) -> Result<(), String> {
    update_issue_branch_link_with_cache_dir(repo_path, branch_name, Some(issue_number), cache_dir)
}

fn clear_issue_branch_link_with_cache_dir(
    repo_path: &Path,
    branch_name: &str,
    cache_dir: &Path,
) -> Result<(), String> {
    update_issue_branch_link_with_cache_dir(repo_path, branch_name, None, cache_dir)
}

fn update_issue_branch_link_with_cache_dir(
    repo_path: &Path,
    branch_name: &str,
    issue_number: Option<u64>,
    cache_dir: &Path,
) -> Result<(), String> {
    let branch_name = branch_name.trim();
    if branch_name.is_empty() {
        return Ok(());
    }
    let Some(repo_hash) = gwt::index_worker::detect_repo_hash(repo_path) else {
        return Err("repository hash is unavailable for issue linkage".to_string());
    };
    let path = cache_dir
        .join("issue-links")
        .join(format!("{}.json", repo_hash.as_str()));

    let mut store = match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice::<IssueBranchLinkStore>(&bytes)
            .map_err(|error| format!("failed to parse issue linkage store: {error}"))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            IssueBranchLinkStore::default()
        }
        Err(error) => return Err(format!("failed to read issue linkage store: {error}")),
    };

    match issue_number {
        Some(issue_number) => {
            store.branches.insert(branch_name.to_string(), issue_number);
        }
        None => {
            if store.branches.remove(branch_name).is_none() {
                return Ok(());
            }
        }
    }

    let bytes = serde_json::to_vec_pretty(&store)
        .map_err(|error| format!("failed to serialize issue linkage store: {error}"))?;
    gwt_github::cache::write_atomic(&path, &bytes)
        .map_err(|error| format!("failed to write issue linkage store: {error}"))
}

fn codex_hook_discovery_mode_for_launch_config(
    config: &gwt_agent::LaunchConfig,
) -> gwt_skills::CodexHookDiscoveryMode {
    if config.agent_id != gwt_agent::AgentId::Codex {
        return gwt_skills::CodexHookDiscoveryMode::WorkspaceHome;
    }
    if let Some(mode) =
        codex_hook_discovery_mode_from_selected_codex_version(config.tool_version.as_deref())
    {
        return mode;
    }
    if config.runtime_target != gwt_agent::LaunchRuntimeTarget::Host {
        return gwt_skills::CodexHookDiscoveryMode::Both;
    }
    detect_installed_codex_hook_discovery_mode(config)
        .unwrap_or(gwt_skills::CodexHookDiscoveryMode::Both)
}

pub(super) fn codex_hook_discovery_mode_from_selected_codex_version(
    version: Option<&str>,
) -> Option<gwt_skills::CodexHookDiscoveryMode> {
    let version = version?.trim();
    if version.is_empty() || version == "installed" {
        return None;
    }
    if version == "latest" {
        return Some(gwt_skills::CodexHookDiscoveryMode::WorkspaceHome);
    }
    codex_hook_discovery_mode_from_semver(version)
}

pub(super) fn codex_hook_discovery_mode_from_codex_version_output(
    output: &str,
) -> Option<gwt_skills::CodexHookDiscoveryMode> {
    output
        .split_whitespace()
        .find_map(codex_hook_discovery_mode_from_semver)
}

fn codex_hook_discovery_mode_from_semver(raw: &str) -> Option<gwt_skills::CodexHookDiscoveryMode> {
    let token = raw
        .trim()
        .trim_start_matches('v')
        .trim_matches(|c| c == ',' || c == ';');
    let version = semver::Version::parse(token).ok()?;
    let boundary =
        semver::Version::parse("0.131.0-alpha.21").expect("valid Codex hook discovery boundary");
    Some(if version < boundary {
        gwt_skills::CodexHookDiscoveryMode::WorktreeLocal
    } else {
        gwt_skills::CodexHookDiscoveryMode::WorkspaceHome
    })
}

fn detect_installed_codex_hook_discovery_mode(
    config: &gwt_agent::LaunchConfig,
) -> Option<gwt_skills::CodexHookDiscoveryMode> {
    let mut command = std::process::Command::new(&config.command);
    command.arg("--version").envs(&config.env_vars);
    for key in &config.remove_env {
        command.env_remove(key);
    }
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let mut text = String::new();
    text.push_str(&String::from_utf8_lossy(&output.stdout));
    text.push(' ');
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    codex_hook_discovery_mode_from_codex_version_output(&text)
}

pub(super) fn maybe_register_codex_managed_hook_trust_for_launch(
    profile_config_path: &Path,
    worktree_path: &Path,
    agent_id: &gwt_agent::AgentId,
    runtime_target: gwt_agent::LaunchRuntimeTarget,
    docker_service: Option<&str>,
    codex_home: Option<&Path>,
    codex_hook_discovery_mode: gwt_skills::CodexHookDiscoveryMode,
) -> Result<Option<gwt_skills::CodexHookTrustReport>, String> {
    if agent_id != &gwt_agent::AgentId::Codex {
        return Ok(None);
    }

    let settings = if profile_config_path.exists() {
        match gwt_config::Settings::load_from_path(profile_config_path) {
            Ok(settings) => settings,
            Err(error) => {
                tracing::warn!(
                    profile_config = %profile_config_path.display(),
                    error = %error,
                    "failed to read gwt config while preparing Codex hook trust; continuing launch"
                );
                gwt_config::Settings::default()
            }
        }
    } else {
        gwt_config::Settings::default()
    };
    if settings.agent.codex_trust_managed_hooks == Some(false) {
        return Ok(None);
    }

    match runtime_target {
        gwt_agent::LaunchRuntimeTarget::Host => {
            let Some(codex_config_path) = codex_home
                .map(|home| home.join("config.toml"))
                .or_else(|| codex_config_path_for_profile_config(profile_config_path))
            else {
                tracing::warn!(
                    profile_config = %profile_config_path.display(),
                    "cannot derive Codex config path while preparing Codex hook trust; continuing launch"
                );
                return Ok(None);
            };
            match gwt_skills::register_codex_managed_hook_trust_for_mode(
                worktree_path,
                &codex_config_path,
                codex_hook_discovery_mode,
            ) {
                Ok(report) => Ok(Some(report)),
                Err(error) => {
                    tracing::warn!(
                        worktree = %worktree_path.display(),
                        codex_config = %codex_config_path.display(),
                        error = %error,
                        "failed to register gwt-managed Codex hook trust; continuing launch"
                    );
                    Ok(None)
                }
            }
        }
        gwt_agent::LaunchRuntimeTarget::Docker => {
            if let Err(error) = gwt_agent::register_codex_managed_hook_trust_in_docker(
                worktree_path,
                docker_service,
                codex_hook_discovery_mode,
            ) {
                tracing::warn!(
                    worktree = %worktree_path.display(),
                    error = %error,
                    "failed to register gwt-managed Codex hook trust in Docker; continuing launch"
                );
            }
            Ok(None)
        }
    }
}

fn codex_config_path_for_profile_config(profile_config_path: &Path) -> Option<PathBuf> {
    let gwt_config_dir = profile_config_path.parent()?;
    if gwt_config_dir.file_name().and_then(|name| name.to_str()) != Some(".gwt") {
        return None;
    }
    Some(gwt_config_dir.parent()?.join(".codex").join("config.toml"))
}

impl AppRuntime {
    pub(super) fn latest_resumable_branch_session(
        &self,
        project_root: &Path,
        branch_name: &str,
    ) -> Option<gwt_agent::Session> {
        // Resolve from the in-memory cache so the Resume click never blocks the
        // main UI thread on disk I/O. Freshness is guaranteed by
        // [`apply_refreshed_launch_wizard_sessions`], which the off-thread
        // branch load dispatches before any Resume button is enabled (#2995).
        let normalized_branch_name = normalize_branch_name(branch_name);
        self.launch_wizard_cache
            .latest_resumable_branch_session(project_root, &normalized_branch_name)
    }

    /// Apply a freshly disk-loaded session set to the Launch Wizard cache.
    /// Dispatched from the off-thread branch load (#2995) so branch Resume
    /// availability and the subsequent cache-based resume resolution reflect
    /// session TOMLs the hook CLI wrote out-of-process after launch — without
    /// the main thread ever performing the session-directory scan.
    pub(crate) fn apply_refreshed_launch_wizard_sessions(
        &mut self,
        sessions: Vec<gwt_agent::Session>,
    ) {
        self.launch_wizard_cache.replace_sessions(sessions);
    }

    pub(crate) fn live_sessions_for_branch(
        &self,
        tab_id: &str,
        branch_name: &str,
    ) -> Vec<LiveSessionEntry> {
        let mut entries = self
            .active_agent_sessions
            .values()
            .filter(|session| session.tab_id == tab_id && session.branch_name == branch_name)
            .map(|session| LiveSessionEntry {
                session_id: session.session_id.clone(),
                window_id: session.window_id.clone(),
                agent_id: session.agent_id.clone(),
                kind: "agent".to_string(),
                name: session.display_name.clone(),
                detail: Some(session.worktree_path.display().to_string()),
                active: true,
                runtime_status: self
                    .window_status(&session.window_id)
                    .unwrap_or(WindowProcessStatus::Running),
            })
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| {
            match (
                self.launch_wizard_cache.session_by_id(&left.session_id),
                self.launch_wizard_cache.session_by_id(&right.session_id),
            ) {
                (Some(left_session), Some(right_session)) => right_session
                    .last_activity_at
                    .cmp(&left_session.last_activity_at)
                    .then_with(|| right_session.updated_at.cmp(&left_session.updated_at))
                    .then_with(|| right_session.created_at.cmp(&left_session.created_at))
                    .then_with(|| right_session.id.cmp(&left_session.id)),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => left.name.cmp(&right.name),
            }
        });
        entries
    }

    pub(crate) fn active_session_branches_for_tab(
        &self,
        tab_id: &str,
    ) -> std::collections::HashSet<String> {
        self.active_agent_sessions
            .values()
            .filter(|session| session.tab_id == tab_id)
            .map(|session| session.branch_name.clone())
            .collect()
    }

    pub(crate) fn handle_launch_complete(
        &mut self,
        window_id: String,
        result: AgentLaunchResult,
    ) -> Vec<OutboundEvent> {
        let workspace_resume_context = self.pending_workspace_resume_contexts.remove(&window_id);
        let launch_feedback_context = self.pending_launch_feedback_contexts.remove(&window_id);
        let auto_resume_source_session_id = self.pending_auto_resume_sources.remove(&window_id);
        self.inflight_launches
            .retain(|_, (pending_window_id, _)| pending_window_id != &window_id);
        match result {
            Ok((
                process_launch,
                session_id,
                branch_name,
                display_name,
                worktree_path,
                agent_id,
                linked_issue_number,
                base_branch,
                runtime_target,
                agent_project_root,
            )) => {
                let Some(address) = self.window_lookup.get(&window_id).cloned() else {
                    return self.launch_error_events(
                        window_id,
                        "Window not found".to_string(),
                        launch_feedback_context.clone(),
                    );
                };
                let Some(tab) = self.tab(&address.tab_id) else {
                    return self.launch_error_events(
                        window_id,
                        "Project tab not found".to_string(),
                        launch_feedback_context.clone(),
                    );
                };
                // SPEC-2359 W-16 (FR-387): a launch fetches origin refs, so
                // piggyback the cross-machine intake (30s throttle keeps
                // launch bursts cheap).
                self.spawn_work_events_ingest(tab.project_root.clone(), false);
                let Some(window) = tab.workspace.window(&address.raw_id) else {
                    return self.launch_error_events(
                        window_id,
                        "Window not found".to_string(),
                        launch_feedback_context.clone(),
                    );
                };
                let tab_id = address.tab_id.clone();
                let project_root = tab.project_root.clone();
                let geometry = window.geometry.clone();
                let session_id_for_restore = session_id.clone();

                self.active_agent_sessions.insert(
                    window_id.clone(),
                    ActiveAgentSession {
                        window_id: window_id.clone(),
                        session_id,
                        agent_id: agent_id.to_string(),
                        branch_name,
                        display_name,
                        worktree_path: worktree_path.clone(),
                        agent_project_root,
                        runtime_target,
                        tab_id: tab_id.clone(),
                    },
                );
                let _ = gwt_agent::persist_session_restore_window_on_startup(
                    &self.sessions_dir,
                    &session_id_for_restore,
                    true,
                );
                if let Some(tab) = self.tab_mut(&tab_id) {
                    let _ = tab
                        .workspace
                        .set_session_id(&address.raw_id, Some(session_id_for_restore.clone()));
                }
                if let Some(source_session_id) = auto_resume_source_session_id {
                    mark_auto_resume_source_completed(&self.sessions_dir, &source_session_id);
                }
                self.refresh_launch_wizard_session_cache(&window_id);

                // SPEC-2809 — Launch Wizard always spawns an AI agent
                // launch sequence (binary resolve / env prep / PTY
                // spawn) so the Console window's `agent` tab shows the
                // wizard pipeline up to the moment xterm.js takes over.
                let stage_id = next_agent_launch_stage_id(&self.agent_launch_stage_counter);
                emit_agent_launch_stage(
                    stage_id,
                    "resolve_binary",
                    &format!("wizard launch {}", process_launch.command),
                );
                emit_agent_launch_stage(
                    stage_id,
                    "prepare_env",
                    &format!("worktree={}", worktree_path.display()),
                );
                emit_agent_launch_stage(
                    stage_id,
                    "spawn_pty",
                    &format!("argv=[{}]", process_launch.args.join(" ")),
                );
                match self.spawn_process_window_with_console_kind(
                    &window_id,
                    geometry,
                    process_launch,
                    Some(gwt_core::process_console::ProcessKind::AgentBootstrap),
                ) {
                    Ok(()) => {
                        emit_agent_launch_stage(stage_id, "ready", "PTY handoff complete");
                        let linkage_result = match linked_issue_number {
                            Some(issue_number) => record_issue_branch_link_with_cache_dir(
                                &worktree_path,
                                &self.active_agent_sessions[&window_id].branch_name,
                                issue_number,
                                &self.issue_link_cache_dir,
                            ),
                            None => clear_issue_branch_link_with_cache_dir(
                                &worktree_path,
                                &self.active_agent_sessions[&window_id].branch_name,
                                &self.issue_link_cache_dir,
                            ),
                        };
                        if let Err(error) = linkage_result {
                            tracing::warn!(
                                worktree = %worktree_path.display(),
                                branch = %self.active_agent_sessions[&window_id].branch_name,
                                ?linked_issue_number,
                                error = %error,
                                "issue branch linkage update skipped after agent launch"
                            );
                        }
                        let mut workspace_projection_updated = false;
                        let live_session_ids: std::collections::HashSet<String> = self
                            .active_agent_sessions
                            .values()
                            .map(|session| session.session_id.clone())
                            .collect();
                        let active_session = &self.active_agent_sessions[&window_id];
                        if let Some(base_branch) = base_branch.as_deref() {
                            match save_start_work_workspace_projection(
                                &project_root,
                                active_session,
                                base_branch,
                                linked_issue_number,
                                workspace_resume_context.as_ref(),
                                &live_session_ids,
                            ) {
                                Ok(()) => {
                                    workspace_projection_updated = true;
                                }
                                Err(error) => {
                                    tracing::warn!(
                                        project_root = %project_root.display(),
                                        branch = %active_session.branch_name,
                                        error = %error,
                                        "workspace projection update skipped after Start Work launch"
                                    );
                                }
                            }
                        } else if let Some(context) = workspace_resume_context.as_ref() {
                            match save_resumed_workspace_projection(
                                &project_root,
                                active_session,
                                None,
                                linked_issue_number,
                                context,
                                &live_session_ids,
                            ) {
                                Ok(()) => {
                                    workspace_projection_updated = true;
                                }
                                Err(error) => {
                                    tracing::warn!(
                                        project_root = %project_root.display(),
                                        branch = %active_session.branch_name,
                                        error = %error,
                                        "workspace projection update skipped after Workspace Resume launch"
                                    );
                                }
                            }
                        }
                        let _ = self.persist();
                        self.launch_error_terminal_details.remove(&window_id);
                        let mut events = vec![self.workspace_state_broadcast()];
                        if workspace_projection_updated
                            && self.active_tab_id.as_deref() == Some(tab_id.as_str())
                        {
                            if let Some(tab) = self.tab(&tab_id) {
                                if let Some(projection) =
                                    self.active_work_projection_for_tab(&tab_id, tab)
                                {
                                    events.push(OutboundEvent::broadcast(
                                        BackendEvent::ActiveWorkProjection {
                                            projection: Box::new(projection),
                                        },
                                    ));
                                }
                            }
                        }
                        let composed_status = self
                            .window_status(&window_id)
                            .unwrap_or(WindowProcessStatus::Running);
                        events.extend(Self::status_events(window_id, composed_status, None));
                        events
                    }
                    Err(error) => {
                        self.launch_error_events(window_id, error, launch_feedback_context)
                    }
                }
            }
            Err(error) => self.launch_error_events(window_id, error, launch_feedback_context),
        }
    }

    pub(crate) fn handle_shell_launch_complete(
        &mut self,
        window_id: String,
        result: Result<ProcessLaunch, String>,
    ) -> Vec<OutboundEvent> {
        match result {
            Ok(process_launch) => {
                let Some(address) = self.window_lookup.get(&window_id).cloned() else {
                    return self.launch_error_events(
                        window_id,
                        "Window not found".to_string(),
                        None,
                    );
                };
                let Some(tab) = self.tab(&address.tab_id) else {
                    return self.launch_error_events(
                        window_id,
                        "Project tab not found".to_string(),
                        None,
                    );
                };
                let Some(window) = tab.workspace.window(&address.raw_id) else {
                    return self.launch_error_events(
                        window_id,
                        "Window not found".to_string(),
                        None,
                    );
                };
                let geometry = window.geometry.clone();

                // SPEC-2809 (revised) — second Launch Wizard exit path
                // emits the same launch banner sequence as the primary
                // handler so the Console window's `agent` tab is
                // consistent regardless of which wizard outcome the user
                // came in through.
                let stage_id = next_agent_launch_stage_id(&self.agent_launch_stage_counter);
                emit_agent_launch_stage(
                    stage_id,
                    "resolve_binary",
                    &format!("wizard launch {}", process_launch.command),
                );
                emit_agent_launch_stage(
                    stage_id,
                    "prepare_env",
                    &format!("argv=[{}]", process_launch.args.join(" ")),
                );
                match self.spawn_process_window_with_console_kind(
                    &window_id,
                    geometry,
                    process_launch,
                    Some(gwt_core::process_console::ProcessKind::AgentBootstrap),
                ) {
                    Ok(()) => {
                        emit_agent_launch_stage(stage_id, "ready", "PTY handoff complete");
                        self.launch_error_terminal_details.remove(&window_id);
                        let mut events = vec![self.workspace_state_broadcast()];
                        let composed_status = self
                            .window_status(&window_id)
                            .unwrap_or(WindowProcessStatus::Running);
                        events.extend(Self::status_events(window_id, composed_status, None));
                        events
                    }
                    Err(error) => {
                        emit_agent_launch_stage(stage_id, "error", &error);
                        self.launch_error_events(window_id, error, None)
                    }
                }
            }
            Err(error) => self.launch_error_events(window_id, error, None),
        }
    }

    pub(crate) fn start_window(
        &mut self,
        tab_id: &str,
        raw_id: &str,
        preset: WindowPreset,
        geometry: WindowGeometry,
    ) -> Vec<OutboundEvent> {
        self.register_window(tab_id, raw_id);
        let window_id = combined_window_id(tab_id, raw_id);
        if !preset.requires_process() {
            self.set_window_status(tab_id, raw_id, WindowProcessStatus::Running);
            return Self::status_events(window_id, WindowProcessStatus::Running, None);
        }

        let project_root = self
            .tab(tab_id)
            .map(|tab| tab.project_root.clone())
            .unwrap_or_else(|| PathBuf::from("."));

        let shell = match detect_shell_program() {
            Ok(shell) => shell,
            Err(error) => {
                let detail = error.to_string();
                self.set_window_status(tab_id, raw_id, WindowProcessStatus::Error);
                self.window_details
                    .insert(window_id.clone(), detail.clone());
                return Self::status_events(window_id, WindowProcessStatus::Error, Some(detail));
            }
        };

        let launch = match resolve_launch_spec_with_fallback(preset, &shell) {
            Ok(launch) => launch,
            Err(error) => {
                let detail = error.to_string();
                self.set_window_status(tab_id, raw_id, WindowProcessStatus::Error);
                self.window_details
                    .insert(window_id.clone(), detail.clone());
                return Self::status_events(window_id, WindowProcessStatus::Error, Some(detail));
            }
        };

        let effective_env = match self.active_profile_spawn_env() {
            Ok(env) => env,
            Err(error) => {
                self.set_window_status(tab_id, raw_id, WindowProcessStatus::Error);
                self.window_details.insert(window_id.clone(), error.clone());
                return Self::status_events(window_id, WindowProcessStatus::Error, Some(error));
            }
        }
        .with_project_root(&project_root);
        let (env, remove_env) = effective_env.into_parts();

        // SPEC-2809 (revised) — Surface the launch pipeline for AI
        // agent presets (Codex / Claude / Gemini / Agent) so the Console
        // window's `agent` tab shows what gwt is doing leading up to the
        // PTY spawn. Plain `Shell` panes do not emit launch banners
        // because nothing distinguishes them from arbitrary terminals.
        let is_agent_preset = matches!(
            preset,
            WindowPreset::Claude | WindowPreset::Codex | WindowPreset::Agent
        );
        let console_kind =
            is_agent_preset.then_some(gwt_core::process_console::ProcessKind::AgentBootstrap);
        let stage_id =
            is_agent_preset.then(|| next_agent_launch_stage_id(&self.agent_launch_stage_counter));
        if let Some(id) = stage_id {
            emit_agent_launch_stage(
                id,
                "resolve_binary",
                &format!("{} ({})", preset.title(), launch.command),
            );
            emit_agent_launch_stage(
                id,
                "prepare_env",
                &format!("project_root={}", project_root.display()),
            );
            emit_agent_launch_stage(
                id,
                "spawn_pty",
                &format!("argv=[{}]", launch.args.join(" ")),
            );
        }
        match self.spawn_process_window_with_console_kind(
            &window_id,
            geometry,
            ProcessLaunch {
                command: launch.command,
                args: launch.args,
                env,
                remove_env,
                cwd: Some(project_root),
            },
            console_kind,
        ) {
            Ok(()) => {
                if let Some(id) = stage_id {
                    emit_agent_launch_stage(id, "ready", "PTY handoff complete");
                }
                let composed_status = self
                    .window_status(&window_id)
                    .unwrap_or(WindowProcessStatus::Running);
                Self::status_events(window_id, composed_status, None)
            }
            Err(error) => {
                if let Some(id) = stage_id {
                    emit_agent_launch_stage(id, "error", &error);
                }
                self.set_window_status(tab_id, raw_id, WindowProcessStatus::Error);
                self.window_details.insert(window_id.clone(), error.clone());
                Self::status_events(window_id, WindowProcessStatus::Error, Some(error))
            }
        }
    }

    #[allow(dead_code)]
    pub(crate) fn spawn_process_window(
        &mut self,
        id: &str,
        geometry: WindowGeometry,
        launch: ProcessLaunch,
    ) -> Result<(), String> {
        self.spawn_process_window_with_console_kind(id, geometry, launch, None)
    }

    pub(crate) fn spawn_process_window_with_console_kind(
        &mut self,
        id: &str,
        geometry: WindowGeometry,
        launch: ProcessLaunch,
        console_kind: Option<gwt_core::process_console::ProcessKind>,
    ) -> Result<(), String> {
        let (cols, rows) = geometry_to_pty_size(&geometry);
        let pane = Pane::new_with_spawn_config(
            id.to_string(),
            gwt_terminal::pty::SpawnConfig {
                command: launch.command,
                args: launch.args,
                cols,
                rows,
                env: launch.env,
                remove_env: launch.remove_env,
                cwd: launch.cwd,
            },
        )
        .map_err(|error| error.to_string())?;
        let pane = Arc::new(Mutex::new(pane));

        let output_thread = self.spawn_output_thread(id.to_string(), pane.clone(), console_kind);
        let status_thread = self.spawn_status_thread(id.to_string(), pane.clone());
        if let Some(address) = self.window_lookup.get(id).cloned() {
            self.window_pty_statuses
                .insert(id.to_string(), WindowProcessStatus::Running);
            self.window_hook_states.remove(id);
            self.set_window_status(
                &address.tab_id,
                &address.raw_id,
                WindowProcessStatus::Running,
            );
        }
        self.window_details.remove(id);
        // Publish the PTY handle to the WebSocket fast-path registry BEFORE
        // inserting the runtime so that the first `terminal_input` from the
        // frontend (which can arrive immediately after `TerminalStatus`) has a
        // target to write to. Registry holds a cloned `Arc<PtyHandle>`; the
        // real owner remains the `Mutex<Pane>` in `WindowRuntime`.
        self.register_pty_writer(id, &pane);
        self.runtimes.insert(
            id.to_string(),
            WindowRuntime {
                pane,
                output_thread: Some(output_thread),
                status_thread: Some(status_thread),
            },
        );
        Ok(())
    }

    pub(crate) fn spawn_agent_window(
        &mut self,
        tab_id: &str,
        config: gwt_agent::LaunchConfig,
        bounds: WindowGeometry,
        workspace_resume_context: Option<WorkspaceResumeContext>,
    ) -> Result<Vec<OutboundEvent>, String> {
        self.spawn_agent_window_with_placement(
            tab_id,
            config,
            AgentWindowPlacement::Centered(bounds),
            workspace_resume_context,
            None,
            None,
        )
    }

    pub(crate) fn spawn_agent_window_with_feedback(
        &mut self,
        tab_id: &str,
        config: gwt_agent::LaunchConfig,
        bounds: WindowGeometry,
        workspace_resume_context: Option<WorkspaceResumeContext>,
        launch_feedback_context: LaunchFeedbackContext,
    ) -> Result<Vec<OutboundEvent>, String> {
        self.spawn_agent_window_with_placement(
            tab_id,
            config,
            AgentWindowPlacement::Centered(bounds),
            workspace_resume_context,
            Some(launch_feedback_context),
            None,
        )
    }

    pub(crate) fn spawn_agent_window_in_agent_kanban(
        &mut self,
        tab_id: &str,
        config: gwt_agent::LaunchConfig,
        bounds: WindowGeometry,
        workspace_resume_context: Option<WorkspaceResumeContext>,
        launch_feedback_context: Option<LaunchFeedbackContext>,
        target: AgentKanbanLaunchTarget,
    ) -> Result<Vec<OutboundEvent>, String> {
        self.spawn_agent_window_with_placement(
            tab_id,
            config,
            AgentWindowPlacement::Centered(bounds),
            workspace_resume_context,
            launch_feedback_context,
            Some(target),
        )
    }

    pub(crate) fn spawn_agent_window_at_geometry(
        &mut self,
        tab_id: &str,
        config: gwt_agent::LaunchConfig,
        geometry: WindowGeometry,
        workspace_resume_context: Option<WorkspaceResumeContext>,
    ) -> Result<Vec<OutboundEvent>, String> {
        self.spawn_agent_window_with_placement(
            tab_id,
            config,
            AgentWindowPlacement::Exact(geometry),
            workspace_resume_context,
            None,
            None,
        )
    }

    pub(crate) fn live_agent_window_for_work(
        &self,
        tab_id: &str,
        branch: Option<&str>,
        worktree_path: Option<&Path>,
    ) -> Option<String> {
        let normalized_branch = branch
            .map(normalize_branch_name)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        self.active_agent_sessions
            .iter()
            .find(|(window_id, session)| {
                session.tab_id == tab_id
                    && self.window_lookup.contains_key(window_id.as_str())
                    && self
                        .window_status(window_id.as_str())
                        .is_some_and(|status| {
                            !matches!(
                                status,
                                WindowProcessStatus::Stopped | WindowProcessStatus::Error
                            )
                        })
                    && active_agent_session_matches_work(
                        session,
                        normalized_branch.as_deref(),
                        worktree_path,
                    )
            })
            .map(|(window_id, _)| window_id.clone())
    }

    pub(crate) fn focus_existing_live_work_agent_events(
        &mut self,
        window_id: &str,
        bounds: Option<WindowGeometry>,
    ) -> Vec<OutboundEvent> {
        let mut events = self.restore_window_events(window_id);
        events.extend(self.focus_window_events(window_id, bounds));
        if events.is_empty() {
            vec![self.workspace_state_broadcast()]
        } else {
            events
        }
    }

    fn spawn_agent_window_with_placement(
        &mut self,
        tab_id: &str,
        config: gwt_agent::LaunchConfig,
        placement: AgentWindowPlacement,
        workspace_resume_context: Option<WorkspaceResumeContext>,
        launch_feedback_context: Option<LaunchFeedbackContext>,
        agent_kanban_target: Option<AgentKanbanLaunchTarget>,
    ) -> Result<Vec<OutboundEvent>, String> {
        if let Some(window_id) = self.live_agent_window_for_work(
            tab_id,
            config.branch.as_deref(),
            config.working_dir.as_deref(),
        ) {
            return Ok(
                self.focus_existing_live_work_agent_events(&window_id, Some(placement.bounds()))
            );
        }
        // SPEC-2359 W-17 (FR-398, Issue #3034): the live-window check above
        // only sees launches whose agent session is already live. A re-click
        // while the previous launch is still materializing (window registered,
        // session pending) must focus that pending window, not spawn a twin.
        let inflight_key = inflight_launch_key(tab_id, &config);
        {
            let window_lookup = &self.window_lookup;
            self.inflight_launches.retain(|_, (window_id, started)| {
                started.elapsed() < INFLIGHT_LAUNCH_TTL
                    && window_lookup.contains_key(window_id.as_str())
            });
        }
        if let Some(key) = inflight_key.as_deref() {
            if let Some((window_id, _)) = self.inflight_launches.get(key) {
                let window_id = window_id.clone();
                return Ok(self
                    .focus_existing_live_work_agent_events(&window_id, Some(placement.bounds())));
            }
        }
        let issue_link_cache_dir = self.issue_link_cache_dir.clone();
        let tab = self
            .tab_mut(tab_id)
            .ok_or_else(|| "Project tab not found".to_string())?;
        let project_root_path = tab.project_root.clone();
        let project_root = project_root_path.display().to_string();
        let title = config.display_name.clone();
        let purpose_title = workspace_resume_context
            .as_ref()
            .and_then(WorkspaceResumeContext::purpose_title)
            .or_else(|| {
                agent_launch_purpose_title(
                    &project_root_path,
                    config.linked_issue_number,
                    config.branch.as_deref(),
                    &issue_link_cache_dir,
                )
            });
        let window = match placement {
            AgentWindowPlacement::Centered(bounds) => {
                tab.workspace
                    .add_window_with_title(WindowPreset::Agent, title, true, bounds)
            }
            AgentWindowPlacement::Exact(geometry) => tab
                .workspace
                .add_window_at_geometry_with_title(WindowPreset::Agent, title, true, geometry),
        };
        if let Some(purpose_title) = purpose_title {
            let _ = tab
                .workspace
                .set_purpose_title(&window.id, Some(purpose_title));
        }
        let _ = tab
            .workspace
            .set_agent_id(&window.id, config.agent_id.command().to_string());
        if let Some(target) = agent_kanban_target.as_ref() {
            let _ = tab.workspace.place_agent_window_in_kanban(
                &window.id,
                &target.board_id,
                target.lane_id,
                None,
            );
        }
        self.register_window(tab_id, &window.id);
        let window_id = combined_window_id(tab_id, &window.id);

        self.window_pty_statuses
            .insert(window_id.clone(), WindowProcessStatus::Running);
        self.window_hook_states.remove(&window_id);
        if let Some(key) = inflight_key {
            self.inflight_launches
                .insert(key, (window_id.clone(), std::time::Instant::now()));
        }

        let mut events = vec![self.workspace_state_broadcast()];
        let composed_status = self
            .window_status(&window_id)
            .unwrap_or(WindowProcessStatus::Running);
        events.extend(Self::status_events(
            window_id.clone(),
            composed_status,
            Some("Launching...".to_string()),
        ));

        let proxy = self.proxy.clone();
        let sessions_dir = self.sessions_dir.clone();
        let hook_forward_target = self.hook_forward_target.clone();
        let profile_config_path = self.profile_config_path()?;
        if let Some(context) = workspace_resume_context {
            self.pending_workspace_resume_contexts
                .insert(window_id.clone(), context);
        }
        if let Some(context) = launch_feedback_context {
            self.pending_launch_feedback_contexts
                .insert(window_id.clone(), context);
        }

        thread::spawn(move || {
            Self::spawn_agent_window_async(
                proxy,
                sessions_dir,
                project_root,
                window_id,
                config,
                profile_config_path,
                hook_forward_target,
            );
        });

        Ok(events)
    }

    pub(crate) fn spawn_agent_window_async(
        proxy: AppEventProxy,
        sessions_dir: PathBuf,
        project_root: String,
        window_id: String,
        mut config: gwt_agent::LaunchConfig,
        profile_config_path: PathBuf,
        hook_forward_target: Option<HookForwardTarget>,
    ) {
        // SPEC-2014 FR-139..142 — while a Docker launch prepares (preflight,
        // compose ps/up incl. image build, exec probes), mirror docker-kind
        // Process Console lines into the agent terminal. Host launches keep
        // their immediate-PTY behavior untouched (FR-142).
        let docker_output_mirror =
            (config.runtime_target == gwt_agent::LaunchRuntimeTarget::Docker).then(|| {
                launch_output_mirror::DockerLaunchOutputMirror::start(
                    proxy.clone(),
                    window_id.clone(),
                )
            });
        let result = (|| {
            proxy.send(UserEvent::LaunchProgress {
                window_id: window_id.clone(),
                message: "Preparing worktree...".to_string(),
            });
            resolve_launch_worktree(Path::new(&project_root), &mut config)?;

            proxy.send(UserEvent::LaunchProgress {
                window_id: window_id.clone(),
                message: "Starting Docker service...".to_string(),
            });
            apply_docker_runtime_to_launch_config(Path::new(&project_root), &mut config)?;

            proxy.send(UserEvent::LaunchProgress {
                window_id: window_id.clone(),
                message: "Configuring work...".to_string(),
            });
            let worktree_path = gwt_core::paths::normalize_windows_child_process_path(
                &config
                    .working_dir
                    .clone()
                    .unwrap_or_else(|| PathBuf::from(&project_root)),
            );
            if config.working_dir.is_some() {
                config.working_dir = Some(worktree_path.clone());
            }
            gwt_agent::LaunchEnvironment::from_active_profile(
                &profile_config_path,
                config.runtime_target,
            )?
            .with_project_root(&worktree_path)
            .apply_to_parts(&mut config.env_vars, &mut config.remove_env);
            let codex_hook_discovery_mode = codex_hook_discovery_mode_for_launch_config(&config);
            refresh_managed_gwt_assets_for_agent_with_codex_hook_discovery_mode(
                &worktree_path,
                &config.agent_id,
                codex_hook_discovery_mode,
            )
            .map_err(|error| error.to_string())?;
            let codex_home = config.env_vars.get("CODEX_HOME").map(PathBuf::from);
            if let Some(report) = maybe_register_codex_managed_hook_trust_for_launch(
                &profile_config_path,
                &worktree_path,
                &config.agent_id,
                config.runtime_target,
                config.docker_service.as_deref(),
                codex_home.as_deref(),
                codex_hook_discovery_mode,
            )? {
                if !report.trusted_entries.is_empty() {
                    proxy.send(UserEvent::LaunchProgress {
                        window_id: window_id.clone(),
                        message: format!(
                            "Trusted {} gwt-managed Codex hooks.",
                            report.trusted_entries.len()
                        ),
                    });
                }
            }

            if config.runtime_target == gwt_agent::LaunchRuntimeTarget::Host {
                let fallback_report = apply_host_package_runner_fallback_checked(&mut config)?;
                for message in fallback_report.messages {
                    proxy.send(UserEvent::LaunchProgress {
                        window_id: window_id.clone(),
                        message,
                    });
                }
            }
            install_launch_gwt_bin_env(&mut config.env_vars, config.runtime_target)?;
            apply_windows_host_shell_wrapper(&mut config)?;

            let branch_name = config.branch.clone().unwrap_or_else(|| "work".to_string());

            let agent_id = config.agent_id.clone();
            let mut session =
                gwt_agent::Session::new(&worktree_path, branch_name.clone(), agent_id.clone());
            session.project_state_root = Some(
                gwt_core::paths::normalize_windows_child_process_path(Path::new(&project_root)),
            );
            session.display_name = config.display_name.clone();
            session.tool_version = config.tool_version.clone();
            session.model = config.model.clone();
            session.reasoning_level = config.reasoning_level.clone();
            session.session_mode = config.session_mode;
            session.skip_permissions = config.skip_permissions;
            session.fast_mode = config.fast_mode;
            session.codex_fast_mode = config.codex_fast_mode;
            session.runtime_target = config.runtime_target;
            session.docker_service = config.docker_service.clone();
            session.docker_lifecycle_intent = config.docker_lifecycle_intent;
            session.linked_issue_number = config.linked_issue_number;
            session.launch_command = config.command.clone();
            session.launch_args = config.args.clone();
            session.windows_shell = config.windows_shell;
            if session.session_mode == gwt_agent::SessionMode::Resume {
                session.agent_session_id = config.resume_session_id.clone();
            }
            session.update_status(gwt_agent::AgentStatus::Running);

            let session_id = session.id.clone();
            let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, &session_id);
            config.env_vars.insert(
                gwt_agent::GWT_SESSION_ID_ENV.to_string(),
                session_id.clone(),
            );
            config.env_vars.insert(
                gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV.to_string(),
                runtime_path.display().to_string(),
            );
            if let Some(target) = hook_forward_target {
                config
                    .env_vars
                    .insert(gwt_agent::GWT_HOOK_FORWARD_URL_ENV.to_string(), target.url);
                config.env_vars.insert(
                    gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV.to_string(),
                    target.token,
                );
            }
            config
                .env_vars
                .entry("COLORTERM".to_string())
                .or_insert_with(|| "truecolor".to_string());
            finalize_docker_agent_launch_config(Path::new(&project_root), &mut config)?;
            let runtime_target = config.runtime_target;
            let agent_project_root = if runtime_target == gwt_agent::LaunchRuntimeTarget::Docker {
                resolve_docker_launch_plan(&worktree_path, config.docker_service.as_deref())?
                    .container_cwd
            } else {
                config
                    .env_vars
                    .get("GWT_PROJECT_ROOT")
                    .cloned()
                    .unwrap_or_else(|| worktree_path.display().to_string())
            };

            session
                .save(&sessions_dir)
                .map_err(|error| error.to_string())?;
            gwt_agent::SessionRuntimeState::new(gwt_agent::AgentStatus::Running)
                .save(&runtime_path)
                .map_err(|error| error.to_string())?;

            let process_launch = ProcessLaunch {
                command: config.command.clone(),
                args: config.args.clone(),
                env: config.env_vars.clone(),
                remove_env: config.remove_env.clone(),
                cwd: config.working_dir.clone(),
            };

            Ok((
                process_launch,
                session_id,
                branch_name,
                config.display_name,
                worktree_path,
                agent_id,
                config.linked_issue_number,
                config.base_branch.clone(),
                runtime_target,
                agent_project_root,
            ))
        })();

        // Drop (= final drain + join) BEFORE dispatching the result so the
        // tail of the mirrored docker output lands in the terminal ahead of
        // the success transition or the `[gwt] Launch failed` summary —
        // otherwise the failure summary gets buried mid-stream.
        drop(docker_output_mirror);

        match result {
            Ok((
                process_launch,
                session_id,
                branch_name,
                display_name,
                worktree_path,
                agent_id,
                linked_issue_number,
                base_branch,
                runtime_target,
                agent_project_root,
            )) => {
                dispatch_agent_launch_success(
                    proxy,
                    window_id,
                    (
                        process_launch,
                        session_id,
                        branch_name,
                        display_name,
                        worktree_path,
                        agent_id,
                        linked_issue_number,
                        base_branch,
                        runtime_target,
                        agent_project_root,
                    ),
                    |proxy, project_index_root| {
                        crate::project_index_bootstrap::ProjectIndexBootstrapService::global()
                            .spawn(proxy, project_index_root);
                    },
                );
            }
            Err(error) => {
                proxy.send(UserEvent::LaunchComplete {
                    window_id,
                    result: Err(error),
                });
            }
        }
    }

    /// SPEC-2359 Phase W-12 Slice 4 (FR-352): handle a user-initiated Work close
    /// from the Work surface. `close_kind` is `"done"` or `"discarded"`.
    ///
    /// Behavior:
    /// - If the owning agent session (derived from `work_id`) is still live, the
    ///   close is blocked and the worktree is left untouched (FR-352). The
    ///   owning agent must be stopped first.
    /// - Otherwise (a Paused Work with no running agent), the worktree is removed
    ///   (worktree only — branch / PR are retained) and the terminal close is
    ///   recorded in the work history. A `done` close records a Done event; a
    ///   `discarded` close records a Discard event. Both remove the Work from the
    ///   active Work surface. Re-closing an already-closed Work is a noop.
    pub(crate) fn close_work(&mut self, work_id: &str, close_kind: &str) -> Vec<OutboundEvent> {
        let work_id = work_id.trim();
        if work_id.is_empty() {
            return Vec::new();
        }
        let close_kind = match close_kind.trim().to_ascii_lowercase().as_str() {
            "done" => gwt_core::workspace_projection::WorkCloseKind::Done,
            "discarded" => gwt_core::workspace_projection::WorkCloseKind::Discarded,
            other => {
                tracing::warn!(
                    work_id = %work_id,
                    close_kind = %other,
                    "ignoring Work close with unknown close_kind"
                );
                return Vec::new();
            }
        };

        let Some(project_root) = self.active_project_root().map(Path::to_path_buf) else {
            tracing::warn!(work_id = %work_id, "Work close has no active project tab");
            return Vec::new();
        };

        // The session id of an agent-session Work is encoded in the Work id
        // (`work-session-<session_id>`). A live agent owns the Work when any
        // active session matches that id.
        let session_id = work_id
            .strip_prefix("work-session-")
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let has_live_agent = session_id.is_some_and(|session_id| {
            self.active_agent_sessions
                .values()
                .any(|session| session.session_id == session_id)
        });

        // Resolve the worktree path from the retained work history so a Paused
        // Work can have its worktree removed without a live session.
        let worktree_path = self.resolve_work_worktree_path(&project_root, work_id);

        let decision =
            gwt_core::workspace_projection::decide_work_close(has_live_agent, worktree_path);

        match decision {
            gwt_core::workspace_projection::WorkCloseDecision::BlockedLiveAgent => {
                // FR-352: never clean up a Work while its agent session is live.
                tracing::warn!(
                    work_id = %work_id,
                    session_id = session_id.unwrap_or_default(),
                    "Work close blocked: owning agent session is still live; stop the agent before closing"
                );
                return Vec::new();
            }
            gwt_core::workspace_projection::WorkCloseDecision::CleanupWorktree {
                worktree_path,
            } => {
                self.remove_work_worktree_only(&project_root, &worktree_path);
            }
            gwt_core::workspace_projection::WorkCloseDecision::RecordOnly => {
                // No resolvable worktree path: record the close without any
                // filesystem side effect.
            }
        }

        // Record the terminal close in the work history. Idempotent against an
        // already-closed Work, so a duplicate close emits no new event.
        let now = chrono::Utc::now();
        let recorded = match close_kind {
            gwt_core::workspace_projection::WorkCloseKind::Done => {
                gwt_core::workspace_projection::emit_workspace_done_event_if_absent(
                    &project_root,
                    work_id,
                    now,
                )
            }
            gwt_core::workspace_projection::WorkCloseKind::Discarded => {
                gwt_core::workspace_projection::emit_workspace_discard_event_if_absent(
                    &project_root,
                    work_id,
                    now,
                )
            }
        };
        if let Err(error) = recorded {
            tracing::warn!(
                work_id = %work_id,
                error = %error,
                "failed to record Work terminal close event"
            );
        }

        // Broadcast the refreshed projection so the Work leaves the active
        // surface for every connected client.
        self.active_work_projection_broadcast_for_active_tab()
            .into_iter()
            .collect()
    }

    /// SPEC-2359 Phase W-12 Slice 4 (FR-352): resolve the worktree path for
    /// `work_id` from the retained work history's execution containers. Returns
    /// `None` when the Work has no recorded worktree, in which case the close is
    /// recorded without filesystem cleanup.
    fn resolve_work_worktree_path(&self, project_root: &Path, work_id: &str) -> Option<PathBuf> {
        let projection = self
            .work_items_cache
            .borrow_mut()
            .load_or_synthesize(project_root)
            .ok()?;
        let item = projection
            .work_items
            .iter()
            .find(|item| item.id == work_id)?;
        item.execution_containers
            .iter()
            .find_map(|container| container.worktree_path.clone())
    }

    /// SPEC-2359 Phase W-12 Slice 4 (FR-352): remove the worktree at
    /// `worktree_path` (worktree only — the branch and any PR are retained). A
    /// missing or already-removed worktree is treated as success so the close
    /// stays robust; other failures are logged but do not abort recording the
    /// close.
    fn remove_work_worktree_only(&self, project_root: &Path, worktree_path: &Path) {
        let main_repo_path = match gwt_git::worktree::main_worktree_root(project_root) {
            Ok(path) => path,
            Err(error) => {
                tracing::warn!(
                    project_root = %project_root.display(),
                    worktree_path = %worktree_path.display(),
                    error = %error,
                    "Work close could not resolve main worktree root; skipping worktree removal"
                );
                return;
            }
        };
        let manager = gwt_git::WorktreeManager::new(&main_repo_path);
        if let Err(error) = manager.remove_force(worktree_path) {
            tracing::warn!(
                worktree_path = %worktree_path.display(),
                error = %error,
                "Work close worktree removal failed; recording the close anyway"
            );
        }
    }

    pub(crate) fn mark_agent_session_stopped(&mut self, window_id: &str) {
        let Some(session) = self.active_agent_sessions.remove(window_id) else {
            return;
        };
        if let Some(project_root) = self
            .tab(&session.tab_id)
            .map(|tab| tab.project_root.clone())
        {
            // SPEC-2359 Phase W-12 Slice 5a (FR-350): persist a Paused marker
            // before clearing the agent from the live projection so the Work is
            // retained on the Work surface until the user explicitly closes it.
            self.persist_paused_work_for_stopped_session(&project_root, &session);
            if let Err(error) = gwt_core::workspace_projection::mark_workspace_agent_stopped(
                &project_root,
                &session.session_id,
                Some(&session.window_id),
            ) {
                tracing::warn!(
                    error = %error,
                    project_root = %project_root.display(),
                    session_id = %session.session_id,
                    window_id = %session.window_id,
                    "failed to clean stopped Agent from Workspace projection"
                );
            }
        }
        let _ = gwt_agent::persist_session_status(
            &self.sessions_dir,
            &session.session_id,
            gwt_agent::AgentStatus::Stopped,
        );
        self.launch_wizard_cache.mark_stopped(&session.session_id);
    }

    /// SPEC-2359 Phase W-12 Slice 5a (FR-350): record a Pause work event for a
    /// stopped agent session so the Work persists in the work history and keeps
    /// surfacing as Paused. The Work id is the session-derived canonical id
    /// (`work-session-<session_id>`) so a later resume groups the live agent onto
    /// the same row and dedupes the Paused entry away. Identity (title / branch /
    /// worktree / board refs) is recovered from the saved projection's matching
    /// agent and git details, falling back to the live session when unavailable.
    fn persist_paused_work_for_stopped_session(
        &self,
        project_root: &Path,
        session: &ActiveAgentSession,
    ) {
        let session_id = session.session_id.trim();
        if session_id.is_empty() {
            return;
        }
        let work_id = format!("work-session-{session_id}");
        let projection = gwt_core::workspace_projection::load_workspace_projection(project_root)
            .ok()
            .flatten();
        let agent_summary = projection.as_ref().and_then(|projection| {
            projection
                .agents
                .iter()
                .find(|agent| agent.session_id == session_id)
        });
        // #3065: owner / summary / the title fallback must come from the
        // session's own Work item (resolved by branch container inside the
        // background thread below), never from the repo-shared projection —
        // its identity belongs to whatever Work last wrote it.
        let agent_title = agent_summary
            .and_then(|agent| {
                agent
                    .title_summary
                    .clone()
                    .or_else(|| agent.current_focus.clone())
            })
            .filter(|value| !value.trim().is_empty());
        let board_refs = projection
            .as_ref()
            .map(|projection| projection.board_refs.clone())
            .unwrap_or_default();
        let branch = agent_summary
            .and_then(|agent| agent.branch.clone())
            .or_else(|| {
                projection
                    .as_ref()
                    .and_then(|projection| projection.git_details.as_ref())
                    .and_then(|details| details.branch.clone())
            })
            .or_else(|| Some(session.branch_name.clone()))
            .filter(|value| !value.trim().is_empty());
        let worktree_path = agent_summary
            .and_then(|agent| agent.worktree_path.clone())
            .or_else(|| {
                projection
                    .as_ref()
                    .and_then(|projection| projection.git_details.as_ref())
                    .and_then(|details| details.worktree_path.clone())
            })
            .or_else(|| Some(session.worktree_path.clone()));
        let git_details = projection
            .as_ref()
            .and_then(|projection| projection.git_details.clone());
        let execution_container = (branch.is_some() || worktree_path.is_some()).then(|| {
            gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                branch,
                worktree_path,
                pr_number: git_details.as_ref().and_then(|details| details.pr_number),
                pr_url: git_details
                    .as_ref()
                    .and_then(|details| details.pr_url.clone()),
                pr_state: git_details
                    .as_ref()
                    .and_then(|details| details.pr_state.clone()),
            }
        });
        // Close-latency root fix (2026-06-12): the record loads + saves the
        // home works.json (megabytes once a project has hundreds of Works).
        // Doing that synchronously on the UI event loop made every agent
        // window × stall for seconds (sampled: serde to_vec_pretty dominating
        // the close handler). Inputs are gathered synchronously above from
        // the in-memory projection; the file IO runs on a background thread
        // and the workspace projection watcher broadcasts the refreshed rows
        // once the write lands.
        let project_root = project_root.to_path_buf();
        let session_id = session_id.to_string();
        let log_session_id = session.session_id.clone();
        let lookup_branch = execution_container
            .as_ref()
            .and_then(|container| container.branch.clone());
        let lookup_worktree = execution_container
            .as_ref()
            .and_then(|container| container.worktree_path.clone());
        let record = thread::spawn(move || {
            // #3065: resolve identity from the session's own Work item. The
            // works.json IO already happens on this background thread for the
            // record itself, so the lookup adds no UI-loop cost.
            let own_item = gwt_core::workspace_projection::load_workspace_work_items(&project_root)
                .ok()
                .flatten()
                .and_then(|works| {
                    gwt_core::workspace_projection::find_work_item_for_container(
                        &works,
                        &project_root,
                        lookup_branch.as_deref(),
                        lookup_worktree.as_deref(),
                    )
                    .map(|item| {
                        (
                            item.title.clone(),
                            item.summary.clone().or_else(|| item.intent.clone()),
                            item.owner.clone(),
                        )
                    })
                });
            let (item_title, summary, owner) = own_item.unwrap_or((String::new(), None, None));
            let title =
                agent_title.or_else(|| Some(item_title).filter(|value| !value.trim().is_empty()));
            if let Err(error) = gwt_core::workspace_projection::record_workspace_work_paused_event(
                &project_root,
                &work_id,
                title.as_deref(),
                summary.as_deref(),
                owner.as_deref(),
                &board_refs,
                execution_container,
                Some(&session_id),
                chrono::Utc::now(),
            ) {
                tracing::warn!(
                    error = %error,
                    project_root = %project_root.display(),
                    session_id = %log_session_id,
                    work_id = %work_id,
                    "failed to persist Paused Work for stopped Agent session"
                );
            }
        });
        // Unit tests assert the projection immediately after a stop, so the
        // write is joined for determinism there; production detaches it.
        #[cfg(test)]
        let _ = record.join();
        #[cfg(not(test))]
        drop(record);
    }

    pub(crate) fn clear_agent_window_startup_restore(&self, window_id: &str) {
        let Some(session) = self.active_agent_sessions.get(window_id) else {
            return;
        };
        let _ = gwt_agent::persist_session_restore_window_on_startup(
            &self.sessions_dir,
            &session.session_id,
            false,
        );
    }

    fn refresh_launch_wizard_session_cache(&mut self, window_id: &str) {
        let Some(session) = self.active_agent_sessions.get(window_id) else {
            return;
        };
        let path = self
            .sessions_dir
            .join(format!("{}.toml", session.session_id));
        match gwt_agent::Session::load_and_migrate(&path) {
            Ok(session) => self.launch_wizard_cache.record_session(session),
            Err(error) => tracing::warn!(
                path = %path.display(),
                error = %error,
                "failed to refresh Launch Wizard session cache"
            ),
        }
    }
}

#[cfg(test)]
#[path = "agent_launch_stage_tests.rs"]
mod agent_launch_stage_tests;
