//! Agent detection, launch, and session management for gwt.
//!
//! This crate provides a unified interface for discovering, configuring,
//! launching, and tracking coding agent sessions (Claude Code, Codex,
//! Gemini, OpenCode, Copilot, and custom agents).

pub mod audit;
pub mod backend;
pub mod backend_store;
pub mod claude_capabilities;
pub mod custom;
pub mod detect;
pub mod environment;
pub mod launch;
pub mod migration;
pub mod prepare;
pub mod presets;
pub mod session;
pub mod store;
pub mod types;
pub mod version_cache;

#[cfg(test)]
pub(crate) mod test_capture;

pub use audit::{
    is_secret_env_key, redact_env_value_for_audit, redact_secrets_in_agent, REDACTED_PLACEHOLDER,
};
pub use backend::{AgentBackendProfile, BuiltinAgentId};
pub use backend_store::{
    add_backend, delete_backend, load_backends_for_agent, save_backends_for_agent, update_backend,
};
pub use claude_capabilities::{
    claude_capability_snapshot, claude_ultracode_supported, claude_workflows_enabled,
    detect_claude_version_raw, parse_claude_semver, supports_ultracode, workflows_enabled_from,
    ClaudeCapabilitySnapshot,
};
pub use custom::CustomCodingAgent;
pub use detect::{AgentDetector, DetectedAgent};
pub use environment::LaunchEnvironment;
pub use launch::{
    apply_host_bunx_cache_fast_path, canonical_launch_args, normalize_launch_args,
    resolve_host_npx_fallback_executable, resolve_runner, AgentLaunchBuilder,
    ExecutionLaunchIntent, LaunchConfig, ManualLaunchRuntimeEvidence, ManualLaunchRuntimeProof,
    ManualLaunchSuccessorPredecessor, ResolvedRunner,
};
pub use migration::{migrate_legacy_backend_rows, resolve_legacy_backend_remap, MigrationReport};
pub use prepare::{
    apply_host_package_runner_fallback, apply_host_package_runner_fallback_with_probe,
    branch_worktree_path, hook_forward_url_for_launch_runtime, install_launch_gwt_bin_env,
    install_launch_gwt_bin_env_with_lookup, pane_websocket_url_for_launch_runtime,
    prepare_agent_launch, register_codex_managed_hook_trust_in_docker,
    resolve_host_runner_health_checked, resolve_host_runner_health_checked_with_probe_and_repair,
    resolve_launch_worktree, resolve_launch_worktree_request, resolve_public_gwt_bin_with_lookup,
    HookForwardEnv, HostRunnerHealthReport, HostRunnerProbeKind, HostRunnerProbeOutcome,
    PreparedAgentLaunch, PreparedProcessLaunch, ResolvedHostPackagePlan,
    WindowsNpxCacheRepairCandidate,
};
pub use presets::{
    claude_code_openai_compat_preset, list_presets, seed_agent, ClaudeCodeOpenaiCompatInput,
    PresetDefinition, PresetError, PresetId,
};
pub use session::{
    active_launch_handshake_path, begin_session_active_launch_handshake_under_lease,
    begin_session_manual_handoff_under_lease, clear_session_active_launch_handshake_under_lease,
    clear_session_manual_handoff_under_lease, current_thread_holds_session_lease,
    durable_session_launch_command, inspect_session_path, manual_handoff_path,
    mark_session_active_launch_handshake_child_spawned_under_lease, persist_agent_session_id,
    persist_session_completed_stop, persist_session_execution_binding, persist_session_hook_event,
    persist_session_restore_window_on_startup,
    persist_session_running_state_if_execution_identity_matches, persist_session_status,
    persist_session_terminal_status_for_exact_runtime_under_lease,
    persist_session_terminal_status_if_execution_identity_matches,
    persist_session_terminal_status_if_execution_identity_matches_under_lease,
    read_session_active_launch_handshake_under_lease, read_session_manual_handoff_under_lease,
    remove_session_if_execution_identity_matches,
    remove_session_if_execution_identity_matches_or_missing_with,
    remove_session_if_execution_identity_matches_with, reset_runtime_state_dir,
    reset_runtime_state_dir_for_pid, rotate_session_execution_capability,
    runtime_state_dir_for_pid, runtime_state_path, runtime_state_path_for_pid,
    session_active_launch_handshake_matches_under_lease,
    session_manual_handoff_matches_under_lease, sessions_dir_from_runtime_path, update_session,
    update_session_if_changed, validate_docker_runtime_worktree_path,
    validate_session_id_path_component, with_session_lease, with_session_lease_wait,
    with_session_path_lease, with_session_path_lease_wait, AgentSessionHistoryEntry,
    DockerRuntimeBinding, ExecutionBindingIdentity, PendingDiscussionResume, Session,
    SessionActiveLaunchHandshake, SessionActiveLaunchPhase, SessionExecutionBinding,
    SessionExecutionIdentity, SessionManualHandoffFence, SessionPathState, SessionRuntimeState,
    ToolRuntimeProvenance, ToolRuntimeResolutionReason, ToolRuntimeRunnerKind, GWT_BIN_PATH_ENV,
    GWT_CONTINUE_WORK_READY_NONCE_ENV, GWT_HOOK_FORWARD_TOKEN_ENV, GWT_HOOK_FORWARD_URL_ENV,
    GWT_PANE_WS_URL_ENV, GWT_SESSION_ID_ENV, GWT_SESSION_RUNTIME_PATH_ENV,
};
pub use store::{
    load_custom_agents_from_path, load_stored_custom_agents_from_path,
    migrate_and_load_stored_custom_agents, save_stored_custom_agents_to_path, StoredCustomAgent,
    DISABLE_GLOBAL_CUSTOM_AGENTS_ENV,
};
pub use types::{
    builtin_agent_descriptor_for_command, builtin_agent_descriptors, resolve_agent_id, AgentColor,
    AgentId, AgentInfo, AgentStatus, BuiltinAgentDescriptor, DockerLifecycleIntent,
    LaunchRuntimeTarget, SessionMode, WindowsShellKind, WorkflowBypass,
};
pub use version_cache::{build_version_options, VersionCache, VersionOption};
