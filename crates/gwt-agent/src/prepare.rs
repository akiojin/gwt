use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::{ExitStatus, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use reqwest::Url;
use tokio::io::{AsyncRead, AsyncReadExt};

use crate::{
    environment::LaunchEnvironment,
    launch::LaunchConfig,
    session::{
        runtime_state_path, Session, SessionRuntimeState, GWT_BIN_PATH_ENV,
        GWT_CONTINUE_WORK_READY_NONCE_ENV, GWT_HOOK_FORWARD_TOKEN_ENV, GWT_HOOK_FORWARD_URL_ENV,
        GWT_SESSION_ID_ENV, GWT_SESSION_RUNTIME_PATH_ENV,
    },
    types::{AgentId, DockerLifecycleIntent, LaunchRuntimeTarget},
};

const DOCKER_GWTD_BIN_PATH: &str = "/usr/local/bin/gwtd";
const DOCKER_HOST_GWT_BIN_NAME: &str = "gwt-linux";
const DOCKER_HOST_GWTD_BIN_NAME: &str = "gwtd-linux";
const DOCKER_GWT_OVERRIDE_HEADER: &str =
    "# Auto-generated docker-compose override for gwt bundle mounting";
const DOCKER_GWT_OVERRIDE_FILE_NAME: &str = "docker-compose.gwt.override.yml";
const DOCKER_USER_OVERRIDE_FILE_NAME: &str = "docker-compose.override.yml";
const START_WORK_BASE_BRANCH_CANDIDATES: [&str; 1] = ["origin/develop"];

#[derive(Clone, PartialEq, Eq)]
pub struct PreparedProcessLaunch {
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub remove_env: Vec<String>,
    pub cwd: Option<PathBuf>,
}

impl std::fmt::Debug for PreparedProcessLaunch {
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
            .collect::<Vec<_>>();
        let redacted_args = self
            .args
            .iter()
            .map(|argument| {
                let Some((key, _)) = argument.split_once('=') else {
                    return argument.clone();
                };
                if private_launch_env_key(key) {
                    format!("{key}=<redacted>")
                } else {
                    argument.clone()
                }
            })
            .collect::<Vec<_>>();

        formatter
            .debug_struct("PreparedProcessLaunch")
            .field("command", &self.command)
            .field("args", &redacted_args)
            .field("env", &redacted_env)
            .field("remove_env", &self.remove_env)
            .field("cwd", &self.cwd)
            .finish()
    }
}

fn private_launch_env_key(key: &str) -> bool {
    matches!(
        key,
        GWT_CONTINUE_WORK_READY_NONCE_ENV
            | GWT_HOOK_FORWARD_TOKEN_ENV
            | GWT_SESSION_ID_ENV
            | GWT_SESSION_RUNTIME_PATH_ENV
    )
}

#[derive(Debug, Clone)]
pub struct PreparedAgentLaunch {
    pub process_launch: PreparedProcessLaunch,
    pub session: Session,
    pub runtime_path: PathBuf,
    pub worktree_path: PathBuf,
}

#[derive(Clone, PartialEq, Eq)]
pub struct HookForwardEnv {
    pub url: String,
    pub token: String,
}

struct PreparedLaunchFinalization<'a> {
    container_runtime: Option<&'a gwt_docker::detect::ResolvedContainerRuntime>,
}

impl std::fmt::Debug for HookForwardEnv {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HookForwardEnv")
            .field("url", &self.url)
            .field("token", &"<redacted>")
            .finish()
    }
}

