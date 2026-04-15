//! Agent detection, launch, and session management for gwt.
//!
//! This crate provides a unified interface for discovering, configuring,
//! launching, and tracking coding agent sessions (Claude Code, Codex,
//! Gemini, OpenCode, Copilot, and custom agents).

pub mod custom;
pub mod detect;
pub mod launch;
pub mod session;
pub mod types;
pub mod version_cache;

pub use custom::CustomCodingAgent;
pub use detect::{AgentDetector, DetectedAgent};
pub use launch::{
    normalize_launch_args, resolve_runner, AgentLaunchBuilder, LaunchConfig, ResolvedRunner,
};
pub use session::{
    persist_session_status, reset_runtime_state_dir, reset_runtime_state_dir_for_pid,
    runtime_state_dir_for_pid, runtime_state_path, runtime_state_path_for_pid,
    PendingDiscussionResume, Session, SessionRuntimeState, GWT_SESSION_ID_ENV,
    GWT_SESSION_RUNTIME_PATH_ENV, GWT_BIN_PATH_ENV,
};
pub use types::{
    AgentColor, AgentId, AgentInfo, AgentStatus, DockerLifecycleIntent, LaunchRuntimeTarget,
    SessionMode, WorkflowBypass,
};
pub use version_cache::{build_version_options, VersionCache, VersionOption};
