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
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;

use super::{
    active_agent_session_matches_work, agent_launch_purpose_title,
    apply_docker_runtime_to_launch_config, apply_host_package_runner_fallback_checked,
    apply_windows_host_shell_wrapper, combined_window_id, detect_shell_program,
    docker_binary_for_launch, finalize_docker_agent_launch_config, geometry_to_pty_size,
    install_launch_gwt_bin_env, intake_hook_config_is_disposable, is_ephemeral_intake_worktree,
    launch_output_mirror, normalize_branch_name, pin_host_codex_latest_runner,
    refresh_managed_gwt_assets_for_agent_with_codex_hook_discovery_mode,
    resolve_docker_launch_plan, resolve_launch_spec_with_fallback, resolve_launch_worktree,
    same_worktree_path, save_resumed_workspace_projection, save_start_work_workspace_projection,
    ActiveAgentSession, AgentCapabilityIssuer, AgentKanbanLaunchTarget, AppEventProxy, AppRuntime,
    BackendEvent, LaunchFeedbackContext, LiveSessionEntry, OutboundEvent, Pane, UserEvent,
    WindowGeometry, WindowPreset, WindowProcessStatus, WindowRuntime, WorkspaceResumeContext,
};

const RECOVERY_CONTINUATION_MAX_VISIBLE_ITEMS: usize = 12;
const RECOVERY_CONTINUATION_MAX_CHARS: usize = 12_000;

#[derive(Clone)]
pub struct ProcessLaunch {
    pub(crate) command: String,
    pub(crate) args: Vec<String>,
    pub(crate) env: HashMap<String, String>,
    pub(crate) remove_env: Vec<String>,
    pub(crate) cwd: Option<PathBuf>,
}

fn private_launch_env_key(key: &str) -> bool {
    matches!(
        key,
        gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV
            | gwt_agent::GWT_SESSION_ID_ENV
            | gwt_agent::GWT_RECOVERY_ID_ENV
            | gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV
            | gwt::codex_bridge::CODEX_REMOTE_AUTH_TOKEN_ENV
    )
}