/// Translate the daemon's host-only hook endpoint for the selected launch
/// runtime. Host launches retain the issued URL byte-for-byte.
pub fn hook_forward_url_for_launch_runtime(
    host_url: &str,
    runtime_target: LaunchRuntimeTarget,
    container_runtime_kind: Option<gwt_docker::ContainerRuntimeKind>,
) -> Result<String, String> {
    if runtime_target == LaunchRuntimeTarget::Host {
        return Ok(host_url.to_string());
    }

    let bridge_host = container_runtime_kind
        .ok_or_else(|| "container hook forwarding requires a resolved runtime kind".to_string())?
        .host_bridge_name();
    let mut url =
        Url::parse(host_url).map_err(|error| format!("invalid host hook forward URL: {error}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!(
            "container hook forwarding requires an HTTP(S) URL scheme, got '{}'",
            url.scheme()
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("container hook forwarding URL must not contain user credentials".to_string());
    }
    let host = url
        .host_str()
        .ok_or_else(|| "host hook forward URL is missing a host".to_string())?;
    if !is_loopback_hook_forward_host(host) {
        return Err(format!(
            "container hook forwarding requires a loopback host endpoint before bridge translation, got '{host}'"
        ));
    }
    if url.port().is_none() {
        return Err(
            "container hook forwarding requires an explicit port before bridge translation"
                .to_string(),
        );
    }
    if url.path() != "/internal/hook-live" || url.query().is_some() || url.fragment().is_some() {
        return Err(format!(
            "container hook forwarding URL must use the exact /internal/hook-live path without query or fragment, got '{}'",
            url.path()
        ));
    }
    url.set_host(Some(bridge_host)).map_err(|_| {
        format!("failed to install container host bridge name '{bridge_host}' in hook URL")
    })?;
    Ok(url.into())
}

/// Select the pane WebSocket endpoint for the launch runtime.
///
/// Every managed agent uses the capability-authenticated agent listener.
/// Host agents keep its loopback URL; container agents rewrite that URL to
/// the runtime's reserved host bridge.
pub fn pane_websocket_url_for_launch_runtime(
    _browser_listener_url: &str,
    agent_listener_url: &str,
    runtime_target: LaunchRuntimeTarget,
    container_runtime_kind: Option<gwt_docker::ContainerRuntimeKind>,
) -> Result<String, String> {
    let mut url = Url::parse(agent_listener_url)
        .map_err(|error| format!("invalid agent pane WebSocket URL: {error}"))?;
    if !matches!(url.scheme(), "ws" | "wss") {
        return Err(format!(
            "managed pane access requires a WebSocket URL scheme, got '{}'",
            url.scheme()
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("managed pane WebSocket URL must not contain user credentials".to_string());
    }
    let host = url
        .host_str()
        .ok_or_else(|| "managed pane WebSocket URL is missing a host".to_string())?;
    if !is_loopback_hook_forward_host(host) {
        return Err(format!(
            "managed pane WebSocket access requires a loopback agent endpoint, got '{host}'"
        ));
    }
    if url.port().is_none() {
        return Err("managed pane WebSocket URL requires an explicit port".to_string());
    }
    if url.path() != "/internal/pane-ws" || url.query().is_some() || url.fragment().is_some() {
        return Err(format!(
            "managed pane WebSocket URL must use the exact /internal/pane-ws path without query or fragment, got '{}'",
            url.path()
        ));
    }
    if runtime_target == LaunchRuntimeTarget::Host {
        return Ok(url.into());
    }

    let bridge_host = container_runtime_kind
        .ok_or_else(|| "container pane access requires a resolved runtime kind".to_string())?
        .host_bridge_name();
    url.set_host(Some(bridge_host)).map_err(|_| {
        format!("failed to install container host bridge name '{bridge_host}' in pane URL")
    })?;
    Ok(url.into())
}

fn is_loopback_hook_forward_host(host: &str) -> bool {
    let normalized = host
        .strip_prefix('[')
        .and_then(|candidate| candidate.strip_suffix(']'))
        .unwrap_or(host);
    normalized.eq_ignore_ascii_case("localhost")
        || normalized
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn install_hook_forward_env(
    config: &mut LaunchConfig,
    target: Option<HookForwardEnv>,
    container_runtime: Option<&gwt_docker::detect::ResolvedContainerRuntime>,
) -> Result<(), String> {
    let Some(target) = target else {
        return Ok(());
    };
    let url = hook_forward_url_for_launch_runtime(
        &target.url,
        config.runtime_target,
        container_runtime.map(gwt_docker::detect::ResolvedContainerRuntime::kind),
    )?;
    config
        .env_vars
        .insert(GWT_HOOK_FORWARD_URL_ENV.to_string(), url);
    config
        .env_vars
        .insert(GWT_HOOK_FORWARD_TOKEN_ENV.to_string(), target.token);
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DockerBundleMounts {
    host_gwt: PathBuf,
    host_gwtd: PathBuf,
}

#[derive(Debug, Clone)]
struct DockerLaunchPlan {
    compose_files: Vec<PathBuf>,
    service: String,
    container_cwd: String,
    target_arch: String,
}

impl DockerLaunchPlan {
    fn include_compose_override(&mut self, override_file: PathBuf) {
        if !self.compose_files.iter().any(|file| file == &override_file) {
            self.compose_files.push(override_file);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DockerExecProgram {
    executable: String,
    args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ContainerRuntimeProgram {
    executable: String,
    args: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HostRunnerProbeKind {
    Direct,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostRunnerProbeOutcome {
    pub success: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    pub error: Option<String>,
}

impl HostRunnerProbeOutcome {
    pub fn success() -> Self {
        Self {
            success: true,
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
            timed_out: false,
            error: None,
        }
    }

    pub fn failure_with_stderr(stderr: &str) -> Self {
        Self {
            success: false,
            exit_code: Some(1),
            stdout: String::new(),
            stderr: stderr.to_string(),
            timed_out: false,
            error: None,
        }
    }

    pub fn timeout() -> Self {
        Self {
            success: false,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            timed_out: true,
            error: None,
        }
    }

    /// Synthesized outcome for a runner that never spawned (e.g. the direct
    /// executable is not on PATH). SPEC-3864 T-011: it carries no
    /// `exit_code`, so the diagnostic cannot read as a real process exit.
    pub fn unresolved(reason: &str) -> Self {
        Self {
            success: false,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            timed_out: false,
            error: Some(reason.to_string()),
        }
    }

    fn combined_output(&self) -> String {
        format!("{}\n{}", self.stdout, self.stderr)
    }

    fn diagnostic(&self, env_vars: &HashMap<String, String>) -> String {
        let mut parts = Vec::new();
        if let Some(error) = &self.error {
            parts.push(redact_runner_probe_text(error, env_vars));
        }
        if self.timed_out {
            parts.push("probe timed out".to_string());
        }
        if let Some(code) = self.exit_code {
            parts.push(format!("exit status {code}"));
        }
        let output = self.combined_output();
        let output = output.trim();
        if !output.is_empty() {
            let redacted = redact_runner_probe_text(output, env_vars);
            parts.push(truncate_runner_diagnostic(&redacted, 1200));
        }
        if parts.is_empty() {
            "probe failed without output.".to_string()
        } else {
            format!("Probe detail: {}.", parts.join("; "))
        }
    }
}

const DIRECT_RUNNER_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Stable phrase for transient launch failures that Issue Monitor retries.
pub const TRANSIENT_LAUNCH_RETRY_HINT: &str =
    "gwt retries this launch automatically without spending an attempt";

/// Classify automatic-retry diagnostics and concurrent remote-ref races.
pub fn is_transient_launch_failure(message: &str) -> bool {
    message.contains(TRANSIENT_LAUNCH_RETRY_HINT)
        || gwt_git::worktree::is_remote_tracking_ref_cas_conflict(message)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostRunnerHealthReport {
    /// Strict version evidence measured from the installed executable.
    pub version_output: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct DevContainerLaunchDefaults {
    service: Option<String>,
    workspace_folder: Option<String>,
    compose_file: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DockerLaunchServiceAction {
    Connect,
    Start,
    Restart,
    Recreate,
}

type HostRunnerProbe = dyn FnMut(
    HostRunnerProbeKind,
    &str,
    Vec<String>,
    &HashMap<String, String>,
    &[String],
    Option<PathBuf>,
) -> HostRunnerProbeOutcome;
type GwtBinLookup = dyn Fn(&str) -> Option<PathBuf>;

struct PrepareLaunchDeps<'a> {
    current_exe: &'a Path,
    probe_host_runner: &'a mut HostRunnerProbe,
    lookup_gwt_bin: &'a GwtBinLookup,
}

pub fn prepare_agent_launch<F>(
    repo_path: &Path,
    sessions_dir: &Path,
    config: LaunchConfig,
    hook_forward: Option<HookForwardEnv>,
    refresh_worktree_assets: F,
) -> Result<PreparedAgentLaunch, String>
where
    F: FnMut(&Path) -> Result<(), String>,
{
    let current_exe = std::env::current_exe().map_err(|error| format!("current_exe: {error}"))?;
    let mut probe_host_runner = probe_host_runner_outcome
        as fn(
            HostRunnerProbeKind,
            &str,
            Vec<String>,
            &HashMap<String, String>,
            &[String],
            Option<PathBuf>,
        ) -> HostRunnerProbeOutcome;
    let lookup_gwt_bin = |command: &str| which::which(command).ok();
    prepare_agent_launch_with(
        repo_path,
        sessions_dir,
        config,
        hook_forward,
        refresh_worktree_assets,
        PrepareLaunchDeps {
            current_exe: &current_exe,
            probe_host_runner: &mut probe_host_runner,
            lookup_gwt_bin: &lookup_gwt_bin,
        },
    )
}

fn prepare_agent_launch_with<FRefresh>(
    repo_path: &Path,
    sessions_dir: &Path,
    mut config: LaunchConfig,
    hook_forward: Option<HookForwardEnv>,
    mut refresh_worktree_assets: FRefresh,
    deps: PrepareLaunchDeps<'_>,
) -> Result<PreparedAgentLaunch, String>
where
    FRefresh: FnMut(&Path) -> Result<(), String>,
{
    let PrepareLaunchDeps {
        current_exe,
        probe_host_runner,
        lookup_gwt_bin,
    } = deps;

    resolve_launch_worktree(repo_path, &mut config)?;
    normalize_launch_config_working_dir(&mut config);
    let container_runtime = apply_docker_runtime_to_launch_config(repo_path, &mut config)?;

    let worktree_path = normalize_child_process_path(
        &config
            .working_dir
            .clone()
            .unwrap_or_else(|| repo_path.to_path_buf()),
    );
    if config.working_dir.is_some() {
        config.working_dir = Some(worktree_path.clone());
    }
    let launch_env = match config.runtime_target {
        LaunchRuntimeTarget::Host => {
            LaunchEnvironment::from_base_env(crate::environment::host_process_env())
        }
        LaunchRuntimeTarget::Docker => LaunchEnvironment::empty(),
    };
    launch_env
        .with_project_root(&worktree_path)
        .apply_to_config(&mut config);
    refresh_worktree_assets(&worktree_path)?;

    resolve_host_runner_health_checked_with_probe(&mut config, probe_host_runner)?;

    install_launch_gwt_bin_env_with_lookup(
        &mut config.env_vars,
        config.runtime_target,
        current_exe,
        lookup_gwt_bin,
    )?;

    let branch_name = config
        .branch
        .clone()
        .unwrap_or_else(|| "workspace".to_string());
    let session = Session::from_launch_config(&worktree_path, branch_name, &config);
    let runtime_path = runtime_state_path(sessions_dir, &session.id);

    config
        .env_vars
        .insert(GWT_SESSION_ID_ENV.to_string(), session.id.clone());
    config.env_vars.insert(
        GWT_SESSION_RUNTIME_PATH_ENV.to_string(),
        runtime_path.display().to_string(),
    );
    install_hook_forward_env(&mut config, hook_forward, container_runtime.as_ref())?;
    config
        .env_vars
        .entry("COLORTERM".to_string())
        .or_insert_with(|| "truecolor".to_string());

    finalize_and_persist_prepared_launch(
        repo_path,
        sessions_dir,
        config,
        session,
        runtime_path,
        worktree_path,
        PreparedLaunchFinalization {
            container_runtime: container_runtime.as_ref(),
        },
    )
}

fn finalize_and_persist_prepared_launch(
    repo_path: &Path,
    sessions_dir: &Path,
    mut config: LaunchConfig,
    mut session: Session,
    runtime_path: PathBuf,
    worktree_path: PathBuf,
    finalization: PreparedLaunchFinalization<'_>,
) -> Result<PreparedAgentLaunch, String> {
    let docker_runtime_worktree = finalize_docker_agent_launch_config_with_runtime(
        repo_path,
        &mut config,
        finalization.container_runtime,
    )?;
    if let Some(runtime_worktree) = docker_runtime_worktree {
        let project_state_root = normalize_child_process_path(repo_path);
        session.project_state_root = Some(project_state_root.clone());
        session.bind_docker_runtime(runtime_worktree, &project_state_root)?;
    }

    session.codex_auth_root = config
        .working_dir
        .as_deref()
        .and_then(|cwd| config.validated_codex_auth_root_for_cwd(cwd));
    session
        .save(sessions_dir)
        .map_err(|error| error.to_string())?;
    SessionRuntimeState::new(crate::AgentStatus::Running)
        .save(&runtime_path)
        .map_err(|error| error.to_string())?;

    Ok(PreparedAgentLaunch {
        process_launch: PreparedProcessLaunch {
            command: config.command,
            args: config.args,
            env: config.env_vars,
            remove_env: config.remove_env,
            cwd: config.working_dir,
        },
        session,
        runtime_path,
        worktree_path,
    })
}

fn normalize_child_process_path(path: &Path) -> PathBuf {
    gwt_core::paths::normalize_windows_child_process_path(path)
}

fn normalize_launch_config_working_dir(config: &mut LaunchConfig) {
    if let Some(dir) = config.working_dir.as_ref() {
        let normalized = normalize_child_process_path(dir);
        config.working_dir = Some(normalized.clone());
        config.env_vars.insert(
            "GWT_PROJECT_ROOT".to_string(),
            normalized.display().to_string(),
        );
    }
}

fn set_worktree_launch_path(
    working_dir: &mut Option<PathBuf>,
    env_vars: &mut HashMap<String, String>,
    path: &Path,
) {
    let path = normalize_child_process_path(path);
    *working_dir = Some(path.clone());
    env_vars.insert("GWT_PROJECT_ROOT".to_string(), path.display().to_string());
}

pub fn branch_worktree_path(repo_path: &Path, branch_name: &str) -> Option<PathBuf> {
    let main_repo_path = gwt_git::worktree::main_worktree_root(repo_path).ok()?;
    let manager = gwt_git::WorktreeManager::new(&main_repo_path);
    let mut worktrees = manager.list().ok()?;
    if let Some(path) = usable_worktree_path_for_branch(&worktrees, branch_name) {
        return Some(path);
    }
    if worktrees_have_stale_branch_entry(&worktrees, branch_name) {
        manager.prune().ok()?;
        worktrees = manager.list().ok()?;
        return usable_worktree_path_for_branch(&worktrees, branch_name);
    }
    None
}

pub fn resolve_launch_worktree(repo_path: &Path, config: &mut LaunchConfig) -> Result<(), String> {
    resolve_launch_worktree_request(
        repo_path,
        config.branch.as_deref(),
        config.base_branch.as_deref(),
        &mut config.working_dir,
        &mut config.env_vars,
    )?;
    normalize_launch_config_working_dir(config);
    Ok(())
}

pub fn resolve_launch_worktree_request(
    repo_path: &Path,
    branch_name: Option<&str>,
    base_branch: Option<&str>,
    working_dir: &mut Option<PathBuf>,
    env_vars: &mut HashMap<String, String>,
) -> Result<(), String> {
    let Some(branch_name) = branch_name.map(str::to_string) else {
        return Ok(());
    };
    if working_dir.is_some() {
        return Ok(());
    }

    let main_repo_path = match gwt_git::worktree::main_worktree_root(repo_path) {
        Ok(path) => path,
        Err(error) => {
            if base_branch.is_none()
                && matches!(
                    gwt_git::detect_repo_type(repo_path),
                    gwt_git::RepoType::NonRepo
                )
            {
                return Ok(());
            }
            return Err(error.to_string());
        }
    };
    let manager = gwt_git::WorktreeManager::new(&main_repo_path);
    let mut worktrees = manager.list().map_err(|err| err.to_string())?;
    if let Some(existing_worktree) = usable_worktree_path_for_branch(&worktrees, &branch_name) {
        set_worktree_launch_path(working_dir, env_vars, &existing_worktree);
        return Ok(());
    }
    if worktrees_have_stale_branch_entry(&worktrees, &branch_name) {
        manager
            .prune()
            .map_err(|err| format!("failed to prune stale worktrees: {err}"))?;
        worktrees = manager.list().map_err(|err| err.to_string())?;
        if let Some(existing_worktree) = usable_worktree_path_for_branch(&worktrees, &branch_name) {
            set_worktree_launch_path(working_dir, env_vars, &existing_worktree);
            return Ok(());
        }
    }

    let mut base_branch = base_branch
        .map(str::to_string)
        .unwrap_or_else(|| "develop".to_string());
    let mut remote_base_ref = origin_remote_ref(&base_branch);
    let remote_branch_ref = origin_remote_ref(&branch_name);

    if is_start_work_branch_name(&branch_name) {
        manager
            .prepare_start_work_remote_develop()
            .map_err(|err| format!("failed to prepare origin/develop for Start Work: {err}"))?;
        base_branch = "origin/develop".to_string();
        remote_base_ref = origin_remote_ref(&base_branch);
    } else {
        manager
            .fetch_origin()
            .map_err(|err| format!("failed to fetch origin: {err}"))?;
    }

    if !manager
        .remote_branch_exists(&remote_base_ref)
        .map_err(|err| format!("failed to verify remote base branch {remote_base_ref}: {err}"))?
    {
        if let Some(fallback_base_branch) =
            refallback_start_work_base_branch(&branch_name, &base_branch, |candidate| {
                let candidate_ref = origin_remote_ref(candidate);
                manager.remote_branch_exists(&candidate_ref).map_err(|err| {
                    format!("failed to verify remote base branch {candidate_ref}: {err}")
                })
            })?
        {
            base_branch = fallback_base_branch;
            remote_base_ref = origin_remote_ref(&base_branch);
        } else {
            return Err(format!(
                "remote base branch does not exist: {remote_base_ref}"
            ));
        }
    }

    if !manager
        .remote_branch_exists(&remote_branch_ref)
        .map_err(|err| format!("failed to verify remote branch {remote_branch_ref}: {err}"))?
    {
        manager
            .create_remote_branch_from_base(&remote_base_ref, &branch_name)
            .map_err(|err| {
                format!(
                    "failed to create remote branch {remote_branch_ref} from {remote_base_ref}: {err}"
                )
            })?;
        manager
            .fetch_origin()
            .map_err(|err| format!("failed to refresh origin refs after push: {err}"))?;
    }

    let preferred_worktree_path =
        gwt_git::worktree::sibling_worktree_path(&main_repo_path, &branch_name);
    let worktree_path = first_available_worktree_path(&preferred_worktree_path, &worktrees)
        .ok_or_else(|| {
            format!("failed to resolve available worktree path for branch {branch_name}")
        })?;
    if local_branch_exists(&main_repo_path, &branch_name)? {
        manager
            .create(&branch_name, &worktree_path)
            .map_err(|err| err.to_string())?;
    } else {
        manager
            .create_from_remote(&remote_branch_ref, &branch_name, &worktree_path)
            .map_err(|err| err.to_string())?;
    }

    set_worktree_launch_path(working_dir, env_vars, &worktree_path);
    Ok(())
}

/// Check the installed Host executable before Session persistence or dispatch.
pub fn resolve_host_runner_health_checked(
    config: &mut LaunchConfig,
) -> Result<HostRunnerHealthReport, String> {
    resolve_host_runner_health_checked_with_probe(config, probe_host_runner_outcome)
}

#[doc(hidden)]
pub fn resolve_host_runner_health_checked_with_probe<F>(
    config: &mut LaunchConfig,
    mut probe: F,
) -> Result<HostRunnerHealthReport, String>
where
    F: FnMut(
        HostRunnerProbeKind,
        &str,
        Vec<String>,
        &HashMap<String, String>,
        &[String],
        Option<PathBuf>,
    ) -> HostRunnerProbeOutcome,
{
    if config.runtime_target != LaunchRuntimeTarget::Host
        || config.agent_id.builtin_descriptor().is_none()
    {
        return Ok(HostRunnerHealthReport::default());
    }
    let direct_command = crate::launch::resolve_direct_runner_with_effective_env(
        &config.command,
        &config.env_vars,
        &config.remove_env,
        config.working_dir.as_deref(),
    );
    let outcome = direct_command.as_deref().map_or_else(
        || HostRunnerProbeOutcome::unresolved("installed executable not resolved"),
        |command| {
            probe(
                HostRunnerProbeKind::Direct,
                command,
                crate::launch::builtin_version_probe_args(&config.agent_id)
                    .expect("built-in descriptor checked above"),
                &config.env_vars,
                &config.remove_env,
                config.working_dir.clone(),
            )
        },
    );
    if !outcome.success {
        let setup = config
            .agent_id
            .distribution()
            .install_shell_command()
            .map_or_else(
                || "Install the agent manually and relaunch.".to_string(),
                |command| format!("Install it with `{command}` and relaunch."),
            );
        return Err(format!(
            "{} installed runner failed its health check. {} {setup}",
            config.agent_id.display_name(),
            outcome.diagnostic(&config.env_vars),
        ));
    }
    let report = HostRunnerHealthReport {
        version_output: strict_semver_probe_evidence(&outcome),
    };
    if config.agent_id == AgentId::ClaudeCode
        && config.reasoning_level.as_deref() == Some("ultracode")
        && !report
            .version_output
            .as_deref()
            .is_some_and(|version| crate::claude_capabilities::supports_ultracode(version, true))
    {
        return Err(format!(
            "Claude Code ultracode requires version >= 2.1.154; selected runner version: {}",
            report.version_output.as_deref().unwrap_or("unknown"),
        ));
    }
    config.command = direct_command.expect("successful probe has a resolved command");
    config.tool_version = report.version_output.clone();
    Ok(report)
}

pub fn install_launch_gwt_bin_env(
    env_vars: &mut HashMap<String, String>,
    runtime_target: LaunchRuntimeTarget,
) -> Result<(), String> {
    let current_exe = std::env::current_exe().map_err(|error| format!("current_exe: {error}"))?;
    install_launch_gwt_bin_env_with_lookup(env_vars, runtime_target, &current_exe, |command| {
        which::which(command).ok()
    })
}

pub fn install_launch_gwt_bin_env_with_lookup(
    env_vars: &mut HashMap<String, String>,
    runtime_target: LaunchRuntimeTarget,
    current_exe: &Path,
    lookup: impl FnOnce(&str) -> Option<PathBuf>,
) -> Result<(), String> {
    let gwt_bin = match runtime_target {
        LaunchRuntimeTarget::Docker => DOCKER_GWTD_BIN_PATH.to_string(),
        LaunchRuntimeTarget::Host => resolve_public_gwt_bin_with_lookup(current_exe, lookup)
            .to_string_lossy()
            .into_owned(),
    };
    match runtime_target {
        LaunchRuntimeTarget::Docker => {
            env_vars.insert(GWT_BIN_PATH_ENV.to_string(), gwt_bin);
        }
        LaunchRuntimeTarget::Host => {
            env_vars
                .entry(GWT_BIN_PATH_ENV.to_string())
                .or_insert(gwt_bin);
        }
    }
    if let Some(resolved) = env_vars.get(GWT_BIN_PATH_ENV).cloned() {
        if let Some(parent) = Path::new(&resolved).parent() {
            match runtime_target {
                LaunchRuntimeTarget::Docker => {
                    prepend_posix_dir_to_path(env_vars, parent);
                }
                LaunchRuntimeTarget::Host => {
                    prepend_dir_to_path(env_vars, parent);
                }
            }
        }
    }
    Ok(())
}

/// Prepend `dir` to the PATH-style entry in `env_vars` unless it is empty or
/// already present.
///
/// Returns `true` if the entry was updated, `false` for a no-op (empty `dir`,
/// dir already on PATH, or `join_paths` failure). Key lookup is
/// case-insensitive: Windows processes may expose the variable as `Path` or
/// `path`, and a case-sensitive read would produce a duplicate `PATH` key
/// alongside the original, corrupting command lookup once the child process
/// inherits both. PATH parsing uses [`std::env::split_paths`] /
/// [`std::env::join_paths`] so the `:` / `;` separator difference between
/// Unix and Windows is handled automatically.
pub fn prepend_dir_to_path(env_vars: &mut HashMap<String, String>, dir: &Path) -> bool {
    if dir.as_os_str().is_empty() {
        return false;
    }
    let existing_key = env_vars
        .keys()
        .find(|key| key.eq_ignore_ascii_case("PATH"))
        .cloned();
    let key = existing_key.unwrap_or_else(|| "PATH".to_string());
    let existing_path = env_vars
        .get(&key)
        .map(String::as_str)
        .unwrap_or_default()
        .to_string();
    let mut entries: Vec<PathBuf> = if existing_path.is_empty() {
        Vec::new()
    } else {
        std::env::split_paths(&existing_path).collect()
    };
    let dir_buf = dir.to_path_buf();
    if entries.iter().any(|entry| entry == &dir_buf) {
        return false;
    }
    entries.insert(0, dir_buf);
    let Ok(joined) = std::env::join_paths(&entries) else {
        return false;
    };
    env_vars.insert(key, joined.to_string_lossy().into_owned());
    true
}

/// Prepend `dir` to a POSIX PATH value used inside Docker containers.
pub fn prepend_posix_dir_to_path(env_vars: &mut HashMap<String, String>, dir: &Path) -> bool {
    if dir.as_os_str().is_empty() {
        return false;
    }
    let existing_key = env_vars
        .keys()
        .find(|key| key.eq_ignore_ascii_case("PATH"))
        .cloned();
    let key = existing_key.unwrap_or_else(|| "PATH".to_string());
    let existing_path = env_vars
        .get(&key)
        .map(String::as_str)
        .unwrap_or_default()
        .to_string();
    let dir_value = dir.to_string_lossy().into_owned();
    if dir_value.is_empty() {
        return false;
    }
    let mut entries: Vec<String> = if existing_path.is_empty() {
        Vec::new()
    } else {
        existing_path.split(':').map(str::to_string).collect()
    };
    if entries.iter().any(|entry| entry == &dir_value) {
        return false;
    }
    entries.insert(0, dir_value);
    env_vars.insert(key, entries.join(":"));
    true
}

pub fn resolve_public_gwt_bin_with_lookup(
    current_exe: &Path,
    lookup: impl FnOnce(&str) -> Option<PathBuf>,
) -> PathBuf {
    if is_named_gwt_binary(current_exe) && !is_bunx_temp_executable(current_exe) {
        if let Some(candidate) = sibling_gwtd_binary(current_exe) {
            return candidate;
        }
    }

    if should_prefer_path_gwt(current_exe) {
        if let Some(candidate) = lookup("gwtd").filter(|candidate| {
            !same_path(candidate, current_exe) && !is_bunx_temp_executable(candidate)
        }) {
            return candidate;
        }
        if let Some(candidate) = sibling_gwtd_binary(current_exe) {
            return candidate;
        }
    }
    current_exe.to_path_buf()
}

fn sibling_gwtd_binary(path: &Path) -> Option<PathBuf> {
    if !is_named_gwt_binary(path) {
        return None;
    }
    let sibling_name = match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) if ext.eq_ignore_ascii_case("exe") => "gwtd.exe".to_string(),
        _ => "gwtd".to_string(),
    };
    Some(path.with_file_name(sibling_name))
}

fn apply_docker_runtime_to_launch_config(
    repo_path: &Path,
    config: &mut LaunchConfig,
) -> Result<Option<gwt_docker::detect::ResolvedContainerRuntime>, String> {
    if config.runtime_target != LaunchRuntimeTarget::Docker {
        return Ok(None);
    }

    let worktree = normalize_child_process_path(
        &config
            .working_dir
            .clone()
            .unwrap_or_else(|| repo_path.to_path_buf()),
    );
    let launch = resolve_docker_launch_plan(&worktree, config.docker_service.as_deref())?;
    let runtime =
        gwt_docker::detect::ResolvedContainerRuntime::resolve(&docker_binary_for_launch())?;
    ensure_docker_launch_runtime_ready_for_runtime(&runtime)?;
    let mut launch = launch;
    let (compose_override_file, managed_override_changed) = ensure_docker_gwt_binary_setup(
        &worktree,
        &launch.service,
        &launch.target_arch,
        runtime.kind(),
    )?;
    launch.include_compose_override(compose_override_file);
    let lifecycle_intent = if managed_override_changed {
        DockerLifecycleIntent::Recreate
    } else {
        config.docker_lifecycle_intent
    };
    ensure_docker_launch_service_ready(&launch, lifecycle_intent)?;
    maybe_inject_docker_sandbox_env(&launch, config)?;
    install_launch_gwt_bin_env(&mut config.env_vars, LaunchRuntimeTarget::Docker)?;
    let runtime_program = resolve_docker_exec_program(&launch, config)?;
    config.command = runtime_program.executable;
    config.args = runtime_program.args;
    config
        .env_vars
        .insert("GWT_PROJECT_ROOT".to_string(), launch.container_cwd.clone());
    config.docker_service = Some(launch.service);
    Ok(Some(runtime))
}

fn finalize_docker_agent_launch_config_with_runtime(
    repo_path: &Path,
    config: &mut LaunchConfig,
    container_runtime: Option<&gwt_docker::detect::ResolvedContainerRuntime>,
) -> Result<Option<String>, String> {
    if config.runtime_target != LaunchRuntimeTarget::Docker {
        return Ok(None);
    }
    let container_runtime = container_runtime
        .ok_or_else(|| "Docker launch finalization requires a resolved runtime".to_string())?;

    let worktree = normalize_child_process_path(
        &config
            .working_dir
            .clone()
            .unwrap_or_else(|| repo_path.to_path_buf()),
    );
    let launch = resolve_docker_launch_plan(&worktree, config.docker_service.as_deref())?;
    let runtime_program = ContainerRuntimeProgram {
        executable: config.command.clone(),
        args: config.args.clone(),
    };

    let mut args = docker_compose_command_prefix(&launch);
    let runtime_worktree_path = launch.container_cwd;
    args.extend([
        "exec".to_string(),
        "-w".to_string(),
        runtime_worktree_path.clone(),
    ]);
    args.extend(docker_compose_exec_env_args(&config.env_vars));
    args.push(launch.service);
    args.push(runtime_program.executable);
    args.extend(runtime_program.args);

    config.command = container_runtime.binary().to_string();
    config.args = args;
    Ok(Some(runtime_worktree_path))
}

fn probe_host_runner_outcome(
    kind: HostRunnerProbeKind,
    command: &str,
    args: Vec<String>,
    env_vars: &HashMap<String, String>,
    remove_env: &[String],
    cwd: Option<PathBuf>,
) -> HostRunnerProbeOutcome {
    probe_host_runner_with_timeout(
        kind,
        command,
        args,
        env_vars,
        remove_env,
        cwd,
        DIRECT_RUNNER_PROBE_TIMEOUT,
        Duration::from_millis(50),
    )
}

#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
pub fn probe_host_runner_with_timeout(
    kind: HostRunnerProbeKind,
    command: &str,
    args: Vec<String>,
    env_vars: &HashMap<String, String>,
    remove_env: &[String],
    cwd: Option<PathBuf>,
    timeout: Duration,
    poll_interval: Duration,
) -> HostRunnerProbeOutcome {
    let hub = gwt_core::process_console::global();
    probe_host_runner_bounded_with_hub(
        HostRunnerProbeRequest {
            kind,
            command,
            args,
            env_vars,
            remove_env,
            cwd,
            timeout,
            poll_interval,
        },
        &hub,
    )
}

fn truncate_runner_diagnostic(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

const MIN_RUNNER_PROBE_SECRET_VALUE_CHARS: usize = 8;
const RUNNER_PROBE_CAPTURE_LIMIT_BYTES: usize = 16 * 1024;
const RUNNER_PROBE_TRUNCATED_MARKER: &str = "\n[gwt] probe output truncated\n";
const RUNNER_PROBE_CLEANUP_RESERVE_MAX: Duration = Duration::from_millis(250);

fn runner_probe_secret_values(env_vars: &HashMap<String, String>) -> Vec<String> {
    let mut values = env_vars
        .values()
        .filter_map(|value| {
            let value = gwt_core::process_console::strip_ansi(value);
            (value.chars().count() >= MIN_RUNNER_PROBE_SECRET_VALUE_CHARS).then_some(value)
        })
        .collect::<Vec<_>>();
    values.sort_by_key(|value| std::cmp::Reverse(value.len()));
    values.dedup();
    values
}

fn redact_runner_probe_text(value: &str, env_vars: &HashMap<String, String>) -> String {
    redact_runner_probe_text_with_values(
        value,
        &runner_probe_secret_values(env_vars),
        MIN_RUNNER_PROBE_SECRET_VALUE_CHARS,
    )
}

fn strict_semver_probe_evidence(outcome: &HostRunnerProbeOutcome) -> Option<String> {
    outcome
        .combined_output()
        .split_whitespace()
        .filter_map(|token| {
            let token = token.trim_matches(|character: char| {
                !character.is_ascii_alphanumeric() && !matches!(character, '.' | '-' | '+' | '_')
            });
            let token = token.strip_prefix('v').unwrap_or(token);
            semver::Version::parse(token).ok()
        })
        .map(|version| version.to_string())
        .next()
}

fn runner_probe_environment(
    env_vars: &HashMap<String, String>,
    remove_env: &[String],
) -> HashMap<String, String> {
    const ALLOWLIST: &[&str] = &[
        "PATH",
        "PATHEXT",
        "SYSTEMROOT",
        "WINDIR",
        "COMSPEC",
        "HOME",
        "USERPROFILE",
        "HOMEDRIVE",
        "HOMEPATH",
        "TEMP",
        "TMP",
        "TMPDIR",
        "TERM",
    ];
    let mut environment = ALLOWLIST
        .iter()
        .filter_map(|key| {
            std::env::var(key)
                .ok()
                .map(|value| ((*key).to_string(), value))
        })
        .collect::<HashMap<_, _>>();
    environment.retain(|key, _| {
        !remove_env
            .iter()
            .any(|removed| removed.eq_ignore_ascii_case(key))
    });
    // Match launch selection and PTY spawn: explicit values override removals.
    for key in ALLOWLIST {
        if let Some(value) = env_vars.get(*key) {
            environment.insert((*key).to_string(), value.clone());
        }
    }
    environment
}

fn redact_runner_probe_text_with_values(
    value: &str,
    secret_values: &[String],
    minimum_prefix_chars: usize,
) -> String {
    let ansi_safe = strip_runner_probe_ansi(value);
    let mut redacted = secret_values.iter().fold(
        gwt_core::process_console::redact_line(&ansi_safe),
        |redacted, secret| redacted.replace(secret, gwt_core::process_console::REDACTED),
    );
    for secret in secret_values {
        let partial = secret
            .char_indices()
            .skip(1)
            .map(|(index, _)| index)
            .chain(std::iter::once(secret.len()))
            .skip(minimum_prefix_chars.saturating_sub(1))
            .filter(|length| {
                *length <= redacted.len() && redacted.is_char_boundary(redacted.len() - length)
            })
            .filter(|length| redacted.ends_with(&secret[..*length]))
            .last();
        if let Some(length) = partial {
            redacted.truncate(redacted.len() - length);
            redacted.push_str(gwt_core::process_console::REDACTED);
        }
    }
    redacted
}

fn strip_runner_probe_ansi(value: &str) -> String {
    let complete_prefix = value.rfind('\u{1b}').and_then(|index| {
        let suffix = &value[index..];
        let incomplete = suffix == "\u{1b}"
            || (suffix.starts_with("\u{1b}[")
                && !suffix
                    .as_bytes()
                    .iter()
                    .skip(2)
                    .any(|byte| (0x40..=0x7e).contains(byte)))
            || (suffix.starts_with("\u{1b}]")
                && !suffix.ends_with('\u{7}')
                && !suffix.ends_with("\u{1b}\\"));
        incomplete.then_some(&value[..index])
    });
    gwt_core::process_console::strip_ansi(complete_prefix.unwrap_or(value))
}

#[derive(Default)]
struct RunnerProbeCapture {
    bytes: Vec<u8>,
    truncated: bool,
}

impl RunnerProbeCapture {
    fn append(&mut self, bytes: &[u8]) {
        let remaining = RUNNER_PROBE_CAPTURE_LIMIT_BYTES.saturating_sub(self.bytes.len());
        let retained = remaining.min(bytes.len());
        self.bytes.extend_from_slice(&bytes[..retained]);
        self.truncated |= retained < bytes.len();
    }

    fn render(&self, env_vars: &HashMap<String, String>) -> String {
        let raw = decode_runner_probe_capture(&self.bytes, self.truncated);
        let redacted = if self.truncated {
            redact_runner_probe_text_with_values(&raw, &runner_probe_secret_values(env_vars), 1)
        } else {
            redact_runner_probe_text(&raw, env_vars)
        };
        let mut rendered =
            truncate_runner_capture_bytes(&redacted, RUNNER_PROBE_CAPTURE_LIMIT_BYTES);
        if self.truncated || rendered.len() < redacted.len() {
            rendered.push_str(RUNNER_PROBE_TRUNCATED_MARKER);
        }
        rendered
    }
}

fn decode_runner_probe_capture(bytes: &[u8], truncated: bool) -> std::borrow::Cow<'_, str> {
    if truncated {
        if let Err(error) = std::str::from_utf8(bytes) {
            if error.error_len().is_none() {
                return String::from_utf8_lossy(&bytes[..error.valid_up_to()]);
            }
        }
    }
    String::from_utf8_lossy(bytes)
}

fn truncate_runner_capture_bytes(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let mut boundary = max_bytes;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value[..boundary].to_string()
}

struct HostRunnerProbeRequest<'a> {
    kind: HostRunnerProbeKind,
    command: &'a str,
    args: Vec<String>,
    env_vars: &'a HashMap<String, String>,
    remove_env: &'a [String],
    cwd: Option<PathBuf>,
    timeout: Duration,
    poll_interval: Duration,
}

fn probe_host_runner_bounded_with_hub(
    request: HostRunnerProbeRequest<'_>,
    hub: &gwt_core::process_console::ProcessConsoleHub,
) -> HostRunnerProbeOutcome {
    let HostRunnerProbeRequest {
        kind,
        command,
        args,
        env_vars,
        remove_env,
        cwd,
        timeout,
        poll_interval,
    } = request;
    let spawn_id = next_agent_spawn_id();
    let label = runner_probe_trace_label(kind);
    let start = Instant::now();
    tracing::info!(
        target: "gwt.process.summary",
        kind = "agent",
        spawn_id = spawn_id,
        label = %label,
        probe_kind = ?kind,
        phase = "start",
        "process start",
    );

    let mut request = gwt_core::process::ProcessPlanRequest::new(command)
        .args(&args)
        .inherit_env(false);
    for (key, value) in runner_probe_environment(env_vars, remove_env) {
        request = request.env(key, value);
    }
    if let Some(cwd) = cwd {
        request = request.current_dir(cwd);
    }
    let mut process = match gwt_core::process::resolved_command(request) {
        Ok(process) => process,
        Err(_error) => {
            let message =
                "[gwt] failed to resolve the runner health probe safely; verify the configured executable and PATH"
                    .to_string();
            push_runner_probe_console_line(
                hub,
                spawn_id,
                gwt_core::process_console::ProcessStream::Stderr,
                &message,
            );
            tracing::info!(
                target: "gwt.process.summary",
                kind = "agent",
                spawn_id = spawn_id,
                label = %label,
                phase = "end",
                exit_code = None::<i64>,
                duration_ms = start.elapsed().as_millis() as u64,
                success = false,
                resolution_error = "process resolution rejected",
                "process end",
            );
            return HostRunnerProbeOutcome {
                success: false,
                exit_code: None,
                stdout: String::new(),
                stderr: message,
                timed_out: false,
                error: Some("runner health probe process resolution was rejected".to_string()),
            };
        }
    };
    process
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let operation_deadline = start.checked_add(timeout).unwrap_or(start);
    let cleanup_reserve = (timeout / 4).min(RUNNER_PROBE_CLEANUP_RESERVE_MAX);
    let execution_deadline = operation_deadline
        .checked_sub(cleanup_reserve)
        .unwrap_or(start);
    let outcome = run_runner_probe_in_isolated_runtime(
        process,
        env_vars,
        execution_deadline,
        operation_deadline,
        poll_interval,
    );
    publish_runner_probe_capture(hub, spawn_id, &outcome.stdout, &outcome.stderr);
    let note = if outcome.timed_out {
        Some("timeout")
    } else if outcome.error.is_some() {
        Some("probe error")
    } else {
        None
    };
    tracing::info!(
        target: "gwt.process.summary",
        kind = "agent",
        spawn_id = spawn_id,
        label = %label,
        phase = "end",
        exit_code = outcome.exit_code.map(|code| code as i64),
        duration_ms = start.elapsed().as_millis() as u64,
        success = outcome.success,
        note,
        "process end",
    );
    outcome
}

fn runner_probe_trace_label(_kind: HostRunnerProbeKind) -> &'static str {
    "direct runner health probe"
}

fn run_runner_probe_in_isolated_runtime(
    process: std::process::Command,
    env_vars: &HashMap<String, String>,
    execution_deadline: Instant,
    operation_deadline: Instant,
    poll_interval: Duration,
) -> HostRunnerProbeOutcome {
    thread::scope(|scope| {
        let worker = thread::Builder::new()
            .name("gwt-runner-probe".to_string())
            .spawn_scoped(scope, move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|error| error.to_string())?;
                Ok::<_, String>(runtime.block_on(run_runner_probe_async(
                    process,
                    env_vars,
                    execution_deadline,
                    operation_deadline,
                    poll_interval,
                )))
            });
        match worker {
            Ok(worker) => match worker.join() {
                Ok(Ok(outcome)) => outcome,
                Ok(Err(error)) => runner_probe_internal_error(error),
                Err(_) => runner_probe_internal_error("probe runtime thread panicked".to_string()),
            },
            Err(error) => runner_probe_internal_error(format!(
                "failed to start isolated probe runtime thread: {error}"
            )),
        }
    })
}

fn runner_probe_internal_error(error: String) -> HostRunnerProbeOutcome {
    HostRunnerProbeOutcome {
        success: false,
        exit_code: None,
        stdout: String::new(),
        stderr: String::new(),
        timed_out: false,
        error: Some(error),
    }
}

async fn run_runner_probe_async(
    mut process: std::process::Command,
    env_vars: &HashMap<String, String>,
    execution_deadline: Instant,
    operation_deadline: Instant,
    poll_interval: Duration,
) -> HostRunnerProbeOutcome {
    let mut process_tree = match RunnerProbeProcessTree::prepare(&mut process) {
        Ok(process_tree) => process_tree,
        Err(()) => {
            return runner_probe_internal_error(
                "failed to establish runner health probe process-tree ownership".to_string(),
            );
        }
    };
    let mut process = tokio::process::Command::from(process);
    process.kill_on_drop(true);
    let mut child = match process.spawn() {
        Ok(child) => child,
        Err(error) => {
            let message = format!("[gwt] failed to start runner health probe: {error}");
            return HostRunnerProbeOutcome {
                success: false,
                exit_code: None,
                stdout: String::new(),
                stderr: message,
                timed_out: false,
                error: Some(error.to_string()),
            };
        }
    };
    let Some(process_id) = child.id() else {
        let tree_terminated = process_tree.terminate();
        let direct_child_reaped =
            stop_and_reap_runner_probe_child(&mut child, operation_deadline).await;
        let cleanup = RunnerProbeCleanup {
            process_tree_terminated: tree_terminated,
            direct_child_reaped,
            readers_released: true,
        };
        let error = with_runner_probe_cleanup_error(
            "runner health probe did not expose a process identifier".to_string(),
            cleanup,
        );
        return runner_probe_internal_error(error);
    };
    if process_tree.after_spawn(process_id).is_err() {
        let tree_terminated = process_tree.terminate();
        let direct_child_reaped =
            stop_and_reap_runner_probe_child(&mut child, operation_deadline).await;
        let cleanup = RunnerProbeCleanup {
            process_tree_terminated: tree_terminated,
            direct_child_reaped,
            readers_released: true,
        };
        let error = if cleanup.complete() {
            "failed to assign runner health probe process-tree ownership".to_string()
        } else {
            format!(
                "failed to assign runner health probe process-tree ownership; {}",
                cleanup.error_message()
            )
        };
        return runner_probe_internal_error(error);
    }
    let captured_stdout = Arc::new(Mutex::new(RunnerProbeCapture::default()));
    let captured_stderr = Arc::new(Mutex::new(RunnerProbeCapture::default()));
    let mut stdout_reader = child
        .stdout
        .take()
        .map(|stdout| spawn_runner_probe_stream_capture(stdout, Arc::clone(&captured_stdout)));
    let mut stderr_reader = child
        .stderr
        .take()
        .map(|stderr| spawn_runner_probe_stream_capture(stderr, Arc::clone(&captured_stderr)));

    let completion = tokio::time::timeout_at(
        tokio::time::Instant::from_std(execution_deadline),
        await_runner_probe_completion(
            &mut child,
            &mut stdout_reader,
            &mut stderr_reader,
            execution_deadline,
            poll_interval,
        ),
    )
    .await;
    match completion {
        Ok(Ok(status)) => {
            let cleanup = cleanup_runner_probe_process(
                &mut child,
                &mut process_tree,
                operation_deadline,
                &mut stdout_reader,
                &mut stderr_reader,
            )
            .await;
            let cleanup_complete = cleanup.complete();
            HostRunnerProbeOutcome {
                success: status.success() && cleanup_complete,
                exit_code: status.code(),
                stdout: captured_runner_probe_string(&captured_stdout, env_vars),
                stderr: captured_runner_probe_string(&captured_stderr, env_vars),
                timed_out: false,
                error: (!cleanup_complete).then(|| cleanup.error_message()),
            }
        }
        Ok(Err(RunnerProbeWaitError::Failure(error))) => {
            let cleanup = cleanup_runner_probe_process(
                &mut child,
                &mut process_tree,
                operation_deadline,
                &mut stdout_reader,
                &mut stderr_reader,
            )
            .await;
            HostRunnerProbeOutcome {
                success: false,
                exit_code: None,
                stdout: captured_runner_probe_string(&captured_stdout, env_vars),
                stderr: captured_runner_probe_string(&captured_stderr, env_vars),
                timed_out: false,
                error: Some(with_runner_probe_cleanup_error(error, cleanup)),
            }
        }
        Ok(Err(RunnerProbeWaitError::Deadline)) | Err(_) => {
            let cleanup = cleanup_runner_probe_process(
                &mut child,
                &mut process_tree,
                operation_deadline,
                &mut stdout_reader,
                &mut stderr_reader,
            )
            .await;
            HostRunnerProbeOutcome {
                success: false,
                exit_code: None,
                stdout: captured_runner_probe_string(&captured_stdout, env_vars),
                stderr: captured_runner_probe_string(&captured_stderr, env_vars),
                timed_out: true,
                error: (!cleanup.complete()).then(|| cleanup.error_message()),
            }
        }
    }
}

fn with_runner_probe_cleanup_error(error: String, cleanup: RunnerProbeCleanup) -> String {
    if cleanup.complete() {
        error
    } else {
        format!("{error}; {}", cleanup.error_message())
    }
}

#[cfg(unix)]
struct RunnerProbeProcessTree {
    process_group: Option<libc::pid_t>,
}

#[cfg(unix)]
impl RunnerProbeProcessTree {
    fn prepare(command: &mut std::process::Command) -> Result<Self, ()> {
        use std::os::unix::process::CommandExt;

        command.process_group(0);
        Ok(Self {
            process_group: None,
        })
    }

    fn after_spawn(&mut self, process_id: u32) -> Result<(), ()> {
        self.process_group = Some(process_id as libc::pid_t);
        Ok(())
    }

    fn terminate(&mut self) -> bool {
        let Some(process_group) = self.process_group.take() else {
            return true;
        };
        // SAFETY: the command was configured with process_group(0) before
        // spawn, so the negative PID targets only the probe process group.
        let result = unsafe { libc::kill(-process_group, libc::SIGKILL) };
        result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
    }
}

#[cfg(windows)]
struct RunnerProbeProcessTree {
    job: gwt_core::process_tree::WindowsJobObject,
}

#[cfg(windows)]
impl RunnerProbeProcessTree {
    fn prepare(command: &mut std::process::Command) -> Result<Self, ()> {
        let job = gwt_core::process_tree::WindowsJobObject::new().map_err(|_| ())?;
        gwt_core::process_tree::WindowsJobObject::configure_suspended(command);
        Ok(Self { job })
    }

    fn after_spawn(&mut self, process_id: u32) -> Result<(), ()> {
        self.job.assign_and_resume(process_id).map_err(|_| ())
    }

    fn terminate(&mut self) -> bool {
        self.job.terminate()
    }
}

#[cfg(not(any(unix, windows)))]
struct RunnerProbeProcessTree;

#[cfg(not(any(unix, windows)))]
impl RunnerProbeProcessTree {
    fn prepare(_command: &mut std::process::Command) -> Result<Self, ()> {
        Ok(Self)
    }

    fn after_spawn(&mut self, _process_id: u32) -> Result<(), ()> {
        Err(())
    }

    fn terminate(&mut self) -> bool {
        false
    }
}

impl Drop for RunnerProbeProcessTree {
    fn drop(&mut self) {
        let _ = self.terminate();
    }
}

type RunnerProbeReaderTask = tokio::task::JoinHandle<Result<(), std::io::ErrorKind>>;

enum RunnerProbeWaitError {
    Deadline,
    Failure(String),
}

async fn await_runner_probe_completion(
    child: &mut tokio::process::Child,
    stdout: &mut Option<RunnerProbeReaderTask>,
    stderr: &mut Option<RunnerProbeReaderTask>,
    deadline: Instant,
    poll_interval: Duration,
) -> Result<ExitStatus, RunnerProbeWaitError> {
    let mut exit_status = None;
    loop {
        if Instant::now() >= deadline {
            return Err(RunnerProbeWaitError::Deadline);
        }
        if exit_status.is_none() {
            exit_status = child
                .try_wait()
                .map_err(|error| RunnerProbeWaitError::Failure(error.to_string()))?;
        }
        if runner_probe_streams_finished(stdout, stderr) {
            if let Some(status) = exit_status.take() {
                if Instant::now() >= deadline {
                    return Err(RunnerProbeWaitError::Deadline);
                }
                join_runner_probe_streams(stdout, stderr)
                    .await
                    .map_err(RunnerProbeWaitError::Failure)?;
                if Instant::now() >= deadline {
                    return Err(RunnerProbeWaitError::Deadline);
                }
                return Ok(status);
            }
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        tokio::time::sleep(poll_interval.min(remaining)).await;
    }
}

fn runner_probe_streams_finished(
    stdout: &Option<RunnerProbeReaderTask>,
    stderr: &Option<RunnerProbeReaderTask>,
) -> bool {
    stdout
        .as_ref()
        .is_none_or(tokio::task::JoinHandle::is_finished)
        && stderr
            .as_ref()
            .is_none_or(tokio::task::JoinHandle::is_finished)
}

async fn join_runner_probe_stream(
    reader: &mut Option<RunnerProbeReaderTask>,
    stream: &str,
) -> Result<(), String> {
    match reader.take() {
        Some(reader) => match reader.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(kind)) => Err(format!(
                "runner probe {stream} read failed ({kind:?}); check the runner executable and retry"
            )),
            Err(error) if error.is_cancelled() => Err(format!(
                "runner probe {stream} reader was cancelled unexpectedly; retry the launch"
            )),
            Err(_) => Err(format!(
                "runner probe {stream} reader terminated unexpectedly; retry the launch"
            )),
        },
        None => Ok(()),
    }
}

async fn join_runner_probe_streams(
    stdout: &mut Option<RunnerProbeReaderTask>,
    stderr: &mut Option<RunnerProbeReaderTask>,
) -> Result<(), String> {
    let (stdout_result, stderr_result) = tokio::join!(
        join_runner_probe_stream(stdout, "stdout"),
        join_runner_probe_stream(stderr, "stderr")
    );
    match (stdout_result, stderr_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(stdout_error), Err(stderr_error)) => Err(format!("{stdout_error}; {stderr_error}")),
    }
}

async fn abort_and_join_runner_probe_stream(reader: &mut Option<RunnerProbeReaderTask>) {
    if let Some(reader) = reader.take() {
        reader.abort();
        let _ = reader.await;
    }
}

#[derive(Debug, Clone, Copy)]
struct RunnerProbeCleanup {
    process_tree_terminated: bool,
    direct_child_reaped: bool,
    readers_released: bool,
}

impl RunnerProbeCleanup {
    fn complete(self) -> bool {
        self.process_tree_terminated && self.direct_child_reaped && self.readers_released
    }

    fn error_message(self) -> String {
        let mut failures = Vec::new();
        if !self.process_tree_terminated {
            failures.push("process tree termination failed");
        }
        if !self.direct_child_reaped {
            failures.push("direct child reap exceeded deadline");
        }
        if !self.readers_released {
            failures.push("output reader release failed");
        }
        format!("probe cleanup incomplete: {}", failures.join(", "))
    }
}

async fn cleanup_runner_probe_process(
    child: &mut tokio::process::Child,
    process_tree: &mut RunnerProbeProcessTree,
    deadline: Instant,
    stdout: &mut Option<RunnerProbeReaderTask>,
    stderr: &mut Option<RunnerProbeReaderTask>,
) -> RunnerProbeCleanup {
    let process_tree_terminated = process_tree.terminate();
    let process_tree_cleanup = std::future::ready(process_tree_terminated);
    let direct_child_cleanup = stop_and_reap_runner_probe_child(child, deadline);
    finish_runner_probe_cleanup(process_tree_cleanup, direct_child_cleanup, stdout, stderr).await
}

async fn finish_runner_probe_cleanup<TreeCleanup, ChildCleanup>(
    process_tree_cleanup: TreeCleanup,
    direct_child_cleanup: ChildCleanup,
    stdout: &mut Option<RunnerProbeReaderTask>,
    stderr: &mut Option<RunnerProbeReaderTask>,
) -> RunnerProbeCleanup
where
    TreeCleanup: std::future::Future<Output = bool>,
    ChildCleanup: std::future::Future<Output = bool>,
{
    let reader_cleanup = async {
        tokio::join!(
            abort_and_join_runner_probe_stream(stdout),
            abort_and_join_runner_probe_stream(stderr)
        );
        true
    };
    let (process_tree_terminated, direct_child_reaped, readers_released) =
        tokio::join!(process_tree_cleanup, direct_child_cleanup, reader_cleanup);
    RunnerProbeCleanup {
        process_tree_terminated,
        direct_child_reaped,
        readers_released,
    }
}

async fn stop_and_reap_runner_probe_child(
    child: &mut tokio::process::Child,
    deadline: Instant,
) -> bool {
    match child.try_wait() {
        Ok(Some(_)) => return true,
        Ok(None) => {
            // Closing a Windows Job may win this race and make start_kill
            // report an already-terminated process. Waiting is still the
            // authoritative reap operation, so do not fail early here.
            let _ = child.start_kill();
        }
        Err(_) => return false,
    }
    tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), child.wait())
        .await
        .is_ok_and(|result| result.is_ok())
}

fn spawn_runner_probe_stream_capture<R>(
    reader: R,
    captured: Arc<Mutex<RunnerProbeCapture>>,
) -> RunnerProbeReaderTask
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut reader = reader;
        let mut buffer = [0_u8; 8 * 1024];
        loop {
            match reader.read(&mut buffer).await {
                Ok(0) => return Ok(()),
                Ok(read) => {
                    if let Ok(mut captured) = captured.lock() {
                        captured.append(&buffer[..read]);
                    }
                }
                Err(error) => return Err(error.kind()),
            }
        }
    })
}

