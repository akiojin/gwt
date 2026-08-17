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
///
/// Daemon endpoint reuse is keyed by this version, so a bump forces
/// the bootstrap path to discard endpoints persisted by older daemons
/// instead of accepting a connection that would later fail at the
/// frame layer.
///
/// History:
/// - `1`: initial untyped post-handshake frames (`{"ack":true}` etc).
/// - `2`: typed `ClientFrame` / `DaemonFrame` post-handshake schema
///   plus per-channel broadcast fan-out (SPEC-2077 Phase H1
///   primitives + GREEN integration). Older clients/daemons will
///   reject the handshake and force a respawn instead of attempting
///   to exchange frames they cannot parse.
pub const DAEMON_PROTOCOL_VERSION: u32 = 2;

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

/// Environment variable pinning the directory used for shortened daemon
/// sockets.
///
/// Set it to a short, private, writable directory when every default
/// candidate is itself too long. An explicit value replaces the default
/// candidate list entirely, so a wrong value fails loudly instead of
/// silently falling back somewhere the operator did not choose.
pub const DAEMON_SOCKET_DIR_ENV: &str = "GWT_DAEMON_SOCKET_DIR";

/// Byte capacity of `sockaddr_un.sun_path` on this platform: 104 on
/// macOS/BSD, 108 on Linux.
#[cfg(unix)]
pub const UNIX_SOCKET_PATH_CAPACITY: usize =
    std::mem::size_of::<libc::sockaddr_un>() - std::mem::offset_of!(libc::sockaddr_un, sun_path);

/// Longest socket path `bind(2)` accepts. One byte of `sun_path` is
/// reserved for the NUL terminator, which is exactly the boundary
/// `std::os::unix::net::UnixListener::bind` enforces before it reaches the
/// kernel.
#[cfg(unix)]
pub const MAX_UNIX_SOCKET_PATH_LEN: usize = UNIX_SOCKET_PATH_CAPACITY - 1;

/// Where the daemon's Unix socket ended up relative to its endpoint file.
#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonSocketPlacement {
    /// Beside the endpoint metadata inside the project runtime root. This
    /// is the ordinary layout and keeps the socket inside the same
    /// owner-scoped directory as the endpoint it belongs to.
    Colocated,
    /// Inside a private per-user runtime directory, used when the
    /// colocated path would overflow `sun_path`.
    Shortened,
}

/// A daemon socket location plus how it was chosen.
#[cfg(unix)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonSocketPath {
    pub path: PathBuf,
    pub placement: DaemonSocketPlacement,
}

#[cfg(unix)]
const SHORT_SOCKET_DIR_MODE: u32 = 0o700;
/// Hex prefix width of the endpoint digest used as the shortened socket
/// file name. 64 bits is far beyond collision range for the handful of
/// worktrees one user runs, and keeps the whole name at 21 bytes.
#[cfg(unix)]
const SHORT_SOCKET_NAME_HEX_LEN: usize = 16;

/// Ordered directories to try when the colocated socket path is too long.
///
/// Reads [`DAEMON_SOCKET_DIR_ENV`], `XDG_RUNTIME_DIR`, and `TMPDIR` from
/// the environment; see [`short_socket_base_candidates_for`] for the
/// selection rules.
#[cfg(unix)]
pub fn short_socket_base_candidates() -> Vec<PathBuf> {
    short_socket_base_candidates_for(
        env_dir(DAEMON_SOCKET_DIR_ENV),
        env_dir("XDG_RUNTIME_DIR"),
        env_dir("TMPDIR"),
    )
}