impl std::fmt::Debug for ProcessLaunch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let redacted_env = self
            .env
            .iter()
            .map(|(key, value)| {
                (
                    key.as_str(),
                    if private_launch_env_key(key) {
                        "<redacted>"
                    } else {
                        value.as_str()
                    },
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        let redacted_args = self
            .args
            .iter()
            .map(|argument| {
                private_launch_env_assignment(argument)
                    .map(|key| format!("{key}=<redacted>"))
                    .unwrap_or_else(|| argument.clone())
            })
            .collect::<Vec<_>>();

        formatter
            .debug_struct("ProcessLaunch")
            .field("command", &self.command)
            .field("args", &redacted_args)
            .field("env", &redacted_env)
            .field("remove_env", &self.remove_env)
            .field("cwd", &self.cwd)
            .finish()
    }
}

fn private_launch_env_assignment(argument: &str) -> Option<&str> {
    let (key, _) = argument.split_once('=')?;
    private_launch_env_key(key).then_some(key)
}

fn install_agent_capability_env(
    env: &mut HashMap<String, String>,
    issuer: Option<&AgentCapabilityIssuer>,
    project_root: &Path,
    session_id: &str,
    runtime_target: gwt_agent::LaunchRuntimeTarget,
    container_runtime_binary: &str,
) -> Result<(), String> {
    let Some(issuer) = issuer else {
        return Ok(());
    };
    let target = issuer.issue(project_root, session_id)?;
    let forward_url = gwt::daemon_runtime::hook_forward_url_for_launch_runtime(
        &target.url,
        runtime_target,
        container_runtime_binary,
    )?;
    env.insert(gwt_agent::GWT_HOOK_FORWARD_URL_ENV.to_string(), forward_url);
    env.insert(
        gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV.to_string(),
        target.token,
    );
    Ok(())
}

fn executable_basename(command: &str) -> &str {
    command
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(command)
        .trim_end_matches(".exe")
        .trim_end_matches(".cmd")
        .trim_end_matches(".bat")
}

fn codex_runner_prefix_len(command: &str, args: &[String]) -> Result<usize, String> {
    match executable_basename(command).to_ascii_lowercase().as_str() {
        "codex" => Ok(0),
        "bunx" | "npx" => {
            let mut package_index = 0;
            if args.get(package_index).is_some_and(|arg| arg == "--yes") {
                package_index += 1;
            }
            let package = args.get(package_index).ok_or_else(|| {
                "Codex package runner is missing its @openai/codex package spec".to_string()
            })?;
            if package == "@openai/codex" || package.starts_with("@openai/codex@") {
                Ok(package_index + 1)
            } else {
                Err("Codex package runner does not target @openai/codex".to_string())
            }
        }
        _ => Err("Codex Host runner cannot be paired with an exact app-server".to_string()),
    }
}

struct PreparedContainerCodexBridge {
    app_server: gwt::codex_bridge::CodexAppServerLaunch,
    runner_prefix_len: usize,
    compose_files: Vec<PathBuf>,
    service: String,
    container_cwd: String,
    recovery_attachments: Option<gwt::codex_bridge::ContainerRecoveryAttachmentBundle>,
}

enum PreparedCodexBridge {
    Host(gwt::codex_bridge::CodexAppServerLaunch),
    Container(PreparedContainerCodexBridge),
}

fn codex_app_server_for_runner(
    config: &gwt_agent::LaunchConfig,
    runner_prefix_len: usize,
    cwd: Option<PathBuf>,
) -> gwt::codex_bridge::CodexAppServerLaunch {
    let mut args = config.args[..runner_prefix_len].to_vec();
    args.extend([
        "app-server".to_string(),
        "--listen".to_string(),
        "stdio://".to_string(),
    ]);
    gwt::codex_bridge::CodexAppServerLaunch {
        command: config.command.clone(),
        args,
        env: HashMap::new(),
        remove_env: vec![gwt::codex_bridge::CODEX_REMOTE_AUTH_TOKEN_ENV.to_string()],
        cwd,
    }
}

fn prepare_codex_remote_bridge(
    config: &mut gwt_agent::LaunchConfig,
) -> Result<Option<PreparedCodexBridge>, String> {
    if config.agent_id != gwt_agent::AgentId::Codex {
        return Ok(None);
    }
    let runner_prefix_len = codex_runner_prefix_len(&config.command, &config.args)?;
    match config.runtime_target {
        gwt_agent::LaunchRuntimeTarget::Host => {
            let endpoint =
                gwt::codex_bridge::codex_bridge_endpoint().map_err(|error| error.to_string())?;
            let app_server =
                codex_app_server_for_runner(config, runner_prefix_len, config.working_dir.clone());
            config.args.splice(
                runner_prefix_len..runner_prefix_len,
                [
                    "--remote".to_string(),
                    endpoint,
                    "--remote-auth-token-env".to_string(),
                    gwt::codex_bridge::CODEX_REMOTE_AUTH_TOKEN_ENV.to_string(),
                ],
            );
            Ok(Some(PreparedCodexBridge::Host(app_server)))
        }
        gwt_agent::LaunchRuntimeTarget::Docker => {
            let worktree = config
                .working_dir
                .as_deref()
                .ok_or_else(|| "Docker Codex launch is missing its worktree".to_string())?;
            let plan = resolve_docker_launch_plan(worktree, config.docker_service.as_deref())?;
            let app_server = codex_app_server_for_runner(
                config,
                runner_prefix_len,
                Some(PathBuf::from(&plan.container_cwd)),
            );
            Ok(Some(PreparedCodexBridge::Container(
                PreparedContainerCodexBridge {
                    app_server,
                    runner_prefix_len,
                    compose_files: plan.compose_files_for_runtime(),
                    service: plan.service,
                    container_cwd: plan.container_cwd,
                    recovery_attachments: None,
                },
            )))
        }
    }
}

fn prepare_container_checkpoint_attachments(
    project_root: &Path,
    config: &mut gwt_agent::LaunchConfig,
) -> Result<Option<gwt::codex_bridge::ContainerRecoveryAttachmentBundle>, String> {
    if config.agent_id != gwt_agent::AgentId::Codex
        || config.runtime_target != gwt_agent::LaunchRuntimeTarget::Docker
    {
        return Ok(None);
    }
    if !config
        .recovery_continuation
        .as_ref()
        .is_some_and(|continuation| continuation.inherit_checkpoint)
    {
        return Ok(None);
    }
    let store = gwt_core::recovery::RecoveryStore::for_project_dir(
        gwt_core::paths::gwt_project_dir_for_repo_path(project_root),
    );
    prepare_container_checkpoint_attachments_from_store(&store, config)
}

fn prepare_container_checkpoint_attachments_from_store(
    store: &gwt_core::recovery::RecoveryStore,
    config: &mut gwt_agent::LaunchConfig,
) -> Result<Option<gwt::codex_bridge::ContainerRecoveryAttachmentBundle>, String> {
    let continuation = config
        .recovery_continuation
        .as_ref()
        .filter(|continuation| continuation.inherit_checkpoint)
        .ok_or_else(|| "checkpoint continuation provenance is missing".to_string())?;
    let record = store
        .load(&continuation.source_recovery_id)
        .map_err(|error| format!("load source recovery attachments: {error}"))?
        .ok_or_else(|| "source recovery attachments are unavailable".to_string())?;
    if record.checkpoint_revision != continuation.source_checkpoint_revision {
        return Err(format!(
            "source recovery attachment revision changed: expected {}, found {}",
            continuation.source_checkpoint_revision, record.checkpoint_revision
        ));
    }
    let references = record
        .checkpoint
        .as_ref()
        .map(|checkpoint| checkpoint.attachment_refs.as_slice())
        .unwrap_or_default();
    let Some(bundle) =
        gwt::codex_bridge::prepare_container_recovery_attachments(store, references)?
    else {
        return Ok(None);
    };
    let container_paths = bundle.container_paths()?;
    let prompt = gwt_core::recovery::build_checkpoint_continuation_prompt_with_attachment_paths(
        &record,
        &container_paths,
        RECOVERY_CONTINUATION_MAX_VISIBLE_ITEMS,
        RECOVERY_CONTINUATION_MAX_CHARS,
    )
    .map_err(|error| format!("build container recovery attachment prompt: {error}"))?;
    let previous_prompt = config
        .initial_prompt
        .as_deref()
        .ok_or_else(|| "checkpoint continuation is missing its prompt".to_string())?;
    let trailing_prompt = config
        .args
        .last_mut()
        .ok_or_else(|| "checkpoint continuation is missing its prompt argument".to_string())?;
    if trailing_prompt != previous_prompt {
        return Err("checkpoint continuation prompt argument changed before staging".to_string());
    }
    *trailing_prompt = prompt.clone();
    config.initial_prompt = Some(prompt);
    Ok(Some(bundle))
}

/// Prepare the Host-only Codex split without starting either process.
///
/// The app-server retains the exact executable and package/version prefix used
/// by the TUI. Only the fixed app-server subcommand differs. The route
/// capability itself is injected later, immediately before PTY spawn.
#[cfg(test)]
fn prepare_host_codex_remote_bridge(
    config: &mut gwt_agent::LaunchConfig,
) -> Result<Option<gwt::codex_bridge::CodexAppServerLaunch>, String> {
    if config.agent_id != gwt_agent::AgentId::Codex
        || config.runtime_target != gwt_agent::LaunchRuntimeTarget::Host
    {
        return Ok(None);
    }

    match prepare_codex_remote_bridge(config)? {
        Some(PreparedCodexBridge::Host(app_server)) => Ok(Some(app_server)),
        Some(PreparedCodexBridge::Container(_)) | None => Ok(None),
    }
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

fn pending_codex_bridge_routes(
) -> &'static Mutex<HashMap<String, gwt::codex_bridge::CodexLaunchBridgeLease>> {
    static ROUTES: OnceLock<Mutex<HashMap<String, gwt::codex_bridge::CodexLaunchBridgeLease>>> =
        OnceLock::new();
    ROUTES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn stage_codex_bridge_route(window_id: String, route: gwt::codex_bridge::CodexLaunchBridgeLease) {
    pending_codex_bridge_routes()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .insert(window_id, route);
}

fn take_staged_codex_bridge_route(
    window_id: &str,
) -> Option<gwt::codex_bridge::CodexLaunchBridgeLease> {
    pending_codex_bridge_routes()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .remove(window_id)
}

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
    launch_config_from_persisted_session_inner(session, None)
}

/// Build a fresh-provider launch that carries only the bounded, durable
/// checkpoint prompt. In particular, this rebuilds provider argv instead of
/// mutating an exact-resume config, so a rejected provider root can never leak
/// its resume flag or id into the continuation attempt.
pub(super) fn launch_checkpoint_continuation_config(
    session: &gwt_agent::Session,
    prompt: &str,
) -> gwt_agent::LaunchConfig {
    launch_config_from_persisted_session_inner(session, Some(prompt))
}

fn launch_config_from_persisted_session_inner(
    session: &gwt_agent::Session,
    checkpoint_prompt: Option<&str>,
) -> gwt_agent::LaunchConfig {
    let agent_id = session.agent_id.clone();
    let mut builder = gwt_agent::AgentLaunchBuilder::new(agent_id);
    builder = builder.working_dir(session.worktree_path.clone());
    if session.is_ephemeral {
        builder = builder.ephemeral(session.ephemeral_base_ref.clone());
    } else if !session.branch.is_empty() {
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

    if let Some(prompt) = checkpoint_prompt {
        builder = builder
            .session_mode(gwt_agent::SessionMode::Normal)
            .initial_prompt(prompt.to_string())
            .extra_arg(prompt.to_string());
    } else if let Some(resume_id) = session.exact_resume_session_id() {
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

fn persist_recovery_checkpoint_before_spawn(
    project_root: &Path,
    worktree_path: &Path,
    config: &gwt_agent::LaunchConfig,
    session: &mut gwt_agent::Session,
    project_dir_override: Option<&Path>,
) -> Result<(), String> {
    let output = gwt_core::process::hidden_command("git")
        .args(["rev-parse", "--verify", "HEAD^{commit}"])
        .current_dir(worktree_path)
        .output()
        .map_err(|error| format!("resolve recovery base OID: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "resolve recovery base OID: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let launch_head_oid = String::from_utf8(output.stdout)
        .map_err(|error| format!("decode recovery base OID: {error}"))?
        .trim()
        .to_string();
    if launch_head_oid.is_empty() {
        return Err("resolve recovery base OID: git returned an empty OID".to_string());
    }

    let recovery_id = session
        .recovery_id
        .clone()
        .ok_or_else(|| "new session is missing recovery identity".to_string())?;
    session.launch_base_oid = Some(launch_head_oid.clone());
    session
        .advance_recovery_launch_stage(
            gwt_agent::session::RecoveryLaunchStage::WorktreeMaterialized,
        )
        .map_err(|error| format!("advance Session worktree recovery stage: {error}"))?;
    let session_kind = if config.is_ephemeral {
        gwt_core::recovery::RecoverySessionKind::Intake
    } else {
        gwt_core::recovery::RecoverySessionKind::Execution
    };
    let runtime = match config.runtime_target {
        gwt_agent::LaunchRuntimeTarget::Host => "host",
        gwt_agent::LaunchRuntimeTarget::Docker => "docker",
    };
    let project_dir = project_dir_override
        .map(Path::to_path_buf)
        .unwrap_or_else(|| gwt_core::paths::gwt_project_dir_for_repo_path(project_root));
    let store = gwt_core::recovery::RecoveryStore::for_project_dir(project_dir);
    let request = gwt_core::recovery::CreateRecovery {
        recovery_id: recovery_id.clone(),
        session_id: session.id.clone(),
        repo_id: gwt_core::paths::project_scope_hash(project_root).to_string(),
        session_kind,
        worktree_path: worktree_path.to_path_buf(),
        launch_base_ref: config
            .ephemeral_base_ref
            .clone()
            .or_else(|| config.base_branch.clone()),
        launch_base_oid: launch_head_oid.clone(),
        launch_head_oid: launch_head_oid.clone(),
        provider: config.agent_id.to_string(),
        model: config.model.clone(),
        runtime: runtime.to_string(),
        initial_prompt: config.initial_prompt.clone().unwrap_or_default(),
        created_at: session.created_at,
    };
    store
        .create(request, format!("create:{}", session.id))
        .map_err(|error| format!("persist recovery checkpoint before spawn: {error}"))?;
    if session_kind == gwt_core::recovery::RecoverySessionKind::Intake {
        gwt_git::recovery::ensure_recovery_base_pin(project_root, &recovery_id, &launch_head_oid)
            .map_err(|error| format!("pin Intake recovery base before spawn: {error}"))?;
    }

    if let Some(continuation) = config.recovery_continuation.as_ref() {
        if continuation.target_recovery_id != recovery_id {
            return Err("checkpoint continuation target identity changed before spawn".to_string());
        }
        store
            .link_continuation(
                gwt_core::recovery::RecoveryContinuationLink {
                    source_recovery_id: continuation.source_recovery_id.clone(),
                    target_recovery_id: recovery_id.clone(),
                    source_checkpoint_revision: continuation.source_checkpoint_revision,
                    definitive_reason: continuation.reason.clone(),
                    linked_at: chrono::Utc::now(),
                },
                format!("link-continuation:{}", session.id),
            )
            .map_err(|error| {
                format!("persist checkpoint continuation provenance before spawn: {error}")
            })?;
    }

    if let Some(root_id) = config.resume_session_id.as_deref() {
        store
            .bind_root(
                &recovery_id,
                gwt_core::recovery::ProviderRootBinding {
                    root_id: root_id.to_string(),
                    session_tree_id: None,
                    quality: gwt_core::recovery::BindingQuality::Preassigned,
                    bound_at: chrono::Utc::now(),
                },
                format!("preassign-root:{}", session.id),
            )
            .map_err(|error| format!("persist exact recovery binding before spawn: {error}"))?;
        session.provider_binding_quality =
            Some(gwt_agent::session::ProviderBindingQuality::Verified);
        session
            .observe_provider_root_role(gwt_agent::session::ProviderRootRole::Root)
            .map_err(|error| format!("record preassigned provider root role: {error}"))?;
    }
    Ok(())
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
    // SPEC-3170 FR-001: `claude_ultracode_supported()` spawns `claude --version`
    // (a Node CLI, ~100-200ms cold) and `claude_workflows_enabled()` reads a
    // settings file. They are stable for a session, so we resolve them once at
    // cache load time and reuse the booleans on every wizard open instead of
    // re-probing on the tao main event-loop thread (the measured open hitch).
    claude_ultracode_supported: bool,
    claude_workflows_enabled: bool,
}

impl LaunchWizardMemoryCache {
    pub(crate) fn load(sessions_dir: &Path) -> Self {
        // Resolve the Claude capability probes once here (see field docs). The
        // wizard open path then reads the cached booleans rather than spawning
        // `claude --version` per open.
        let claude_workflows_enabled = gwt_agent::claude_workflows_enabled();
        let claude_ultracode_supported = gwt_agent::claude_ultracode_supported();
        Self {
            sessions: Self::load_sessions(sessions_dir),
            agent_options: Self::load_agent_options(),
            claude_ultracode_supported,
            claude_workflows_enabled,
        }
    }

    #[cfg(test)]
    pub(crate) fn load_with_agent_options(
        sessions_dir: &Path,
        agent_options: Vec<gwt::AgentOption>,
    ) -> Self {
        Self::load_with_agent_options_and_capabilities(sessions_dir, agent_options, false, false)
    }

    #[cfg(test)]
    pub(crate) fn load_with_agent_options_and_capabilities(
        sessions_dir: &Path,
        agent_options: Vec<gwt::AgentOption>,
        claude_ultracode_supported: bool,
        claude_workflows_enabled: bool,
    ) -> Self {
        Self {
            sessions: Self::load_sessions(sessions_dir),
            agent_options,
            claude_ultracode_supported,
            claude_workflows_enabled,
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

    /// SPEC-3170 FR-001: cached `claude --version`-derived ultracode capability,
    /// resolved once at load time so wizard open never re-spawns the probe.
    pub(super) fn claude_ultracode_supported(&self) -> bool {
        self.claude_ultracode_supported
    }

    /// SPEC-3170 FR-001: cached Claude dynamic-workflows capability, resolved
    /// once at load time so wizard open never re-reads the settings file.
    pub(super) fn claude_workflows_enabled(&self) -> bool {
        self.claude_workflows_enabled
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
    let mut command = gwt_core::process::hidden_command(&config.command);
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

    /// SPEC-2359 US-83 / FR-444: update the live wizard's "open existing branch"
    /// picker candidates (computed off the UI thread after `fetch_origin`) and
    /// re-emit its state so the picker renders. Scoped to the matching
    /// `wizard_id` so a stale background result can't clobber a newer wizard.
    pub(crate) fn apply_launch_wizard_branch_candidates(
        &mut self,
        wizard_id: String,
        candidates: Vec<String>,
    ) -> Vec<OutboundEvent> {
        if let Some(session) = self.launch_wizard.as_mut() {
            if session.wizard_id == wizard_id {
                session.wizard.open_branch_candidates = candidates;
                return vec![self.launch_wizard_state_outbound()];
            }
        }
        Vec::new()
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
        let mut codex_bridge_route = take_staged_codex_bridge_route(&window_id);
        self.inflight_launches
            .retain(|_, (pending_window_id, _)| pending_window_id != &window_id);
        match result {
            Ok((
                mut process_launch,
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
                    self.mark_pending_auto_resume_attention(
                        &window_id,
                        "Recovery launch lost its target window before provider readiness",
                    );
                    return self.launch_error_events(
                        window_id,
                        "Window not found".to_string(),
                        launch_feedback_context.clone(),
                    );
                };
                let Some(tab) = self.tab(&address.tab_id) else {
                    self.mark_pending_auto_resume_attention(
                        &window_id,
                        "Recovery launch lost its project before provider readiness",
                    );
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
                    self.mark_pending_auto_resume_attention(
                        &window_id,
                        "Recovery launch lost its target window before provider readiness",
                    );
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
                if let Some(route) = codex_bridge_route.as_ref() {
                    route.install_auth_token(&mut process_launch.env);
                }
                if let Err(error) = gwt_agent::persist_recovery_launch_stage(
                    &self.sessions_dir,
                    &session_id_for_restore,
                    gwt_agent::session::RecoveryLaunchStage::SpawnRequested,
                ) {
                    self.mark_pending_auto_resume_attention(
                        &window_id,
                        "Recovery spawn request could not be persisted before PTY launch",
                    );
                    return self.launch_error_events(
                        window_id,
                        format!(
                            "persist SpawnRequested recovery boundary before PTY spawn: {error}"
                        ),
                        launch_feedback_context,
                    );
                }
                match self.spawn_process_window_with_console_kind(
                    &window_id,
                    geometry,
                    process_launch,
                    Some(gwt_core::process_console::ProcessKind::AgentBootstrap),
                ) {
                    Ok(()) => {
                        if let Err(error) = gwt_agent::persist_recovery_launch_stage(
                            &self.sessions_dir,
                            &session_id_for_restore,
                            gwt_agent::session::RecoveryLaunchStage::ProcessSpawned,
                        ) {
                            // A live provider without a durable ProcessSpawned
                            // boundary would be indistinguishable from a
                            // never-started launch after a crash. Stop it and
                            // keep the recovery visible for operator attention.
                            self.stop_window_runtime(&window_id);
                            self.mark_pending_auto_resume_attention(
                                &window_id,
                                "Recovery launch stage could not be persisted after PTY spawn",
                            );
                            return self.launch_error_events(
                                window_id,
                                format!(
                                    "persist ProcessSpawned recovery boundary after PTY spawn: {error}"
                                ),
                                launch_feedback_context,
                            );
                        }
                        if let Some(route) = codex_bridge_route.take() {
                            self.codex_bridge_routes.insert(window_id.clone(), route);
                        }
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
                        events.extend(Self::status_events(
                            window_id.clone(),
                            composed_status,
                            None,
                        ));
                        if let Some(issue_number) = launch_feedback_context
                            .as_ref()
                            .and_then(|context| context.issue_monitor_issue_number)
                        {
                            events.extend(
                                self.issue_monitor_launch_succeeded_events(
                                    issue_number,
                                    &window_id,
                                ),
                            );
                        }
                        events
                    }
                    Err(error) => {
                        self.mark_pending_auto_resume_attention(
                            &window_id,
                            "Recovery PTY failed before provider readiness",
                        );
                        self.launch_error_events(window_id, error, launch_feedback_context)
                    }
                }
            }
            Err(error) => {
                self.mark_pending_auto_resume_attention(
                    &window_id,
                    "Recovery launch failed before provider readiness",
                );
                self.launch_error_events(window_id, error, launch_feedback_context)
            }
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

    pub(crate) fn spawn_agent_window_with_feedback_at_geometry(
        &mut self,
        tab_id: &str,
        config: gwt_agent::LaunchConfig,
        geometry: WindowGeometry,
        workspace_resume_context: Option<WorkspaceResumeContext>,
        launch_feedback_context: LaunchFeedbackContext,
    ) -> Result<Vec<OutboundEvent>, String> {
        self.spawn_agent_window_with_placement(
            tab_id,
            config,
            AgentWindowPlacement::Exact(geometry),
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
        let events = self.focus_window_events(window_id, bounds);
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
        let agent_capability_issuer = self.agent_capability_issuer.clone();
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
                agent_capability_issuer,
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
        agent_capability_issuer: Option<AgentCapabilityIssuer>,
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
            // SPEC-3247 FR-002: select lane-specific coordination guidance from
            // the launch's ephemeral intake flag (same source as the
            // GWT_SESSION_KIND env export in prepare.rs), so an intake session
            // materializes curation-framed guidance without Work-state
            // instructions.
            let session_kind = gwt_skills::SessionKind::from_is_ephemeral(config.is_ephemeral);
            // SPEC-3248 (hooks v2 P0): materialize the lane file — the
            // deterministic source of truth hooks read via the lane registry —
            // from the authoritative launch-time lane (is_ephemeral). Best
            // effort: a write failure must not block the launch, and hooks fall
            // back to the execution default when the file is absent.
            let _ = gwt_skills::write_lane_file(
                &worktree_path,
                gwt_skills::LaneRegistry::for_session_kind(session_kind),
            );
            refresh_managed_gwt_assets_for_agent_with_codex_hook_discovery_mode(
                &worktree_path,
                &config.agent_id,
                codex_hook_discovery_mode,
                session_kind,
            )
            .map_err(|error| {
                // Attribute managed-asset failures to the worktree so the
                // operator sees which worktree's setup failed, not a bare
                // skill-writer error.
                format!(
                    "managed asset setup failed for worktree {}: {error}",
                    worktree_path.display()
                )
            })?;
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
                if let Some(version) = pin_host_codex_latest_runner(&mut config)? {
                    proxy.send(UserEvent::LaunchProgress {
                        window_id: window_id.clone(),
                        message: format!("Pinned Codex latest to {version}."),
                    });
                }
            }
            install_launch_gwt_bin_env(&mut config.env_vars, config.runtime_target)?;
            // SPEC-3248 P8a: derive the execution entrypoint from the raw
            // launch argv BEFORE the Windows host shell wrapper rewrites it
            // (the `$gwt-*` prompt token moves into an env var / embedded
            // script on wrapped launches).
            let execution_entrypoint = gwt::cli::execution_state::entrypoint_from_launch(
                &config.args,
                config.session_mode == gwt_agent::SessionMode::Resume,
            );
            let container_recovery_attachments =
                prepare_container_checkpoint_attachments(Path::new(&project_root), &mut config)?;
            let mut prepared_codex_bridge = prepare_codex_remote_bridge(&mut config)?;
            if let Some(attachments) = container_recovery_attachments {
                let Some(PreparedCodexBridge::Container(container)) =
                    prepared_codex_bridge.as_mut()
                else {
                    return Err(
                        "container recovery attachments require a Codex sidecar".to_string()
                    );
                };
                container.recovery_attachments = Some(attachments);
            }
            apply_windows_host_shell_wrapper(&mut config)?;

            let branch_name = config.branch.clone().unwrap_or_else(|| "work".to_string());

            let agent_id = config.agent_id.clone();
            let mut session = gwt_agent::Session::from_launch_config(
                &worktree_path,
                branch_name.clone(),
                &config,
            );
            session.project_state_root = Some(
                gwt_core::paths::normalize_windows_child_process_path(Path::new(&project_root)),
            );
            if session.session_mode == gwt_agent::SessionMode::Resume {
                session.agent_session_id = config.resume_session_id.clone();
            }
            session.update_status(gwt_agent::AgentStatus::Running);

            // Persist the complete launch ledger before publishing the
            // Recovery Record. If the process dies between these two durable
            // writes, startup's idempotent legacy importer can reconstruct the
            // missing record from this Session; the inverse ordering leaves a
            // Recovery Center candidate that cannot launch without a Session.
            session
                .save(&sessions_dir)
                .map_err(|error| error.to_string())?;

            persist_recovery_checkpoint_before_spawn(
                Path::new(&project_root),
                &worktree_path,
                &config,
                &mut session,
                None,
            )?;

            let session_id = session.id.clone();
            let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, &session_id);
            config.env_vars.insert(
                gwt_agent::GWT_SESSION_ID_ENV.to_string(),
                session_id.clone(),
            );
            if let Some(recovery_id) = session.recovery_id.clone() {
                config
                    .env_vars
                    .insert(gwt_agent::GWT_RECOVERY_ID_ENV.to_string(), recovery_id);
            }
            // SPEC-3247 FR-001: export the session-kind signal into the spawned
            // agent's env HERE, in the production spawn path (the `prepare.rs`
            // helper is an alternate path with no production callers). Derived
            // from the same `config.is_ephemeral` as the materialization
            // guidance kind above, so the runtime signal and the materialized
            // guidance never disagree. Absent/unknown decodes to Execution
            // downstream (FR-004).
            let session_kind_env = gwt_skills::SessionKind::from_is_ephemeral(config.is_ephemeral)
                .as_env_str()
                .to_string();
            config.env_vars.insert(
                gwt_skills::GWT_SESSION_KIND_ENV.to_string(),
                session_kind_env,
            );
            config.env_vars.insert(
                gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV.to_string(),
                runtime_path.display().to_string(),
            );
            let runtime_target = config.runtime_target;
            let container_runtime_binary = docker_binary_for_launch();
            install_agent_capability_env(
                &mut config.env_vars,
                agent_capability_issuer.as_ref(),
                Path::new(&project_root),
                &session_id,
                runtime_target,
                &container_runtime_binary,
            )?;
            config
                .env_vars
                .entry("COLORTERM".to_string())
                .or_insert_with(|| "truecolor".to_string());
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

            // SPEC-3248 P8a (T-107): materialize the Execution Control Record
            // for linked-owner execution launches — SPEC and plain Issue alike
            // — before prompt injection (the prompt rides the argv; the
            // process spawns after this closure returns). Best-effort like
            // the lane file: a write failure must not block the launch, and
            // the Stop gate fails open when the record is absent. Intake
            // (ephemeral) sessions own no execution lifecycle, and subordinate
            // launches (independent review dispatch) are opted out.
            if !config.is_ephemeral && !config.suppress_execution_control {
                if let Some(owner_number) = config.linked_issue_number {
                    let owner_kind =
                        gwt::cli::execution_state::detect_owner_kind(&worktree_path, owner_number);
                    if let Err(error) = gwt::cli::execution_state::materialize_at_launch(
                        &worktree_path,
                        owner_kind,
                        owner_number,
                        &session_id,
                        &execution_entrypoint,
                        config.session_mode == gwt_agent::SessionMode::Resume,
                    ) {
                        tracing::warn!(
                            ?error,
                            owner_number,
                            "execution control record materialization failed"
                        );
                    }
                }
            }
            let codex_bridge_route = if let Some(prepared) = prepared_codex_bridge.as_mut() {
                // Runtime/session/hook environment is finalized only after the
                // recovery checkpoint. Mirror it into app-server without the
                // bridge bearer token, which stays solely in the route lease
                // until the main thread is about to spawn the remote TUI.
                let recovery_id = session.recovery_id.clone().ok_or_else(|| {
                    "Codex bridge session is missing recovery identity".to_string()
                })?;
                let durability: Arc<dyn gwt::codex_bridge::CodexDurabilitySink> =
                    Arc::new(gwt::codex_bridge::RecoveryCodexDurability::new(
                        sessions_dir.clone(),
                        session_id.clone(),
                        recovery_id,
                        gwt_core::paths::gwt_project_dir_for_repo_path(Path::new(&project_root)),
                        worktree_path.clone(),
                    ));
                let ready_proxy = proxy.clone();
                let failure_proxy = proxy.clone();
                let ready_window_id = window_id.clone();
                let failure_window_id = window_id.clone();
                let ready_session_id = session_id.clone();
                let failure_session_id = session_id.clone();
                let on_ready: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
                    ready_proxy.send(UserEvent::CodexBridgeReady {
                        window_id: ready_window_id.clone(),
                        session_id: ready_session_id.clone(),
                    });
                });
                let on_failure: Arc<dyn Fn(gwt::codex_bridge::CodexBridgeFailure) + Send + Sync> =
                    Arc::new(move |failure| {
                        failure_proxy.send(UserEvent::CodexBridgeFailure {
                            window_id: failure_window_id.clone(),
                            session_id: failure_session_id.clone(),
                            failure,
                        });
                    });
                Some(match prepared {
                    PreparedCodexBridge::Host(app_server) => {
                        app_server.env = config.env_vars.clone();
                        app_server
                            .env
                            .remove(gwt::codex_bridge::CODEX_REMOTE_AUTH_TOKEN_ENV);
                        app_server.remove_env = config.remove_env.clone();
                        if !app_server
                            .remove_env
                            .iter()
                            .any(|key| key == gwt::codex_bridge::CODEX_REMOTE_AUTH_TOKEN_ENV)
                        {
                            app_server
                                .remove_env
                                .push(gwt::codex_bridge::CODEX_REMOTE_AUTH_TOKEN_ENV.to_string());
                        }
                        app_server.cwd = config.working_dir.clone();
                        gwt::codex_bridge::CodexLaunchBridgeLease::Host(
                            gwt::codex_bridge::register_codex_bridge_route(
                                gwt::codex_bridge::CodexBridgeRouteConfig {
                                    app_server: app_server.clone(),
                                    expected_resume_id: config.resume_session_id.clone(),
                                    durability,
                                    on_ready,
                                    on_failure,
                                },
                            )
                            .map_err(|error| error.to_string())?,
                        )
                    }
                    PreparedCodexBridge::Container(container) => {
                        container.app_server.env = config.env_vars.clone();
                        container
                            .app_server
                            .env
                            .remove(gwt::codex_bridge::CODEX_REMOTE_AUTH_TOKEN_ENV);
                        container.app_server.remove_env = config.remove_env.clone();
                        if !container
                            .app_server
                            .remove_env
                            .iter()
                            .any(|key| key == gwt::codex_bridge::CODEX_REMOTE_AUTH_TOKEN_ENV)
                        {
                            container
                                .app_server
                                .remove_env
                                .push(gwt::codex_bridge::CODEX_REMOTE_AUTH_TOKEN_ENV.to_string());
                        }
                        let lease = gwt::codex_bridge::start_container_codex_bridge(
                            gwt::codex_bridge::CodexContainerBridgeConfig {
                                compose_files: container.compose_files.clone(),
                                service: container.service.clone(),
                                working_dir: Some(container.container_cwd.clone()),
                                app_server: container.app_server.clone(),
                                expected_resume_id: config.resume_session_id.clone(),
                                recovery_attachments: container.recovery_attachments.take(),
                                durability,
                                on_ready,
                                on_failure,
                            },
                        )?;
                        config.args.splice(
                            container.runner_prefix_len..container.runner_prefix_len,
                            [
                                "--remote".to_string(),
                                lease.endpoint().to_string(),
                                "--remote-auth-token-env".to_string(),
                                gwt::codex_bridge::CODEX_REMOTE_AUTH_TOKEN_ENV.to_string(),
                            ],
                        );
                        // Value stays empty until the main-thread PTY handoff;
                        // only the name is needed while Compose argv is built.
                        config.env_vars.insert(
                            gwt::codex_bridge::CODEX_REMOTE_AUTH_TOKEN_ENV.to_string(),
                            String::new(),
                        );
                        gwt::codex_bridge::CodexLaunchBridgeLease::Container(lease)
                    }
                })
            } else {
                None
            };

            finalize_docker_agent_launch_config(Path::new(&project_root), &mut config)?;

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
                codex_bridge_route,
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
                codex_bridge_route,
            )) => {
                if let Some(route) = codex_bridge_route {
                    stage_codex_bridge_route(window_id.clone(), route);
                }
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
    /// - Otherwise (a Paused Work with no running agent), only the terminal close
    ///   is recorded in Work history. Worktree, branch, and PR are unchanged;
    ///   worktree deletion remains an independent vetted cleanup operation. A
    ///   `done` close records a Done event and `discarded` records a Discard
    ///   event. Re-closing an already-closed Work is a noop.
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

        let decision = gwt_core::workspace_projection::decide_work_close(has_live_agent, None);

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
            gwt_core::workspace_projection::WorkCloseDecision::RecordOnly => {
                // Work close records lifecycle only. Worktree deletion is an
                // independent cleanup operation with its own safety gates.
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

    pub(crate) fn mark_agent_session_stopped(&mut self, window_id: &str) {
        let Some(session) = self.active_agent_sessions.remove(window_id) else {
            return;
        };
        let recovery_retained = self.retain_stopped_recovery(&session);
        // SPEC-3214 (FR-002 / T-005 / T-007): an ephemeral intake session runs
        // in a throwaway detached `.intake-*` worktree and produces NO Work
        // identity. On session end, remove the worktree when clean; keep it
        // when dirty so uncommitted work is never lost. Skip the Paused-Work /
        // projection persistence entirely.
        if self.is_ephemeral_intake_session(&session) {
            if !recovery_retained {
                self.finalize_ephemeral_intake_worktree(&session);
                let _ = gwt_agent::persist_session_status(
                    &self.sessions_dir,
                    &session.session_id,
                    gwt_agent::AgentStatus::Stopped,
                );
            }
            self.launch_wizard_cache.mark_stopped(&session.session_id);
            return;
        }
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
        if !recovery_retained {
            let _ = gwt_agent::persist_session_status(
                &self.sessions_dir,
                &session.session_id,
                gwt_agent::AgentStatus::Stopped,
            );
        }
        self.launch_wizard_cache.mark_stopped(&session.session_id);
    }

    /// Persist provider-stop evidence before projecting any managed Recovery
    /// Session as stopped. Intake additionally uses the return value to retain
    /// its disposable worktree; Execution uses it to preserve resumability.
    fn retain_stopped_recovery(&self, active: &ActiveAgentSession) -> bool {
        let session_path = self
            .sessions_dir
            .join(format!("{}.toml", active.session_id));
        let source = match gwt_agent::Session::load_and_migrate(&session_path) {
            Ok(source) => source,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return false,
            Err(error) => {
                // A running recovery launch always materializes its Session
                // before spawn. If that ledger is unreadable, cleanup cannot
                // prove the Recovery is unowned and must fail closed.
                tracing::warn!(
                    session_id = %active.session_id,
                    error = %error,
                    "keeping stopped Recovery because its Session ledger is unavailable"
                );
                return true;
            }
        };
        let Some(recovery_id) = source.recovery_id.as_deref() else {
            return false;
        };
        let project_root = source
            .project_state_root
            .as_deref()
            .unwrap_or(&source.worktree_path);
        let store = gwt_core::recovery::RecoveryStore::for_project_dir(
            gwt_core::paths::gwt_project_dir_for_repo_path(project_root),
        );
        let record = match store.load(recovery_id) {
            Ok(Some(record)) => record,
            Ok(None) => return false,
            Err(error) => {
                tracing::warn!(
                    recovery_id,
                    error = %error,
                    "keeping stopped Recovery because RecoveryStore inventory failed"
                );
                return true;
            }
        };
        let expected_kind = match (source.session_kind, source.is_ephemeral) {
            (Some(gwt_skills::SessionKind::Intake), true) => {
                Some(gwt_core::recovery::RecoverySessionKind::Intake)
            }
            (Some(gwt_skills::SessionKind::Execution), false) => {
                Some(gwt_core::recovery::RecoverySessionKind::Execution)
            }
            _ => None,
        };
        if record.session_id != source.id || expected_kind != Some(record.session_kind) {
            tracing::warn!(
                recovery_id,
                session_id = %source.id,
                "keeping stopped Recovery because its Session identity does not match"
            );
            return true;
        }
        if matches!(
            record.lifecycle,
            gwt_core::recovery::RecoveryLifecycle::Resolved
                | gwt_core::recovery::RecoveryLifecycle::Discarded
        ) {
            return false;
        }

        let stopped_at = chrono::Utc::now();
        let session_kind = match record.session_kind {
            gwt_core::recovery::RecoverySessionKind::Intake => "Intake",
            gwt_core::recovery::RecoverySessionKind::Execution => "Execution",
        };
        let stopped_reason = format!("gwt observed the {session_kind} provider process stop");
        let proof_required = record.launch_stage
            >= gwt_core::recovery::RecoveryLaunchStage::SpawnRequested
            && !record.launch_stage.is_terminal()
            && matches!(
                record.lifecycle,
                gwt_core::recovery::RecoveryLifecycle::Launching
                    | gwt_core::recovery::RecoveryLifecycle::Running
                    | gwt_core::recovery::RecoveryLifecycle::Interrupted
            )
            && record.recovery_lease.is_none();
        let interruption = if proof_required {
            store.interrupt_after_supervisor_stop(
                recovery_id,
                record.generation,
                &source.id,
                stopped_at,
                &stopped_reason,
                format!(
                    "runtime-supervisor-stop-v1:{}:{}",
                    source.id, record.generation
                ),
            )
        } else if record.launch_stage < gwt_core::recovery::RecoveryLaunchStage::Ready
            && !matches!(
                record.lifecycle,
                gwt_core::recovery::RecoveryLifecycle::Interrupted
                    | gwt_core::recovery::RecoveryLifecycle::Attention
                    | gwt_core::recovery::RecoveryLifecycle::Recovering
            )
        {
            store.set_lifecycle(
                recovery_id,
                gwt_core::recovery::RecoveryLifecycle::Interrupted,
                Some(stopped_reason),
                format!(
                    "runtime-provider-stop-v1:{}:{}",
                    source.id, record.generation
                ),
            )
        } else {
            Ok(record)
        };
        if let Err(error) = interruption {
            // Never turn a ledger race or storage failure into destructive
            // worktree cleanup. Startup reconciliation can retry the proof.
            tracing::warn!(
                recovery_id,
                error = %error,
                "failed to persist stopped Recovery; retaining its durable state"
            );
        }
        if let Err(error) = gwt_agent::update_session(&self.sessions_dir, &source.id, |session| {
            session.update_status(gwt_agent::AgentStatus::Interrupted);
            Ok(())
        }) {
            tracing::warn!(
                session_id = %source.id,
                error = %error,
                "failed to project stopped Recovery into Session state"
            );
        }
        true
    }

    /// SPEC-3214 (codex #3235 review): whether a stopped session is an
    /// ephemeral intake session. The `.intake-*` basename alone is not enough —
    /// a normal branch worktree a user happens to name `.intake-*` must keep its
    /// Paused-Work / resume behavior. The definitive signal is that the intake
    /// worktree is DETACHED (branchless), which only `create_detached` produces.
    /// A worktree that is already gone is treated as ephemeral (it was reaped).
    pub(super) fn is_ephemeral_intake_session(&self, session: &ActiveAgentSession) -> bool {
        if !is_ephemeral_intake_worktree(&session.worktree_path) {
            return false;
        }
        let Some(main_repo_path) = self
            .tab(&session.tab_id)
            .map(|tab| tab.project_root.clone())
            .and_then(|root| gwt_git::worktree::main_worktree_root(&root).ok())
        else {
            return !session.worktree_path.exists();
        };
        match gwt_git::WorktreeManager::new(&main_repo_path).list() {
            Ok(worktrees) => worktrees
                .iter()
                .find(|info| same_worktree_path(&info.path, &session.worktree_path))
                // On a branch → a real worktree, not intake. Detached → intake.
                .is_none_or(|info| info.branch.is_none()),
            // Cannot enumerate: fall back to "gone means it was ephemeral".
            Err(_) => !session.worktree_path.exists(),
        }
    }

    /// SPEC-3214 (FR-002): tear down an ephemeral intake worktree when its
    /// session ends. A clean worktree is force-removed; a dirty one is kept and
    /// logged so uncommitted work is never destroyed (the user-facing retention
    /// notice ships with the intake UI in a later phase).
    fn finalize_ephemeral_intake_worktree(&self, session: &ActiveAgentSession) {
        let worktree_path = session.worktree_path.as_path();
        let main_repo_path = self
            .tab(&session.tab_id)
            .map(|tab| tab.project_root.clone())
            .and_then(|root| gwt_git::worktree::main_worktree_root(&root).ok())
            .unwrap_or_else(|| worktree_path.to_path_buf());
        let manager = gwt_git::WorktreeManager::new(&main_repo_path);

        match manager.ephemeral_worktree_has_local_work_with(worktree_path, |entry| {
            intake_hook_config_is_disposable(worktree_path, entry)
        }) {
            Ok(true) => {
                tracing::warn!(
                    worktree_path = %worktree_path.display(),
                    "ephemeral intake worktree has local work (changes, ignored files, or commits); keeping it so nothing is lost"
                );
                return;
            }
            Ok(false) => {}
            Err(error) => {
                // Fail closed: if we cannot prove the worktree is empty, keep it.
                tracing::warn!(
                    worktree_path = %worktree_path.display(),
                    error = %error,
                    "could not determine intake worktree cleanliness; keeping it"
                );
                return;
            }
        }

        if let Err(error) = manager.remove_force(worktree_path) {
            tracing::warn!(
                worktree_path = %worktree_path.display(),
                error = %error,
                "failed to remove clean ephemeral intake worktree"
            );
        }
    }

    /// Compatibility hook for the runtime-status path. Current intake cleanup
    /// runs synchronously in `mark_agent_session_stopped()` after classifying
    /// the session by detached `.intake-*` worktree state, so there is no
    /// deferred queue to drain here.
    pub(crate) fn take_ephemeral_worktree_cleanup_events(&mut self) -> Vec<OutboundEvent> {
        Vec::new()
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
        let agent_summary = projection
            .as_ref()
            .and_then(|projection| projection.latest_agent_for_session(session_id));
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

#[cfg(test)]
mod process_launch_privacy_tests {
    use super::*;

    #[test]
    fn process_launch_debug_redacts_agent_capability_and_session_identity() {
        const TOKEN_SENTINEL: &str = "agent-capability-secret-sentinel";
        const SESSION_SENTINEL: &str = "session-secret-sentinel";
        let launch = ProcessLaunch {
            command: "docker".to_string(),
            args: vec![
                format!("{}={TOKEN_SENTINEL}", gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV),
                format!("{}={SESSION_SENTINEL}", gwt_agent::GWT_SESSION_ID_ENV),
            ],
            env: HashMap::from([
                (
                    gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV.to_string(),
                    TOKEN_SENTINEL.to_string(),
                ),
                (
                    gwt_agent::GWT_SESSION_ID_ENV.to_string(),
                    SESSION_SENTINEL.to_string(),
                ),
            ]),
            remove_env: Vec::new(),
            cwd: None,
        };

        let debug = format!("{launch:?}");
        assert!(!debug.contains(TOKEN_SENTINEL), "token leaked: {debug}");
        assert!(!debug.contains(SESSION_SENTINEL), "session leaked: {debug}");
        assert!(debug.contains("<redacted>"));
    }

    #[test]
    fn launch_env_receives_one_capability_bound_to_each_session() {
        let projects = tempfile::tempdir().expect("project roots");
        let project_a = projects.path().join("project-a");
        let project_b = projects.path().join("project-b");
        std::fs::create_dir_all(&project_a).expect("project A");
        std::fs::create_dir_all(&project_b).expect("project B");
        let issuer = AgentCapabilityIssuer::for_test("http://127.0.0.1:45123/internal/hook-live");
        let mut session_a = HashMap::new();
        let mut session_b = HashMap::new();

        install_agent_capability_env(
            &mut session_a,
            Some(&issuer),
            &project_a,
            "session-a",
            gwt_agent::LaunchRuntimeTarget::Host,
            "docker",
        )
        .expect("session A env");
        install_agent_capability_env(
            &mut session_b,
            Some(&issuer),
            &project_b,
            "session-b",
            gwt_agent::LaunchRuntimeTarget::Host,
            "docker",
        )
        .expect("session B env");

        assert_eq!(
            session_a.get(gwt_agent::GWT_HOOK_FORWARD_URL_ENV),
            session_b.get(gwt_agent::GWT_HOOK_FORWARD_URL_ENV)
        );
        assert_ne!(
            session_a.get(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV),
            session_b.get(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV)
        );
        assert!(session_a
            .get(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV)
            .is_some_and(|token| !token.is_empty()));
        assert!(session_b
            .get(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV)
            .is_some_and(|token| !token.is_empty()));
    }

    #[test]
    fn docker_launch_env_receives_the_container_host_gateway_url() {
        let project = tempfile::tempdir().expect("project root");
        let issuer = AgentCapabilityIssuer::for_test(
            "http://127.0.0.1:45123/internal/hook-live?generation=7",
        );
        let mut env = HashMap::new();

        install_agent_capability_env(
            &mut env,
            Some(&issuer),
            project.path(),
            "session-docker",
            gwt_agent::LaunchRuntimeTarget::Docker,
            "docker",
        )
        .expect("Docker agent capability env");

        assert_eq!(
            env.get(gwt_agent::GWT_HOOK_FORWARD_URL_ENV)
                .map(String::as_str),
            Some("http://host.docker.internal:45123/internal/hook-live?generation=7")
        );
        assert!(env
            .get(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV)
            .is_some_and(|token| !token.is_empty()));
    }
}

#[cfg(test)]
mod fr001_capability_cache_tests {
    use super::LaunchWizardMemoryCache;

    // SPEC-3170 FR-001: the Claude capability probes are resolved once at cache
    // load time and the getters return the stored booleans verbatim, so opening
    // the Launch wizard reads cached values instead of re-spawning
    // `claude --version` on the tao main event-loop thread.
    #[test]
    fn caches_claude_capabilities_for_reuse_without_reprobe() {
        let dir = tempfile::tempdir().expect("tempdir");

        let on = LaunchWizardMemoryCache::load_with_agent_options_and_capabilities(
            dir.path(),
            Vec::new(),
            true,
            true,
        );
        assert!(on.claude_ultracode_supported());
        assert!(on.claude_workflows_enabled());

        let off = LaunchWizardMemoryCache::load_with_agent_options_and_capabilities(
            dir.path(),
            Vec::new(),
            false,
            false,
        );
        assert!(!off.claude_ultracode_supported());
        assert!(!off.claude_workflows_enabled());
    }
}

#[cfg(test)]
mod recovery_checkpoint_tests {
    use std::path::Path;

    use super::{
        launch_checkpoint_continuation_config, launch_config_from_persisted_session,
        persist_recovery_checkpoint_before_spawn,
    };

    fn run_git(repo: &Path, args: &[&str]) {
        let status = gwt_core::process::hidden_command("git")
            .args(args)
            .current_dir(repo)
            .status()
            .expect("run git");
        assert!(status.success(), "git {args:?}");
    }

    #[test]
    fn production_launch_persists_recovery_before_provider_spawn() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("create repo");
        run_git(&repo, &["init", "-q", "-b", "develop"]);
        run_git(&repo, &["config", "user.name", "Codex"]);
        run_git(&repo, &["config", "user.email", "codex@example.com"]);
        std::fs::write(repo.join("README.md"), "recovery\n").expect("write fixture");
        run_git(&repo, &["add", "README.md"]);
        run_git(&repo, &["commit", "-qm", "init"]);

        let config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
            .working_dir(&repo)
            .ephemeral(Some("develop".to_string()))
            .initial_prompt("Investigate interrupted Intake")
            .extra_arg("Investigate interrupted Intake")
            .build();
        let mut session = gwt_agent::Session::from_launch_config(&repo, "work", &config);
        let project_dir = temp.path().join("project-store");

        persist_recovery_checkpoint_before_spawn(
            &repo,
            &repo,
            &config,
            &mut session,
            Some(&project_dir),
        )
        .expect("persist pre-spawn checkpoint");

        let recovery_id = session.recovery_id.as_deref().expect("recovery id");
        let record = gwt_core::recovery::RecoveryStore::for_project_dir(&project_dir)
            .load(recovery_id)
            .expect("load recovery")
            .expect("record");
        assert_eq!(record.initial_prompt, "Investigate interrupted Intake");
        assert_eq!(
            session.recovery_launch_stage,
            Some(gwt_agent::session::RecoveryLaunchStage::WorktreeMaterialized)
        );
        assert_eq!(
            session.launch_base_oid.as_deref(),
            Some(record.launch_base_oid.as_str())
        );
        assert_eq!(
            gwt_git::recovery::verify_recovery_base_pin(
                &repo,
                recovery_id,
                &record.launch_base_oid,
            )
            .expect("pre-spawn Intake base pin"),
            gwt_git::recovery::recovery_base_ref_name(recovery_id).unwrap()
        );
    }

    #[test]
    fn persisted_intake_resume_keeps_ephemeral_lane_and_authoritative_base() {
        let mut session =
            gwt_agent::Session::new("/tmp/.intake-4", "work", gwt_agent::AgentId::Codex);
        session.is_ephemeral = true;
        session.session_kind = Some(gwt_skills::SessionKind::Intake);
        session.ephemeral_base_ref = Some("origin/develop".to_string());
        session.session_mode = gwt_agent::SessionMode::Resume;
        session.agent_session_id = Some("019b-root".to_string());

        let config = launch_config_from_persisted_session(&session);

        assert!(config.is_ephemeral);
        assert_eq!(config.ephemeral_base_ref.as_deref(), Some("origin/develop"));
        assert!(config.branch.is_none(), "Intake restore remains branchless");
        assert_eq!(
            config.working_dir.as_deref(),
            Some(Path::new("/tmp/.intake-4"))
        );
        assert_eq!(config.resume_session_id.as_deref(), Some("019b-root"));
    }

    #[test]
    fn checkpoint_continuation_starts_normal_session_with_prompt() {
        let mut session =
            gwt_agent::Session::new("/tmp/.intake-4", "", gwt_agent::AgentId::ClaudeCode);
        session.is_ephemeral = true;
        session.session_kind = Some(gwt_skills::SessionKind::Intake);
        session.ephemeral_base_ref = Some("origin/develop".to_string());
        session.session_mode = gwt_agent::SessionMode::Resume;
        session.agent_session_id = Some("stale-provider-root".to_string());
        let prompt = "Continue the interrupted Intake from its durable checkpoint.";

        let config = launch_checkpoint_continuation_config(&session, prompt);

        assert_eq!(config.session_mode, gwt_agent::SessionMode::Normal);
        assert_eq!(config.resume_session_id, None);
        assert_eq!(config.initial_prompt.as_deref(), Some(prompt));
        assert_eq!(config.args.last().map(String::as_str), Some(prompt));
        assert!(
            !config.args.iter().any(|arg| arg == "stale-provider-root"),
            "checkpoint continuation must not retain exact-resume argv"
        );
        assert!(config.is_ephemeral);
        assert_eq!(config.ephemeral_base_ref.as_deref(), Some("origin/develop"));
    }

    #[test]
    fn checkpoint_continuation_persists_bidirectional_provenance_before_spawn() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("create repo");
        run_git(&repo, &["init", "-q", "-b", "develop"]);
        run_git(&repo, &["config", "user.name", "Codex"]);
        run_git(&repo, &["config", "user.email", "codex@example.com"]);
        std::fs::write(repo.join("README.md"), "recovery\n").expect("write fixture");
        run_git(&repo, &["add", "README.md"]);
        run_git(&repo, &["commit", "-qm", "init"]);
        let project_dir = temp.path().join("project-store");
        let store = gwt_core::recovery::RecoveryStore::for_project_dir(&project_dir);
        store
            .create(
                gwt_core::recovery::CreateRecovery {
                    recovery_id: "source-recovery".to_string(),
                    session_id: "source-session".to_string(),
                    repo_id: gwt_core::paths::project_scope_hash(&repo).to_string(),
                    session_kind: gwt_core::recovery::RecoverySessionKind::Intake,
                    worktree_path: repo.clone(),
                    launch_base_ref: Some("develop".to_string()),
                    launch_base_oid: "1".repeat(40),
                    launch_head_oid: "1".repeat(40),
                    provider: "claude".to_string(),
                    model: None,
                    runtime: "host".to_string(),
                    initial_prompt: "Investigate".to_string(),
                    created_at: chrono::Utc::now(),
                },
                "create-source",
            )
            .expect("create source recovery");
        let reason = "Exact provider resume rejected: No conversation found with session ID";
        store
            .prepare_successor(
                gwt_core::recovery::RecoveryContinuationLink {
                    source_recovery_id: "source-recovery".to_string(),
                    target_recovery_id: "target-recovery".to_string(),
                    source_checkpoint_revision: 0,
                    definitive_reason: reason.to_string(),
                    linked_at: chrono::Utc::now(),
                },
                "prepare-target-recovery",
            )
            .expect("prepare target identity before materialization");
        let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::ClaudeCode)
            .working_dir(&repo)
            .ephemeral(Some("develop".to_string()))
            .initial_prompt("Continue durable checkpoint")
            .extra_arg("Continue durable checkpoint")
            .build();
        config.recovery_continuation = Some(gwt_agent::RecoveryContinuationHandoff {
            source_session_id: "source-session".to_string(),
            source_recovery_id: "source-recovery".to_string(),
            target_recovery_id: "target-recovery".to_string(),
            source_checkpoint_revision: 0,
            reason: reason.to_string(),
            inherit_checkpoint: true,
        });
        let mut session = gwt_agent::Session::from_launch_config(&repo, "", &config);
        assert_eq!(session.recovery_id.as_deref(), Some("target-recovery"));

        persist_recovery_checkpoint_before_spawn(
            &repo,
            &repo,
            &config,
            &mut session,
            Some(&project_dir),
        )
        .expect("persist linked continuation");

        let mut retry_config = config.clone();
        retry_config.recovery_retry_session_id = Some(session.id.clone());
        retry_config.recovery_retry_created_at = Some(session.created_at);
        let mut retry_session = gwt_agent::Session::from_launch_config(&repo, "", &retry_config);
        assert_eq!(retry_session.id, session.id);
        assert_eq!(retry_session.recovery_id, session.recovery_id);
        persist_recovery_checkpoint_before_spawn(
            &repo,
            &repo,
            &retry_config,
            &mut retry_session,
            Some(&project_dir),
        )
        .expect("retry same pre-spawn successor identity idempotently");

        let source = store.load("source-recovery").unwrap().unwrap();
        let target = store.load("target-recovery").unwrap().unwrap();
        let link = target.continuation_source.expect("target source link");
        assert_eq!(link.source_recovery_id, "source-recovery");
        assert_eq!(link.target_recovery_id, "target-recovery");
        assert_eq!(link.source_checkpoint_revision, 0);
        assert_eq!(link.definitive_reason, reason);
        assert_eq!(source.continuation_targets, vec![link]);
        assert_eq!(store.list().unwrap().len(), 2);
    }
}

#[cfg(test)]
mod codex_remote_bridge_launch_tests {
    use super::{
        prepare_codex_remote_bridge, prepare_container_checkpoint_attachments_from_store,
        prepare_host_codex_remote_bridge, PreparedCodexBridge,
    };

    #[test]
    fn direct_codex_uses_same_runner_for_remote_tui_and_app_server() {
        let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex).build();
        config.command = "codex".to_string();
        config.args = vec![
            "--no-alt-screen".to_string(),
            "resume".to_string(),
            "root-1".to_string(),
        ];

        let app_server = prepare_host_codex_remote_bridge(&mut config)
            .expect("prepare bridge")
            .expect("Codex Host is bridged");

        assert_eq!(app_server.command, "codex");
        assert_eq!(app_server.args, ["app-server", "--listen", "stdio://"]);
        assert_eq!(config.args[0], "--remote");
        assert!(config.args[1].starts_with("ws://127.0.0.1:"));
        assert_eq!(config.args[2], "--remote-auth-token-env");
        assert_eq!(
            config.args[3],
            gwt::codex_bridge::CODEX_REMOTE_AUTH_TOKEN_ENV
        );
        assert!(app_server
            .remove_env
            .iter()
            .any(|key| key == gwt::codex_bridge::CODEX_REMOTE_AUTH_TOKEN_ENV));
        assert!(config.args[4..].starts_with(&["--no-alt-screen".to_string()]));
    }

    #[test]
    fn version_pinned_npx_runner_is_identical_for_tui_and_app_server() {
        let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex).build();
        config.command = "npx.cmd".to_string();
        config.args = vec![
            "--yes".to_string(),
            "@openai/codex@0.144.5".to_string(),
            "--no-alt-screen".to_string(),
        ];

        let app_server = prepare_host_codex_remote_bridge(&mut config)
            .expect("prepare bridge")
            .expect("Codex Host is bridged");

        assert_eq!(app_server.command, "npx.cmd");
        assert_eq!(
            app_server.args,
            [
                "--yes",
                "@openai/codex@0.144.5",
                "app-server",
                "--listen",
                "stdio://"
            ]
        );
        assert_eq!(&config.args[..2], ["--yes", "@openai/codex@0.144.5"]);
        assert_eq!(config.args[2], "--remote");
    }

    #[test]
    fn docker_and_non_codex_launches_remain_unbridged() {
        let mut docker = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
            .runtime_target(gwt_agent::LaunchRuntimeTarget::Docker)
            .build();
        let docker_args = docker.args.clone();
        assert!(prepare_host_codex_remote_bridge(&mut docker)
            .expect("Docker is a supported no-op")
            .is_none());
        assert_eq!(docker.args, docker_args);

        let mut claude = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::ClaudeCode).build();
        let claude_args = claude.args.clone();
        assert!(prepare_host_codex_remote_bridge(&mut claude)
            .expect("Claude is a supported no-op")
            .is_none());
        assert_eq!(claude.args, claude_args);
    }

    #[test]
    fn docker_codex_prepares_container_local_sidecar_without_host_endpoint() {
        let repo = tempfile::tempdir().expect("repo");
        std::fs::write(
            repo.path().join("docker-compose.yml"),
            "services:\n  app:\n    image: node:22\n    working_dir: /workspace\n    volumes:\n      - .:/workspace\n",
        )
        .expect("compose");
        let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
            .runtime_target(gwt_agent::LaunchRuntimeTarget::Docker)
            .working_dir(repo.path())
            .docker_service("app")
            .build();
        config.command = "npx".to_string();
        config.args = vec![
            "--yes".to_string(),
            "@openai/codex@0.144.5".to_string(),
            "--no-alt-screen".to_string(),
        ];

        let prepared = prepare_codex_remote_bridge(&mut config)
            .expect("prepare")
            .expect("Docker Codex bridge");
        let PreparedCodexBridge::Container(container) = prepared else {
            panic!("expected container sidecar");
        };

        assert_eq!(container.service, "app");
        assert_eq!(container.container_cwd, "/workspace");
        assert_eq!(container.runner_prefix_len, 2);
        assert_eq!(container.app_server.command, "npx");
        assert_eq!(
            container.app_server.args,
            [
                "--yes",
                "@openai/codex@0.144.5",
                "app-server",
                "--listen",
                "stdio://"
            ]
        );
        assert!(container
            .app_server
            .remove_env
            .iter()
            .any(|key| key == gwt::codex_bridge::CODEX_REMOTE_AUTH_TOKEN_ENV));
        assert!(config.args.iter().all(|arg| !arg.starts_with("ws://")));
    }

    #[test]
    fn docker_checkpoint_prompt_uses_only_sidecar_container_attachment_paths() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = gwt_core::recovery::RecoveryStore::new(temp.path().join("recovery-store"));
        let attachment = store
            .copy_attachment_bytes("design evidence.png", b"durable container evidence")
            .expect("copy attachment");
        store
            .create(
                gwt_core::recovery::CreateRecovery {
                    recovery_id: "source-container-recovery".to_string(),
                    session_id: "source-container-session".to_string(),
                    repo_id: "container-repo".to_string(),
                    session_kind: gwt_core::recovery::RecoverySessionKind::Intake,
                    worktree_path: temp.path().to_path_buf(),
                    launch_base_ref: Some("develop".to_string()),
                    launch_base_oid: "1".repeat(40),
                    launch_head_oid: "1".repeat(40),
                    provider: "codex".to_string(),
                    model: None,
                    runtime: "docker".to_string(),
                    initial_prompt: "Review the image".to_string(),
                    created_at: chrono::Utc::now(),
                },
                "create-container-source",
            )
            .expect("create source");
        store
            .bind_root(
                "source-container-recovery",
                gwt_core::recovery::ProviderRootBinding {
                    root_id: "source-container-root".to_string(),
                    session_tree_id: None,
                    quality: gwt_core::recovery::BindingQuality::Verified,
                    bound_at: chrono::Utc::now(),
                },
                "bind-container-source",
            )
            .expect("bind source");
        let source = store
            .replace_checkpoint(
                "source-container-recovery",
                "source-container-root",
                0,
                gwt_core::recovery::SemanticCheckpoint {
                    summary: "Continue reviewing the supplied design evidence.".to_string(),
                    attachment_refs: vec![attachment.clone()],
                    ..gwt_core::recovery::SemanticCheckpoint::default()
                },
                "checkpoint-container-source",
            )
            .expect("checkpoint source");

        let seed = "Continue from the durable checkpoint.";
        let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
            .runtime_target(gwt_agent::LaunchRuntimeTarget::Docker)
            .initial_prompt(seed)
            .extra_arg(seed)
            .build();
        config.recovery_continuation = Some(gwt_agent::RecoveryContinuationHandoff {
            source_session_id: "source-container-session".to_string(),
            source_recovery_id: source.recovery_id.clone(),
            target_recovery_id: "target-container-recovery".to_string(),
            source_checkpoint_revision: source.checkpoint_revision,
            reason: "test container continuation".to_string(),
            inherit_checkpoint: true,
        });

        let bundle = prepare_container_checkpoint_attachments_from_store(&store, &mut config)
            .expect("prepare container attachments")
            .expect("attachment bundle");
        let container_paths = bundle.container_paths().expect("container manifest");
        let digest = attachment.content_id.strip_prefix("sha256:").unwrap();
        let host_blob = store
            .root()
            .join("attachments")
            .join("sha256")
            .join(&digest[..2])
            .join(digest)
            .to_string_lossy()
            .into_owned();
        let prompt = config.initial_prompt.as_deref().expect("final prompt");

        assert_eq!(config.args.last().map(String::as_str), Some(prompt));
        assert!(prompt.contains(&container_paths[0]));
        assert!(container_paths[0].starts_with("/tmp/gwt-codex-recovery-"));
        assert!(!prompt.contains(&host_blob));
        assert!(!format!("{bundle:?}").contains(&host_blob));
    }
}
