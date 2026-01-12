//! Error types for gwt-core
//!
//! Error codes are categorized as follows:
//! - E1xxx: Git operation errors
//! - E2xxx: Worktree operation errors
//! - E3xxx: Configuration errors
//! - E4xxx: Agent launch errors
//! - E5xxx: Web API errors

use std::path::PathBuf;
use thiserror::Error;

/// Result type alias using GwtError
pub type Result<T> = std::result::Result<T, GwtError>;

/// Main error type for gwt-core
#[derive(Error, Debug)]
pub enum GwtError {
    // E1xxx: Git operation errors
    #[error("[E1001] Repository not found: {path}")]
    RepositoryNotFound { path: PathBuf },

    #[error("[E1002] Not a git repository: {path}")]
    NotAGitRepository { path: PathBuf },

    #[error("[E1003] Branch not found: {name}")]
    BranchNotFound { name: String },

    #[error("[E1004] Branch already exists: {name}")]
    BranchAlreadyExists { name: String },

    #[error("[E1005] Remote not found: {name}")]
    RemoteNotFound { name: String },

    #[error("[E1006] Fetch failed: {reason}")]
    FetchFailed { reason: String },

    #[error("[E1007] Pull failed (not fast-forward): {reason}")]
    PullNotFastForward { reason: String },

    #[error("[E1008] Uncommitted changes detected")]
    UncommittedChanges,

    #[error("[E1009] Unpushed commits detected")]
    UnpushedCommits,

    #[error("[E1010] Branch diverged from remote: {branch}")]
    BranchDiverged { branch: String },

    #[error("[E1011] Git command failed: {command}")]
    GitCommandFailed { command: String, reason: String },

    #[error("[E1012] Git executable not found")]
    GitNotFound,

    #[error("[E1013] Git operation failed: {operation}: {details}")]
    GitOperationFailed { operation: String, details: String },

    #[error("[E1014] Branch create failed: {name}: {details}")]
    BranchCreateFailed { name: String, details: String },

    #[error("[E1015] Branch delete failed: {name}: {details}")]
    BranchDeleteFailed { name: String, details: String },

    // E2xxx: Worktree operation errors
    #[error("[E2001] Worktree not found: {path}")]
    WorktreeNotFound { path: PathBuf },

    #[error("[E2002] Worktree already exists: {path}")]
    WorktreeAlreadyExists { path: PathBuf },

    #[error("[E2003] Failed to create worktree: {reason}")]
    WorktreeCreateFailed { reason: String },

    #[error("[E2004] Failed to remove worktree: {reason}")]
    WorktreeRemoveFailed { reason: String },

    #[error("[E2005] Protected branch cannot be deleted: {branch}")]
    ProtectedBranch { branch: String },

    #[error("[E2006] Worktree path invalid: {path}")]
    WorktreePathInvalid { path: PathBuf },

    #[error("[E2007] Orphaned worktree detected: {path}")]
    OrphanedWorktree { path: PathBuf },

    #[error("[E2008] Worktree locked by another process: {path}")]
    WorktreeLocked { path: PathBuf },

    #[error("[E2009] Path exists but not a stale worktree - please remove manually: {path}")]
    WorktreePathConflict { path: PathBuf },

    // E3xxx: Configuration errors
    #[error("[E3001] Configuration file not found: {path}")]
    ConfigNotFound { path: PathBuf },

    #[error("[E3002] Configuration parse error: {reason}")]
    ConfigParseError { reason: String },

    #[error("[E3003] Configuration write error: {reason}")]
    ConfigWriteError { reason: String },

    #[error("[E3004] Invalid configuration value: {key} = {value}")]
    ConfigInvalidValue { key: String, value: String },

    #[error("[E3005] Profile not found: {name}")]
    ProfileNotFound { name: String },

    #[error("[E3006] Session not found: {id}")]
    SessionNotFound { id: String },

    #[error("[E3007] Migration failed (JSON to TOML): {reason}")]
    MigrationFailed { reason: String },

    // E4xxx: Agent launch errors
    #[error("[E4001] Agent not found: {name}")]
    AgentNotFound { name: String },

    #[error("[E4002] Agent launch failed: {name}, reason: {reason}")]
    AgentLaunchFailed { name: String, reason: String },

    #[error("[E4003] Agent configuration invalid: {name}")]
    AgentConfigInvalid { name: String },

    #[error("[E4004] Agent process terminated unexpectedly: {name}")]
    AgentTerminated { name: String },

    // E5xxx: Web API errors
    #[error("[E5001] Server bind failed: {address}")]
    ServerBindFailed { address: String },

    #[error("[E5002] WebSocket connection failed: {reason}")]
    WebSocketFailed { reason: String },

    #[error("[E5003] API request failed: {endpoint}")]
    ApiRequestFailed { endpoint: String },