/// Pure form of [`short_socket_base_candidates`].
///
/// An explicit override wins outright. Otherwise the per-user runtime
/// directories come first (`XDG_RUNTIME_DIR` on systemd hosts, `TMPDIR`
/// which is already private per user on macOS) and `/tmp` is the
/// last resort that always exists.
#[cfg(unix)]
pub fn short_socket_base_candidates_for(
    socket_dir_override: Option<PathBuf>,
    xdg_runtime_dir: Option<PathBuf>,
    tmpdir: Option<PathBuf>,
) -> Vec<PathBuf> {
    if let Some(dir) = socket_dir_override {
        return vec![dir];
    }

    let mut candidates: Vec<PathBuf> = Vec::with_capacity(3);
    for candidate in [xdg_runtime_dir, tmpdir, Some(PathBuf::from("/tmp"))]
        .into_iter()
        .flatten()
    {
        if !candidates.contains(&candidate) {
            candidates.push(candidate);
        }
    }
    candidates
}

#[cfg(unix)]
fn env_dir(key: &str) -> Option<PathBuf> {
    std::env::var_os(key)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// Resolve the Unix socket path for a daemon endpoint, shortening it when
/// the colocated path would overflow `sockaddr_un.sun_path`.
///
/// Uses [`short_socket_base_candidates`] for the shortened case; see
/// [`resolve_daemon_socket_path_in`] for the contract.
#[cfg(unix)]
pub fn resolve_daemon_socket_path(endpoint_path: &Path) -> Result<DaemonSocketPath> {
    resolve_daemon_socket_path_in(endpoint_path, &short_socket_base_candidates())
}

/// Resolve the Unix socket path for a daemon endpoint against an explicit
/// candidate list.
///
/// The colocated `<endpoint>.sock` is preferred and returned untouched
/// whenever it fits. Otherwise the socket moves to
/// `<base>/gwt-ipc-<uid>/<endpoint digest>.sock` for the first candidate
/// base that is an existing directory, yields a path within the platform
/// limit, and can be made private to the current user.
///
/// The name is derived from the full endpoint path, so the same
/// gwt home plus [`RuntimeScope`] always resolves to the same socket —
/// stale-socket cleanup and endpoint reuse keep working across daemon
/// restarts — while distinct scopes never share one.
///
/// Fails with a diagnosis naming the oversized path, the platform limit,
/// why each candidate was rejected, and [`DAEMON_SOCKET_DIR_ENV`] as the
/// recovery lever, rather than deferring to `bind(2)`'s bare
/// `path must be shorter than SUN_LEN`.
#[cfg(unix)]
pub fn resolve_daemon_socket_path_in(
    endpoint_path: &Path,
    bases: &[PathBuf],
) -> Result<DaemonSocketPath> {
    let colocated = endpoint_path.with_extension("sock");
    if colocated.as_os_str().len() <= MAX_UNIX_SOCKET_PATH_LEN {
        return Ok(DaemonSocketPath {
            path: colocated,
            placement: DaemonSocketPlacement::Colocated,
        });
    }

    let file_name = short_socket_file_name(endpoint_path);
    let dir_name = short_socket_dir_name();
    let mut rejections: Vec<String> = Vec::new();

    for base in bases {
        if !base.is_dir() {
            rejections.push(format!("{}: not an existing directory", base.display()));
            continue;
        }

        let dir = base.join(&dir_name);
        let candidate = dir.join(&file_name);
        let len = candidate.as_os_str().len();
        if len > MAX_UNIX_SOCKET_PATH_LEN {
            rejections.push(format!(
                "{}: shortened path would still be {len} bytes",
                base.display()
            ));
            continue;
        }

        match ensure_private_dir(&dir) {
            Ok(()) => {
                return Ok(DaemonSocketPath {
                    path: candidate,
                    placement: DaemonSocketPlacement::Shortened,
                })
            }
            Err(reason) => rejections.push(format!("{}: {reason}", base.display())),
        }
    }

    let rejected = if rejections.is_empty() {
        "no candidate directories were configured".to_string()
    } else {
        rejections.join("; ")
    };
    Err(GwtError::Config(format!(
        "daemon socket path exceeds this platform's {MAX_UNIX_SOCKET_PATH_LEN}-byte sun_path \
         limit and no shorter socket directory was usable. socket path: {socket} ({len} bytes). \
         rejected candidates: {rejected}. recovery: set {DAEMON_SOCKET_DIR_ENV} to a short, \
         private, writable directory such as /tmp, or move the gwt home or the project to a \
         shorter path",
        socket = colocated.display(),
        len = colocated.as_os_str().len(),
    )))
}

#[cfg(unix)]
fn short_socket_dir_name() -> String {
    format!("gwt-ipc-{}", effective_uid())
}

#[cfg(unix)]
fn short_socket_file_name(endpoint_path: &Path) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(endpoint_path.to_string_lossy().as_bytes());
    let digest = hex::encode(hasher.finalize());
    format!("{}.sock", &digest[..SHORT_SOCKET_NAME_HEX_LEN])
}