fn captured_runner_probe_string(
    captured: &Arc<Mutex<RunnerProbeCapture>>,
    env_vars: &HashMap<String, String>,
) -> String {
    captured
        .lock()
        .map(|value| value.render(env_vars))
        .unwrap_or_else(|_| String::new())
}

fn publish_runner_probe_capture(
    hub: &gwt_core::process_console::ProcessConsoleHub,
    spawn_id: u64,
    stdout: &str,
    stderr: &str,
) {
    for (stream, output) in [
        (gwt_core::process_console::ProcessStream::Stdout, stdout),
        (gwt_core::process_console::ProcessStream::Stderr, stderr),
    ] {
        for piece in output.split(['\n', '\r']).filter(|piece| !piece.is_empty()) {
            push_runner_probe_console_line(hub, spawn_id, stream, piece);
        }
    }
}

fn push_runner_probe_console_line(
    hub: &gwt_core::process_console::ProcessConsoleHub,
    spawn_id: u64,
    stream: gwt_core::process_console::ProcessStream,
    message: &str,
) {
    if message.is_empty() {
        return;
    }
    let stripped = gwt_core::process_console::strip_ansi(message);
    let redacted = gwt_core::process_console::redact_line(&stripped);
    hub.push(gwt_core::process_console::ProcessLine::new(
        gwt_core::process_console::ProcessKind::AgentBootstrap,
        spawn_id,
        stream,
        redacted,
    ));
}

/// Name the executable a failed launch could not find (Issue #4445 AC-2).
///
/// A Windows npm shim prints its quoted target path and then the shell's own
/// "not recognized" message, which is localized while the path is not. Naming
/// the path turns a `Process exited with status 1` report into a one-step
/// diagnosis instead of a directory inspection.
pub fn missing_launcher_binary_detail(tail: &str) -> Option<String> {
    missing_launcher_binary_detail_with(tail, |candidate| {
        Path::new(candidate).symlink_metadata().is_ok()
    })
}

fn missing_launcher_binary_detail_with(
    tail: &str,
    exists: impl Fn(&str) -> bool,
) -> Option<String> {
    tail.split('"')
        .skip(1)
        .step_by(2)
        .map(str::trim)
        .find(|candidate| is_absolute_windows_executable(candidate) && !exists(candidate))
        .map(|missing| {
            format!(
                "Launcher binary not found: {missing} — the package cache no longer contains the executable its launcher points at."
            )
        })
}

fn is_absolute_windows_executable(candidate: &str) -> bool {
    let mut characters = candidate.chars();
    characters
        .next()
        .is_some_and(|drive| drive.is_ascii_alphabetic())
        && characters.next() == Some(':')
        && matches!(characters.next(), Some('\\' | '/'))
        && candidate.to_ascii_lowercase().ends_with(".exe")
}

static AGENT_SPAWN_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn next_agent_spawn_id() -> u64 {
    AGENT_SPAWN_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

#[cfg(test)]
fn command_matches_runner(command: &str, runner: &str) -> bool {
    let path = Path::new(command);
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .or_else(|| path.file_name().and_then(|name| name.to_str()))
        .is_some_and(|name| name.eq_ignore_ascii_case(runner))
}

#[cfg(test)]
fn ensure_docker_launch_runtime_ready() -> Result<(), String> {
    let docker_bin = std::env::var("GWT_DOCKER_BIN").unwrap_or_else(|_| "docker".to_string());
    ensure_docker_launch_runtime_ready_with(&docker_bin, gwt_docker::launch_preflight)
}

fn ensure_docker_launch_runtime_ready_for_runtime(
    runtime: &gwt_docker::detect::ResolvedContainerRuntime,
) -> Result<(), String> {
    ensure_docker_launch_runtime_ready_with(runtime.binary(), || {
        gwt_docker::launch_preflight_for_resolved_runtime(runtime)
    })
}

fn ensure_docker_launch_runtime_ready_with(
    attempted_binary: &str,
    preflight: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let path = std::env::var("PATH").unwrap_or_default();
    tracing::info!(
        target: "gwt::launch::preflight",
        runtime_target = "docker",
        attempted_binary,
        path = %path,
        "docker preflight started"
    );
    let result = preflight();
    match &result {
        Ok(()) => {
            tracing::info!(
                target: "gwt::launch::preflight",
                runtime_target = "docker",
                outcome = "ready",
                attempted_binary,
                "docker preflight completed"
            );
        }
        Err(error) => {
            tracing::error!(
                target: "gwt::launch::preflight",
                runtime_target = "docker",
                outcome = "failed",
                attempted_binary,
                path = %path,
                error = %error,
                "docker preflight failed"
            );
        }
    }
    result
}

fn docker_bundle_mounts_for_gwt_home(gwt_home: &Path) -> DockerBundleMounts {
    let gwt_bin_dir = gwt_home.join("bin");
    DockerBundleMounts {
        host_gwt: gwt_bin_dir.join(DOCKER_HOST_GWT_BIN_NAME),
        host_gwtd: gwt_bin_dir.join(DOCKER_HOST_GWTD_BIN_NAME),
    }
}

#[cfg(test)]
fn docker_bundle_mounts_for_home(home: &Path) -> DockerBundleMounts {
    docker_bundle_mounts_for_gwt_home(&home.join(".gwt"))
}

fn docker_compose_mount_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
fn docker_bundle_override_content(
    service: &str,
    bundle: &DockerBundleMounts,
    container_runtime_binary: &str,
) -> Result<String, String> {
    let runtime = gwt_docker::container_runtime_kind(container_runtime_binary)?;
    Ok(docker_bundle_override_content_for_runtime(
        service, bundle, runtime,
    ))
}

fn docker_bundle_override_content_for_runtime(
    service: &str,
    bundle: &DockerBundleMounts,
    runtime: gwt_docker::ContainerRuntimeKind,
) -> String {
    let host_gwtd = docker_compose_mount_path(&bundle.host_gwtd);
    let extra_hosts = runtime
        .compose_extra_host()
        .map(|mapping| format!("    extra_hosts:\n      - \"{mapping}\"\n"))
        .unwrap_or_default();
    format!(
        concat!(
            "{header}\n",
            "services:\n",
            "  {service}:\n",
            "    volumes:\n",
            "      - \"{host_gwtd}:{path}:ro\"\n",
            "{extra_hosts}"
        ),
        header = DOCKER_GWT_OVERRIDE_HEADER,
        service = service,
        host_gwtd = host_gwtd,
        path = DOCKER_GWTD_BIN_PATH,
        extra_hosts = extra_hosts,
    )
}

fn ensure_docker_gwt_binary_setup(
    repo_path: &Path,
    service: &str,
    target_arch: &str,
    runtime: gwt_docker::ContainerRuntimeKind,
) -> Result<(PathBuf, bool), String> {
    let gwt_home = gwt_core::paths::gwt_home();
    ensure_docker_gwt_binary_setup_for_gwt_home(repo_path, service, &gwt_home, runtime, |bundle| {
        eprintln!(
            "Installing Linux gwt bundle for Docker at {} and {}",
            bundle.host_gwt.display(),
            bundle.host_gwtd.display()
        );
        let installed = gwt_core::update::UpdateManager::new().install_latest_docker_linux_bundle(
            target_arch,
            &bundle.host_gwt,
            &bundle.host_gwtd,
        )?;
        eprintln!(
            "Installed Linux gwt bundle v{} for Docker",
            installed.version
        );
        Ok(())
    })
}

fn docker_compose_override_path(repo_path: &Path) -> PathBuf {
    repo_path.join(DOCKER_GWT_OVERRIDE_FILE_NAME)
}

fn docker_compose_user_override_path(repo_path: &Path) -> PathBuf {
    repo_path.join(DOCKER_USER_OVERRIDE_FILE_NAME)
}

fn is_legacy_gwt_generated_override(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .is_ok_and(|content| content.starts_with(DOCKER_GWT_OVERRIDE_HEADER))
}

#[cfg(test)]
fn ensure_docker_gwt_binary_setup_for_home<F>(
    repo_path: &Path,
    service: &str,
    home: &Path,
    container_runtime_binary: &str,
    install_bundle: F,
) -> Result<(PathBuf, bool), String>
where
    F: FnMut(&DockerBundleMounts) -> Result<(), String>,
{
    let gwt_home = home.join(".gwt");
    let runtime = gwt_docker::container_runtime_kind(container_runtime_binary)?;
    ensure_docker_gwt_binary_setup_for_gwt_home(
        repo_path,
        service,
        &gwt_home,
        runtime,
        install_bundle,
    )
}

fn ensure_docker_gwt_binary_setup_for_gwt_home<F>(
    repo_path: &Path,
    service: &str,
    gwt_home: &Path,
    runtime: gwt_docker::ContainerRuntimeKind,
    mut install_bundle: F,
) -> Result<(PathBuf, bool), String>
where
    F: FnMut(&DockerBundleMounts) -> Result<(), String>,
{
    use std::fs;

    let bundle = docker_bundle_mounts_for_gwt_home(gwt_home);

    if !docker_bundle_binary_ready(&bundle.host_gwt)
        || !docker_bundle_binary_ready(&bundle.host_gwtd)
    {
        install_bundle(&bundle).map_err(|err| {
            format!(
                "Failed to install Linux gwt bundle for Docker: {err}\n\
                 Expected cached binaries at {} and {}",
                bundle.host_gwt.display(),
                bundle.host_gwtd.display()
            )
        })?;
    }

    if !docker_bundle_binary_ready(&bundle.host_gwt)
        || !docker_bundle_binary_ready(&bundle.host_gwtd)
    {
        return Err(format!(
            "Linux gwt bundle setup did not create expected Docker binaries at {} and {}",
            bundle.host_gwt.display(),
            bundle.host_gwtd.display()
        ));
    }

    let override_path = docker_compose_override_path(repo_path);
    let override_content = docker_bundle_override_content_for_runtime(service, &bundle, runtime);
    let rewrite_override = fs::read_to_string(&override_path)
        .map(|existing| existing != override_content)
        .unwrap_or(true);
    if rewrite_override {
        fs::write(&override_path, override_content).map_err(|err| {
            format!(
                "Failed to write generated Docker compose override: {err}\n\
                 Manually create {} with gwt/gwtd bundle mounts",
                override_path.display()
            )
        })?;
    }

    Ok((override_path, rewrite_override))
}

fn docker_bundle_binary_ready(path: &Path) -> bool {
    path.metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0)
}