    #[error("[E5004] PTY spawn failed: {reason}")]
    PtySpawnFailed { reason: String },

    // Generic errors
    #[error("[E9001] IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("[E9002] Internal error: {0}")]
    Internal(String),
}

impl GwtError {
    /// Get the error code as a string (e.g., "E1001")
    pub fn code(&self) -> &'static str {
        match self {
            // E1xxx
            Self::RepositoryNotFound { .. } => "E1001",
            Self::NotAGitRepository { .. } => "E1002",
            Self::BranchNotFound { .. } => "E1003",
            Self::BranchAlreadyExists { .. } => "E1004",
            Self::RemoteNotFound { .. } => "E1005",
            Self::FetchFailed { .. } => "E1006",
            Self::PullNotFastForward { .. } => "E1007",
            Self::UncommittedChanges => "E1008",
            Self::UnpushedCommits => "E1009",
            Self::BranchDiverged { .. } => "E1010",
            Self::GitCommandFailed { .. } => "E1011",
            Self::GitNotFound => "E1012",
            Self::GitOperationFailed { .. } => "E1013",
            Self::BranchCreateFailed { .. } => "E1014",
            Self::BranchDeleteFailed { .. } => "E1015",
            // E2xxx
            Self::WorktreeNotFound { .. } => "E2001",
            Self::WorktreeAlreadyExists { .. } => "E2002",
            Self::WorktreeCreateFailed { .. } => "E2003",
            Self::WorktreeRemoveFailed { .. } => "E2004",
            Self::ProtectedBranch { .. } => "E2005",
            Self::WorktreePathInvalid { .. } => "E2006",
            Self::OrphanedWorktree { .. } => "E2007",
            Self::WorktreeLocked { .. } => "E2008",
            Self::WorktreePathConflict { .. } => "E2009",
            // E3xxx
            Self::ConfigNotFound { .. } => "E3001",
            Self::ConfigParseError { .. } => "E3002",
            Self::ConfigWriteError { .. } => "E3003",
            Self::ConfigInvalidValue { .. } => "E3004",
            Self::ProfileNotFound { .. } => "E3005",
            Self::SessionNotFound { .. } => "E3006",
            Self::MigrationFailed { .. } => "E3007",
            // E4xxx
            Self::AgentNotFound { .. } => "E4001",
            Self::AgentLaunchFailed { .. } => "E4002",
            Self::AgentConfigInvalid { .. } => "E4003",
            Self::AgentTerminated { .. } => "E4004",
            // E5xxx
            Self::ServerBindFailed { .. } => "E5001",
            Self::WebSocketFailed { .. } => "E5002",
            Self::ApiRequestFailed { .. } => "E5003",
            Self::PtySpawnFailed { .. } => "E5004",
            // E9xxx
            Self::Io(_) => "E9001",
            Self::Internal(_) => "E9002",
        }
    }

    /// Get the error category
    pub fn category(&self) -> ErrorCategory {
        match self.code().chars().nth(1).and_then(|c| c.to_digit(10)) {
            Some(1) => ErrorCategory::Git,
            Some(2) => ErrorCategory::Worktree,
            Some(3) => ErrorCategory::Config,
            Some(4) => ErrorCategory::Agent,
            Some(5) => ErrorCategory::WebApi,
            _ => ErrorCategory::Internal,
        }
    }
}

/// Error category for grouping errors
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCategory {
    Git,
    Worktree,
    Config,
    Agent,
    WebApi,
    Internal,
}

impl std::fmt::Display for ErrorCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Git => write!(f, "Git"),
            Self::Worktree => write!(f, "Worktree"),
            Self::Config => write!(f, "Config"),
            Self::Agent => write!(f, "Agent"),
            Self::WebApi => write!(f, "WebApi"),
            Self::Internal => write!(f, "Internal"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_code() {
        let err = GwtError::RepositoryNotFound {
            path: PathBuf::from("/tmp/repo"),
        };
        assert_eq!(err.code(), "E1001");
        assert_eq!(err.category(), ErrorCategory::Git);
    }

    #[test]
    fn test_error_display() {
        let err = GwtError::BranchNotFound {
            name: "feature/test".to_string(),
        };
        assert!(err.to_string().contains("[E1003]"));
        assert!(err.to_string().contains("feature/test"));
    }

    #[test]
    fn test_error_category() {
        assert_eq!(
            GwtError::RepositoryNotFound {
                path: PathBuf::new()
            }
            .category(),
            ErrorCategory::Git
        );
        assert_eq!(
            GwtError::WorktreeNotFound {
                path: PathBuf::new()
            }
            .category(),
            ErrorCategory::Worktree
        );
        assert_eq!(
            GwtError::ConfigNotFound {
                path: PathBuf::new()
            }
            .category(),
            ErrorCategory::Config
        );
    }
}