#[cfg(unix)]
fn effective_uid() -> u32 {
    // SAFETY: `geteuid` has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

/// Create or validate a directory that only the current user can reach.
///
/// Moving the socket out of the project runtime root must not widen the
/// local-only, owner-scoped IPC boundary, so an existing entry is accepted
/// only when it is a real directory (not a symlink) owned by this user,
/// and its mode is forced back to 0700.
#[cfg(unix)]
fn ensure_private_dir(dir: &Path) -> std::result::Result<(), String> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};

    match fs::DirBuilder::new()
        .mode(SHORT_SOCKET_DIR_MODE)
        .create(dir)
    {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(err) => return Err(format!("cannot create {}: {err}", dir.display())),
    }

    let metadata =
        fs::symlink_metadata(dir).map_err(|err| format!("cannot stat {}: {err}", dir.display()))?;
    if !metadata.is_dir() {
        return Err(format!(
            "{} exists but is not a directory owned by this user",
            dir.display()
        ));
    }

    let own_uid = effective_uid();
    if metadata.uid() != own_uid {
        return Err(format!(
            "{} is owned by uid {} rather than {own_uid}",
            dir.display(),
            metadata.uid()
        ));
    }

    if metadata.permissions().mode() & 0o777 != SHORT_SOCKET_DIR_MODE {
        fs::set_permissions(dir, fs::Permissions::from_mode(SHORT_SOCKET_DIR_MODE))
            .map_err(|err| format!("cannot restrict {} to 0700: {err}", dir.display()))?;
    }

    Ok(())
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
    /// Publish a payload to a daemon broadcast channel. Subscribers of
    /// `channel` receive a [`DaemonFrame::Event`] with the same payload.
    /// This is the gwt → gwtd companion to `Subscribe`; it is what
    /// Phase H1 GREEN domain handlers use to fan a state change out
    /// across all gwt instances on the same project scope.
    Publish { channel: String, payload: Value },
    /// Request a snapshot of the daemon's current runtime stats.
    Status,
}

/// Tagged frame envelope returned by `gwtd`.
///
/// Wire format is newline-delimited JSON. `Ack` is the canonical reply for
/// a successfully processed [`ClientFrame`]; `Event` carries a daemon
/// broadcast payload (Phase H1 ships board projection events; H2-H4
/// will add runtime status / hook events / launch lifecycle on the
/// same variant); `Error` represents a frame that the daemon rejected;
/// `Status` returns the snapshot requested via [`ClientFrame::Status`].
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
    /// Number of currently-connected IPC clients, including the one
    /// asking for the status snapshot.
    #[serde(default)]
    pub connections: usize,
    /// Latest atomic Issue Monitor projection owned by this daemon. Older
    /// daemons omit it; clients must not treat a missing projection from a live
    /// daemon as permission to reconstruct runtime state from stale cache.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue_monitor: Option<Value>,
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
            "daemon handshake rejected: {reason}"
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
        .map_err(|e| GwtError::Other(format!("serialize daemon endpoint failed: {e}")))?;
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
        .map_err(|e| GwtError::Other(format!("parse daemon endpoint failed: {e}")))
}

fn remove_endpoint_file(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.into()),
    }
}
