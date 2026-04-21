//! Agent detection, launch, and session management for gwt.
//!
//! This crate provides a unified interface for discovering, configuring,
//! launching, and tracking coding agent sessions (Claude Code, Codex,
//! Gemini, OpenCode, Copilot, and custom agents).

pub mod audit;
pub mod custom;
pub mod detect;
pub mod launch;
pub mod presets;
pub mod session;
pub mod store;
pub mod types;
pub mod version_cache;

pub use audit::{
    is_secret_env_key, redact_env_value_for_audit, redact_secrets_in_agent, REDACTED_PLACEHOLDER,
};
pub use custom::CustomCodingAgent;
pub use detect::{AgentDetector, DetectedAgent};
pub use launch::{
    canonical_launch_args, normalize_launch_args, resolve_runner, AgentLaunchBuilder, LaunchConfig,
    ResolvedRunner,
};
pub use presets::{
    claude_code_openai_compat_preset, list_presets, seed_agent, ClaudeCodeOpenaiCompatInput,
    PresetDefinition, PresetError, PresetId,
};
pub use session::{
    persist_agent_session_id, persist_session_status, reset_runtime_state_dir,
    reset_runtime_state_dir_for_pid, runtime_state_dir_for_pid, runtime_state_path,
    runtime_state_path_for_pid, PendingDiscussionResume, Session, SessionRuntimeState,
    GWT_BIN_PATH_ENV, GWT_HOOK_FORWARD_TOKEN_ENV, GWT_HOOK_FORWARD_URL_ENV, GWT_SESSION_ID_ENV,
    GWT_SESSION_RUNTIME_PATH_ENV,
};
pub use store::{
    load_custom_agents_from_path, load_stored_custom_agents_from_path,
    save_stored_custom_agents_to_path, StoredCustomAgent, DISABLE_GLOBAL_CUSTOM_AGENTS_ENV,
};
pub use types::{
    AgentColor, AgentId, AgentInfo, AgentStatus, DockerLifecycleIntent, LaunchRuntimeTarget,
    SessionMode, WorkflowBypass,
};
pub use version_cache::{build_version_options, VersionCache, VersionOption};