fn maybe_inject_docker_sandbox_env(
    launch: &DockerLaunchPlan,
    config: &mut LaunchConfig,
) -> Result<(), String> {
    if cfg!(windows) || !matches!(config.agent_id, AgentId::ClaudeCode) || !config.skip_permissions
    {
        return Ok(());
    }

    let is_root =
        gwt_docker::compose_service_user_is_root_with_files(&launch.compose_files, &launch.service)
            .map_err(|err| {
                format!(
                    "Failed to determine Docker user for service '{}': {err}",
                    launch.service
                )
            })?;
    if is_root {
        config
            .env_vars
            .insert("IS_SANDBOX".to_string(), "1".to_string());
    }
    Ok(())
}

fn docker_compose_exec_env_args(env_vars: &HashMap<String, String>) -> Vec<String> {
    let mut keys = env_vars.keys().collect::<Vec<_>>();
    keys.sort();

    let mut args = Vec::new();
    for key in keys {
        let key = key.trim();
        if key.is_empty() || !is_valid_docker_env_key(key) {
            continue;
        }
        // Never override the container's PATH: the image PATH carries the
        // in-container toolchain locations (e.g. /root/.bun/bin for bunx).
        // Injecting the host-assembled PATH made the post-probe agent exec
        // fail with `exec: "bunx": executable file not found in $PATH`
        // while the env-free probe succeeded (Issue #3029).
        if key.eq_ignore_ascii_case("PATH") {
            continue;
        }
        args.push("-e".to_string());
        if private_launch_env_key(key) {
            args.push(key.to_string());
        } else {
            let value = env_vars.get(key).map(String::as_str).unwrap_or_default();
            args.push(format!("{key}={value}"));
        }
    }
    args
}

fn is_valid_docker_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

fn resolve_docker_exec_program(
    launch: &DockerLaunchPlan,
    config: &LaunchConfig,
) -> Result<DockerExecProgram, String> {
    ensure_docker_launch_command_ready(launch, &config.command)?;
    Ok(DockerExecProgram {
        executable: config.command.clone(),
        args: config.args.clone(),
    })
}

fn ensure_docker_launch_command_ready(
    launch: &DockerLaunchPlan,
    command: &str,
) -> Result<(), String> {
    let available = gwt_docker::compose_service_has_command_with_files(
        &launch.compose_files,
        &launch.service,
        command,
    )
    .map_err(|err| err.to_string())?;
    if available {
        Ok(())
    } else {
        Err(format!(
            "Command '{command}' is not available in Docker service '{}'",
            launch.service
        ))
    }
}

fn ensure_docker_launch_service_ready(
    launch: &DockerLaunchPlan,
    intent: DockerLifecycleIntent,
) -> Result<(), String> {
    let status =
        gwt_docker::compose_service_status_with_files(&launch.compose_files, &launch.service)
            .map_err(|err| err.to_string())?;
    match normalize_docker_launch_action(intent, status) {
        DockerLaunchServiceAction::Connect => Ok(()),
        DockerLaunchServiceAction::Start => {
            gwt_docker::compose_up_with_files(&launch.compose_files, &launch.service)
                .map_err(|err| err.to_string())?;
            Ok(())
        }
        DockerLaunchServiceAction::Restart => {
            gwt_docker::compose_restart_with_files(&launch.compose_files, &launch.service)
                .map_err(|err| err.to_string())
        }
        DockerLaunchServiceAction::Recreate => {
            gwt_docker::compose_up_force_recreate_with_files(&launch.compose_files, &launch.service)
                .map_err(|err| err.to_string())
        }
    }
}

fn normalize_docker_launch_action(
    intent: DockerLifecycleIntent,
    status: gwt_docker::ComposeServiceStatus,
) -> DockerLaunchServiceAction {
    use gwt_docker::ComposeServiceStatus;

    match intent {
        DockerLifecycleIntent::Recreate => DockerLaunchServiceAction::Recreate,
        DockerLifecycleIntent::Restart if status == ComposeServiceStatus::Running => {
            DockerLaunchServiceAction::Restart
        }
        DockerLifecycleIntent::Connect
        | DockerLifecycleIntent::Start
        | DockerLifecycleIntent::Restart
        | DockerLifecycleIntent::CreateAndStart => match status {
            ComposeServiceStatus::Running => DockerLaunchServiceAction::Connect,
            ComposeServiceStatus::Unknown
            | ComposeServiceStatus::Stopped
            | ComposeServiceStatus::Exited
            | ComposeServiceStatus::NotFound => DockerLaunchServiceAction::Start,
        },
    }
}

fn resolve_docker_launch_plan(
    worktree: &Path,
    selected_service: Option<&str>,
) -> Result<DockerLaunchPlan, String> {
    let files = gwt_docker::detect_docker_files(worktree);
    let compose_file = docker_compose_file_for_launch(worktree, &files)?.ok_or_else(|| {
        "Docker launch requires a docker-compose.yml or devcontainer compose target".to_string()
    })?;
    let services = gwt_docker::parse_compose_file(&compose_file).map_err(|err| err.to_string())?;
    if services.is_empty() {
        return Err("Docker launch requires at least one compose service".to_string());
    }

    let devcontainer_defaults = docker_devcontainer_defaults(worktree, &files);
    let service_name = selected_service
        .map(str::to_string)
        .or_else(|| {
            devcontainer_defaults
                .as_ref()
                .and_then(|defaults| defaults.service.clone())
        })
        .or_else(|| {
            if services.len() == 1 {
                services.first().map(|service| service.name.clone())
            } else {
                None
            }
        })
        .ok_or_else(|| {
            "Multiple Docker services detected; select a Docker service in Launch Agent Wizard"
                .to_string()
        })?;

    let service = services
        .iter()
        .find(|service| service.name == service_name)
        .ok_or_else(|| {
            format!("Selected Docker service was not found in compose file: {service_name}")
        })?;

    let container_cwd = devcontainer_defaults
        .as_ref()
        .and_then(|defaults| defaults.workspace_folder.clone())
        .or_else(|| service.working_dir.clone())
        .or_else(|| compose_workspace_mount_target(worktree, service))
        .ok_or_else(|| {
            format!(
                "Docker service {} is missing working_dir/workspaceFolder and no project-root volume mount was detected",
                service.name
            )
        })?;

    Ok(DockerLaunchPlan {
        compose_files: docker_launch_compose_files(worktree, &compose_file),
        service: service.name.clone(),
        container_cwd,
        target_arch: docker_bundle_target_arch(service)?,
    })
}

fn docker_binary_for_launch() -> String {
    std::env::var("GWT_DOCKER_BIN").unwrap_or_else(|_| "docker".to_string())
}

fn docker_bundle_target_arch(service: &gwt_docker::ComposeService) -> Result<String, String> {
    if let Some(platform) = service.platform.as_deref() {
        return docker_platform_target_arch(platform).ok_or_else(|| {
            format!(
                "Docker service {} specifies unsupported platform {}; expected x86_64/amd64 or aarch64/arm64",
                service.name, platform
            )
        });
    }
    Ok(host_docker_target_arch())
}

fn docker_platform_target_arch(platform: &str) -> Option<String> {
    let platform = platform.trim();
    let arch = platform
        .split('/')
        .nth(1)
        .filter(|value| !value.is_empty())
        .unwrap_or(platform);
    normalize_docker_target_arch(arch)
}

fn host_docker_target_arch() -> String {
    normalize_docker_target_arch(std::env::consts::ARCH)
        .unwrap_or_else(|| std::env::consts::ARCH.to_string())
}

fn normalize_docker_target_arch(raw: &str) -> Option<String> {
    match raw
        .trim()
        .to_ascii_lowercase()
        .split('/')
        .next()
        .unwrap_or_default()
    {
        "x86_64" | "amd64" | "x64" => Some("x86_64".to_string()),
        "aarch64" | "arm64" => Some("aarch64".to_string()),
        _ => None,
    }
}

fn docker_launch_compose_files(worktree: &Path, compose_file: &Path) -> Vec<PathBuf> {
    let mut files = vec![compose_file.to_path_buf()];
    let user_override_file = docker_compose_user_override_path(worktree);
    if user_override_file.is_file() && !is_legacy_gwt_generated_override(&user_override_file) {
        files.push(user_override_file);
    }
    let generated_override_file = docker_compose_override_path(worktree);
    if generated_override_file.is_file() {
        files.push(generated_override_file);
    }
    files
}

fn docker_compose_command_prefix(launch: &DockerLaunchPlan) -> Vec<String> {
    let mut args = vec!["compose".to_string()];
    for compose_file in &launch.compose_files {
        args.push("-f".to_string());
        args.push(compose_file.display().to_string());
    }
    args
}

fn docker_compose_file_for_launch(
    project_root: &Path,
    files: &gwt_docker::DockerFiles,
) -> Result<Option<PathBuf>, String> {
    Ok(docker_devcontainer_defaults(project_root, files)
        .and_then(|defaults| defaults.compose_file)
        .or_else(|| files.compose_file.clone()))
}

fn docker_devcontainer_defaults(
    project_root: &Path,
    files: &gwt_docker::DockerFiles,
) -> Option<DevContainerLaunchDefaults> {
    let devcontainer_dir = files.devcontainer_dir.as_ref()?;
    let path = devcontainer_dir.join("devcontainer.json");
    if !path.is_file() {
        return None;
    }

    let config = gwt_docker::DevContainerConfig::load(&path).ok()?;
    let compose_file = config
        .docker_compose_file
        .as_ref()
        .and_then(|value| {
            value
                .to_vec()
                .into_iter()
                .map(|candidate| devcontainer_dir.join(candidate))
                .find(|path| path.is_file())
        })
        .or_else(|| files.compose_file.clone())
        .or_else(|| {
            let fallback = project_root.join("docker-compose.yml");
            fallback.is_file().then_some(fallback)
        });

    Some(DevContainerLaunchDefaults {
        service: config.service,
        workspace_folder: config.workspace_folder,
        compose_file,
    })
}

fn compose_workspace_mount_target(
    project_root: &Path,
    service: &gwt_docker::ComposeService,
) -> Option<String> {
    service
        .volumes
        .iter()
        .find(|mount| mount_source_matches_project_root(&mount.source, project_root))
        .map(|mount| mount.target.clone())
}

fn mount_source_matches_project_root(source: &str, project_root: &Path) -> bool {
    let normalized = source
        .trim()
        .trim_end_matches(['/', '\\'])
        .trim_end_matches("/.");

    if matches!(normalized, "." | "$PWD" | "${PWD}") {
        return true;
    }

    let source_path = Path::new(normalized);
    source_path.is_absolute() && same_path(source_path, project_root)
}

fn first_available_worktree_path(
    preferred_path: &Path,
    worktrees: &[gwt_git::WorktreeInfo],
) -> Option<PathBuf> {
    if !worktree_path_is_occupied(preferred_path, worktrees) && !preferred_path.exists() {
        return Some(preferred_path.to_path_buf());
    }

    for suffix in 2usize.. {
        let candidate = suffixed_worktree_path(preferred_path, suffix)?;
        if !worktree_path_is_occupied(&candidate, worktrees) && !candidate.exists() {
            return Some(candidate);
        }
    }

    None
}

fn suffixed_worktree_path(path: &Path, suffix: usize) -> Option<PathBuf> {
    let file_name = path.file_name()?.to_str()?;
    let mut candidate = path.to_path_buf();
    candidate.set_file_name(format!("{file_name}-{suffix}"));
    Some(candidate)
}

fn worktree_path_is_occupied(path: &Path, worktrees: &[gwt_git::WorktreeInfo]) -> bool {
    worktrees
        .iter()
        .any(|worktree| same_path(&worktree.path, path))
}

fn usable_worktree_path_for_branch(
    worktrees: &[gwt_git::WorktreeInfo],
    branch_name: &str,
) -> Option<PathBuf> {
    worktrees
        .iter()
        .find(|worktree| {
            worktree.branch.as_deref() == Some(branch_name) && usable_worktree_entry(worktree)
        })
        .map(|worktree| worktree.path.clone())
}

fn worktrees_have_stale_branch_entry(
    worktrees: &[gwt_git::WorktreeInfo],
    branch_name: &str,
) -> bool {
    worktrees.iter().any(|worktree| {
        worktree.branch.as_deref() == Some(branch_name) && !usable_worktree_entry(worktree)
    })
}

fn usable_worktree_entry(worktree: &gwt_git::WorktreeInfo) -> bool {
    !worktree.prunable && worktree.path.exists()
}

fn origin_remote_ref(branch_name: &str) -> String {
    if let Some(ref_name) = branch_name.strip_prefix("refs/remotes/") {
        ref_name.to_string()
    } else if branch_name.starts_with("origin/") {
        branch_name.to_string()
    } else {
        format!("origin/{branch_name}")
    }
}

fn refallback_start_work_base_branch<E>(
    branch_name: &str,
    selected_base_branch: &str,
    mut remote_branch_exists: impl FnMut(&str) -> Result<bool, E>,
) -> Result<Option<String>, E> {
    if !is_start_work_branch_name(branch_name)
        || !START_WORK_BASE_BRANCH_CANDIDATES.contains(&selected_base_branch)
    {
        return Ok(None);
    }
    if remote_branch_exists(selected_base_branch)? {
        return Ok(Some(selected_base_branch.to_string()));
    }
    for candidate in START_WORK_BASE_BRANCH_CANDIDATES {
        if candidate == selected_base_branch {
            continue;
        }
        if remote_branch_exists(candidate)? {
            return Ok(Some(candidate.to_string()));
        }
    }
    Ok(None)
}

fn is_start_work_branch_name(branch_name: &str) -> bool {
    branch_name
        .strip_prefix("work/")
        .is_some_and(|name| !name.is_empty())
}

fn local_branch_exists(repo_path: &Path, branch_name: &str) -> Result<bool, String> {
    let output = gwt_core::process::hidden_command("git")
        .args(["show-ref", "--verify", "--quiet"])
        .arg(format!("refs/heads/{branch_name}"))
        .current_dir(repo_path)
        .status()
        .map_err(|err| format!("git show-ref --verify refs/heads/{branch_name}: {err}"))?;
    Ok(output.success())
}

fn should_prefer_path_gwt(current_exe: &Path) -> bool {
    is_bunx_temp_executable(current_exe) || !is_named_gwtd_binary(current_exe)
}

fn is_named_gwt_binary(path: &Path) -> bool {
    normalized_path_segments(path)
        .into_iter()
        .next_back()
        .map(|value| value.trim_end_matches(".exe").to_string())
        .is_some_and(|value| value.eq_ignore_ascii_case("gwt"))
}

fn is_named_gwtd_binary(path: &Path) -> bool {
    normalized_path_segments(path)
        .into_iter()
        .next_back()
        .map(|value| value.trim_end_matches(".exe").to_string())
        .is_some_and(|value| value.eq_ignore_ascii_case("gwtd"))
}

fn is_bunx_temp_executable(path: &Path) -> bool {
    normalized_path_segments(path)
        .into_iter()
        .any(|segment| segment.starts_with("bunx-"))
}

fn normalized_path_segments(path: &Path) -> Vec<String> {
    let normalized = path.to_string_lossy().replace('\\', "/");
    normalized
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(str::to_string)
        .collect()
}

