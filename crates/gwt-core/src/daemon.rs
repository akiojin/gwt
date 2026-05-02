//! Shared runtime-daemon contract types and bootstrap helpers.

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    error::{GwtError, Result},
    paths::{ensure_dir, project_scope_hash},
    worktree_hash::compute_worktree_hash,
};

/// Protocol version spoken by `gwt` and `gwtd`.
pub const DAEMON_PROTOCOL_VERSION: u32 = 1;

/// Runtime backend target for daemon-managed execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeTarget {
    Host,
    Docker,
}

/// Scope key identifying one daemon ownership boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeScope {
    pub repo_hash: String,
    pub worktree_hash: String,
    pub project_root: PathBuf,
    pub target: RuntimeTarget,
}

impl RuntimeScope {
    pub fn new(
        repo_hash: impl Into<String>,
        worktree_hash: impl Into<String>,
        project_root: PathBuf,
        target: RuntimeTarget,
    ) -> Result<Self> {
        let repo_hash = repo_hash.into();
        if repo_hash.trim().is_empty() {
            return Err(GwtError::Config(
                "runtime scope repo_hash must not be empty".into(),
            ));
        }

        let worktree_hash = worktree_hash.into();
        if worktree_hash.trim().is_empty() {
            return Err(GwtError::Config(
                "runtime scope worktree_hash must not be empty".into(),
            ));
        }

        if !project_root.is_absolute() {
            return Err(GwtError::Config(format!(
                "runtime scope project_root must be absolute: {}",
                project_root.display()
            )));
        }

        let project_root = dunce::canonicalize(&project_root).unwrap_or(project_root);
        Ok(Self {
            repo_hash,
            worktree_hash,
            project_root,
            target,
        })
    }

    pub fn from_project_root(project_root: &Path, target: RuntimeTarget) -> Result<Self> {
        let repo_hash = project_scope_hash(project_root).as_str().to_string();
        let worktree_hash = compute_worktree_hash(project_root)?.as_str().to_string();
        Self::new(repo_hash, worktree_hash, project_root.to_path_buf(), target)
    }

    pub fn daemon_dir(&self, gwt_home: &Path) -> PathBuf {
        gwt_home
            .join("projects")
            .join(&self.repo_hash)
            .join("runtime")
            .join("daemon")
    }

    pub fn endpoint_path(&self, gwt_home: &Path) -> PathBuf {
        self.daemon_dir(gwt_home)
            .join(format!("{}.json", self.worktree_hash))
    }
}

/// Persisted daemon endpoint metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonEndpoint {
    pub protocol_version: u32,
    pub daemon_version: String,
    pub scope: RuntimeScope,
    pub pid: u32,
    pub bind: String,
    pub auth_token: String,
    pub updated_at_unix_ms: i64,
}

impl DaemonEndpoint {
    pub fn new(
        scope: RuntimeScope,
        pid: u32,
        bind: String,
        auth_token: String,
        daemon_version: String,
    ) -> Self {
        Self {
            protocol_version: DAEMON_PROTOCOL_VERSION,
            daemon_version,
            scope,
            pid,
            bind,
            auth_token,
            updated_at_unix_ms: chrono::Utc::now().timestamp_millis(),
        }
    }

    pub fn is_usable<F>(
        &self,
        expected_scope: &RuntimeScope,
        expected_protocol_version: u32,
        is_process_alive: F,
    ) -> bool
    where
        F: Fn(u32) -> bool,
    {
        self.protocol_version == expected_protocol_version
            && self.scope == *expected_scope
            && self.pid > 0
            && !self.bind.trim().is_empty()
            && !self.auth_token.trim().is_empty()
            && is_process_alive(self.pid)
    }
}

/// Hook event payload forwarded into the daemon runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookEnvelope {
    pub protocol_version: u32,
    pub scope: RuntimeScope,
    pub hook_name: String,
    pub session_id: Option<String>,
    pub cwd: PathBuf,
    pub payload: Value,
}

/// Client-to-daemon IPC handshake request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IpcHandshakeRequest {
    pub protocol_version: u32,
    pub auth_token: String,
    pub scope: RuntimeScope,
}

/// Daemon-to-client IPC handshake response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IpcHandshakeResponse {
    pub protocol_version: u32,
    pub daemon_version: String,
    pub accepted: bool,
    pub rejection_reason: Option<String>,
}

