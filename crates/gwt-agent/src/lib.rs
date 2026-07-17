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
pub mod legacy_recovery;
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
    claude_ultracode_supported, claude_workflows_enabled, detect_claude_version_raw,
    parse_claude_semver, supports_ultracode, workflows_enabled_from,
};
pub use custom::CustomCodingAgent;
pub use detect::{AgentDetector, DetectedAgent};
pub use environment::LaunchEnvironment;
pub use launch::{
    canonical_launch_args, normalize_launch_args, resolve_host_npx_fallback_executable,
    resolve_runner, AgentLaunchBuilder, LaunchConfig, RecoveryContinuationHandoff, ResolvedRunner,
};
pub use legacy_recovery::{
    import_legacy_recovery_placeholders, import_legacy_recovery_sessions,
    LegacyRecoveryAttentionReason, LegacyRecoveryImportError, LegacyRecoveryImportReport,
    LegacyRecoveryImportStage, LegacyRecoveryImportedAttention, LegacyRecoveryImportedExact,
    LegacyRecoveryPlaceholder, LegacyRecoveryPlaceholderKind, LegacyRecoveryPlaceholderSource,
    LegacyRecoverySkipReason, LegacyRecoverySkipped,
};
pub use migration::{migrate_legacy_backend_rows, resolve_legacy_backend_remap, MigrationReport};
pub use prepare::{
    apply_host_package_runner_fallback, apply_host_package_runner_fallback_with_probe,
    branch_worktree_path, hook_forward_url_for_launch_runtime, install_launch_gwt_bin_env,
    install_launch_gwt_bin_env_with_lookup, prepare_agent_launch,
    register_codex_managed_hook_trust_in_docker, resolve_launch_worktree,
    resolve_launch_worktree_request, resolve_public_gwt_bin_with_lookup, HookForwardEnv,
    PreparedAgentLaunch, PreparedProcessLaunch,
};
pub use presets::{
    claude_code_openai_compat_preset, list_presets, seed_agent, ClaudeCodeOpenaiCompatInput,
    PresetDefinition, PresetError, PresetId,
};
pub use session::{
    discover_session_toml_paths, persist_agent_session_id, persist_observed_provider_session_id,
    persist_recovery_launch_stage, persist_session_completed_stop, persist_session_hook_event,
    persist_session_restore_window_on_startup, persist_session_status, reset_runtime_state_dir,
    reset_runtime_state_dir_for_pid, runtime_state_dir_for_pid, runtime_state_path,
    runtime_state_path_for_pid, sessions_dir_from_runtime_path, update_session,
    update_session_with_inventory_budget, AgentSessionHistoryEntry, PendingDiscussionResume,
    ProviderRootObservationRole, Session, SessionInventoryReadBudget, SessionRuntimeState,
    GWT_BIN_PATH_ENV, GWT_HOOK_FORWARD_TOKEN_ENV, GWT_HOOK_FORWARD_URL_ENV, GWT_RECOVERY_ID_ENV,
    GWT_SESSION_ID_ENV, GWT_SESSION_RUNTIME_PATH_ENV, MAX_SESSION_DIRECTORY_ENTRIES,
    MAX_SESSION_TOML_AGGREGATE_BYTES, MAX_SESSION_TOML_BYTES, MAX_SESSION_TOML_FILES,
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