fn same_path(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }

    let left = dunce::canonicalize(left).unwrap_or_else(|_| left.to_path_buf());
    let right = dunce::canonicalize(right).unwrap_or_else(|_| right.to_path_buf());
    left == right
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::custom::{CustomAgentType, CustomCodingAgent};
    use crate::{AgentLaunchBuilder, SessionMode};
    use std::{
        fs,
        sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            Arc, Mutex,
        },
    };
    use tempfile::tempdir;

    fn resolved_test_docker_runtime(
        directory: &Path,
    ) -> gwt_docker::detect::ResolvedContainerRuntime {
        let binary = directory.join("docker");
        gwt_docker::detect::ResolvedContainerRuntime::from_probe_output_for_tests(
            binary.to_str().expect("UTF-8 fake Docker CLI path"),
            gwt_core::process_console::SpawnOutput {
                exit_code: Some(0),
                stdout: "Docker version 28.3.0, build test\n".to_string(),
                stderr: String::new(),
                stdout_lines: 1,
                stderr_lines: 0,
            },
        )
        .expect("resolve fake Docker runtime")
    }

    #[test]
    fn hook_forward_launch_runtime_preserves_host_and_maps_container_aliases() {
        let source = "http://127.0.0.1:45123/internal/hook-live";
        assert_eq!(
            hook_forward_url_for_launch_runtime(source, LaunchRuntimeTarget::Host, None,)
                .expect("host URL"),
            source
        );
        assert_eq!(
            hook_forward_url_for_launch_runtime(
                source,
                LaunchRuntimeTarget::Docker,
                Some(gwt_docker::ContainerRuntimeKind::Docker),
            )
            .expect("Docker URL"),
            "http://host.docker.internal:45123/internal/hook-live"
        );
        assert_eq!(
            hook_forward_url_for_launch_runtime(
                "https://[::1]:46000/internal/hook-live",
                LaunchRuntimeTarget::Docker,
                Some(gwt_docker::ContainerRuntimeKind::Podman),
            )
            .expect("Podman URL"),
            "https://host.containers.internal:46000/internal/hook-live"
        );
    }

    #[test]
    fn hook_forward_launch_runtime_rejects_noncanonical_container_endpoints() {
        for source in [
            "http://example.com:45123/internal/hook-live",
            "http://127.0.0.1/internal/hook-live",
            "http://127.0.0.1:45123/internal/hook-live/subpath",
            "http://127.0.0.1:45123/internal/hook-live?generation=7",
            "http://127.0.0.1:45123/internal/hook-live#fragment",
            "file:///internal/hook-live",
        ] {
            assert!(
                hook_forward_url_for_launch_runtime(
                    source,
                    LaunchRuntimeTarget::Docker,
                    Some(gwt_docker::ContainerRuntimeKind::Docker),
                )
                .is_err(),
                "container endpoint should fail closed: {source}"
            );
        }
    }

    #[test]
    fn pane_websocket_launch_runtime_selects_the_listener_for_each_runtime() {
        let browser = "ws://127.0.0.1:46234/ws";
        let agent = "ws://127.0.0.1:45123/internal/pane-ws";
        assert_eq!(
            pane_websocket_url_for_launch_runtime(browser, agent, LaunchRuntimeTarget::Host, None,)
                .expect("host pane URL"),
            agent
        );
        assert_eq!(
            pane_websocket_url_for_launch_runtime(
                browser,
                agent,
                LaunchRuntimeTarget::Docker,
                Some(gwt_docker::ContainerRuntimeKind::Docker),
            )
            .expect("Docker pane URL"),
            "ws://host.docker.internal:45123/internal/pane-ws"
        );
        assert_eq!(
            pane_websocket_url_for_launch_runtime(
                browser,
                "wss://[::1]:45123/internal/pane-ws",
                LaunchRuntimeTarget::Docker,
                Some(gwt_docker::ContainerRuntimeKind::Podman),
            )
            .expect("Podman pane URL"),
            "wss://host.containers.internal:45123/internal/pane-ws"
        );
    }

    #[test]
    fn pane_websocket_launch_runtime_rejects_noncanonical_container_endpoints() {
        for source in [
            "http://127.0.0.1:45123/internal/pane-ws",
            "ws://127.0.0.1/internal/pane-ws",
            "ws://127.0.0.1:46234/internal/hook-live",
            "ws://127.0.0.1:45123/internal/pane-ws?generation=7",
            "ws://127.0.0.1:45123/internal/pane-ws#fragment",
            "ws://example.test:45123/internal/pane-ws",
        ] {
            assert!(
                pane_websocket_url_for_launch_runtime(
                    "ws://127.0.0.1:46234/ws",
                    source,
                    LaunchRuntimeTarget::Docker,
                    Some(gwt_docker::ContainerRuntimeKind::Docker),
                )
                .is_err(),
                "container pane endpoint should fail closed: {source}"
            );
        }
    }

    #[test]
    fn pane_websocket_launch_runtime_rejects_noncanonical_host_endpoints() {
        for source in [
            "http://127.0.0.1:45123/internal/pane-ws",
            "ws://127.0.0.1/internal/pane-ws",
            "ws://127.0.0.1:46234/ws",
            "ws://127.0.0.1:45123/internal/pane-ws?generation=7",
            "ws://127.0.0.1:45123/internal/pane-ws#fragment",
            "ws://example.test:45123/internal/pane-ws",
            "ws://user@127.0.0.1:45123/internal/pane-ws",
        ] {
            assert!(
                pane_websocket_url_for_launch_runtime(
                    "ws://127.0.0.1:46234/ws",
                    source,
                    LaunchRuntimeTarget::Host,
                    None,
                )
                .is_err(),
                "host pane endpoint should fail closed: {source}"
            );
        }
    }

    #[test]
    fn private_launch_env_values_stay_out_of_docker_argv_and_debug() {
        const TOKEN_SENTINEL: &str = "agent-capability-secret-sentinel";
        const SESSION_SENTINEL: &str = "session-identity-secret-sentinel";
        const RUNTIME_SENTINEL: &str = "/private/runtime/session.json";
        let mut config = AgentLaunchBuilder::new(AgentId::ClaudeCode).build();
        config.runtime_target = LaunchRuntimeTarget::Docker;
        let runtime_dir = tempdir().expect("runtime tempdir");
        let container_runtime = resolved_test_docker_runtime(runtime_dir.path());
        install_hook_forward_env(
            &mut config,
            Some(HookForwardEnv {
                url: "http://localhost:45123/internal/hook-live".to_string(),
                token: TOKEN_SENTINEL.to_string(),
            }),
            Some(&container_runtime),
        )
        .expect("install Docker hook forwarding environment");
        config
            .env_vars
            .insert(GWT_SESSION_ID_ENV.to_string(), SESSION_SENTINEL.to_string());
        config.env_vars.insert(
            GWT_SESSION_RUNTIME_PATH_ENV.to_string(),
            RUNTIME_SENTINEL.to_string(),
        );
        config
            .env_vars
            .insert("TERM".to_string(), "xterm-256color".to_string());
        assert_eq!(
            config
                .env_vars
                .get(GWT_HOOK_FORWARD_URL_ENV)
                .map(String::as_str),
            Some("http://host.docker.internal:45123/internal/hook-live")
        );
        let env = config.env_vars;
        let args = docker_compose_exec_env_args(&env);

        for key in [
            GWT_HOOK_FORWARD_TOKEN_ENV,
            GWT_SESSION_ID_ENV,
            GWT_SESSION_RUNTIME_PATH_ENV,
        ] {
            assert!(args.windows(2).any(|pair| pair == ["-e", key]));
        }
        for private_value in [TOKEN_SENTINEL, SESSION_SENTINEL, RUNTIME_SENTINEL] {
            assert!(!args.iter().any(|arg| arg.contains(private_value)));
        }
        assert!(args.iter().any(|arg| arg == "TERM=xterm-256color"));

        let process_launch = PreparedProcessLaunch {
            command: "docker".to_string(),
            args: vec![
                "compose".to_string(),
                format!("{GWT_HOOK_FORWARD_TOKEN_ENV}={TOKEN_SENTINEL}"),
                format!("{GWT_SESSION_ID_ENV}={SESSION_SENTINEL}"),
            ],
            env,
            remove_env: Vec::new(),
            cwd: None,
        };
        let debug = format!("{process_launch:?}");
        for private_value in [TOKEN_SENTINEL, SESSION_SENTINEL, RUNTIME_SENTINEL] {
            assert!(
                !debug.contains(private_value),
                "private value leaked through Debug: {debug}"
            );
        }
        assert!(debug.contains("<redacted>"));

        let hook_forward = HookForwardEnv {
            url: "http://127.0.0.1:45123/internal/hook-live".to_string(),
            token: TOKEN_SENTINEL.to_string(),
        };
        let debug = format!("{hook_forward:?}");
        assert!(!debug.contains(TOKEN_SENTINEL));
        assert!(debug.contains("<redacted>"));
    }

    #[test]
    fn docker_compose_exec_env_args_does_not_override_container_path() {
        let mut env = HashMap::new();
        env.insert("PATH".to_string(), "/usr/local/bin".to_string());
        env.insert(
            "GWT_BIN_PATH".to_string(),
            "/usr/local/bin/gwtd".to_string(),
        );
        let args = docker_compose_exec_env_args(&env);
        assert!(
            !args.iter().any(|arg| arg == "PATH=/usr/local/bin"),
            "PATH must not be injected into the container: {args:?}"
        );
        assert!(
            args.iter()
                .any(|arg| arg == "GWT_BIN_PATH=/usr/local/bin/gwtd"),
            "non-PATH env vars must still be injected: {args:?}"
        );
    }

    fn sample_direct_codex_launch_config(worktree: &Path) -> LaunchConfig {
        let mut config = AgentLaunchBuilder::new(AgentId::Codex)
            .working_dir(worktree)
            .branch("feature/demo")
            .model("gpt-5.6-codex")
            .session_mode(SessionMode::Continue)
            .skip_permissions(true)
            .extra_arg("--search")
            .build();
        config.command = "/opt/homebrew/bin/codex".to_string();
        config.runtime_target = LaunchRuntimeTarget::Host;
        config
    }

    type ProbeRecords = Arc<Mutex<Vec<(String, Vec<String>)>>>;

    fn recording_probe(
        healthy: impl Fn(&str) -> bool + 'static,
    ) -> (ProbeRecords, Box<HostRunnerProbe>) {
        let records = Arc::new(Mutex::new(Vec::new()));
        let observed_records = Arc::clone(&records);
        let probe = move |_kind: HostRunnerProbeKind,
                          command: &str,
                          args: Vec<String>,
                          _env: &HashMap<String, String>,
                          _remove_env: &[String],
                          _cwd: Option<PathBuf>| {
            observed_records
                .lock()
                .expect("probe records")
                .push((command.to_string(), args));
            if healthy(command) {
                HostRunnerProbeOutcome::success()
            } else {
                HostRunnerProbeOutcome::failure_with_stderr("injected probe failure")
            }
        };
        (records, Box::new(probe))
    }

    fn prepare_test_launch(
        worktree: &Path,
        sessions_dir: &Path,
        config: LaunchConfig,
        mut probe: Box<HostRunnerProbe>,
    ) -> Result<PreparedAgentLaunch, String> {
        let lookup_gwt_bin = |_command: &str| Some(PathBuf::from("/usr/local/bin/gwtd"));
        prepare_agent_launch_with(
            worktree,
            sessions_dir,
            config,
            None,
            |_path| Ok(()),
            PrepareLaunchDeps {
                current_exe: Path::new("/usr/local/bin/gwt"),
                probe_host_runner: probe.as_mut(),
                lookup_gwt_bin: &lookup_gwt_bin,
            },
        )
    }

    fn init_git_repo(path: &Path) {
        fs::create_dir_all(path).expect("create repo dir");
        let init = gwt_core::process::hidden_command("git")
            .args(["init", "-q", "-b", "develop"])
            .current_dir(path)
            .status()
            .expect("git init");
        assert!(init.success(), "git init failed");
        let config_name = gwt_core::process::hidden_command("git")
            .args(["config", "user.name", "Codex"])
            .current_dir(path)
            .status()
            .expect("git config user.name");
        assert!(config_name.success(), "git config user.name failed");
        let config_email = gwt_core::process::hidden_command("git")
            .args(["config", "user.email", "codex@example.com"])
            .current_dir(path)
            .status()
            .expect("git config user.email");
        assert!(config_email.success(), "git config user.email failed");
        fs::write(path.join("README.md"), "repo\n").expect("write readme");
        let add = gwt_core::process::hidden_command("git")
            .args(["add", "README.md"])
            .current_dir(path)
            .status()
            .expect("git add");
        assert!(add.success(), "git add failed");
        let commit = gwt_core::process::hidden_command("git")
            .args(["commit", "-qm", "init"])
            .current_dir(path)
            .status()
            .expect("git commit");
        assert!(commit.success(), "git commit failed");
    }

    #[test]
    fn missing_launcher_detail_names_the_executable_the_shim_points_at() {
        let missing = r"C:\Users\dev\AppData\Local\npm-cache\_npx\2842f47953679de1\node_modules\.bin\..\@anthropic-ai\claude-code\bin\claude.exe";
        let tail = format!(
            "'\"{missing}\"' \u{306f}\u{3001}\u{5185}\u{90e8}\u{30b3}\u{30de}\u{30f3}\u{30c9}\u{307e}\u{305f}\u{306f}\u{5916}\u{90e8}\u{30b3}\u{30de}\u{30f3}\u{30c9}\u{3068}\u{3057}\u{3066}\u{8a8d}\u{8b58}\u{3055}\u{308c}\u{3066}\u{3044}\u{307e}\u{305b}\u{3093}\u{3002}"
        );

        let detail = super::missing_launcher_binary_detail_with(&tail, |_| false)
            .expect("a quoted missing executable is named");

        assert!(
            detail.contains(missing),
            "the diagnosis must name the path that was not found: {detail}"
        );
    }

    #[test]
    fn missing_launcher_detail_stays_silent_without_a_missing_executable() {
        assert_eq!(
            super::missing_launcher_binary_detail_with(
                "'\"C:\\tools\\claude.exe\"' failed",
                |_| { true }
            ),
            None,
            "an executable that exists is a different failure"
        );
        assert_eq!(
            super::missing_launcher_binary_detail_with("Process exited with status 1", |_| false),
            None,
            "an exit status without a quoted path names nothing"
        );
    }

    #[cfg(unix)]
    #[test]
    fn prepared_healthy_direct_runner_is_one_absolute_executable_for_probe_persist_and_dispatch() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempdir().expect("tempdir");
        let worktree = temp.path().join("repo-feature");
        let sessions_dir = temp.path().join("sessions");
        let bin = worktree.join("tools");
        fs::create_dir_all(&bin).expect("create relative PATH directory");
        let codex = bin.join("codex");
        fs::write(&codex, "#!/bin/sh\nprintf 'codex-cli 0.133.0\\n'\n").expect("write codex");
        fs::set_permissions(&codex, fs::Permissions::from_mode(0o755)).expect("chmod codex");
        let mut config = sample_direct_codex_launch_config(&worktree);
        config.command = "codex".to_string();
        config
            .env_vars
            .insert("PATH".to_string(), "tools".to_string());
        config.remove_env.push("PATH".to_string());
        let expected = codex.display().to_string();
        let probes = Arc::new(Mutex::new(Vec::<(String, Vec<String>)>::new()));
        let observed = Arc::clone(&probes);
        let probe: Box<HostRunnerProbe> =
            Box::new(move |_kind, command, args, _env, _remove_env, _cwd| {
                observed
                    .lock()
                    .expect("probe records")
                    .push((command.to_string(), args));
                HostRunnerProbeOutcome {
                    success: true,
                    exit_code: Some(0),
                    stdout: "codex-cli 0.133.0\n".to_string(),
                    stderr: String::new(),
                    timed_out: false,
                    error: None,
                }
            });

        let prepared = prepare_test_launch(&worktree, &sessions_dir, config, probe)
            .expect("healthy absolute direct runner");
        let persisted = Session::load(&sessions_dir.join(format!("{}.toml", prepared.session.id)))
            .expect("persisted Session");
        let probes = probes.lock().expect("probe records");

        assert_eq!(
            probes.as_slice(),
            &[(expected.clone(), vec!["--version".to_string()])]
        );
        assert_eq!(prepared.process_launch.command, expected);
        assert_eq!(prepared.session.launch_command, expected);
        assert_eq!(persisted.launch_command, expected);
    }

    #[cfg(unix)]
    #[test]
    fn failed_direct_resolution_candidate_does_not_mutate_launch_config() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempdir().expect("tempdir");
        let worktree = temp.path().join("repo-feature");
        let bin = worktree.join("tools");
        fs::create_dir_all(&bin).expect("create relative PATH directory");
        for name in ["codex", "bunx", "npx"] {
            let runner = bin.join(name);
            fs::write(&runner, "#!/bin/sh\nexit 1\n").expect("write runner");
            fs::set_permissions(&runner, fs::Permissions::from_mode(0o755)).expect("chmod runner");
        }
        let mut config = sample_direct_codex_launch_config(&worktree);
        config.command = "codex".to_string();
        config
            .env_vars
            .insert("PATH".to_string(), "tools".to_string());
        config.remove_env.push("PATH".to_string());
        let original = format!("{config:?}");
        let mut probes = Vec::new();

        let result = resolve_host_runner_health_checked_with_probe(
            &mut config,
            |_kind, command, _args, _env, _remove_env, _cwd| {
                probes.push(command.to_string());
                HostRunnerProbeOutcome::failure_with_stderr("unhealthy")
            },
        );

        assert!(result.is_err());
        assert_eq!(format!("{config:?}"), original);
        assert_eq!(
            probes.first(),
            Some(&bin.join("codex").display().to_string())
        );
    }

    #[test]
    fn prepare_agent_launch_rejects_unhealthy_direct_runner_without_package_fallback() {
        let temp = tempdir().expect("tempdir");
        let worktree = temp.path().join("repo-feature");
        let sessions_dir = temp.path().join("sessions");
        fs::create_dir_all(&worktree).expect("create worktree");
        let (probes, probe) = recording_probe(|_command| false);

        let result = prepare_test_launch(
            &worktree,
            &sessions_dir,
            sample_direct_codex_launch_config(&worktree),
            probe,
        );

        let error = result.expect_err("broken direct runner must not be dispatched");
        let probes = probes.lock().expect("probe records");
        assert_eq!(probes.len(), 1, "only the installed runner may be probed");
        assert!(
            error.contains("install"),
            "missing installation guidance: {error}"
        );
        assert!(
            !sessions_dir.exists()
                || fs::read_dir(&sessions_dir)
                    .expect("read sessions dir")
                    .next()
                    .is_none(),
            "Session persistence must not precede runner health"
        );
    }

    #[test]
    fn prepare_agent_launch_preserves_healthy_direct_runner_and_args() {
        let temp = tempdir().expect("tempdir");
        let worktree = temp.path().join("repo-feature");
        let sessions_dir = temp.path().join("sessions");
        fs::create_dir_all(&worktree).expect("create worktree");
        let config = sample_direct_codex_launch_config(&worktree);
        let original_command = config.command.clone();
        let original_args = config.args.clone();
        let (probes, probe) = recording_probe(|_command| true);

        let prepared = prepare_test_launch(&worktree, &sessions_dir, config, probe)
            .expect("healthy direct runner");

        assert_eq!(prepared.process_launch.args, original_args);
        let probes = probes.lock().expect("probe records");
        assert_eq!(probes.len(), 1);
        assert_eq!(prepared.process_launch.command, probes[0].0);
        assert!(command_matches_runner(
            &prepared.process_launch.command,
            "codex"
        ));
        if !cfg!(windows) {
            assert_eq!(prepared.process_launch.command, original_command);
        }
        assert_eq!(probes[0].1, vec!["--version".to_string()]);
    }

    #[test]
    fn prepare_agent_launch_uses_descriptor_version_argv_for_copilot() {
        let temp = tempdir().expect("tempdir");
        let worktree = temp.path().join("repo-feature");
        let sessions_dir = temp.path().join("sessions");
        fs::create_dir_all(&worktree).expect("create worktree");
        let mut config = AgentLaunchBuilder::new(AgentId::Copilot)
            .working_dir(&worktree)
            .build();
        config.command = "/usr/local/bin/gh".to_string();
        let (probes, probe) = recording_probe(|_command| true);

        prepare_test_launch(&worktree, &sessions_dir, config, probe)
            .expect("healthy Copilot direct runner");

        let probes = probes.lock().expect("probe records");
        assert_eq!(probes.len(), 1);
        assert_eq!(
            probes[0].1,
            vec!["copilot".to_string(), "--version".to_string()]
        );
    }

    #[test]
    fn prepare_agent_launch_does_not_probe_custom_direct_runner() {
        let temp = tempdir().expect("tempdir");
        let worktree = temp.path().join("repo-feature");
        let sessions_dir = temp.path().join("sessions");
        fs::create_dir_all(&worktree).expect("create worktree");
        let config = AgentLaunchBuilder::new(AgentId::Custom("my-agent".into()))
            .working_dir(&worktree)
            .extra_arg("--custom")
            .build();
        let original_command = config.command.clone();
        let original_args = config.args.clone();
        let (probes, probe) = recording_probe(|_command| false);

        let prepared =
            prepare_test_launch(&worktree, &sessions_dir, config, probe).expect("custom launch");

        assert!(probes.lock().expect("probe records").is_empty());
        assert_eq!(prepared.process_launch.command, original_command);
        assert_eq!(prepared.process_launch.args, original_args);
    }

    #[test]
    fn prepare_agent_launch_does_not_probe_direct_runner_after_docker_selection() {
        let temp = tempdir().expect("tempdir");
        let project = temp.path().join("project-without-compose");
        let sessions_dir = temp.path().join("sessions");
        fs::create_dir_all(&project).expect("create project");
        let mut config = sample_direct_codex_launch_config(&project);
        config.runtime_target = LaunchRuntimeTarget::Docker;
        config.docker_service = Some("app".to_string());
        let (probes, probe) = recording_probe(|_command| false);

        let result = prepare_test_launch(&project, &sessions_dir, config, probe);

        assert!(result.is_err(), "missing Docker compose must fail first");
        assert_eq!(
            probes.lock().expect("probe records").len(),
            0,
            "host runner probe must not run for Docker"
        );
    }

    #[test]
    fn host_runner_health_does_not_probe_or_mutate_custom_bunx_agent() {
        let temp = tempdir().expect("tempdir");
        let custom = CustomCodingAgent {
            id: "review-bot".to_string(),
            display_name: "Review Bot".to_string(),
            agent_type: CustomAgentType::Bunx,
            command: "@example/review-bot@latest".to_string(),
            default_args: vec!["--review".to_string()],
            mode_args: None,
            skip_permissions_args: Vec::new(),
            env: HashMap::from([("REVIEW_MODE".to_string(), "strict".to_string())]),
            supports_resume_picker: false,
        };
        let mut config = AgentLaunchBuilder::new(AgentId::Custom("review-bot".to_string()))
            .custom_agent(custom)
            .working_dir(temp.path())
            .extra_arg("--format=json")
            .build();
        let original = format!("{config:?}");
        let mut probe_calls = 0;

        let report = resolve_host_runner_health_checked_with_probe(
            &mut config,
            |_kind, _command, _args, _env, _remove_env, _cwd| {
                probe_calls += 1;
                HostRunnerProbeOutcome::success()
            },
        )
        .expect("Custom Bunx launch must bypass built-in runner health policy");

        assert_eq!(report, HostRunnerHealthReport::default());
        assert_eq!(probe_calls, 0, "Custom Bunx must not be probed");
        assert_eq!(
            format!("{config:?}"),
            original,
            "Custom Bunx command, args, and environment must remain byte-identical"
        );
    }

    #[test]
    fn host_runner_health_error_does_not_echo_url_userinfo_from_command() {
        const SECRET: &str = "health-command-secret-sentinel-85207";
        let temp = tempdir().expect("tempdir");
        let mut config = AgentLaunchBuilder::new(AgentId::OpenClaw)
            .working_dir(temp.path())
            .build();
        config.command = format!("https://runner:{SECRET}@example.test/openclaw");

        let error = resolve_host_runner_health_checked_with_probe(
            &mut config,
            |_kind, _command, _args, _env, _remove_env, _cwd| {
                HostRunnerProbeOutcome::failure_with_stderr("runner unavailable")
            },
        )
        .expect_err("OpenClaw has no package fallback");

        assert!(
            !error.contains(SECRET),
            "health error leaked command userinfo: {error}"
        );
        assert!(error.contains("OpenClaw"));
    }

    #[test]
    fn host_runner_probe_diagnostic_redacts_nonstandard_environment_value() {
        const SECRET: &str = "health-probe-jwt-sentinel-24681";
        let env_vars = HashMap::from([("CI_JOB_JWT".to_string(), SECRET.to_string())]);
        let diagnostic = HostRunnerProbeOutcome {
            success: false,
            exit_code: Some(1),
            stdout: String::new(),
            stderr: format!("runner echoed {SECRET}"),
            timed_out: false,
            error: Some(format!("runtime rejected {SECRET}")),
        }
        .diagnostic(&env_vars);

        assert!(diagnostic.contains("exit status 1"));
        assert!(diagnostic.contains(gwt_core::process_console::REDACTED));
        assert!(
            !diagnostic.contains(SECRET),
            "health diagnostic leaked a nonstandard environment value: {diagnostic}"
        );
    }

    #[test]
    fn healthy_direct_report_carries_only_strict_semver_version_evidence() {
        const SECRET: &str = "version-output-secret-sentinel-95173";
        let temp = tempdir().expect("tempdir");
        let mut config = sample_direct_codex_launch_config(temp.path());
        config
            .env_vars
            .insert("RUNNER_API_TOKEN".to_string(), SECRET.to_string());
        let mut probe_calls = 0;

        let report = resolve_host_runner_health_checked_with_probe(
            &mut config,
            |_kind, _command, _args, _env, _remove_env, _cwd| {
                probe_calls += 1;
                HostRunnerProbeOutcome {
                    success: true,
                    exit_code: Some(0),
                    stdout: format!("codex-cli 0.133.0 https://user:{SECRET}@example.test/runner"),
                    stderr: String::new(),
                    timed_out: false,
                    error: None,
                }
            },
        )
        .expect("healthy direct runner");

        assert_eq!(probe_calls, 1);
        let version_output = report.version_output.expect("version output evidence");
        assert_eq!(version_output, "0.133.0");
        assert_eq!(config.tool_version.as_deref(), Some("0.133.0"));
        assert!(!version_output.contains(SECRET));
    }

    #[test]
    fn claude_ultracode_rejects_the_actual_old_runner_version() {
        let temp = tempdir().expect("tempdir");
        let mut config = sample_direct_codex_launch_config(temp.path());
        config.agent_id = AgentId::ClaudeCode;
        config.command = "/opt/homebrew/bin/claude".into();
        config.reasoning_level = Some("ultracode".into());
        let original = config.clone();
        let result =
            resolve_host_runner_health_checked_with_probe(&mut config, |kind, _, _, _, _, _| {
                assert_eq!(kind, HostRunnerProbeKind::Direct);
                HostRunnerProbeOutcome {
                    stdout: "2.1.153 (Claude Code)".into(),
                    ..HostRunnerProbeOutcome::success()
                }
            });
        assert!(result
            .expect_err("old actual binary cannot use ultracode")
            .contains("2.1.153"));
        assert_eq!(config.command, original.command);
        assert_eq!(config.tool_version, original.tool_version);
    }

    #[cfg(unix)]
    #[test]
    fn public_prepare_bounds_a_hanging_direct_runner_probe() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempdir().expect("tempdir");
        let worktree = temp.path().join("repo-feature");
        let sessions_dir = temp.path().join("sessions");
        fs::create_dir_all(&worktree).expect("create worktree");
        // SPEC-3864: Antigravity has no runtime package route, so a hanging
        // direct runner cannot be rescued by a bunx/npx fallback.
        let direct = temp.path().join("agy");
        fs::write(&direct, "#!/bin/sh\nsleep 8\nexit 1\n").expect("write hanging runner");
        fs::set_permissions(&direct, fs::Permissions::from_mode(0o755))
            .expect("chmod hanging runner");
        let mut config = AgentLaunchBuilder::new(AgentId::Antigravity)
            .working_dir(&worktree)
            .build();
        config.command = direct.display().to_string();

        let started = std::time::Instant::now();
        let result = prepare_agent_launch(&worktree, &sessions_dir, config, None, |_path| Ok(()));

        assert!(result.is_err(), "hanging direct runner must fail closed");
        assert!(
            started.elapsed() < std::time::Duration::from_secs(7),
            "direct runner health must honor the five-second deadline: {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn runner_probe_forwards_failed_stderr_to_agent_console() {
        let hub = gwt_core::process_console::ProcessConsoleHub::new();
        let (command, args) = if cfg!(windows) {
            (
                "cmd".to_string(),
                vec![
                    "/C".to_string(),
                    "echo probe boom 1>&2 & exit /b 1".to_string(),
                ],
            )
        } else {
            (
                "sh".to_string(),
                vec!["-c".to_string(), "echo probe boom >&2; exit 1".to_string()],
            )
        };

        let outcome = probe_host_runner_bounded_with_hub(
            HostRunnerProbeRequest {
                kind: HostRunnerProbeKind::Direct,
                command: &command,
                args,
                env_vars: &HashMap::new(),
                remove_env: &[],
                cwd: None,
                timeout: Duration::from_secs(2),
                poll_interval: Duration::from_millis(10),
            },
            &hub,
        );

        assert!(!outcome.success);
        let lines = hub.snapshot_kind(gwt_core::process_console::ProcessKind::AgentBootstrap);
        assert!(
            lines.iter().any(|line| {
                line.stream == gwt_core::process_console::ProcessStream::Stderr
                    && line.message.contains("probe boom")
            }),
            "expected failed probe stderr in agent console lines: {lines:?}",
        );
    }

    #[test]
    fn runner_probe_redaction_uses_only_nontrivial_secret_like_env_values() {
        let env = HashMap::from([
            (
                "RUNNER_API_TOKEN".to_string(),
                "probe-token-sentinel-12345".to_string(),
            ),
            (
                "service_key".to_string(),
                "probe-key-sentinel-67890".to_string(),
            ),
            (
                "DB_PASSWORD".to_string(),
                "probe-password-sentinel".to_string(),
            ),
            ("EMPTY_SECRET".to_string(), String::new()),
            ("SHORT_TOKEN".to_string(), "on".to_string()),
            (
                "ANSI_ONLY_TOKEN".to_string(),
                "\u{1b}[31m\u{1b}[0m".to_string(),
            ),
            ("USER".to_string(), "runner".to_string()),
        ]);
        let clean = "runner on ordinary output";
        let raw = format!(
            "{clean}: {} / {} / {}",
            env["RUNNER_API_TOKEN"], env["service_key"], env["DB_PASSWORD"]
        );

        let redacted = redact_runner_probe_text(&raw, &env);

        assert!(redacted.contains(clean));
        assert!(redacted.contains("***redacted***"));
        assert!(!redacted.contains(&env["RUNNER_API_TOKEN"]));
        assert!(!redacted.contains(&env["service_key"]));
        assert!(!redacted.contains(&env["DB_PASSWORD"]));
        assert!(
            redact_runner_probe_text(clean, &env).contains(clean),
            "empty, short, and non-secret env values must not destructively rewrite ordinary output"
        );
    }

    fn ansi_interleave_runner_probe_secret(secret: &str) -> String {
        secret
            .chars()
            .map(|character| format!("{character}\u{1b}[31m\u{1b}[0m"))
            .collect()
    }

    #[test]
    fn runner_probe_strips_ansi_before_dynamic_secret_redaction() {
        let secret = "runner-probe-secret-sentinel-ansi-24680";
        let env = HashMap::from([("RUNNER_API_TOKEN".to_string(), secret.to_string())]);
        let ansi_split = ansi_interleave_runner_probe_secret(secret);

        let redacted = redact_runner_probe_text(&ansi_split, &env);
        let downstream_view = gwt_core::process_console::strip_ansi(&redacted);

        assert!(redacted.contains(gwt_core::process_console::REDACTED));
        assert!(
            !downstream_view.contains(secret),
            "ANSI removal must never reconstruct a dynamically redacted secret: {downstream_view:?}"
        );
    }

    #[test]
    fn runner_probe_capture_redacts_ansi_split_secret_prefix_at_capture_boundary() {
        let secret = "runner-probe-secret-sentinel-boundary-13579";
        let visible_prefix = &secret[..20];
        let env = HashMap::from([("RUNNER_API_TOKEN".to_string(), secret.to_string())]);
        let ansi_prefix = ansi_interleave_runner_probe_secret(visible_prefix);
        let padding_length = RUNNER_PROBE_CAPTURE_LIMIT_BYTES - ansi_prefix.len();
        let mut capture = RunnerProbeCapture::default();
        capture.append("x".repeat(padding_length).as_bytes());
        capture.append(ansi_prefix.as_bytes());
        capture.append(b"overflow");

        let rendered = capture.render(&env);
        let downstream_view = gwt_core::process_console::strip_ansi(&rendered);

        assert!(rendered.contains(gwt_core::process_console::REDACTED));
        assert!(rendered.contains("truncated"));
        assert!(
            !downstream_view.contains(visible_prefix),
            "ANSI removal must not expose a secret prefix retained at the capture boundary"
        );
    }

    #[test]
    fn runner_probe_truncated_capture_redacts_every_secret_prefix_fragment() {
        struct Case {
            secret: &'static str,
            prefix_chars: usize,
            cut_inside_next_char: bool,
        }
        let cases = [
            Case {
                secret: "Q-runner-probe-secret-one",
                prefix_chars: 1,
                cut_inside_next_char: false,
            },
            Case {
                secret: "UVWXYZa-runner-probe-secret-seven",
                prefix_chars: 7,
                cut_inside_next_char: false,
            },
            Case {
                secret: "ABCDEFGH-runner-probe-secret-eight",
                prefix_chars: 8,
                cut_inside_next_char: false,
            },
            Case {
                secret: "秘密鍵runner-probe-secret-multibyte",
                prefix_chars: 2,
                cut_inside_next_char: true,
            },
        ];

        for (index, case) in cases.iter().enumerate() {
            let fragment = case
                .secret
                .chars()
                .take(case.prefix_chars)
                .collect::<String>();
            let mut retained = fragment.as_bytes().to_vec();
            if case.cut_inside_next_char {
                let next = case
                    .secret
                    .chars()
                    .nth(case.prefix_chars)
                    .expect("next multi-byte secret character")
                    .to_string();
                retained.push(next.as_bytes()[0]);
            }
            let padding_length = RUNNER_PROBE_CAPTURE_LIMIT_BYTES - retained.len();
            let captured = Arc::new(Mutex::new(RunnerProbeCapture::default()));
            {
                let mut capture = captured.lock().expect("capture lock");
                capture.append("x".repeat(padding_length).as_bytes());
                capture.append(&retained);
                capture.append(b"overflow");
                assert!(capture.truncated);
            }
            let env = HashMap::from([("RUNNER_API_TOKEN".to_string(), case.secret.to_string())]);
            let outcome_stdout = captured_runner_probe_string(&captured, &env);
            let hub = gwt_core::process_console::ProcessConsoleHub::new();
            publish_runner_probe_capture(&hub, index as u64 + 1, &outcome_stdout, "");
            let lines = hub.snapshot_kind(gwt_core::process_console::ProcessKind::AgentBootstrap);

            assert!(
                !outcome_stdout.contains(&fragment),
                "truncated outcome exposed {}-character fragment {fragment:?}",
                case.prefix_chars
            );
            assert!(
                !lines.iter().any(|line| line.message.contains(&fragment)),
                "console exposed {}-character fragment {fragment:?}: {lines:?}",
                case.prefix_chars
            );
        }
    }

    #[test]
    fn runner_probe_nontruncated_capture_keeps_short_secret_prefix_threshold() {
        let secret = "QwertyZ-runner-probe-secret-nontruncated";
        let fragment = &secret[..7];
        let env = HashMap::from([("RUNNER_API_TOKEN".to_string(), secret.to_string())]);
        let mut capture = RunnerProbeCapture::default();
        capture.append(fragment.as_bytes());

        let rendered = capture.render(&env);

        assert!(!capture.truncated);
        assert_eq!(rendered, fragment);
    }

    struct PendingDropProbeReader {
        dropped: Arc<AtomicBool>,
    }

    impl Drop for PendingDropProbeReader {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::SeqCst);
        }
    }

    impl tokio::io::AsyncRead for PendingDropProbeReader {
        fn poll_read(
            self: std::pin::Pin<&mut Self>,
            _context: &mut std::task::Context<'_>,
            _buffer: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Pending
        }
    }

    struct BytesThenErrorProbeReader {
        bytes: Option<Vec<u8>>,
        raw_error: String,
    }

    impl tokio::io::AsyncRead for BytesThenErrorProbeReader {
        fn poll_read(
            self: std::pin::Pin<&mut Self>,
            _context: &mut std::task::Context<'_>,
            buffer: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            let reader = self.get_mut();
            if let Some(bytes) = reader.bytes.take() {
                buffer.put_slice(&bytes);
                return std::task::Poll::Ready(Ok(()));
            }
            std::task::Poll::Ready(Err(std::io::Error::other(reader.raw_error.clone())))
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn runner_probe_reader_error_after_bytes_fails_closed_without_raw_detail() {
        let raw_payload = "reader-error-raw-payload-sentinel-86420";
        for failing_stream in ["stdout", "stderr"] {
            let captured = Arc::new(Mutex::new(RunnerProbeCapture::default()));
            let reader = Some(spawn_runner_probe_stream_capture(
                BytesThenErrorProbeReader {
                    bytes: Some(raw_payload.as_bytes().to_vec()),
                    raw_error: format!("read failed after {raw_payload}"),
                },
                Arc::clone(&captured),
            ));
            let (mut stdout_reader, mut stderr_reader) = if failing_stream == "stdout" {
                (reader, None)
            } else {
                (None, reader)
            };
            while !runner_probe_streams_finished(&stdout_reader, &stderr_reader) {
                tokio::task::yield_now().await;
            }

            let error = join_runner_probe_streams(&mut stdout_reader, &mut stderr_reader)
                .await
                .expect_err("a read error after captured bytes must fail a successful child");

            assert!(
                error.contains(failing_stream),
                "error must identify the stream: {error:?}"
            );
            assert!(error.contains("Other"), "error must identify the I/O kind");
            assert!(
                !error.contains(raw_payload),
                "read error detail must never include raw payload: {error:?}"
            );
            assert_eq!(
                captured.lock().expect("capture lock").bytes,
                raw_payload.as_bytes(),
                "bytes read before the error must remain available for diagnostics"
            );
            let env = HashMap::from([("RUNNER_API_TOKEN".to_string(), raw_payload.to_string())]);
            let diagnostic = captured_runner_probe_string(&captured, &env);
            assert!(!diagnostic.contains(raw_payload));
            assert!(diagnostic.contains(gwt_core::process_console::REDACTED));
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn runner_probe_reader_cleanup_aborts_awaits_and_drops_pending_reader() {
        let dropped = Arc::new(AtomicBool::new(false));
        let capture = Arc::new(Mutex::new(RunnerProbeCapture::default()));
        let mut reader_task = Some(spawn_runner_probe_stream_capture(
            PendingDropProbeReader {
                dropped: Arc::clone(&dropped),
            },
            capture,
        ));
        tokio::task::yield_now().await;

        abort_and_join_runner_probe_stream(&mut reader_task).await;

        assert!(
            reader_task.is_none(),
            "cleanup must consume the task handle"
        );
        assert!(
            dropped.load(Ordering::SeqCst),
            "cleanup must await cancellation until the reader and its OS pipe handle are dropped"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn runner_probe_reader_cleanup_survives_process_tree_termination_failure() {
        let dropped = Arc::new(AtomicBool::new(false));
        let capture = Arc::new(Mutex::new(RunnerProbeCapture::default()));
        let mut stdout_reader = Some(spawn_runner_probe_stream_capture(
            PendingDropProbeReader {
                dropped: Arc::clone(&dropped),
            },
            capture,
        ));
        let mut stderr_reader = None;
        tokio::task::yield_now().await;

        let cleanup = finish_runner_probe_cleanup(
            async { false },
            async { true },
            &mut stdout_reader,
            &mut stderr_reader,
        )
        .await;

        assert!(!cleanup.process_tree_terminated);
        assert!(!cleanup.complete());
        assert!(stdout_reader.is_none());
        assert!(
            dropped.load(Ordering::SeqCst),
            "failed tree termination must not leave a gwt reader task or pipe handle behind"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn runner_probe_sync_entrypoint_is_safe_inside_tokio_runtime() {
        let hub = gwt_core::process_console::ProcessConsoleHub::new();
        let (command, args) = if cfg!(windows) {
            (
                "cmd".to_string(),
                vec!["/C".to_string(), "exit /b 0".to_string()],
            )
        } else {
            (
                "sh".to_string(),
                vec!["-c".to_string(), "exit 0".to_string()],
            )
        };

        let outcome = probe_host_runner_bounded_with_hub(
            HostRunnerProbeRequest {
                kind: HostRunnerProbeKind::Direct,
                command: &command,
                args,
                env_vars: &HashMap::new(),
                remove_env: &[],
                cwd: None,
                timeout: Duration::from_secs(2),
                poll_interval: Duration::from_millis(10),
            },
            &hub,
        );

        assert!(outcome.success, "nested runtime probe failed: {outcome:?}");
    }

    #[test]
    fn runner_probe_does_not_inherit_secret_launch_environment() {
        let hub = gwt_core::process_console::ProcessConsoleHub::new();
        let secret = "runner-probe-secret-sentinel-24680";
        let env = HashMap::from([("RUNNER_API_TOKEN".to_string(), secret.to_string())]);
        let (command, args) = if cfg!(windows) {
            (
                "cmd".to_string(),
                vec![
                    "/C".to_string(),
                    "if defined RUNNER_API_TOKEN (echo %RUNNER_API_TOKEN% & echo %RUNNER_API_TOKEN% 1>&2 & exit /b 1) else (exit /b 1)"
                        .to_string(),
                ],
            )
        } else {
            (
                "sh".to_string(),
                vec![
                    "-c".to_string(),
                    "printf '%s\\n' \"$RUNNER_API_TOKEN\"; printf '%s\\n' \"$RUNNER_API_TOKEN\" >&2; exit 1"
                        .to_string(),
                ],
            )
        };

        let outcome = probe_host_runner_bounded_with_hub(
            HostRunnerProbeRequest {
                kind: HostRunnerProbeKind::Direct,
                command: &command,
                args,
                env_vars: &env,
                remove_env: &[],
                cwd: None,
                timeout: Duration::from_secs(2),
                poll_interval: Duration::from_millis(10),
            },
            &hub,
        );

        assert!(!outcome.success);
        assert!(!outcome.stdout.contains(secret));
        assert!(!outcome.stderr.contains(secret));
        assert!(!outcome.diagnostic(&env).contains(secret));
        let lines = hub.snapshot_kind(gwt_core::process_console::ProcessKind::AgentBootstrap);
        assert!(!lines.iter().any(|line| line.message.contains(secret)));
        assert!(outcome.stdout.trim().is_empty());
        assert!(outcome.stderr.trim().is_empty());
    }

    #[test]
    fn runner_probe_never_exposes_nonstandard_secret_environment_names() {
        let hub = gwt_core::process_console::ProcessConsoleHub::new();
        let jwt = "runner-probe-jwt-sentinel-13579";
        let npm_auth = "runner-probe-npm-auth-sentinel-86420";
        let env = HashMap::from([
            ("CI_JOB_JWT".to_string(), jwt.to_string()),
            ("NPM_CONFIG__AUTH".to_string(), npm_auth.to_string()),
        ]);
        let (command, args) = if cfg!(windows) {
            (
                "cmd".to_string(),
                vec![
                    "/C".to_string(),
                    "echo %CI_JOB_JWT% & echo %NPM_CONFIG__AUTH% 1>&2 & exit /b 1".to_string(),
                ],
            )
        } else {
            (
                "sh".to_string(),
                vec![
                    "-c".to_string(),
                    "printf '%s\n' \"$CI_JOB_JWT\"; printf '%s\n' \"$NPM_CONFIG__AUTH\" >&2; exit 1"
                        .to_string(),
                ],
            )
        };

        let outcome = probe_host_runner_bounded_with_hub(
            HostRunnerProbeRequest {
                kind: HostRunnerProbeKind::Direct,
                command: &command,
                args,
                env_vars: &env,
                remove_env: &[],
                cwd: None,
                timeout: Duration::from_secs(2),
                poll_interval: Duration::from_millis(10),
            },
            &hub,
        );

        let all_surfaces = format!(
            "{}\n{}\n{}\n{}",
            outcome.stdout,
            outcome.stderr,
            outcome.diagnostic(&env),
            hub.snapshot_kind(gwt_core::process_console::ProcessKind::AgentBootstrap)
                .into_iter()
                .map(|line| line.message)
                .collect::<Vec<_>>()
                .join("\n"),
        );
        assert!(!outcome.success);
        for secret in [jwt, npm_auth] {
            assert!(
                !all_surfaces.contains(secret),
                "runner probe leaked a nonstandard secret environment value: {all_surfaces}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn runner_probe_trace_and_failure_surfaces_never_expose_command_metadata_secrets() {
        use crate::test_capture::{CaptureLayer, CapturedEvents};
        use tracing_subscriber::layer::SubscriberExt;

        const ARG_SECRET: &str = "trace-arg-secret-sentinel-48391";
        const ENV_SECRET: &str = "trace-env-secret-sentinel-72840";
        const USERINFO_SECRET: &str = "trace-userinfo-secret-sentinel-61935";
        let command = format!("https://user:{USERINFO_SECRET}@invalid.example/runner");
        let args = vec![
            "--endpoint".to_string(),
            format!("https://user:{ARG_SECRET}@invalid.example/api"),
        ];
        let env = HashMap::from([("RUNNER_API_TOKEN".to_string(), ENV_SECRET.to_string())]);
        let hub = gwt_core::process_console::ProcessConsoleHub::new();
        let events = CapturedEvents::new();
        let subscriber = tracing_subscriber::registry().with(CaptureLayer::new(events.clone()));

        let outcome = tracing::subscriber::with_default(subscriber, || {
            probe_host_runner_bounded_with_hub(
                HostRunnerProbeRequest {
                    kind: HostRunnerProbeKind::Direct,
                    command: &command,
                    args,
                    env_vars: &env,
                    remove_env: &[],
                    cwd: None,
                    timeout: Duration::from_secs(1),
                    poll_interval: Duration::from_millis(10),
                },
                &hub,
            )
        });

        let all_trace_fields = events
            .snapshot()
            .into_iter()
            .flat_map(|event| event.fields.into_values())
            .collect::<Vec<_>>()
            .join("\n");
        let all_console = hub
            .snapshot_kind(gwt_core::process_console::ProcessKind::AgentBootstrap)
            .into_iter()
            .map(|line| line.message)
            .collect::<Vec<_>>()
            .join("\n");
        let all_failure_surfaces = format!(
            "{}\n{}\n{}\n{}\n{}",
            outcome.stdout,
            outcome.stderr,
            outcome.diagnostic(&env),
            all_trace_fields,
            all_console,
        );

        for secret in [ARG_SECRET, ENV_SECRET, USERINFO_SECRET] {
            assert!(
                !all_failure_surfaces.contains(secret),
                "probe metadata leaked {secret}: {all_failure_surfaces}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn runner_probe_does_not_inherit_url_environment_value() {
        const USERINFO_SECRET: &str = "output-userinfo-secret-sentinel-39184";
        let url = format!("https://runner:{USERINFO_SECRET}@example.test/api");
        let env = HashMap::from([("PROBE_PUBLIC_URL".to_string(), url)]);
        let hub = gwt_core::process_console::ProcessConsoleHub::new();

        let outcome = probe_host_runner_bounded_with_hub(
            HostRunnerProbeRequest {
                kind: HostRunnerProbeKind::Direct,
                command: "sh",
                args: vec![
                    "-c".to_string(),
                    "printf '%s\\n' \"$PROBE_PUBLIC_URL\"; printf '%s\\n' \"$PROBE_PUBLIC_URL\" >&2; exit 1"
                        .to_string(),
                ],
                env_vars: &env,
                remove_env: &[],
                cwd: None,
                timeout: Duration::from_secs(2),
                poll_interval: Duration::from_millis(10),
            },
            &hub,
        );

        let console = hub
            .snapshot_kind(gwt_core::process_console::ProcessKind::AgentBootstrap)
            .into_iter()
            .map(|line| line.message)
            .collect::<Vec<_>>()
            .join("\n");
        let diagnostic = outcome.diagnostic(&env);
        for surface in [
            outcome.stdout.as_str(),
            outcome.stderr.as_str(),
            diagnostic.as_str(),
            console.as_str(),
        ] {
            assert!(
                !surface.contains(USERINFO_SECRET),
                "URL userinfo leaked: {surface}"
            );
        }
        assert!(outcome.stdout.trim().is_empty());
        assert!(outcome.stderr.trim().is_empty());
        assert!(console.trim().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn runner_probe_does_not_inherit_ansi_split_secret_launch_environment() {
        let hub = gwt_core::process_console::ProcessConsoleHub::new();
        let secret = "runner-probe-secret-sentinel-ansi-surface-97531";
        let ansi_split = ansi_interleave_runner_probe_secret(secret);
        let env = HashMap::from([
            ("RUNNER_API_TOKEN".to_string(), secret.to_string()),
            ("PROBE_ANSI_PAYLOAD".to_string(), ansi_split),
        ]);
        let outcome = probe_host_runner_bounded_with_hub(
            HostRunnerProbeRequest {
                kind: HostRunnerProbeKind::Direct,
                command: "sh",
                args: vec![
                    "-c".to_string(),
                    "printf '%s\\n' \"$PROBE_ANSI_PAYLOAD\"; printf '%s\\n' \"$PROBE_ANSI_PAYLOAD\" >&2; exit 1"
                        .to_string(),
                ],
                env_vars: &env,
                remove_env: &[],
                cwd: None,
                timeout: Duration::from_secs(2),
                poll_interval: Duration::from_millis(10),
            },
            &hub,
        );

        let stdout = gwt_core::process_console::strip_ansi(&outcome.stdout);
        let stderr = gwt_core::process_console::strip_ansi(&outcome.stderr);
        let diagnostic = gwt_core::process_console::strip_ansi(&outcome.diagnostic(&env));
        let lines = hub.snapshot_kind(gwt_core::process_console::ProcessKind::AgentBootstrap);

        assert!(
            !stdout.contains(secret),
            "stdout exposed secret: {stdout:?}"
        );
        assert!(
            !stderr.contains(secret),
            "stderr exposed secret: {stderr:?}"
        );
        assert!(
            !diagnostic.contains(secret),
            "diagnostic exposed secret: {diagnostic:?}"
        );
        assert!(
            !lines.iter().any(|line| line.message.contains(secret)),
            "console exposed secret: {lines:?}"
        );
        assert!(stdout.trim().is_empty());
        assert!(stderr.trim().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn runner_probe_rejects_completion_observed_after_absolute_deadline() {
        let hub = gwt_core::process_console::ProcessConsoleHub::new();
        let started = Instant::now();
        let outcome = probe_host_runner_bounded_with_hub(
            HostRunnerProbeRequest {
                kind: HostRunnerProbeKind::Direct,
                command: "sh",
                args: vec!["-c".to_string(), "sleep 0.02; exit 0".to_string()],
                env_vars: &HashMap::new(),
                remove_env: &[],
                cwd: None,
                timeout: Duration::from_millis(200),
                poll_interval: Duration::from_secs(1),
            },
            &hub,
        );

        assert!(
            outcome.timed_out,
            "completion first observed after the absolute execution deadline must fail closed: {outcome:?}"
        );
        assert!(
            started.elapsed() < Duration::from_millis(400),
            "poll interval must not extend the absolute operation deadline"
        );
    }

    #[cfg(unix)]
    #[test]
    fn runner_probe_capture_is_bounded_for_large_unbroken_output() {
        let hub = gwt_core::process_console::ProcessConsoleHub::new();
        let outcome = probe_host_runner_bounded_with_hub(
            HostRunnerProbeRequest {
                kind: HostRunnerProbeKind::Direct,
                command: "sh",
                args: vec![
                    "-c".to_string(),
                    "head -c 131072 /dev/zero | tr '\\0' x; exit 1".to_string(),
                ],
                env_vars: &HashMap::new(),
                remove_env: &[],
                cwd: None,
                timeout: Duration::from_secs(2),
                poll_interval: Duration::from_millis(10),
            },
            &hub,
        );

        assert!(!outcome.success);
        assert!(
            outcome.stdout.len() <= 16 * 1024 + 64,
            "probe diagnostics must retain only bounded output, got {} bytes",
            outcome.stdout.len()
        );
        assert!(
            outcome.stdout.contains("truncated"),
            "bounded capture should make truncation explicit"
        );
    }

    #[cfg(unix)]
    fn unix_process_exists(pid: u32) -> bool {
        gwt_core::process::hidden_command("kill")
            .args(["-0", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    #[cfg(unix)]
    fn terminate_unix_test_process(pid: u32) {
        let _ = gwt_core::process::hidden_command("kill")
            .args(["-KILL", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }

    #[cfg(unix)]
    fn assert_completed_runner_probe_reaps_stdio_detached_descendant(exit_code: i32) {
        let temp = tempdir().expect("tempdir");
        let pid_file = temp.path().join("descendant.pid");
        let hub = gwt_core::process_console::ProcessConsoleHub::new();
        let outcome = probe_host_runner_bounded_with_hub(
            HostRunnerProbeRequest {
                kind: HostRunnerProbeKind::Direct,
                command: "/bin/sh",
                args: vec![
                    "-c".to_string(),
                    format!(
                        "(exec >/dev/null 2>&1; /bin/sleep 6) & printf '%s' \"$!\" > \"$1\"; exit {exit_code}"
                    ),
                    "gwt-runner-probe".to_string(),
                    pid_file.display().to_string(),
                ],
                env_vars: &HashMap::new(),
                remove_env: &[],
                cwd: None,
                timeout: Duration::from_secs(2),
                poll_interval: Duration::from_millis(10),
            },
            &hub,
        );
        let pid = fs::read_to_string(&pid_file)
            .expect("descendant pid")
            .parse::<u32>()
            .expect("numeric descendant pid");
        let exit_deadline = Instant::now() + Duration::from_secs(1);
        while unix_process_exists(pid) && Instant::now() < exit_deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let still_running = unix_process_exists(pid);
        if still_running {
            terminate_unix_test_process(pid);
        }

        assert_eq!(outcome.exit_code, Some(exit_code));
        assert_eq!(outcome.success, exit_code == 0);
        assert!(!outcome.timed_out);
        assert!(
            outcome.error.is_none(),
            "unexpected cleanup error: {outcome:?}"
        );
        assert!(
            !still_running,
            "completed probe descendant {pid} must be terminated for exit {exit_code}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn runner_probe_success_reaps_stdio_detached_descendant() {
        assert_completed_runner_probe_reaps_stdio_detached_descendant(0);
    }

    #[cfg(unix)]
    #[test]
    fn runner_probe_nonzero_exit_reaps_stdio_detached_descendant() {
        assert_completed_runner_probe_reaps_stdio_detached_descendant(7);
    }

    #[cfg(unix)]
    #[test]
    fn runner_probe_deadline_includes_pipe_eof_and_kills_descendants() {
        let temp = tempdir().expect("tempdir");
        let pid_file = temp.path().join("descendant.pid");
        let hub = gwt_core::process_console::ProcessConsoleHub::new();
        let started = Instant::now();
        let outcome = probe_host_runner_bounded_with_hub(
            HostRunnerProbeRequest {
                kind: HostRunnerProbeKind::Direct,
                command: "sh",
                args: vec![
                    "-c".to_string(),
                    "(sleep 6) & printf '%s' \"$!\" > \"$1\"; exit 0".to_string(),
                    "gwt-runner-probe".to_string(),
                    pid_file.display().to_string(),
                ],
                env_vars: &HashMap::new(),
                remove_env: &[],
                cwd: None,
                timeout: Duration::from_secs(2),
                poll_interval: Duration::from_millis(10),
            },
            &hub,
        );
        let elapsed = started.elapsed();
        let pid = fs::read_to_string(&pid_file)
            .expect("descendant pid")
            .parse::<u32>()
            .expect("numeric descendant pid");
        let still_running = unix_process_exists(pid);
        if still_running {
            terminate_unix_test_process(pid);
        }

        assert!(outcome.timed_out, "pipe EOF must share the probe deadline");
        assert!(
            elapsed < Duration::from_secs(3),
            "pipe-held descendant must not extend probe return: {elapsed:?}"
        );
        assert!(!still_running, "probe descendant {pid} must be terminated");
    }

    #[cfg(unix)]
    #[test]
    fn runner_probe_timeout_kills_pipe_holding_descendant() {
        let temp = tempdir().expect("tempdir");
        let pid_file = temp.path().join("descendant.pid");
        let hub = gwt_core::process_console::ProcessConsoleHub::new();
        let outcome = probe_host_runner_bounded_with_hub(
            HostRunnerProbeRequest {
                kind: HostRunnerProbeKind::Direct,
                command: "sh",
                args: vec![
                    "-c".to_string(),
                    "(sleep 6) & printf '%s' \"$!\" > \"$1\"; wait".to_string(),
                    "gwt-runner-probe".to_string(),
                    pid_file.display().to_string(),
                ],
                env_vars: &HashMap::new(),
                remove_env: &[],
                cwd: None,
                timeout: Duration::from_secs(2),
                poll_interval: Duration::from_millis(10),
            },
            &hub,
        );
        let pid = fs::read_to_string(&pid_file)
            .expect("descendant pid")
            .parse::<u32>()
            .expect("numeric descendant pid");
        let still_running = unix_process_exists(pid);
        if still_running {
            terminate_unix_test_process(pid);
        }

        assert!(outcome.timed_out);
        assert!(
            !still_running,
            "timed-out probe descendant {pid} must be terminated"
        );
    }

    #[test]
    fn runner_probe_process_tree_cleanup_has_no_shell_tree_kill_fallback() {
        let source = include_str!("prepare.rs");
        assert!(!source.contains(concat!("task", "kill")));
        assert!(
            source.contains(concat!("impl Drop for Runner", "ProbeProcessTree")),
            "owned process-tree cleanup needs a Drop backstop on every platform"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_runner_probe_job_kills_dead_root_pipe_descendant_before_marker() {
        let temp = tempdir().expect("tempdir");
        let descendant = temp.path().join("descendant.cmd");
        let marker = temp.path().join("late-marker.txt");
        fs::write(
            &descendant,
            "@echo off\r\nping 127.0.0.1 -n 3 >nul\r\necho alive>\"%~1\"\r\n",
        )
        .expect("write descendant script");
        let hub = gwt_core::process_console::ProcessConsoleHub::new();
        let outcome = probe_host_runner_bounded_with_hub(
            HostRunnerProbeRequest {
                kind: HostRunnerProbeKind::Direct,
                command: "cmd.exe",
                args: vec![
                    "/d".to_string(),
                    "/s".to_string(),
                    "/c".to_string(),
                    format!(
                        "start \"\" /b \"{}\" \"{}\" & exit /b 0",
                        descendant.display(),
                        marker.display()
                    ),
                ],
                env_vars: &HashMap::new(),
                remove_env: &[],
                cwd: Some(temp.path().to_path_buf()),
                timeout: Duration::from_millis(500),
                poll_interval: Duration::from_millis(10),
            },
            &hub,
        );

        assert!(
            outcome.timed_out,
            "descendant-held pipe must hit the deadline"
        );
        std::thread::sleep(Duration::from_secs(3));
        assert!(
            !marker.exists(),
            "Job close must kill a dead-root descendant before its delayed marker write"
        );
    }

    #[test]
    fn prepare_agent_launch_persists_session_and_builds_process_launch() {
        let temp = tempdir().expect("tempdir");
        let worktree = temp.path().join("repo-feature");
        let sessions_dir = temp.path().join(".gwt").join("sessions");
        fs::create_dir_all(&worktree).expect("create worktree");

        let refresh_calls = AtomicUsize::new(0);
        let mut config = sample_direct_codex_launch_config(&worktree);
        config
            .env_vars
            .insert("GWT_PROJECT_ROOT".to_string(), "/stale/project".to_string());
        let expected_project_root = worktree.display().to_string();
        let probe_expected_project_root = expected_project_root.clone();
        let mut probe_host_runner =
            move |_kind: HostRunnerProbeKind,
                  command: &str,
                  _args: Vec<String>,
                  env: &HashMap<String, String>,
                  _remove_env: &[String],
                  _cwd: Option<PathBuf>| {
                assert_eq!(
                    env.get("GWT_PROJECT_ROOT").map(String::as_str),
                    Some(probe_expected_project_root.as_str())
                );
                assert!(command_matches_runner(command, "codex"));
                HostRunnerProbeOutcome::success()
            };
        let lookup_gwt_bin =
            |_command: &str| Some(PathBuf::from(r"C:\Users\Example\.bun\bin\gwtd.exe"));
        let prepared = prepare_agent_launch_with(
            &worktree,
            &sessions_dir,
            config,
            Some(HookForwardEnv {
                url: "http://127.0.0.1:7878/hooks".to_string(),
                token: "secret-token".to_string(),
            }),
            |path| {
                assert_eq!(path, worktree.as_path());
                refresh_calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
            PrepareLaunchDeps {
                current_exe: Path::new(
                    r"C:\Users\Example\AppData\Local\Temp\bunx-1234567890-@akiojin\gwt@latest\node_modules\@akiojin\gwt\bin\gwt.exe",
                ),
                probe_host_runner: &mut probe_host_runner,
                lookup_gwt_bin: &lookup_gwt_bin,
            },
        )
        .expect("prepare launch");

        assert_eq!(refresh_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            Path::new(&prepared.process_launch.command)
                .file_stem()
                .and_then(|stem| stem.to_str()),
            Some("codex"),
        );
        assert_eq!(
            prepared.process_launch.cwd.as_deref(),
            Some(worktree.as_path())
        );
        assert_eq!(
            prepared
                .process_launch
                .env
                .get("GWT_PROJECT_ROOT")
                .map(String::as_str),
            Some(expected_project_root.as_str())
        );
        assert_eq!(
            prepared
                .process_launch
                .env
                .get(GWT_BIN_PATH_ENV)
                .map(String::as_str),
            Some(r"C:\Users\Example\.bun\bin\gwtd.exe")
        );
        assert_eq!(
            prepared
                .process_launch
                .env
                .get(GWT_HOOK_FORWARD_URL_ENV)
                .map(String::as_str),
            Some("http://127.0.0.1:7878/hooks")
        );
        assert_eq!(
            prepared
                .process_launch
                .env
                .get(GWT_HOOK_FORWARD_TOKEN_ENV)
                .map(String::as_str),
            Some("secret-token")
        );
        assert!(prepared.runtime_path.exists());
        assert!(sessions_dir
            .join(format!("{}.toml", prepared.session.id))
            .exists());
        assert_eq!(
            Path::new(&prepared.session.launch_command)
                .file_stem()
                .and_then(|stem| stem.to_str()),
            Some("codex"),
        );
        assert_eq!(prepared.session.branch, "feature/demo");
    }

    #[test]
    fn docker_finalizer_returns_the_exact_compose_exec_worktree() {
        let temp = tempdir().expect("tempdir");
        let project = temp.path().join("project");
        fs::create_dir_all(&project).expect("create project");
        fs::write(
            project.join("docker-compose.yml"),
            "services:\n  app:\n    image: alpine:3.19\n    working_dir: /workspace/final\n",
        )
        .expect("write compose");
        let mut config = sample_direct_codex_launch_config(&project);
        config.runtime_target = LaunchRuntimeTarget::Docker;
        config.docker_service = Some("app".to_string());
        let runtime = resolved_test_docker_runtime(temp.path());

        let runtime_worktree =
            finalize_docker_agent_launch_config_with_runtime(&project, &mut config, Some(&runtime))
                .expect("finalize Docker launch")
                .expect("Docker runtime worktree");
        let workdir_index = config
            .args
            .iter()
            .position(|arg| arg == "-w")
            .expect("compose exec -w");

        assert_eq!(runtime_worktree, "/workspace/final");
        assert_eq!(config.command, runtime.binary());
        assert_eq!(config.args.get(workdir_index + 1), Some(&runtime_worktree));
    }

    #[cfg(unix)]
    #[test]
    fn docker_finalizer_reuses_one_stateful_runtime_resolution() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempdir().expect("tempdir");
        let project = temp.path().join("project");
        fs::create_dir_all(&project).expect("create project");
        fs::write(
            project.join("docker-compose.yml"),
            "services:\n  app:\n    image: alpine:3.19\n    working_dir: /workspace/final\n",
        )
        .expect("write compose");
        let wrapper = temp.path().join("stateful-container-wrapper");
        fs::write(
            &wrapper,
            r#"#!/bin/sh
counter="$0.count"
count=0
if [ -f "$counter" ]; then
  read count < "$counter"
fi
count=$((count + 1))
printf '%s\n' "$count" > "$counter"
if [ "$count" -eq 1 ]; then
  printf 'Docker version 28.3.0, build test\n'
else
  printf 'podman version 5.4.2\n'
fi
"#,
        )
        .expect("write stateful wrapper");
        let mut permissions = fs::metadata(&wrapper)
            .expect("wrapper metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&wrapper, permissions).expect("chmod stateful wrapper");
        // test-hygiene: allow-production-probe-deadline Integration test observes real CLI invocation counts and pinned binary reuse.
        let runtime = gwt_docker::detect::ResolvedContainerRuntime::resolve(
            wrapper.to_str().expect("UTF-8 wrapper path"),
        )
        .expect("resolve launch runtime once");
        let mut config = sample_direct_codex_launch_config(&project);
        config.runtime_target = LaunchRuntimeTarget::Docker;
        config.docker_service = Some("app".to_string());

        let runtime_worktree =
            finalize_docker_agent_launch_config_with_runtime(&project, &mut config, Some(&runtime))
                .expect("finalize Docker launch")
                .expect("Docker runtime worktree");

        assert_eq!(runtime_worktree, "/workspace/final");
        assert_eq!(config.command, wrapper.to_string_lossy());
        assert_eq!(
            fs::read_to_string(wrapper.with_extension("count"))
                .expect("read wrapper probe count")
                .trim(),
            "1",
            "agent finalization must not re-probe the pinned runtime"
        );
    }

    #[test]
    fn docker_prepare_persists_the_exact_process_worktree_binding() {
        let temp = tempdir().expect("tempdir");
        let project = temp.path().join("project");
        let sessions_dir = temp.path().join("sessions");
        fs::create_dir_all(&project).expect("create project");
        fs::write(
            project.join("docker-compose.yml"),
            "services:\n  app:\n    image: alpine:3.19\n    working_dir: /workspace/final\n",
        )
        .expect("write compose");

        let mut config = sample_direct_codex_launch_config(&project);
        config.runtime_target = LaunchRuntimeTarget::Docker;
        config.docker_service = Some("app".to_string());
        let session = Session::from_launch_config(&project, "feature/demo", &config);
        let session_id = session.id.clone();
        let runtime_path = runtime_state_path(&sessions_dir, &session_id);
        let container_runtime = resolved_test_docker_runtime(temp.path());

        let prepared = finalize_and_persist_prepared_launch(
            &project,
            &sessions_dir,
            config,
            session,
            runtime_path,
            project.clone(),
            PreparedLaunchFinalization {
                container_runtime: Some(&container_runtime),
            },
        )
        .expect("finalize and persist Docker launch");
        let workdir_index = prepared
            .process_launch
            .args
            .iter()
            .position(|arg| arg == "-w")
            .expect("compose exec -w");
        let process_worktree = prepared
            .process_launch
            .args
            .get(workdir_index + 1)
            .expect("compose exec worktree");
        let session_path = sessions_dir.join(format!("{session_id}.toml"));
        let reloaded = Session::load(&session_path).expect("reload persisted Session");
        let binding = reloaded
            .docker_runtime_binding
            .expect("persisted Docker runtime binding");

        assert_eq!(
            binding.runtime_worktree_path,
            PathBuf::from(process_worktree)
        );
        assert_eq!(
            binding.project_state_scope_hash,
            gwt_core::paths::project_scope_hash(&project)
                .as_str()
                .to_string()
        );
        assert_eq!(
            reloaded.project_state_root.as_deref(),
            Some(normalize_child_process_path(&project).as_path())
        );
    }

    #[test]
    fn docker_prepare_does_not_persist_session_when_finalization_fails() {
        let temp = tempdir().expect("tempdir");
        let project = temp.path().join("project-without-compose");
        let sessions_dir = temp.path().join("sessions");
        fs::create_dir_all(&project).expect("create project");

        let mut config = sample_direct_codex_launch_config(&project);
        config.runtime_target = LaunchRuntimeTarget::Docker;
        config.docker_service = Some("app".to_string());
        let session = Session::from_launch_config(&project, "feature/demo", &config);
        let session_path = sessions_dir.join(format!("{}.toml", session.id));
        let runtime_path = runtime_state_path(&sessions_dir, &session.id);
        let container_runtime = resolved_test_docker_runtime(temp.path());

        let result = finalize_and_persist_prepared_launch(
            &project,
            &sessions_dir,
            config,
            session,
            runtime_path.clone(),
            project.clone(),
            PreparedLaunchFinalization {
                container_runtime: Some(&container_runtime),
            },
        );

        assert!(result.is_err(), "missing compose must fail finalization");
        assert!(!session_path.exists(), "Session TOML must not be created");
        assert!(
            !runtime_path.exists(),
            "runtime state must not precede Docker finalization"
        );
    }

    #[test]
    fn docker_bundle_override_content_mounts_gwtd_only_for_agents() {
        let home = PathBuf::from("/home/example");
        let bundle = docker_bundle_mounts_for_home(&home);
        let content =
            docker_bundle_override_content("app", &bundle, "docker").expect("Docker override");

        assert!(content.contains("/home/example/.gwt/bin/gwtd-linux:/usr/local/bin/gwtd:ro"));
        assert!(!content.contains("/usr/local/bin/gwt:ro"));
        assert!(!content.contains("gwtd-linux:/usr/local/bin/gwt:ro"));
        let volume_lines = content
            .lines()
            .filter(|line| line.contains(":/usr/local/bin/gwtd:ro"))
            .collect::<Vec<_>>();
        assert_eq!(volume_lines.len(), 1);
        assert!(volume_lines
            .iter()
            .all(|line| line.trim_start().starts_with("- ")));

        let parsed: serde_yaml::Value =
            serde_yaml::from_str(&content).expect("override must parse as YAML");
        let services = parsed
            .get("services")
            .and_then(|v| v.as_mapping())
            .expect("services key must be a YAML mapping");
        let service_def = services
            .get(serde_yaml::Value::String("app".to_string()))
            .and_then(|v| v.as_mapping())
            .expect("service entry must be a mapping");
        let volumes = service_def
            .get(serde_yaml::Value::String("volumes".to_string()))
            .and_then(|v| v.as_sequence())
            .expect("volumes must be a sequence");
        assert_eq!(volumes.len(), 1);
        assert_eq!(
            service_def
                .get(serde_yaml::Value::String("extra_hosts".to_string()))
                .and_then(|v| v.as_sequence())
                .map(Vec::as_slice),
            Some(
                [serde_yaml::Value::String(
                    gwt_docker::DOCKER_HOST_GATEWAY_EXTRA_HOST.to_string(),
                )]
                .as_slice()
            )
        );
    }

    #[test]
    fn podman_bundle_override_uses_its_reserved_alias_without_docker_mapping() {
        let bundle = docker_bundle_mounts_for_home(Path::new("/home/example"));
        let content =
            docker_bundle_override_content("app", &bundle, "podman").expect("Podman override");
        let parsed: serde_yaml::Value = serde_yaml::from_str(&content).expect("override YAML");

        assert!(parsed["services"]["app"].get("extra_hosts").is_none());
        assert_eq!(
            gwt_docker::ContainerRuntimeKind::Podman.host_bridge_name(),
            "host.containers.internal"
        );
    }

    #[test]
    fn docker_binary_setup_installs_missing_bundle_before_writing_override() {
        let repo = tempdir().expect("repo tempdir");
        let home = tempdir().expect("home tempdir");
        let mut installer_calls = 0;

        ensure_docker_gwt_binary_setup_for_home(
            repo.path(),
            "app",
            home.path(),
            "docker",
            |bundle| {
                installer_calls += 1;
                fs::create_dir_all(bundle.host_gwt.parent().expect("gwt parent"))
                    .expect("create bin dir");
                fs::write(&bundle.host_gwt, b"linux-gwt").expect("write gwt");
                fs::write(&bundle.host_gwtd, b"linux-gwtd").expect("write gwtd");
                Ok(())
            },
        )
        .expect("docker setup");

        let bundle = docker_bundle_mounts_for_home(home.path());
        assert_eq!(installer_calls, 1);
        assert_eq!(fs::read(&bundle.host_gwt).expect("read gwt"), b"linux-gwt");
        assert_eq!(
            fs::read(&bundle.host_gwtd).expect("read gwtd"),
            b"linux-gwtd"
        );

        let override_content = fs::read_to_string(docker_compose_override_path(repo.path()))
            .expect("override content");
        assert!(override_content.contains("gwtd-linux:/usr/local/bin/gwtd:ro"));
        assert!(!override_content.contains("/usr/local/bin/gwt:ro"));
    }

    #[test]
    fn docker_binary_setup_repairs_directory_placeholders_before_writing_override() {
        let repo = tempdir().expect("repo tempdir");
        let home = tempdir().expect("home tempdir");
        let bundle = docker_bundle_mounts_for_home(home.path());
        fs::create_dir_all(&bundle.host_gwt).expect("create gwt placeholder dir");
        fs::create_dir_all(&bundle.host_gwtd).expect("create gwtd placeholder dir");
        let mut installer_calls = 0;

        ensure_docker_gwt_binary_setup_for_home(
            repo.path(),
            "app",
            home.path(),
            "docker",
            |bundle| {
                installer_calls += 1;
                if bundle.host_gwt.is_dir() {
                    fs::remove_dir_all(&bundle.host_gwt).expect("remove gwt placeholder");
                }
                if bundle.host_gwtd.is_dir() {
                    fs::remove_dir_all(&bundle.host_gwtd).expect("remove gwtd placeholder");
                }
                fs::create_dir_all(bundle.host_gwt.parent().expect("gwt parent"))
                    .expect("create bin dir");
                fs::write(&bundle.host_gwt, b"linux-gwt").expect("write gwt");
                fs::write(&bundle.host_gwtd, b"linux-gwtd").expect("write gwtd");
                Ok(())
            },
        )
        .expect("docker setup");

        assert_eq!(installer_calls, 1);
        assert!(bundle.host_gwt.is_file());
        assert!(bundle.host_gwtd.is_file());
        assert!(docker_compose_override_path(repo.path()).is_file());
    }

    #[test]
    fn docker_binary_setup_skips_installer_when_bundle_exists() {
        let repo = tempdir().expect("repo tempdir");
        let home = tempdir().expect("home tempdir");
        let bundle = docker_bundle_mounts_for_home(home.path());
        fs::create_dir_all(bundle.host_gwt.parent().expect("gwt parent")).expect("create bin dir");
        fs::write(&bundle.host_gwt, b"existing-gwt").expect("write gwt");
        fs::write(&bundle.host_gwtd, b"existing-gwtd").expect("write gwtd");

        ensure_docker_gwt_binary_setup_for_home(repo.path(), "app", home.path(), "docker", |_| {
            panic!("installer should not run when both bundle binaries exist");
        })
        .expect("docker setup");

        assert!(docker_compose_override_path(repo.path()).is_file());
    }

    #[test]
    fn docker_binary_setup_reports_managed_override_change_for_recreate() {
        let repo = tempdir().expect("repo tempdir");
        let home = tempdir().expect("home tempdir");
        let bundle = docker_bundle_mounts_for_home(home.path());
        fs::create_dir_all(bundle.host_gwt.parent().expect("gwt parent")).expect("create bin dir");
        fs::write(&bundle.host_gwt, b"existing-gwt").expect("write gwt");
        fs::write(&bundle.host_gwtd, b"existing-gwtd").expect("write gwtd");

        let (override_path, first_changed) = ensure_docker_gwt_binary_setup_for_home(
            repo.path(),
            "app",
            home.path(),
            "docker",
            |_| panic!("installer should not run when both bundle binaries exist"),
        )
        .expect("first Docker setup");
        assert!(
            first_changed,
            "first managed override write requires recreate"
        );
        assert_eq!(override_path, docker_compose_override_path(repo.path()));

        let (_, second_changed) = ensure_docker_gwt_binary_setup_for_home(
            repo.path(),
            "app",
            home.path(),
            "docker",
            |_| panic!("installer should not run when both bundle binaries exist"),
        )
        .expect("second Docker setup");
        assert!(
            !second_changed,
            "byte-identical managed override must not force a second recreate"
        );
    }

    #[test]
    fn docker_launch_compose_files_skips_legacy_generated_default_override() {
        let temp = tempdir().expect("tempdir");
        let project = temp.path().join("repo");
        fs::create_dir_all(&project).expect("project dir");
        let compose_file = project.join("docker-compose.yml");
        fs::write(&compose_file, "services: {}\n").expect("compose file");
        fs::write(
            docker_compose_user_override_path(&project),
            format!("{DOCKER_GWT_OVERRIDE_HEADER}\nservices: {{}}\n"),
        )
        .expect("legacy default override");

        assert_eq!(
            docker_launch_compose_files(&project, &compose_file),
            vec![compose_file]
        );
    }

    #[test]
    fn finalize_docker_agent_launch_config_wraps_compose_exec() {
        let temp = tempdir().expect("tempdir");
        let project = temp.path().join("project");
        fs::create_dir_all(&project).expect("create project");
        fs::write(
            project.join("docker-compose.yml"),
            "services:\n  app:\n    image: alpine:3.19\n    working_dir: /workspace/app\n",
        )
        .expect("write compose file");

        let mut config = AgentLaunchBuilder::new(AgentId::Codex)
            .working_dir(&project)
            .build();
        config.runtime_target = LaunchRuntimeTarget::Docker;
        config.docker_service = Some("app".to_string());
        config.command = "codex".to_string();
        config.args = crate::canonical_launch_args(&AgentId::Codex);
        config.env_vars = HashMap::from([
            (GWT_SESSION_ID_ENV.to_string(), "sess-123".to_string()),
            (
                GWT_SESSION_RUNTIME_PATH_ENV.to_string(),
                "/tmp/runtime/sess-123.json".to_string(),
            ),
            (
                GWT_BIN_PATH_ENV.to_string(),
                DOCKER_GWTD_BIN_PATH.to_string(),
            ),
        ]);
        let runtime = resolved_test_docker_runtime(temp.path());

        let _runtime_worktree =
            finalize_docker_agent_launch_config_with_runtime(&project, &mut config, Some(&runtime))
                .expect("finalize docker");

        assert_eq!(config.command, runtime.binary());
        assert!(config.args.windows(2).any(|pair| {
            pair[0] == "-f" && pair[1] == project.join("docker-compose.yml").display().to_string()
        }));
        assert!(config.args.contains(&"exec".to_string()));
        assert!(config.args.contains(&"app".to_string()));
        assert!(config.args.contains(&"codex".to_string()));
        assert!(config.args.contains(&"--no-alt-screen".to_string()));
        assert_eq!(
            config
                .args
                .iter()
                .filter(|arg| {
                    arg.as_str() == "--config=features.default_mode_request_user_input=true"
                })
                .count(),
            1,
            "Docker wrapping must preserve the canonical Default-mode override"
        );
    }

    #[test]
    fn resolve_launch_worktree_request_noops_when_repo_is_nonrepo_and_base_is_missing() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo dir");

        let mut working_dir = None;
        let mut env_vars = HashMap::new();
        resolve_launch_worktree_request(
            &repo,
            Some("feature/demo"),
            None,
            &mut working_dir,
            &mut env_vars,
        )
        .expect("non-repo without base branch should no-op");

        assert!(working_dir.is_none());
        assert!(env_vars.is_empty());
    }

    #[test]
    fn resolve_launch_worktree_uses_worktree_list_when_branch_probe_would_fail() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        init_git_repo(&repo);
        let branch = "feature/existing";
        let create_branch = gwt_core::process::hidden_command("git")
            .args(["branch", branch])
            .current_dir(&repo)
            .status()
            .expect("create branch");
        assert!(create_branch.success(), "create branch failed");
        let existing_worktree = temp.path().join("feature-existing");
        let add_worktree = gwt_core::process::hidden_command("git")
            .args(["worktree", "add", "-q"])
            .arg(&existing_worktree)
            .arg(branch)
            .current_dir(&repo)
            .status()
            .expect("git worktree add");
        assert!(add_worktree.success(), "git worktree add failed");
        let detach = gwt_core::process::hidden_command("git")
            .args(["checkout", "--detach", "HEAD"])
            .current_dir(&repo)
            .status()
            .expect("git checkout --detach");
        assert!(detach.success(), "git checkout --detach failed");

        let mut working_dir = None;
        let mut env_vars = HashMap::new();
        let result = resolve_launch_worktree_request(
            &repo,
            Some(branch),
            None,
            &mut working_dir,
            &mut env_vars,
        );
        assert!(
            result.is_ok(),
            "selected branch should resolve through git worktree list before current-branch probe: {result:?}"
        );
        assert!(working_dir
            .as_deref()
            .is_some_and(|value| same_path(value, &existing_worktree)));
        assert!(env_vars
            .get("GWT_PROJECT_ROOT")
            .is_some_and(|value| same_path(Path::new(value), &existing_worktree)));
        assert!(branch_worktree_path(&repo, branch)
            .as_deref()
            .is_some_and(|value| same_path(value, &existing_worktree)));
    }

    #[test]
    fn ensure_docker_launch_runtime_ready_emits_preflight_started_and_failure_when_docker_missing()
    {
        use crate::test_capture::{CaptureLayer, CapturedEvents};
        use tracing_subscriber::layer::SubscriberExt;

        let _lock = preflight_env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let previous_bin = std::env::var_os("GWT_DOCKER_BIN");
        std::env::set_var("GWT_DOCKER_BIN", "/this-binary-does-not-exist-for-gwt-test");

        let events = CapturedEvents::new();
        let subscriber = tracing_subscriber::registry().with(CaptureLayer::new(events.clone()));
        let result =
            tracing::subscriber::with_default(subscriber, ensure_docker_launch_runtime_ready);

        match previous_bin {
            Some(value) => std::env::set_var("GWT_DOCKER_BIN", value),
            None => std::env::remove_var("GWT_DOCKER_BIN"),
        }

        assert!(result.is_err(), "expected Err for missing docker binary");

        let captured = events.snapshot();
        let preflight_events: Vec<_> = captured
            .iter()
            .filter(|event| event.target == "gwt::launch::preflight")
            .collect();
        let info_started: Vec<_> = preflight_events
            .iter()
            .filter(|event| {
                event.level == tracing::Level::INFO
                    && event.fields.get("message").map(String::as_str)
                        == Some("docker preflight started")
            })
            .collect();
        let error_failed: Vec<_> = preflight_events
            .iter()
            .filter(|event| {
                event.level == tracing::Level::ERROR
                    && event.fields.get("message").map(String::as_str)
                        == Some("docker preflight failed")
            })
            .collect();
        assert!(
            !info_started.is_empty(),
            "expected an INFO 'docker preflight started' event; captured = {:?}",
            captured
        );
        assert!(
            !error_failed.is_empty(),
            "expected an ERROR 'docker preflight failed' event; captured = {:?}",
            captured
        );
        let started = info_started[0];
        assert_eq!(
            started.fields.get("attempted_binary").map(String::as_str),
            Some("/this-binary-does-not-exist-for-gwt-test")
        );
        assert!(started.fields.contains_key("path"));
    }

    fn preflight_env_test_lock() -> &'static std::sync::Mutex<()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
    }

    fn test_path(entries: &[&str]) -> String {
        std::env::join_paths(entries.iter().map(Path::new))
            .expect("join test PATH entries")
            .to_string_lossy()
            .into_owned()
    }

    fn posix_path_entries(path: &str) -> Vec<&str> {
        path.split(':').collect()
    }

    // SPEC-2077 Phase I1 (US-7 / FR-020 / FR-021 / FR-022 / SC-010):
    // install_launch_gwt_bin_env_with_lookup must prepend the GWT_BIN_PATH
    // parent directory to env_vars["PATH"] so agent subshells can resolve
    // gwtd / gwt directly without the ${GWT_BIN_PATH:-gwtd} indirection.

    #[test]
    fn public_gwt_bin_prefers_checkout_sibling_before_foreign_path_install() {
        let current_exe = PathBuf::from("/checkout/target/debug/gwt");

        let resolved = resolve_public_gwt_bin_with_lookup(&current_exe, |_command| {
            Some(PathBuf::from("/Applications/GWT.app/Contents/MacOS/gwtd"))
        });

        assert_eq!(resolved, PathBuf::from("/checkout/target/debug/gwtd"));
    }

    #[test]
    fn public_gwt_bin_keeps_stable_path_priority_for_bunx_temp_front_door() {
        let current_exe = PathBuf::from(
            r"C:\Temp\bunx-123-@akiojin\gwt@latest\node_modules\@akiojin\gwt\bin\gwt.exe",
        );
        let stable = PathBuf::from(r"C:\Users\Example\.bun\bin\gwtd.exe");

        let resolved =
            resolve_public_gwt_bin_with_lookup(&current_exe, |_command| Some(stable.clone()));

        assert_eq!(resolved, stable);
    }

    #[test]
    fn install_launch_gwt_bin_env_host_prepends_gwtd_dir_to_path() {
        let fixture = tempdir().expect("fixture");
        let bin_dir = fixture.path().join("gwt-bin");
        let current_exe = bin_dir.join("gwt");
        let gwtd = bin_dir.join("gwtd");
        let mut env_vars = HashMap::from([("PATH".to_string(), test_path(&["/usr/bin", "/bin"]))]);
        install_launch_gwt_bin_env_with_lookup(
            &mut env_vars,
            LaunchRuntimeTarget::Host,
            &current_exe,
            |_command| Some(gwtd.clone()),
        )
        .expect("install");

        assert_eq!(
            env_vars.get(GWT_BIN_PATH_ENV).map(String::as_str),
            Some(gwtd.to_string_lossy().as_ref()),
        );
        let path = env_vars.get("PATH").expect("PATH should be set");
        let entries: Vec<PathBuf> = std::env::split_paths(path).collect();
        assert_eq!(
            entries.first().map(|p| p.as_path()),
            Some(bin_dir.as_path()),
            "GWT_BIN_PATH parent dir must be prepended; got {path}",
        );
        assert!(entries.contains(&PathBuf::from("/usr/bin")));
        assert!(entries.contains(&PathBuf::from("/bin")));
    }

    #[test]
    fn install_launch_gwt_bin_env_host_dedups_existing_path_entry() {
        let mut env_vars = HashMap::from([(
            "PATH".to_string(),
            test_path(&["/Applications/GWT.app/Contents/MacOS", "/usr/bin"]),
        )]);
        let current_exe = PathBuf::from("/Applications/GWT.app/Contents/MacOS/gwt");
        install_launch_gwt_bin_env_with_lookup(
            &mut env_vars,
            LaunchRuntimeTarget::Host,
            &current_exe,
            |_command| Some(PathBuf::from("/Applications/GWT.app/Contents/MacOS/gwtd")),
        )
        .expect("install");

        let entries: Vec<PathBuf> =
            std::env::split_paths(env_vars.get("PATH").expect("PATH")).collect();
        assert_eq!(
            entries,
            vec![
                PathBuf::from("/Applications/GWT.app/Contents/MacOS"),
                PathBuf::from("/usr/bin"),
            ],
            "PATH must not contain duplicate GWT_BIN_PATH parent dir",
        );
    }

    #[test]
    fn install_launch_gwt_bin_env_host_skips_path_update_when_parent_is_empty() {
        let original_path = test_path(&["/usr/bin", "/bin"]);
        let mut env_vars = HashMap::from([("PATH".to_string(), original_path.clone())]);
        let current_exe = PathBuf::from("/tmp/bunx-123-gwt/bin/gwt");
        // Lookup returns a bare filename (Path::parent => Some(""))
        install_launch_gwt_bin_env_with_lookup(
            &mut env_vars,
            LaunchRuntimeTarget::Host,
            &current_exe,
            |_command| Some(PathBuf::from("gwtd")),
        )
        .expect("install");

        assert_eq!(
            env_vars.get(GWT_BIN_PATH_ENV).map(String::as_str),
            Some("gwtd"),
        );
        assert_eq!(
            env_vars.get("PATH").map(String::as_str),
            Some(original_path.as_str()),
            "empty GWT_BIN_PATH parent must be a no-op",
        );
    }

    #[test]
    fn install_launch_gwt_bin_env_host_creates_path_when_absent() {
        let mut env_vars: HashMap<String, String> = HashMap::new();
        let current_exe = PathBuf::from("/Applications/GWT.app/Contents/MacOS/gwt");
        install_launch_gwt_bin_env_with_lookup(
            &mut env_vars,
            LaunchRuntimeTarget::Host,
            &current_exe,
            |_command| Some(PathBuf::from("/Applications/GWT.app/Contents/MacOS/gwtd")),
        )
        .expect("install");

        let path = env_vars.get("PATH").expect("PATH should be created");
        let entries: Vec<PathBuf> = std::env::split_paths(path).collect();
        assert_eq!(
            entries,
            vec![PathBuf::from("/Applications/GWT.app/Contents/MacOS")],
        );
    }

    #[test]
    fn install_launch_gwt_bin_env_docker_dedups_when_dir_already_on_path() {
        let mut env_vars =
            HashMap::from([("PATH".to_string(), "/usr/local/bin:/usr/bin".to_string())]);
        install_launch_gwt_bin_env_with_lookup(
            &mut env_vars,
            LaunchRuntimeTarget::Docker,
            Path::new("/never/used/in/docker"),
            |_command| None,
        )
        .expect("install");

        assert_eq!(
            env_vars.get(GWT_BIN_PATH_ENV).map(String::as_str),
            Some("/usr/local/bin/gwtd"),
        );
        let entries = posix_path_entries(env_vars.get("PATH").expect("PATH"));
        assert_eq!(
            entries,
            vec!["/usr/local/bin", "/usr/bin"],
            "Docker dir already on PATH must dedup",
        );
    }

    #[test]
    fn install_launch_gwt_bin_env_docker_prepends_when_dir_missing_from_path() {
        let mut env_vars = HashMap::from([("PATH".to_string(), "/usr/bin:/bin".to_string())]);
        install_launch_gwt_bin_env_with_lookup(
            &mut env_vars,
            LaunchRuntimeTarget::Docker,
            Path::new("/never/used/in/docker"),
            |_command| None,
        )
        .expect("install");

        let entries = posix_path_entries(env_vars.get("PATH").expect("PATH"));
        assert_eq!(
            entries.first().copied(),
            Some("/usr/local/bin"),
            "Docker dir not on PATH must be prepended; got entries: {entries:?}",
        );
    }

    #[test]
    fn prepend_dir_to_path_preserves_windows_style_path_key() {
        // CodeRabbit P1 regression follow-up: Windows processes may expose
        // the path variable as "Path" (case-insensitive at OS level).
        // prepend_dir_to_path must update the existing key in place rather
        // than creating a duplicate "PATH" entry alongside "Path", otherwise
        // the spawned child process inherits both and command lookup is
        // corrupted. Test values use the platform-native PATH separator so
        // split_paths / join_paths round-trip on the host test runner.
        let existing = std::env::join_paths([Path::new("/usr/bin"), Path::new("/bin")])
            .expect("join_paths existing entries");
        let mut env_vars =
            HashMap::from([("Path".to_string(), existing.to_string_lossy().into_owned())]);
        let updated = prepend_dir_to_path(&mut env_vars, Path::new("/opt/gwt/bin"));
        assert!(updated, "PATH must be updated for Windows-style key");
        assert!(
            !env_vars.contains_key("PATH"),
            "no duplicate uppercase PATH key may be created when Path exists: {env_vars:?}",
        );
        let path = env_vars.get("Path").expect("Path key must be preserved");
        let entries: Vec<PathBuf> = std::env::split_paths(path).collect();
        assert_eq!(
            entries.first().map(|p| p.as_path()),
            Some(Path::new("/opt/gwt/bin")),
            "prepended dir must lead the Path value; got {path}",
        );
        assert!(entries.iter().any(|p| p == Path::new("/usr/bin")));
        assert!(entries.iter().any(|p| p == Path::new("/bin")));
    }

    #[test]
    fn prepend_dir_to_path_preserves_lowercase_path_key() {
        let existing = std::env::join_paths([Path::new("/usr/bin"), Path::new("/bin")])
            .expect("join_paths existing entries");
        let mut env_vars =
            HashMap::from([("path".to_string(), existing.to_string_lossy().into_owned())]);
        let updated = prepend_dir_to_path(&mut env_vars, Path::new("/opt/gwt/bin"));
        assert!(updated);
        assert!(
            !env_vars.contains_key("PATH"),
            "no duplicate uppercase PATH key may be created when lowercase path exists: {env_vars:?}",
        );
        let entries: Vec<PathBuf> =
            std::env::split_paths(env_vars.get("path").expect("path key")).collect();
        assert_eq!(
            entries.first().map(|p| p.as_path()),
            Some(Path::new("/opt/gwt/bin")),
        );
    }

    #[test]
    fn prepend_dir_to_path_dedups_case_insensitive_existing_dir() {
        let existing = std::env::join_paths([Path::new("/opt/gwt/bin"), Path::new("/usr/bin")])
            .expect("join_paths existing entries");
        let original_value = existing.to_string_lossy().into_owned();
        let mut env_vars = HashMap::from([("Path".to_string(), original_value.clone())]);
        let updated = prepend_dir_to_path(&mut env_vars, Path::new("/opt/gwt/bin"));
        assert!(
            !updated,
            "existing entry must be a no-op regardless of key case"
        );
        assert!(!env_vars.contains_key("PATH"));
        assert_eq!(
            env_vars.get("Path").map(String::as_str),
            Some(original_value.as_str())
        );
    }

    #[test]
    fn install_launch_gwt_bin_env_host_preserves_existing_path_order() {
        let mut env_vars = HashMap::from([(
            "PATH".to_string(),
            test_path(&["/profile/bin", "/usr/bin", "/bin"]),
        )]);
        let current_exe = PathBuf::from("/Applications/GWT.app/Contents/MacOS/gwt");
        install_launch_gwt_bin_env_with_lookup(
            &mut env_vars,
            LaunchRuntimeTarget::Host,
            &current_exe,
            |_command| Some(PathBuf::from("/Applications/GWT.app/Contents/MacOS/gwtd")),
        )
        .expect("install");

        let entries: Vec<PathBuf> =
            std::env::split_paths(env_vars.get("PATH").expect("PATH")).collect();
        assert_eq!(
            entries,
            vec![
                PathBuf::from("/Applications/GWT.app/Contents/MacOS"),
                PathBuf::from("/profile/bin"),
                PathBuf::from("/usr/bin"),
                PathBuf::from("/bin"),
            ],
            "existing PATH order must be preserved after prepend",
        );
    }

    #[test]
    fn prepare_agent_launch_host_prepends_gwtd_dir_to_path() {
        let temp = tempdir().expect("tempdir");
        let worktree = temp.path().join("repo-feature");
        let sessions_dir = temp.path().join(".gwt").join("sessions");
        fs::create_dir_all(&worktree).expect("create worktree");

        let mut config = sample_direct_codex_launch_config(&worktree);
        config
            .env_vars
            .insert("PATH".to_string(), test_path(&["/usr/bin", "/bin"]));

        let mut probe_host_runner =
            |_kind: HostRunnerProbeKind,
             _command: &str,
             _args: Vec<String>,
             _env: &HashMap<String, String>,
             _remove_env: &[String],
             _cwd: Option<PathBuf>| { HostRunnerProbeOutcome::success() };
        let bin_dir = temp.path().join("gwt-bin");
        let current_exe = bin_dir.join("gwt");
        let gwtd = bin_dir.join("gwtd");
        let lookup_gwtd = gwtd.clone();
        let lookup_gwt_bin = move |_command: &str| Some(lookup_gwtd.clone());

        let prepared = prepare_agent_launch_with(
            &worktree,
            &sessions_dir,
            config,
            None,
            |_path| Ok(()),
            PrepareLaunchDeps {
                current_exe: &current_exe,
                probe_host_runner: &mut probe_host_runner,
                lookup_gwt_bin: &lookup_gwt_bin,
            },
        )
        .expect("prepare launch");

        assert_eq!(
            prepared
                .process_launch
                .env
                .get(GWT_BIN_PATH_ENV)
                .map(String::as_str),
            Some(gwtd.to_string_lossy().as_ref()),
        );
        let path = prepared
            .process_launch
            .env
            .get("PATH")
            .expect("PATH should be set after install_launch_gwt_bin_env_with_lookup");
        let entries: Vec<PathBuf> = std::env::split_paths(path).collect();
        assert_eq!(
            entries.first().map(|p| p.as_path()),
            Some(bin_dir.as_path()),
            "agent process PATH must start with GWT_BIN_PATH parent dir; got {path}",
        );
        assert!(entries.contains(&PathBuf::from("/usr/bin")));
        assert!(entries.contains(&PathBuf::from("/bin")));
    }

    #[cfg(windows)]
    #[test]
    fn direct_runner_resolution_failure_still_emits_an_end_summary() {
        use crate::test_capture::{CaptureLayer, CapturedEvents};
        use tracing_subscriber::layer::SubscriberExt;

        let temp = tempdir().expect("tempdir");
        let placeholder = temp.path().join("codex.exe");
        fs::write(&placeholder, "Error: native binary not installed\r\n")
            .expect("write unsafe placeholder");
        let env = HashMap::from([
            ("PATH".to_string(), temp.path().display().to_string()),
            ("PATHEXT".to_string(), ".EXE".to_string()),
        ]);
        let events = CapturedEvents::new();
        let subscriber = tracing_subscriber::registry().with(CaptureLayer::new(events.clone()));

        let result = tracing::subscriber::with_default(subscriber, || {
            probe_host_runner_outcome(
                HostRunnerProbeKind::Direct,
                "codex",
                vec!["--version".to_string()],
                &env,
                &[],
                None,
            )
            .success
        });

        assert!(!result);
        let summaries = events
            .snapshot()
            .into_iter()
            .filter(|event| event.target == "gwt.process.summary")
            .collect::<Vec<_>>();
        assert_eq!(summaries.len(), 2, "expected balanced start/end events");
        assert_eq!(
            summaries[1].fields.get("phase").map(String::as_str),
            Some("end")
        );
        assert_eq!(
            summaries[1].fields.get("success").map(String::as_str),
            Some("false")
        );
        assert!(summaries[1].fields.contains_key("resolution_error"));
    }

    #[test]
    fn transient_launch_failure_classifier_covers_fetch_races() {
        assert!(is_transient_launch_failure(
            "failed to prepare origin/develop for Start Work: fetch origin: error: fetching ref refs/remotes/origin/develop failed: incorrect old value provided"
        ));
        assert!(is_transient_launch_failure(
            "failed to fetch origin: fetch origin: error: cannot lock ref 'refs/remotes/origin/develop': is at a but expected b"
        ));
        assert!(!is_transient_launch_failure(
            "exact npx package probe failed for @openai/codex@0.153.2. Probe detail: exit status 1."
        ));
        assert!(!is_transient_launch_failure(
            "failed to prepare origin/develop for Start Work: fatal: could not read Username"
        ));
    }
}