/// Tagged frame envelope sent by `gwt` over the post-handshake IPC stream.
///
/// Wire format is newline-delimited JSON. The `type` discriminator selects
/// the variant. Phase H1+ extends the variants for additional hot paths
/// (subscriptions, runtime status pushes, etc.); the existing variants are
/// stable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientFrame {
    /// Forward a managed-hook event to the daemon for routing.
    Hook(HookEnvelope),
    /// Subscribe to one or more daemon broadcast channels.
    Subscribe { channels: Vec<String> },
    /// Request a snapshot of the daemon's current runtime stats.
    Status,
}

/// Tagged frame envelope returned by `gwtd`.
///
/// Wire format is newline-delimited JSON. `Ack` is the canonical reply for
/// a successfully processed [`ClientFrame`]; `Event` carries a daemon
/// broadcast payload (used once Phase H1+ runtime ownership migrations
/// land); `Error` represents a frame that the daemon rejected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DaemonFrame {
    /// Acknowledgment for the previous client frame.
    Ack,
    /// Broadcast event delivered to subscribed clients.
    Event { channel: String, payload: Value },
    /// The daemon rejected the frame. `message` is human-readable.
    Error { message: String },
    /// Snapshot of daemon runtime stats, returned in response to a
    /// [`ClientFrame::Status`] request.
    Status(DaemonStatus),
}

/// Runtime stats snapshot returned by a [`DaemonFrame::Status`] frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonStatus {
    pub protocol_version: u32,
    pub daemon_version: String,
    pub uptime_seconds: u64,
    pub broadcast_channels: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaemonBootstrapAction {
    Reuse(DaemonEndpoint),
    Spawn { endpoint_path: PathBuf },
}

pub fn validate_handshake(
    endpoint: &DaemonEndpoint,
    request: &IpcHandshakeRequest,
    response: &IpcHandshakeResponse,
) -> Result<()> {
    if request.protocol_version != endpoint.protocol_version {
        return Err(GwtError::Agent(format!(
            "daemon handshake protocol mismatch: client={}, endpoint={}",
            request.protocol_version, endpoint.protocol_version
        )));
    }

    if response.protocol_version != endpoint.protocol_version {
        return Err(GwtError::Agent(format!(
            "daemon handshake protocol mismatch: daemon={}, endpoint={}",
            response.protocol_version, endpoint.protocol_version
        )));
    }

    if request.auth_token != endpoint.auth_token {
        return Err(GwtError::Agent("daemon handshake token mismatch".into()));
    }

    if request.scope != endpoint.scope {
        return Err(GwtError::Agent("daemon handshake scope mismatch".into()));
    }

    if !response.accepted {
        let reason = response
            .rejection_reason
            .as_deref()
            .unwrap_or("unknown rejection");
        return Err(GwtError::Agent(format!(
            "daemon handshake rejected: {}",
            reason
        )));
    }

    Ok(())
}

pub fn persist_endpoint(path: &Path, endpoint: &DaemonEndpoint) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        GwtError::Config(format!(
            "daemon endpoint path must have a parent directory: {}",
            path.display()
        ))
    })?;
    ensure_dir(parent)?;
    let payload = serde_json::to_vec_pretty(endpoint)
        .map_err(|e| GwtError::Other(format!("serialize daemon endpoint failed: {}", e)))?;
    fs::write(path, payload)?;
    Ok(())
}

pub fn resolve_bootstrap_action<F>(
    gwt_home: &Path,
    scope: &RuntimeScope,
    expected_protocol_version: u32,
    is_process_alive: F,
) -> Result<DaemonBootstrapAction>
where
    F: Fn(u32) -> bool,
{
    let endpoint_path = scope.endpoint_path(gwt_home);
    match load_endpoint(&endpoint_path) {
        Ok(endpoint) if endpoint.is_usable(scope, expected_protocol_version, is_process_alive) => {
            Ok(DaemonBootstrapAction::Reuse(endpoint))
        }
        Ok(_) | Err(GwtError::Other(_)) => {
            remove_endpoint_file(&endpoint_path)?;
            Ok(DaemonBootstrapAction::Spawn { endpoint_path })
        }
        Err(GwtError::Io(err)) if err.kind() == std::io::ErrorKind::NotFound => {
            Ok(DaemonBootstrapAction::Spawn { endpoint_path })
        }
        Err(err) => Err(err),
    }
}

fn load_endpoint(path: &Path) -> Result<DaemonEndpoint> {
    let payload = fs::read(path)?;
    serde_json::from_slice(&payload)
        .map_err(|e| GwtError::Other(format!("parse daemon endpoint failed: {}", e)))
}

fn remove_endpoint_file(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.into()),
    }
}
