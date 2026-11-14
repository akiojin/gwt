//! PM agent singleton registration and settings (SPEC-3431).
//!
//! `<gwt_project_dir>/project-state/pm.json` is the source of truth for
//! FR-001 (per-project PM singleton), FR-002 (auto-start opt-out), and the
//! FR-003 restart-backoff bookkeeping. The writer mirrors the Issue Monitor
//! prefs transaction: a stable sibling `.lock` inode serializes cross-process
//! read-modify-write (GUI and gwtd both write this file), and the
//! unique-scratch durable atomic write keeps concurrent writers from tearing
//! the JSON.
//!
//! Liveness is deliberately an injected predicate: whether a registered PM
//! session is still alive is decided by the caller (GUI pane registry or
//! gwt-agent session store), not by this module, so the singleton invariant
//! stays testable without a running pane.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Durable record of the one resident PM session for a project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PmRegistration {
    pub session_id: String,
    pub agent_id: String,
    pub worktree_path: String,
    #[serde(default)]
    pub created_at: Option<String>,
    /// FR-003 crash-loop damper: consecutive crash count observed by the
    /// auto-restart path. Reset on a healthy start.
    #[serde(default)]
    pub consecutive_crashes: u32,
    /// RFC3339 floor before which the auto-restart path must not respawn.
    #[serde(default)]
    pub next_not_before: Option<String>,
}

fn default_auto_start() -> bool {
    true
}

/// SPEC-3431 FR-026: the agent the PM runs as, chosen per project.
///
/// Deliberately narrower than the Issue Monitor's launch profile. The PM is a
/// conversational role on the host: it never needs a Docker runtime, a resume
/// mode, or `--dangerously-skip-permissions`, and offering those knobs would
/// only let a user misconfigure the one agent that must always come up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PmLaunchProfile {
    /// Agent command name, e.g. `claude` or `codex`.
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// The agent a project falls back to when nothing is configured.
///
/// A fresh project has no profile, and the PM must still start — this is the
/// bootstrap trap the Issue Monitor hits when it refuses to launch without
/// one. The PM never inherits a profile from unrelated launches for the same
/// reason: a stale Docker target or exotic model from some other work would
/// silently become the PM's.
pub const PM_DEFAULT_AGENT: &str = "claude";

/// Agents that can resolve the `$gwt-pm` bootstrap prompt.
///
/// Managed assets only reach agents with a skills mirror, and `pm_guidance`
/// writes the `.claude` and `.codex` mirrors. Launching the PM as any other
/// agent would hand it a prompt that resolves to nothing — the exact failure
/// the T-052 materialization fix removed, reintroduced through configuration.
pub const PM_SUPPORTED_AGENTS: &[&str] = &["claude", "codex"];

pub fn pm_agent_is_supported(agent_id: &str) -> bool {
    PM_SUPPORTED_AGENTS.contains(&agent_id)
}

/// Project-scoped PM settings that survive deregistration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PmSettings {
    /// FR-002: opt-out flag. Missing field must read as `true` so prefs
    /// written before this field existed keep auto-starting.
    #[serde(default = "default_auto_start")]
    pub auto_start: bool,
    /// FR-026: absent until the user chooses; see [`PmSettings::launch_profile_or_default`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_profile: Option<PmLaunchProfile>,
    /// FR-035 (user ruling 2026-08-08): resident-loop cycle interval in
    /// seconds. Both the Stop-gate floor and the subscribe timeout the PM is
    /// told to use. Clamped to at least 10s so a typo cannot spin the loop.
    #[serde(default = "default_loop_interval_secs")]
    pub loop_interval_secs: u64,
}

fn default_loop_interval_secs() -> u64 {
    60
}

impl PmSettings {
    /// The effective loop interval, with the runaway floor applied.
    pub fn loop_interval_secs_clamped(&self) -> u64 {
        self.loop_interval_secs.max(10)
    }
}

impl PmSettings {
    /// The profile to launch with: the configured one when it names an agent
    /// that can resolve `$gwt-pm`, the built-in default otherwise. Falling back
    /// rather than refusing keeps FR-002's "opening a project starts the PM"
    /// true even for prefs written by a newer or misconfigured build.
    pub fn launch_profile_or_default(&self) -> PmLaunchProfile {
        match self.launch_profile.as_ref() {
            Some(profile) if pm_agent_is_supported(&profile.agent_id) => profile.clone(),
            Some(profile) => {
                tracing::warn!(
                    agent_id = %profile.agent_id,
                    "configured PM agent cannot resolve the gwt-pm skill; using the default"
                );
                PmLaunchProfile::default_profile()
            }
            None => PmLaunchProfile::default_profile(),
        }
    }
}

impl PmLaunchProfile {
    pub fn default_profile() -> Self {
        Self {
            agent_id: PM_DEFAULT_AGENT.to_string(),
            ..Self::default()
        }
    }
}

impl Default for PmSettings {
    fn default() -> Self {
        Self {
            auto_start: true,
            launch_profile: None,
            loop_interval_secs: default_loop_interval_secs(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PmPrefs {
    #[serde(default)]
    pub registration: Option<PmRegistration>,
    #[serde(default)]
    pub settings: PmSettings,
}

/// Outcome of a singleton registration attempt (FR-001).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PmRegisterOutcome {
    /// No prior registration; the candidate is now registered.
    Registered,
    /// The exact candidate was already registered. No durable write occurred.
    AlreadyRegistered,
    /// A stale (dead) registration was replaced by the candidate.
    ReplacedStale { previous: PmRegistration },
    /// A live PM already exists; the candidate was rejected and the stored
    /// bytes are unchanged. Callers route the user to resume `existing`.
    RejectedLive { existing: PmRegistration },
}

pub fn pm_prefs_path_for_repo_path(repo_path: &Path) -> PathBuf {
    gwt_core::paths::gwt_project_dir_for_repo_path(repo_path).join("project-state/pm.json")
}

pub fn pm_delivery_receipts_path_for_repo_path(repo_path: &Path) -> PathBuf {
    gwt_core::paths::gwt_project_dir_for_repo_path(repo_path)
        .join("project-state/pm-delivery-receipts.jsonl")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PmDeliveryReceiptStatus {
    Prepared,
    Verified,
    Refused,
    Ambiguous,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PmDeliveryReceipt {
    pub operation_id: String,
    pub recorded_at: String,
    pub status: PmDeliveryReceiptStatus,
    pub principal_session_id: String,
    pub target_window_id: String,
    #[serde(default)]
    pub target_session_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmDeliveryPrepareOutcome {
    Prepared,
    Existing(PmDeliveryReceiptStatus),
}

fn is_canonical_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn repair_pm_delivery_receipt_tail(path: &Path) -> io::Result<()> {
    let existing = match fs::read(path) {
        Ok(existing) => existing,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if existing.is_empty() {
        return Ok(());
    }
    let repaired_len = if existing.ends_with(b"\n") {
        let row_end = existing.len() - 1;
        let row_start = existing[..row_end]
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |position| position + 1);
        let final_row = &existing[row_start..row_end];
        if final_row.iter().all(u8::is_ascii_whitespace)
            || serde_json::from_slice::<PmDeliveryReceipt>(final_row).is_ok()
        {
            return Ok(());
        }
        row_start
    } else {
        existing
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |position| position + 1)
    };
    let file = fs::OpenOptions::new().write(true).open(path)?;
    file.set_len(repaired_len as u64)?;
    file.sync_all()?;
    sync_parent_directory(path)
}

fn load_pm_delivery_receipts_unlocked(path: &Path) -> io::Result<Vec<PmDeliveryReceipt>> {
    repair_pm_delivery_receipt_tail(path)?;
    match fs::read_to_string(path) {
        Ok(raw) => raw
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line).map_err(io::Error::other))
            .collect(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

fn append_pm_delivery_receipt_unlocked(path: &Path, receipt: &PmDeliveryReceipt) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    repair_pm_delivery_receipt_tail(path)?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    serde_json::to_writer(&mut file, receipt).map_err(io::Error::other)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    sync_parent_directory(path)
}

pub fn append_pm_delivery_receipt(path: &Path, receipt: &PmDeliveryReceipt) -> io::Result<()> {
    with_stable_pm_state_lock(path, || append_pm_delivery_receipt_unlocked(path, receipt))
}

pub fn load_pm_delivery_receipts(path: &Path) -> io::Result<Vec<PmDeliveryReceipt>> {
    with_stable_pm_state_lock(path, || load_pm_delivery_receipts_unlocked(path))
}

pub fn prepare_pm_delivery_receipt(
    path: &Path,
    prepared: &PmDeliveryReceipt,
) -> io::Result<PmDeliveryPrepareOutcome> {
    if prepared.status != PmDeliveryReceiptStatus::Prepared
        || uuid::Uuid::parse_str(&prepared.operation_id)
            .ok()
            .is_none_or(|operation_id| {
                operation_id.hyphenated().to_string() != prepared.operation_id
            })
        || prepared.principal_session_id.is_empty()
        || prepared.target_window_id.is_empty()
        || prepared.target_session_id.is_empty()
        || !is_canonical_sha256(&prepared.body_sha256)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "PM delivery Prepared receipt has an invalid identity",
        ));
    }
    with_stable_pm_state_lock(path, || {
        let receipts = load_pm_delivery_receipts_unlocked(path)?;
        let existing: Vec<_> = receipts
            .iter()
            .filter(|receipt| receipt.operation_id == prepared.operation_id)
            .collect();
        if let Some(first) = existing.first() {
            if first.principal_session_id != prepared.principal_session_id
                || first.target_window_id != prepared.target_window_id
                || first.target_session_id != prepared.target_session_id
                || first.body_sha256 != prepared.body_sha256
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "PM delivery operation identity conflicts with its durable receipt",
                ));
            }
            let status = existing
                .iter()
                .rev()
                .find(|receipt| receipt.status != PmDeliveryReceiptStatus::Prepared)
                .map_or(PmDeliveryReceiptStatus::Prepared, |receipt| receipt.status);
            return Ok(PmDeliveryPrepareOutcome::Existing(status));
        }
        append_pm_delivery_receipt_unlocked(path, prepared)?;
        Ok(PmDeliveryPrepareOutcome::Prepared)
    })
}

pub fn finish_pm_delivery_receipt(
    path: &Path,
    operation_id: &str,
    target_session_id: &str,
    body_sha256: &str,
    status: PmDeliveryReceiptStatus,
    error_code: Option<&str>,
) -> io::Result<PmDeliveryReceiptStatus> {
    if status == PmDeliveryReceiptStatus::Prepared {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "PM delivery terminal receipt cannot be Prepared",
        ));
    }
    with_stable_pm_state_lock(path, || {
        let receipts = load_pm_delivery_receipts_unlocked(path)?;
        let Some(prepared) = receipts.iter().find(|receipt| {
            receipt.operation_id == operation_id
                && receipt.status == PmDeliveryReceiptStatus::Prepared
        }) else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "PM delivery Prepared receipt is missing",
            ));
        };
        if prepared.target_session_id != target_session_id {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "PM delivery acknowledgement came from a different target Session",
            ));
        }
        if prepared.body_sha256 != body_sha256 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "PM delivery acknowledgement body does not match its Prepared receipt",
            ));
        }
        if let Some(existing) = receipts.iter().rev().find(|receipt| {
            receipt.operation_id == operation_id
                && receipt.status != PmDeliveryReceiptStatus::Prepared
        }) {
            match (existing.status, status) {
                (existing, requested) if existing == requested => return Ok(existing),
                (PmDeliveryReceiptStatus::Verified, PmDeliveryReceiptStatus::Ambiguous) => {
                    return Ok(PmDeliveryReceiptStatus::Verified)
                }
                (PmDeliveryReceiptStatus::Ambiguous, PmDeliveryReceiptStatus::Verified) => {}
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "PM delivery already has a different terminal receipt",
                    ));
                }
            }
        }
        let terminal = PmDeliveryReceipt {
            operation_id: operation_id.to_string(),
            recorded_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            status,
            principal_session_id: prepared.principal_session_id.clone(),
            target_window_id: prepared.target_window_id.clone(),
            target_session_id: prepared.target_session_id.clone(),
            body_sha256: prepared.body_sha256.clone(),
            error_code: error_code.map(str::to_string),
        };
        append_pm_delivery_receipt_unlocked(path, &terminal)?;
        Ok(status)
    })
}

/// Canonical worktree for the project's resident PM session.
pub fn pm_worktree_path_for_repo_path(repo_path: &Path) -> PathBuf {
    gwt_core::paths::gwt_project_dir_for_repo_path(repo_path).join("pm/worktree")
}

/// SPEC-3431 FR-012: durable state for the resident-loop driver. Lives beside
/// `pm.json` so it survives context compaction, crash resume, and session
/// succession — the PM's own notes do not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PmLoopState {
    /// Loop continuations since the last user prompt; the cap parks the PM.
    #[serde(default)]
    pub consecutive_continuations: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_continued_at: Option<String>,
    /// The last real user prompt. The T-093 wake path treats a recently
    /// prompted PM as busy with a human conversation and defers injection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_user_prompt_at: Option<String>,
    /// Whether the most recent forced continuation in the current Stop chain
    /// was the resident loop's own block. Guards the loop against riding a
    /// `stop_hook_active` chain that a different Stop gate started.
    #[serde(default)]
    pub pending_own_block: bool,
    /// The last prompt injection by a wake path (delta or periodic). Both
    /// wake flavours stamp it so they cannot double-fire within one quiet
    /// window, without touching the Stop-gate floor clock.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_wake_at: Option<String>,
    /// Exact protected-delivery operation acknowledged by UserPromptSubmit.
    /// Only the canonical UUID marker is persisted; the prompt body is not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_delivery_operation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_delivery_submitted_at: Option<String>,
    /// A syntactically valid operation whose self-contained body hash did not
    /// match the submitted prompt. This lets the worker stop CR retries
    /// without persisting any prompt content or crash-sensitive reservation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_delivery_rejected_operation_id: Option<String>,
    /// Body-free crash fence for a wake selected by the GUI but not yet
    /// committed by an exact UserPromptSubmit acknowledgement. A restart
    /// must not inject another full body while the old pane generation may
    /// still contain it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_delivery_operation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_delivery_window_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_delivery_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_delivery_body_sha256: Option<String>,
}

pub fn pm_delivery_prompt_sha256(prompt: &str) -> String {
    format!("{:x}", Sha256::digest(prompt.as_bytes()))
}

/// `pm-loop.json` path derived from the PM worktree itself. The hook's cwd is
/// the worktree, whose grandparent is the gwt project dir — no ambient value
/// is consulted, mirroring `is_pm_worktree`'s design.
pub fn pm_loop_state_path_for_pm_worktree(worktree: &Path) -> Option<PathBuf> {
    if !is_pm_worktree(worktree) {
        return None;
    }
    Some(
        worktree
            .parent()?
            .parent()?
            .join("project-state/pm-loop.json"),
    )
}

/// The same `pm-loop.json`, resolved from the project's repo path — the GUI
/// side (T-093 wake path) knows the project, not the hook's cwd.
pub fn pm_loop_state_path_for_repo_path(repo_path: &Path) -> PathBuf {
    gwt_core::paths::gwt_project_dir_for_repo_path(repo_path).join("project-state/pm-loop.json")
}

fn load_pm_loop_state_unlocked(path: &Path) -> io::Result<PmLoopState> {
    match fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str(&raw)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(PmLoopState::default()),
        Err(error) => Err(error),
    }
}

fn save_pm_loop_state_unlocked(path: &Path, state: &PmLoopState) -> io::Result<()> {
    let content = serde_json::to_string_pretty(state).map_err(io::Error::other)?;
    durable_atomic_write(path, content.as_bytes())
}

pub fn load_pm_loop_state(path: &Path) -> io::Result<PmLoopState> {
    with_stable_pm_state_lock(path, || load_pm_loop_state_unlocked(path))
}

pub fn save_pm_loop_state(path: &Path, state: &PmLoopState) -> io::Result<()> {
    with_stable_pm_state_lock(path, || save_pm_loop_state_unlocked(path, state))
}

/// Linearized cross-process read-modify-write for `pm-loop.json`.
///
/// The stable sibling lock survives the atomic rename and the durable writer
/// syncs the scratch bytes plus parent directory before releasing that lock.
pub fn mutate_pm_loop_state<T>(
    path: &Path,
    mutation: impl FnOnce(&mut PmLoopState) -> T,
) -> io::Result<(PmLoopState, T)> {
    with_stable_pm_state_lock(path, || {
        let mut state = load_pm_loop_state_unlocked(path)?;
        let result = mutation(&mut state);
        save_pm_loop_state_unlocked(path, &state)?;
        Ok((state, result))
    })
}

pub fn prepare_pm_wake_delivery(
    path: &Path,
    operation_id: &str,
    window_id: &str,
    session_id: &str,
    body_sha256: &str,
) -> io::Result<bool> {
    if uuid::Uuid::parse_str(operation_id)
        .ok()
        .is_none_or(|parsed| parsed.hyphenated().to_string() != operation_id)
        || window_id.is_empty()
        || session_id.is_empty()
        || !is_canonical_sha256(body_sha256)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "PM wake delivery reservation has an invalid identity",
        ));
    }
    mutate_pm_loop_state(path, |state| {
        let exact = state.pending_delivery_operation_id.as_deref() == Some(operation_id)
            && state.pending_delivery_window_id.as_deref() == Some(window_id)
            && state.pending_delivery_session_id.as_deref() == Some(session_id)
            && state.pending_delivery_body_sha256.as_deref() == Some(body_sha256);
        if exact {
            return true;
        }
        if state.pending_delivery_operation_id.is_some()
            || state.pending_delivery_window_id.is_some()
            || state.pending_delivery_session_id.is_some()
            || state.pending_delivery_body_sha256.is_some()
        {
            return false;
        }
        state.pending_delivery_operation_id = Some(operation_id.to_string());
        state.pending_delivery_window_id = Some(window_id.to_string());
        state.pending_delivery_session_id = Some(session_id.to_string());
        state.pending_delivery_body_sha256 = Some(body_sha256.to_string());
        true
    })
    .map(|(_, reserved)| reserved)
}

pub fn clear_pm_wake_delivery(
    path: &Path,
    operation_id: &str,
    window_id: &str,
) -> io::Result<bool> {
    mutate_pm_loop_state(path, |state| {
        if state.pending_delivery_operation_id.as_deref() != Some(operation_id)
            || state.pending_delivery_window_id.as_deref() != Some(window_id)
        {
            return false;
        }
        state.pending_delivery_operation_id = None;
        state.pending_delivery_window_id = None;
        state.pending_delivery_session_id = None;
        state.pending_delivery_body_sha256 = None;
        true
    })
    .map(|(_, cleared)| cleared)
}

pub fn clear_pm_wake_delivery_for_window(path: &Path, window_id: &str) -> io::Result<bool> {
    mutate_pm_loop_state(path, |state| {
        if state.pending_delivery_window_id.as_deref() != Some(window_id) {
            return false;
        }
        state.pending_delivery_operation_id = None;
        state.pending_delivery_window_id = None;
        state.pending_delivery_session_id = None;
        state.pending_delivery_body_sha256 = None;
        true
    })
    .map(|(_, cleared)| cleared)
}

/// Whether `path` is some project's canonical PM worktree.
///
/// Gates PM-only managed assets, so it anchors on the full shape
/// `<gwt projects dir>/<repo hash>/pm/worktree` rather than the trailing two
/// segments alone — a production branch literally named `pm/worktree` would
/// otherwise be handed the PM operating contract.
pub fn is_pm_worktree(path: &Path) -> bool {
    let Some(pm_dir) = path.parent() else {
        return false;
    };
    if path.file_name() != Some(std::ffi::OsStr::new("worktree"))
        || pm_dir.file_name() != Some(std::ffi::OsStr::new("pm"))
    {
        return false;
    }
    pm_dir
        .parent()
        .and_then(Path::parent)
        .is_some_and(|projects_dir| projects_dir == gwt_core::paths::gwt_projects_dir())
}

/// Per-writer-unique scratch path in the same directory as `path` so the
/// final `rename` stays on one filesystem and is atomic. A fixed scratch name
/// would let concurrent GUI/gwtd writers truncate the same file and tear the
/// JSON (see the Issue Monitor prefs writer).
fn unique_pm_scratch_path(path: &Path) -> PathBuf {
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("pm.json");
    parent.join(format!(
        ".{}.tmp-{}-{}",
        file_name,
        std::process::id(),
        uuid::Uuid::new_v4()
    ))
}

#[cfg(unix)]
fn sync_parent_directory(path: &Path) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::File::open(parent)?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent_directory(_path: &Path) -> io::Result<()> {
    // Windows cannot open a directory as a std::fs::File; the scratch file is
    // still sync_all'd before the atomic rename, matching the repository's
    // other durable writers.
    Ok(())
}

fn durable_atomic_write(path: &Path, content: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let scratch_path = unique_pm_scratch_path(path);
    let result = (|| {
        let mut scratch = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&scratch_path)?;
        scratch.write_all(content)?;
        scratch.sync_all()?;
        // Deadline-aware transactions must not become visible after their
        // acceptance boundary; recheck immediately before the canonical
        // rename (same convention as the Issue Monitor prefs writer).
        gwt_core::operation_deadline::ensure_remaining("PM state durable rename")?;
        fs::rename(&scratch_path, path)?;
        sync_parent_directory(path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&scratch_path);
    }
    result
}

fn with_stable_pm_state_lock<T>(
    path: &Path,
    operation: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    // Lock a stable sibling inode: locking `path` itself would stop
    // protecting future writers as soon as the atomic rename replaces that
    // inode.
    let lock = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path.with_extension("lock"))?;
    gwt_core::operation_deadline::lock_exclusive(&lock)?;
    let result = operation();
    let unlock_result = FileExt::unlock(&lock);
    match (result, unlock_result) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
    }
}

fn with_pm_prefs_lock<T>(path: &Path, operation: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
    with_stable_pm_state_lock(path, operation)
}

fn load_pm_prefs_unlocked(path: &Path) -> io::Result<PmPrefs> {
    if !path.exists() {
        return Ok(PmPrefs::default());
    }
    let content = fs::read_to_string(path)?;
    serde_json::from_str(&content).map_err(|error| {
        let kind = match error.classify() {
            serde_json::error::Category::Syntax | serde_json::error::Category::Eof => {
                io::ErrorKind::InvalidData
            }
            serde_json::error::Category::Data => io::ErrorKind::InvalidInput,
            serde_json::error::Category::Io => io::ErrorKind::Other,
        };
        io::Error::new(kind, error)
    })
}

fn save_pm_prefs_unlocked(path: &Path, prefs: &PmPrefs) -> io::Result<()> {
    let content = serde_json::to_string_pretty(prefs).map_err(io::Error::other)?;
    durable_atomic_write(path, content.as_bytes())
}

pub fn load_pm_prefs(path: &Path) -> io::Result<PmPrefs> {
    load_pm_prefs_unlocked(path)
}

/// Read the durable registration while holding the same stable sibling lock
/// used by singleton writers. Callers use this for authoritative response-loss
/// reconciliation without racing an in-flight registration update.
pub fn load_pm_registration_locked(path: &Path) -> io::Result<Option<PmRegistration>> {
    with_pm_prefs_lock(path, || Ok(load_pm_prefs_unlocked(path)?.registration))
}

/// Hold `pm.lock` while the exact full registration remains current and run
/// one protected operation. A missing or rotated registration returns `None`
/// without invoking the closure or mutating durable state.
pub fn with_exact_pm_registration<T>(
    path: &Path,
    expected: &PmRegistration,
    operation: impl FnOnce(&PmRegistration) -> io::Result<T>,
) -> io::Result<Option<T>> {
    with_pm_prefs_lock(path, || {
        let registration = load_pm_prefs_unlocked(path)?.registration;
        let Some(registration) = registration.filter(|current| current == expected) else {
            return Ok(None);
        };
        operation(&registration).map(Some)
    })
}

/// Read the exact full registration under the singleton lock, then release
/// the cross-process lock before the caller performs any potentially blocking
/// external I/O. A matching result is the linearization point for one already
/// accepted delivery mutation; later registration changes reject every later
/// call but never make GUI registration/teardown wait on a stalled PTY.
pub fn exact_pm_registration_is_current(
    path: &Path,
    expected: &PmRegistration,
) -> io::Result<bool> {
    with_pm_prefs_lock(path, || {
        Ok(load_pm_prefs_unlocked(path)?.registration.as_ref() == Some(expected))
    })
}

pub fn save_pm_prefs(path: &Path, prefs: &PmPrefs) -> io::Result<()> {
    with_pm_prefs_lock(path, || save_pm_prefs_unlocked(path, prefs))
}

/// One cross-process read-modify-write transaction under the stable sibling
/// lock, committing through the durable atomic writer.
pub fn mutate_pm_prefs<T>(
    path: &Path,
    mutation: impl FnOnce(&mut PmPrefs) -> T,
) -> io::Result<(PmPrefs, T)> {
    with_pm_prefs_lock(path, || {
        let mut prefs = load_pm_prefs_unlocked(path)?;
        let result = mutation(&mut prefs);
        save_pm_prefs_unlocked(path, &prefs)?;
        Ok((prefs, result))
    })
}

/// FR-001 singleton gate: register `candidate` unless a live PM already
/// exists. `is_live` judges the stored registration; a dead one is replaced
/// (stale regeneration), a live one rejects the candidate without touching
/// the stored bytes.
pub fn try_register_pm(
    path: &Path,
    candidate: PmRegistration,
    is_live: impl Fn(&PmRegistration) -> bool,
) -> io::Result<(PmPrefs, PmRegisterOutcome)> {
    with_pm_prefs_lock(path, || {
        let mut prefs = load_pm_prefs_unlocked(path)?;
        if prefs.registration.as_ref() == Some(&candidate) {
            return Ok((prefs, PmRegisterOutcome::AlreadyRegistered));
        }
        let outcome = match prefs.registration.take() {
            Some(existing) if is_live(&existing) => {
                // Rejected attempts must leave the canonical bytes untouched:
                // restore and return without saving.
                prefs.registration = Some(existing.clone());
                return Ok((prefs, PmRegisterOutcome::RejectedLive { existing }));
            }
            Some(stale) => PmRegisterOutcome::ReplacedStale { previous: stale },
            None => PmRegisterOutcome::Registered,
        };
        prefs.registration = Some(candidate);
        save_pm_prefs_unlocked(path, &prefs)?;
        Ok((prefs, outcome))
    })
}

/// Replace one exact registration snapshot with `candidate` under the PM
/// lock. The exact candidate is an idempotent success for response-loss
/// retries; missing or changed authority is a conflict and leaves bytes
/// untouched.
pub fn compare_and_swap_pm_registration(
    path: &Path,
    expected: &PmRegistration,
    candidate: PmRegistration,
) -> io::Result<(PmPrefs, bool)> {
    with_pm_prefs_lock(path, || {
        let mut prefs = load_pm_prefs_unlocked(path)?;
        if prefs.registration.as_ref() == Some(&candidate) {
            return Ok((prefs, true));
        }
        if prefs.registration.as_ref() != Some(expected) {
            return Ok((prefs, false));
        }
        prefs.registration = Some(candidate);
        save_pm_prefs_unlocked(path, &prefs)?;
        Ok((prefs, true))
    })
}

/// FR-013 intentional stop: clear the registration when it belongs to
/// `session_id`. Returns whether a matching registration was removed.
/// Settings (auto_start) survive deregistration.
pub fn deregister_pm(path: &Path, session_id: &str) -> io::Result<(PmPrefs, bool)> {
    with_pm_prefs_lock(path, || {
        let mut prefs = load_pm_prefs_unlocked(path)?;
        let matches = prefs
            .registration
            .as_ref()
            .is_some_and(|registration| registration.session_id == session_id);
        if !matches {
            return Ok((prefs, false));
        }
        prefs.registration = None;
        save_pm_prefs_unlocked(path, &prefs)?;
        Ok((prefs, true))
    })
}

/// SPEC-3431 FR-009: is `session_id` the project's registered PM?
///
/// This is the whole privileged-subject rule. It is deliberately an exact
/// match against the durable registration — no trimming, no normalization —
/// so a near-miss id can never inherit PM authority. Liveness needs no
/// separate probe here: the caller *is* the session, so if the registration
/// names it, that PM is running. A missing or unreadable registration is not
/// privileged (fail-closed).
pub fn session_is_registered_pm(prefs_path: &Path, session_id: &str) -> bool {
    if session_id.is_empty() {
        return false;
    }
    load_pm_prefs(prefs_path).is_ok_and(|prefs| {
        prefs
            .registration
            .is_some_and(|registration| registration.session_id == session_id)
    })
}

/// FR-003 crash-loop damper: uptime beyond this resets the consecutive-crash
/// count (the PM ran healthily, so the next crash starts a fresh series).
pub const PM_HEALTHY_UPTIME_SECS: i64 = 600;

/// FR-003 backoff ladder indexed by `consecutive_crashes - 1` (last entry
/// repeats). `0` means respawn immediately.
pub const PM_CRASH_BACKOFF_SECS: &[i64] = &[0, 30, 120, 300];

/// FR-003: record one crash on the registration and derive the respawn
/// verdict. Returns `true` when an immediate respawn is allowed; `false`
/// leaves `next_not_before` as the floor before which the auto-restart path
/// (and the ensure gate) must not respawn. `now` is RFC3339 so callers and
/// tests inject time explicitly.
pub fn apply_pm_crash_backoff(registration: &mut PmRegistration, now: &str) -> bool {
    let healthy_series_reset = registration
        .created_at
        .as_deref()
        .and_then(|created| rfc3339_delta_secs(created, now))
        .is_some_and(|uptime| uptime >= PM_HEALTHY_UPTIME_SECS);
    let count = if healthy_series_reset {
        1
    } else {
        registration.consecutive_crashes.saturating_add(1)
    };
    registration.consecutive_crashes = count;
    let ladder_index = usize::min(
        count.saturating_sub(1) as usize,
        PM_CRASH_BACKOFF_SECS.len() - 1,
    );
    let backoff = PM_CRASH_BACKOFF_SECS[ladder_index];
    if backoff <= 0 {
        registration.next_not_before = None;
        return true;
    }
    registration.next_not_before = rfc3339_plus_secs(now, backoff);
    false
}

/// FR-003: whether the backoff floor allows a respawn at `now`. A missing or
/// unparsable floor allows the respawn (fail-open: the damper protects
/// against loops, not against recovery).
pub fn pm_respawn_allowed(registration: &PmRegistration, now: &str) -> bool {
    match registration.next_not_before.as_deref() {
        None => true,
        Some(floor) => rfc3339_delta_secs(floor, now).is_none_or(|delta| delta >= 0),
    }
}

fn rfc3339_delta_secs(earlier: &str, later: &str) -> Option<i64> {
    let earlier = chrono::DateTime::parse_from_rfc3339(earlier).ok()?;
    let later = chrono::DateTime::parse_from_rfc3339(later).ok()?;
    Some((later - earlier).num_seconds())
}

fn rfc3339_plus_secs(now: &str, secs: i64) -> Option<String> {
    let parsed = chrono::DateTime::parse_from_rfc3339(now).ok()?;
    Some(
        (parsed + chrono::Duration::seconds(secs))
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    )
}

/// Diagnostic snapshot for the `pm.status` JSON operation (FR-001 diagnostic
/// visibility). `session_record_present` / `stale_hint` are populated only
/// when a registration exists; a missing durable session record is a stale
/// hint, not an authoritative liveness verdict — authoritative liveness stays
/// with the GUI spawn gate, which can see live panes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PmStatusReport {
    pub schema_version: u32,
    pub registered: bool,
    pub auto_start: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registration: Option<PmRegistration>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_record_present: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stale_hint: Option<bool>,
    /// FR-014: the PM never occupies an Issue Monitor implementation slot.
    /// Always 0; kept explicit so operators can see the accounting rule.
    /// The global resource cap engine itself remains SPEC #3200's
    /// unimplemented FR — this report only exposes the PM bucket.
    pub implementation_slots_consumed: u32,
    /// FR-014 visibility: resident PM bucket (1 while a PM is registered).
    pub pm_bucket: u32,
    /// FR-009 diagnostic visibility: whether the session asking for this
    /// report is the registered PM, i.e. whether the asymmetric Issue Monitor
    /// boundary is lifted for it. `None` when the caller has no ambient
    /// session identity to judge.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caller_is_registered_pm: Option<bool>,
}

/// Build the `pm.status` report from loaded prefs. The durable-session probe
/// is injected so the report logic stays testable without a real session
/// store.
pub fn pm_status_report(
    prefs: &PmPrefs,
    session_record_present: impl Fn(&str) -> bool,
) -> PmStatusReport {
    pm_status_report_for_caller(prefs, session_record_present, None)
}

/// [`pm_status_report`] plus the FR-009 privilege verdict for `caller_session`.
pub fn pm_status_report_for_caller(
    prefs: &PmPrefs,
    session_record_present: impl Fn(&str) -> bool,
    caller_session: Option<&str>,
) -> PmStatusReport {
    let registration = prefs.registration.clone();
    let record_present = registration
        .as_ref()
        .map(|registration| session_record_present(&registration.session_id));
    PmStatusReport {
        schema_version: 1,
        registered: registration.is_some(),
        auto_start: prefs.settings.auto_start,
        pm_bucket: u32::from(registration.is_some()),
        implementation_slots_consumed: 0,
        caller_is_registered_pm: caller_session.map(|caller| {
            registration
                .as_ref()
                .is_some_and(|current| current.session_id == caller)
        }),
        registration,
        session_record_present: record_present,
        stale_hint: record_present.map(|present| !present),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_prefs_path() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("project-state").join("pm.json");
        (dir, path)
    }

    fn registration(session: &str) -> PmRegistration {
        PmRegistration {
            session_id: session.to_string(),
            agent_id: "claude-code".to_string(),
            worktree_path: "/tmp/pm-worktree".to_string(),
            created_at: Some("2026-08-03T00:00:00Z".to_string()),
            consecutive_crashes: 0,
            next_not_before: None,
        }
    }

    #[test]
    fn pm_loop_mutation_serializes_concurrent_read_modify_write() {
        use std::sync::{Arc, Barrier};

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("project-state/pm-loop.json");
        let writers = 12;
        let barrier = Arc::new(Barrier::new(writers));
        let mut threads = Vec::new();
        for index in 0..writers {
            let path = path.clone();
            let barrier = Arc::clone(&barrier);
            threads.push(std::thread::spawn(move || {
                barrier.wait();
                mutate_pm_loop_state(&path, |state| {
                    state.consecutive_continuations =
                        state.consecutive_continuations.saturating_add(1);
                    state.last_continued_at = Some(format!("writer-{index}"));
                })
                .expect("linearized loop mutation");
            }));
        }
        for thread in threads {
            thread.join().expect("join loop writer");
        }

        let state = load_pm_loop_state(&path).expect("committed loop state");
        assert_eq!(state.consecutive_continuations, writers as u32);
        assert!(state.last_continued_at.is_some());
        let names: Vec<String> = fs::read_dir(path.parent().expect("parent"))
            .expect("read project state")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert!(
            names
                .iter()
                .all(|name| name == "pm-loop.json" || name == "pm-loop.lock"),
            "durable mutation must leave only canonical state and its stable lock: {names:?}"
        );
    }

    #[test]
    fn delivery_receipt_is_durable_body_free_and_recovers_a_partial_tail() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("project-state/pm-delivery-receipts.jsonl");
        let prepared = PmDeliveryReceipt {
            operation_id: "72fc3cd4-ad49-43e3-bf3d-d791357643a3".to_string(),
            recorded_at: "2026-08-11T00:00:00Z".to_string(),
            status: PmDeliveryReceiptStatus::Prepared,
            principal_session_id: "pm-session".to_string(),
            target_window_id: "tab::agent".to_string(),
            target_session_id: "agent-session".to_string(),
            body_sha256: pm_delivery_prompt_sha256("body"),
            error_code: None,
        };
        append_pm_delivery_receipt(&path, &prepared).expect("prepared receipt");
        let mut bytes = fs::read(&path).expect("prepared bytes");
        bytes.extend_from_slice(b"{\"operation_id\":\"partial");
        fs::write(&path, bytes).expect("simulate torn append");
        let terminal = PmDeliveryReceipt {
            status: PmDeliveryReceiptStatus::Verified,
            recorded_at: "2026-08-11T00:00:01Z".to_string(),
            ..prepared.clone()
        };

        append_pm_delivery_receipt(&path, &terminal).expect("terminal receipt repairs tail");

        assert_eq!(
            load_pm_delivery_receipts(&path).expect("receipts"),
            vec![prepared, terminal]
        );
        let raw = fs::read_to_string(&path).expect("receipt text");
        assert!(!raw.contains("partial"));
        assert!(!raw.contains("prompt"));
        assert!(!raw.contains("text"));
    }

    #[test]
    fn delivery_receipt_recovers_a_newline_terminated_torn_final_row() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("project-state/pm-delivery-receipts.jsonl");
        let prepared = PmDeliveryReceipt {
            operation_id: "72fc3cd4-ad49-43e3-bf3d-d791357643b0".to_string(),
            recorded_at: "2026-08-11T00:00:00Z".to_string(),
            status: PmDeliveryReceiptStatus::Prepared,
            principal_session_id: "pm-session".to_string(),
            target_window_id: "tab::agent".to_string(),
            target_session_id: "agent-session".to_string(),
            body_sha256: pm_delivery_prompt_sha256("body"),
            error_code: None,
        };
        append_pm_delivery_receipt(&path, &prepared).expect("prepared receipt");
        let mut bytes = fs::read(&path).expect("prepared bytes");
        bytes.extend_from_slice(b"{\"operation_id\":\"torn\n");
        fs::write(&path, bytes).expect("simulate newline-terminated torn append");

        assert_eq!(
            load_pm_delivery_receipts(&path).expect("valid prefix survives repair"),
            vec![prepared]
        );
        let raw = fs::read_to_string(&path).expect("repaired receipt text");
        assert!(!raw.contains("torn"));
    }

    #[test]
    fn delivery_receipt_rejects_same_operation_with_a_different_body_hash() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("project-state/pm-delivery-receipts.jsonl");
        let prepared = PmDeliveryReceipt {
            operation_id: "72fc3cd4-ad49-43e3-bf3d-d791357643b1".to_string(),
            recorded_at: "2026-08-11T00:00:00Z".to_string(),
            status: PmDeliveryReceiptStatus::Prepared,
            principal_session_id: "pm-session".to_string(),
            target_window_id: "tab::agent".to_string(),
            target_session_id: "agent-session".to_string(),
            body_sha256: pm_delivery_prompt_sha256("original body"),
            error_code: None,
        };
        prepare_pm_delivery_receipt(&path, &prepared).expect("prepare original body");
        let conflicting = PmDeliveryReceipt {
            body_sha256: pm_delivery_prompt_sha256("different body"),
            ..prepared.clone()
        };

        let error = prepare_pm_delivery_receipt(&path, &conflicting)
            .expect_err("one operation id cannot identify two bodies");

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            load_pm_delivery_receipts(&path).expect("original receipt remains authoritative"),
            vec![prepared]
        );
    }

    #[test]
    fn delivery_receipt_transition_is_idempotent_and_target_session_bound() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("project-state/pm-delivery-receipts.jsonl");
        let prepared = PmDeliveryReceipt {
            operation_id: "72fc3cd4-ad49-43e3-bf3d-d791357643a4".to_string(),
            recorded_at: "2026-08-11T00:00:00Z".to_string(),
            status: PmDeliveryReceiptStatus::Prepared,
            principal_session_id: "pm-session".to_string(),
            target_window_id: "tab::agent".to_string(),
            target_session_id: "agent-session".to_string(),
            body_sha256: pm_delivery_prompt_sha256("body"),
            error_code: None,
        };

        assert_eq!(
            prepare_pm_delivery_receipt(&path, &prepared).expect("prepare"),
            PmDeliveryPrepareOutcome::Prepared
        );
        assert_eq!(
            prepare_pm_delivery_receipt(&path, &prepared).expect("idempotent prepare"),
            PmDeliveryPrepareOutcome::Existing(PmDeliveryReceiptStatus::Prepared)
        );
        assert!(finish_pm_delivery_receipt(
            &path,
            &prepared.operation_id,
            "foreign-session",
            &prepared.body_sha256,
            PmDeliveryReceiptStatus::Verified,
            None,
        )
        .is_err());
        assert_eq!(
            finish_pm_delivery_receipt(
                &path,
                &prepared.operation_id,
                "agent-session",
                &prepared.body_sha256,
                PmDeliveryReceiptStatus::Verified,
                None,
            )
            .expect("finish"),
            PmDeliveryReceiptStatus::Verified
        );
        assert_eq!(
            finish_pm_delivery_receipt(
                &path,
                &prepared.operation_id,
                "agent-session",
                &prepared.body_sha256,
                PmDeliveryReceiptStatus::Verified,
                None,
            )
            .expect("idempotent finish"),
            PmDeliveryReceiptStatus::Verified
        );
        assert!(finish_pm_delivery_receipt(
            &path,
            &prepared.operation_id,
            "agent-session",
            &prepared.body_sha256,
            PmDeliveryReceiptStatus::Refused,
            Some("conflicting_terminal"),
        )
        .is_err());
        assert_eq!(
            load_pm_delivery_receipts(&path)
                .expect("receipts")
                .iter()
                .filter(|receipt| receipt.operation_id == prepared.operation_id)
                .count(),
            2,
            "one Prepared and one terminal row are durable exactly once"
        );
    }

    #[test]
    fn delivery_receipt_exact_verified_ack_upgrades_ambiguous_terminal() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("project-state/pm-delivery-receipts.jsonl");
        let prepared = PmDeliveryReceipt {
            operation_id: "72fc3cd4-ad49-43e3-bf3d-d791357643b2".to_string(),
            recorded_at: "2026-08-11T00:00:00Z".to_string(),
            status: PmDeliveryReceiptStatus::Prepared,
            principal_session_id: "pm-session".to_string(),
            target_window_id: "tab::agent".to_string(),
            target_session_id: "agent-session".to_string(),
            body_sha256: pm_delivery_prompt_sha256("body"),
            error_code: None,
        };
        prepare_pm_delivery_receipt(&path, &prepared).expect("prepare delivery");
        finish_pm_delivery_receipt(
            &path,
            &prepared.operation_id,
            &prepared.target_session_id,
            &prepared.body_sha256,
            PmDeliveryReceiptStatus::Ambiguous,
            Some("submit_unverified"),
        )
        .expect("record bounded-wait ambiguity");

        assert_eq!(
            finish_pm_delivery_receipt(
                &path,
                &prepared.operation_id,
                &prepared.target_session_id,
                &prepared.body_sha256,
                PmDeliveryReceiptStatus::Verified,
                None,
            )
            .expect("late exact acknowledgement upgrades ambiguity"),
            PmDeliveryReceiptStatus::Verified
        );
        assert_eq!(
            load_pm_delivery_receipts(&path)
                .expect("receipts")
                .iter()
                .map(|receipt| receipt.status)
                .collect::<Vec<_>>(),
            vec![
                PmDeliveryReceiptStatus::Prepared,
                PmDeliveryReceiptStatus::Ambiguous,
                PmDeliveryReceiptStatus::Verified,
            ]
        );
    }

    #[test]
    fn delivery_receipt_verified_dominates_a_later_ambiguous_transition() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("project-state/pm-delivery-receipts.jsonl");
        let prepared = PmDeliveryReceipt {
            operation_id: "72fc3cd4-ad49-43e3-bf3d-d791357643b3".to_string(),
            recorded_at: "2026-08-11T00:00:00Z".to_string(),
            status: PmDeliveryReceiptStatus::Prepared,
            principal_session_id: "pm-session".to_string(),
            target_window_id: "tab::agent".to_string(),
            target_session_id: "agent-session".to_string(),
            body_sha256: pm_delivery_prompt_sha256("body"),
            error_code: None,
        };
        prepare_pm_delivery_receipt(&path, &prepared).expect("prepare delivery");
        finish_pm_delivery_receipt(
            &path,
            &prepared.operation_id,
            &prepared.target_session_id,
            &prepared.body_sha256,
            PmDeliveryReceiptStatus::Verified,
            None,
        )
        .expect("record exact acknowledgement");

        assert_eq!(
            finish_pm_delivery_receipt(
                &path,
                &prepared.operation_id,
                &prepared.target_session_id,
                &prepared.body_sha256,
                PmDeliveryReceiptStatus::Ambiguous,
                Some("late_timeout"),
            )
            .expect("late ambiguity preserves exact acknowledgement"),
            PmDeliveryReceiptStatus::Verified
        );
        assert_eq!(
            load_pm_delivery_receipts(&path)
                .expect("receipts")
                .iter()
                .map(|receipt| receipt.status)
                .collect::<Vec<_>>(),
            vec![
                PmDeliveryReceiptStatus::Prepared,
                PmDeliveryReceiptStatus::Verified,
            ]
        );
    }

    #[test]
    fn delivery_receipt_rejects_every_other_terminal_conflict() {
        let dir = tempfile::tempdir().expect("tempdir");
        for (operation_id, existing, requested) in [
            (
                "72fc3cd4-ad49-43e3-bf3d-d791357643b6",
                PmDeliveryReceiptStatus::Ambiguous,
                PmDeliveryReceiptStatus::Refused,
            ),
            (
                "72fc3cd4-ad49-43e3-bf3d-d791357643b7",
                PmDeliveryReceiptStatus::Refused,
                PmDeliveryReceiptStatus::Ambiguous,
            ),
            (
                "72fc3cd4-ad49-43e3-bf3d-d791357643b8",
                PmDeliveryReceiptStatus::Refused,
                PmDeliveryReceiptStatus::Verified,
            ),
        ] {
            let path = dir.path().join(format!("{operation_id}.jsonl"));
            let prepared = PmDeliveryReceipt {
                operation_id: operation_id.to_string(),
                recorded_at: "2026-08-11T00:00:00Z".to_string(),
                status: PmDeliveryReceiptStatus::Prepared,
                principal_session_id: "pm-session".to_string(),
                target_window_id: "tab::agent".to_string(),
                target_session_id: "agent-session".to_string(),
                body_sha256: pm_delivery_prompt_sha256("body"),
                error_code: None,
            };
            prepare_pm_delivery_receipt(&path, &prepared).expect("prepare delivery");
            finish_pm_delivery_receipt(
                &path,
                &prepared.operation_id,
                &prepared.target_session_id,
                &prepared.body_sha256,
                existing,
                Some("initial_terminal"),
            )
            .expect("record initial terminal");

            let error = finish_pm_delivery_receipt(
                &path,
                &prepared.operation_id,
                &prepared.target_session_id,
                &prepared.body_sha256,
                requested,
                Some("conflicting_terminal"),
            )
            .expect_err("only exact verification may supersede ambiguity");

            assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
            assert_eq!(
                load_pm_delivery_receipts(&path)
                    .expect("receipts")
                    .iter()
                    .map(|receipt| receipt.status)
                    .collect::<Vec<_>>(),
                vec![PmDeliveryReceiptStatus::Prepared, existing]
            );
        }
    }

    #[test]
    fn wake_delivery_reservation_is_exact_idempotent_and_body_free() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("project-state/pm-loop.json");
        let operation_id = "72fc3cd4-ad49-43e3-bf3d-d791357643a5";
        let prompt = "scheduled supervision prompt";
        let body_sha256 = pm_delivery_prompt_sha256(prompt);

        assert!(prepare_pm_wake_delivery(
            &path,
            operation_id,
            "tab::pm",
            "pm-session",
            &body_sha256,
        )
        .expect("prepare wake"));
        assert!(prepare_pm_wake_delivery(
            &path,
            operation_id,
            "tab::pm",
            "pm-session",
            &body_sha256,
        )
        .expect("idempotent prepare"));
        assert!(!prepare_pm_wake_delivery(
            &path,
            "72fc3cd4-ad49-43e3-bf3d-d791357643a6",
            "tab::pm",
            "pm-session",
            &body_sha256,
        )
        .expect("conflicting prepare"));
        assert!(
            !clear_pm_wake_delivery(&path, operation_id, "tab::foreign").expect("foreign clear")
        );

        let prepared = load_pm_loop_state(&path).expect("prepared state");
        assert_eq!(
            prepared.pending_delivery_operation_id.as_deref(),
            Some(operation_id)
        );
        assert_eq!(
            prepared.pending_delivery_window_id.as_deref(),
            Some("tab::pm")
        );
        assert_eq!(
            prepared.pending_delivery_session_id.as_deref(),
            Some("pm-session")
        );
        assert_eq!(
            prepared.pending_delivery_body_sha256.as_deref(),
            Some(body_sha256.as_str())
        );
        let raw = fs::read_to_string(&path).expect("wake state text");
        assert!(!raw.contains("prompt"));
        assert!(!raw.contains(prompt));

        assert!(clear_pm_wake_delivery(&path, operation_id, "tab::pm").expect("exact clear"));
        let cleared = load_pm_loop_state(&path).expect("cleared state");
        assert_eq!(cleared.pending_delivery_operation_id, None);
        assert_eq!(cleared.pending_delivery_window_id, None);
        assert_eq!(cleared.pending_delivery_session_id, None);
        assert_eq!(cleared.pending_delivery_body_sha256, None);
    }

    #[test]
    fn wake_delivery_reservation_rejects_same_identity_with_a_different_body_hash() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("project-state/pm-loop.json");
        let operation_id = "72fc3cd4-ad49-43e3-bf3d-d791357643b4";
        let original_hash = pm_delivery_prompt_sha256("original wake");
        let conflicting_hash = pm_delivery_prompt_sha256("different wake");
        assert!(prepare_pm_wake_delivery(
            &path,
            operation_id,
            "tab::pm",
            "pm-session",
            &original_hash,
        )
        .expect("prepare original wake"));

        assert!(!prepare_pm_wake_delivery(
            &path,
            operation_id,
            "tab::pm",
            "pm-session",
            &conflicting_hash,
        )
        .expect("different digest conflicts with the durable identity"));
        assert_eq!(
            load_pm_loop_state(&path)
                .expect("original reservation remains authoritative")
                .pending_delivery_body_sha256
                .as_deref(),
            Some(original_hash.as_str())
        );
    }

    #[test]
    fn wake_delivery_reservation_requires_a_lowercase_sha256_digest() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("project-state/pm-loop.json");
        let operation_id = "72fc3cd4-ad49-43e3-bf3d-d791357643b5";

        for invalid in ["short", &"A".repeat(64)] {
            let error =
                prepare_pm_wake_delivery(&path, operation_id, "tab::pm", "pm-session", invalid)
                    .expect_err("invalid digest must be rejected before persistence");
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        }
        assert!(!path.exists());
    }

    #[test]
    fn exact_pm_registration_lease_holds_the_stable_lock_and_rejects_rotation() {
        let (_dir, path) = temp_prefs_path();
        let exact = registration("pm-session-a");
        try_register_pm(&path, exact.clone(), |_| false).expect("seed registration");
        let before = fs::read(&path).expect("seeded bytes");

        let leased = with_exact_pm_registration(&path, &exact, |registration| {
            assert_eq!(registration, &exact);
            let competing = fs::OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .truncate(false)
                .open(path.with_extension("lock"))?;
            let contention = competing
                .try_lock_exclusive()
                .expect_err("lease closure must run while pm.lock remains held");
            assert_eq!(contention.kind(), io::ErrorKind::WouldBlock);
            Ok("leased")
        })
        .expect("exact lease");
        assert_eq!(leased, Some("leased"));

        let mut rotated = exact.clone();
        rotated.consecutive_crashes = 1;
        let mut invoked = false;
        let rejected = with_exact_pm_registration(&path, &rotated, |_| {
            invoked = true;
            Ok(())
        })
        .expect("stale lease probe");
        assert_eq!(rejected, None);
        assert!(!invoked, "stale full registration must be zero-mutation");
        assert_eq!(fs::read(&path).expect("bytes after rejection"), before);
    }

    #[test]
    fn missing_file_loads_default_with_auto_start_true() {
        let (_dir, path) = temp_prefs_path();
        let prefs = load_pm_prefs(&path).expect("load default");
        assert_eq!(prefs.registration, None);
        assert!(prefs.settings.auto_start, "FR-002: auto_start defaults ON");
    }

    #[test]
    fn empty_json_object_defaults_auto_start_true() {
        // Prefs written before the settings field existed must keep
        // auto-starting; a silent false default would disable FR-002 for
        // every existing project.
        let prefs: PmPrefs = serde_json::from_str("{}").expect("parse empty object");
        assert!(prefs.settings.auto_start);
        let prefs: PmPrefs =
            serde_json::from_str("{\"settings\":{}}").expect("parse empty settings");
        assert!(prefs.settings.auto_start);
    }

    #[test]
    fn save_then_load_roundtrips_and_leaves_no_scratch() {
        let (_dir, path) = temp_prefs_path();
        let prefs = PmPrefs {
            registration: Some(registration("session-a")),
            settings: PmSettings {
                auto_start: false,
                ..PmSettings::default()
            },
        };
        save_pm_prefs(&path, &prefs).expect("save");
        let loaded = load_pm_prefs(&path).expect("load");
        assert_eq!(loaded, prefs);
        let names: Vec<String> = fs::read_dir(path.parent().expect("parent"))
            .expect("read dir")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert!(
            names
                .iter()
                .all(|name| name == "pm.json" || name == "pm.lock"),
            "unexpected files after atomic save: {names:?}"
        );
    }

    #[test]
    fn mutate_persists_the_mutation() {
        let (_dir, path) = temp_prefs_path();
        let (committed, _) = mutate_pm_prefs(&path, |prefs| {
            prefs.settings.auto_start = false;
        })
        .expect("mutate");
        assert!(!committed.settings.auto_start);
        let loaded = load_pm_prefs(&path).expect("load");
        assert!(!loaded.settings.auto_start);
    }

    #[test]
    fn register_into_empty_prefs_registers() {
        let (_dir, path) = temp_prefs_path();
        let (prefs, outcome) =
            try_register_pm(&path, registration("session-a"), |_| true).expect("register");
        assert_eq!(outcome, PmRegisterOutcome::Registered);
        assert_eq!(
            prefs.registration.as_ref().map(|r| r.session_id.as_str()),
            Some("session-a")
        );
        let loaded = load_pm_prefs(&path).expect("load");
        assert_eq!(
            loaded.registration.map(|r| r.session_id),
            Some("session-a".to_string())
        );
    }

    #[test]
    fn register_rejects_when_existing_is_live() {
        let (_dir, path) = temp_prefs_path();
        try_register_pm(&path, registration("session-a"), |_| true).expect("seed");
        let (prefs, outcome) =
            try_register_pm(&path, registration("session-b"), |_| true).expect("attempt");
        match outcome {
            PmRegisterOutcome::RejectedLive { existing } => {
                assert_eq!(existing.session_id, "session-a");
            }
            other => panic!("expected RejectedLive, got {other:?}"),
        }
        // The stored registration must be untouched by the rejected attempt.
        assert_eq!(
            prefs.registration.map(|r| r.session_id),
            Some("session-a".to_string())
        );
        let loaded = load_pm_prefs(&path).expect("load");
        assert_eq!(
            loaded.registration.map(|r| r.session_id),
            Some("session-a".to_string())
        );
    }

    #[test]
    fn register_replaces_stale_registration() {
        let (_dir, path) = temp_prefs_path();
        try_register_pm(&path, registration("session-a"), |_| true).expect("seed");
        let (prefs, outcome) =
            try_register_pm(&path, registration("session-b"), |_| false).expect("takeover");
        match outcome {
            PmRegisterOutcome::ReplacedStale { previous } => {
                assert_eq!(previous.session_id, "session-a");
            }
            other => panic!("expected ReplacedStale, got {other:?}"),
        }
        assert_eq!(
            prefs.registration.map(|r| r.session_id),
            Some("session-b".to_string())
        );
    }

    #[test]
    fn registering_the_exact_candidate_is_idempotent_without_rewriting_prefs() {
        let (_dir, path) = temp_prefs_path();
        let candidate = registration("session-a");
        try_register_pm(&path, candidate.clone(), |_| false).expect("seed");
        let before = fs::read(&path).expect("read seeded prefs");

        let (prefs, outcome) =
            try_register_pm(&path, candidate.clone(), |_| true).expect("retry registration");

        assert_eq!(outcome, PmRegisterOutcome::AlreadyRegistered);
        assert_eq!(prefs.registration, Some(candidate));
        assert_eq!(fs::read(&path).expect("read retried prefs"), before);
    }

    #[test]
    fn compare_and_swap_requires_the_exact_predecessor_and_retries_idempotently() {
        let (_dir, path) = temp_prefs_path();
        let source = registration("session-source");
        let successor = registration("session-successor");
        try_register_pm(&path, source.clone(), |_| false).expect("seed source");

        let (prefs, replaced) = compare_and_swap_pm_registration(
            &path,
            &registration("foreign-source"),
            successor.clone(),
        )
        .expect("conflicting CAS");
        assert!(!replaced);
        assert_eq!(prefs.registration, Some(source.clone()));

        let (prefs, replaced) =
            compare_and_swap_pm_registration(&path, &source, successor.clone()).expect("exact CAS");
        assert!(replaced);
        assert_eq!(prefs.registration, Some(successor.clone()));
        let before_retry = fs::read(&path).expect("read committed CAS");

        let (prefs, replaced) = compare_and_swap_pm_registration(&path, &source, successor.clone())
            .expect("response-loss retry");
        assert!(replaced);
        assert_eq!(prefs.registration, Some(successor));
        assert_eq!(fs::read(&path).expect("read retried CAS"), before_retry);
    }

    #[test]
    fn deregister_clears_only_matching_session_and_keeps_settings() {
        let (_dir, path) = temp_prefs_path();
        mutate_pm_prefs(&path, |prefs| {
            prefs.settings.auto_start = false;
        })
        .expect("seed settings");
        try_register_pm(&path, registration("session-a"), |_| true).expect("seed");

        let (prefs, removed) = deregister_pm(&path, "other-session").expect("mismatch");
        assert!(!removed, "non-matching session must not deregister");
        assert!(prefs.registration.is_some());

        let (prefs, removed) = deregister_pm(&path, "session-a").expect("match");
        assert!(removed);
        assert_eq!(prefs.registration, None);
        assert!(
            !prefs.settings.auto_start,
            "FR-002 settings must survive deregistration"
        );
    }

    /// SPEC-3431 FR-026: the profile is optional and additive, so prefs
    /// written before it existed keep working, and a project that never
    /// configures one still launches.
    #[test]
    fn launch_profile_defaults_to_none_and_round_trips() {
        let (_dir, path) = temp_prefs_path();
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, r#"{"settings":{"auto_start":true}}"#).expect("legacy prefs");

        let prefs = load_pm_prefs(&path).expect("load legacy prefs");
        assert_eq!(prefs.settings.launch_profile, None);
        assert_eq!(
            prefs.settings.launch_profile_or_default(),
            PmLaunchProfile {
                agent_id: PM_DEFAULT_AGENT.to_string(),
                model: None,
                reasoning: None,
                version: None,
            }
        );

        let configured = PmLaunchProfile {
            agent_id: "codex".to_string(),
            model: Some("gpt-5.1-codex-max".to_string()),
            reasoning: Some("high".to_string()),
            version: None,
        };
        let mut prefs = prefs;
        prefs.settings.launch_profile = Some(configured.clone());
        save_pm_prefs(&path, &prefs).expect("save");
        let reloaded = load_pm_prefs(&path).expect("reload");
        assert_eq!(reloaded.settings.launch_profile, Some(configured.clone()));
        assert_eq!(reloaded.settings.launch_profile_or_default(), configured);
    }

    /// A profile naming an agent with no skills mirror would hand the PM a
    /// `$gwt-pm` prompt that resolves to nothing — the T-052 failure,
    /// reachable through configuration. Fall back instead of refusing: the PM
    /// must always come up (FR-002).
    #[test]
    fn launch_profile_falls_back_when_the_agent_cannot_resolve_the_pm_skill() {
        for agent_id in ["gemini", "opencode", "hermes", "copilot", ""] {
            let settings = PmSettings {
                auto_start: true,
                launch_profile: Some(PmLaunchProfile {
                    agent_id: agent_id.to_string(),
                    model: Some("some-model".to_string()),
                    ..PmLaunchProfile::default()
                }),
                ..PmSettings::default()
            };
            let resolved = settings.launch_profile_or_default();
            assert_eq!(
                resolved.agent_id, PM_DEFAULT_AGENT,
                "{agent_id} has no gwt-pm mirror and must not be launched as the PM"
            );
            assert_eq!(
                resolved.model, None,
                "falling back must not keep the rejected profile's model"
            );
        }
        assert!(PM_SUPPORTED_AGENTS
            .iter()
            .all(|id| pm_agent_is_supported(id)));
    }

    /// SPEC-3431 T-052: `is_pm_worktree` gates who receives the PM operating
    /// contract, so it must match the canonical location and nothing else.
    #[test]
    fn pm_worktree_path_is_canonical_and_predicate_matches_only_it() {
        let home = tempfile::tempdir().expect("tempdir");
        let _home_guard = gwt_core::test_support::ScopedGwtHome::set(home.path());

        let repo = Path::new("/tmp/some-repo");
        let worktree = pm_worktree_path_for_repo_path(repo);
        assert_eq!(
            worktree,
            gwt_core::paths::gwt_project_dir_for_repo_path(repo).join("pm/worktree")
        );
        assert!(is_pm_worktree(&worktree));

        let pm_dir = worktree.parent().expect("pm dir");
        assert!(
            !is_pm_worktree(pm_dir),
            "the `pm` directory itself is not the worktree"
        );
        assert!(
            !is_pm_worktree(&pm_dir.join("worktree-2")),
            "a sibling directory must not match"
        );
        assert!(
            !is_pm_worktree(Path::new("/tmp/elsewhere/pm/worktree")),
            "a branch named pm/worktree outside ~/.gwt/projects must not match"
        );
    }

    #[test]
    fn prefs_path_lives_under_project_state() {
        let path = pm_prefs_path_for_repo_path(Path::new("/tmp/some-repo"));
        assert!(
            path.ends_with("project-state/pm.json"),
            "unexpected prefs path: {}",
            path.display()
        );
    }

    #[test]
    fn status_report_without_registration_omits_liveness_fields() {
        let prefs = PmPrefs {
            registration: None,
            settings: PmSettings {
                auto_start: false,
                ..PmSettings::default()
            },
        };
        let report = pm_status_report(&prefs, |_| panic!("probe must not run unregistered"));
        assert_eq!(report.schema_version, 1);
        assert!(!report.registered);
        assert!(!report.auto_start);
        assert_eq!(report.registration, None);
        assert_eq!(report.session_record_present, None);
        assert_eq!(report.stale_hint, None);
    }

    #[test]
    fn status_report_with_live_session_record_has_no_stale_hint() {
        let prefs = PmPrefs {
            registration: Some(registration("session-a")),
            ..Default::default()
        };
        let report = pm_status_report(&prefs, |session_id| {
            assert_eq!(session_id, "session-a");
            true
        });
        assert!(report.registered);
        assert!(report.auto_start);
        assert_eq!(
            report.registration.as_ref().map(|r| r.session_id.as_str()),
            Some("session-a")
        );
        assert_eq!(report.session_record_present, Some(true));
        assert_eq!(report.stale_hint, Some(false));
    }

    #[test]
    fn status_report_with_missing_session_record_hints_stale() {
        let prefs = PmPrefs {
            registration: Some(registration("session-a")),
            ..Default::default()
        };
        let report = pm_status_report(&prefs, |_| false);
        assert!(report.registered);
        assert_eq!(report.session_record_present, Some(false));
        assert_eq!(report.stale_hint, Some(true));
    }

    #[test]
    fn status_report_accounts_pm_outside_implementation_slots() {
        // SPEC-3431 T-060 (FR-014): the PM is visible as its own resident
        // bucket and never consumes an implementation slot. The global cap
        // engine itself remains SPEC #3200's unimplemented FR.
        let unregistered = pm_status_report(&PmPrefs::default(), |_| true);
        assert_eq!(unregistered.pm_bucket, 0);
        assert_eq!(unregistered.implementation_slots_consumed, 0);

        let prefs = PmPrefs {
            registration: Some(registration("session-a")),
            ..Default::default()
        };
        let registered = pm_status_report(&prefs, |_| true);
        assert_eq!(registered.pm_bucket, 1);
        assert_eq!(registered.implementation_slots_consumed, 0);
    }

    #[test]
    fn crash_backoff_first_crash_respawns_immediately() {
        // FR-003: one crash is recoverable without delay.
        let mut reg = registration("session-a");
        reg.created_at = Some("2026-08-03T00:00:00Z".to_string());
        let respawn = apply_pm_crash_backoff(&mut reg, "2026-08-03T00:01:00Z");
        assert!(respawn);
        assert_eq!(reg.consecutive_crashes, 1);
        assert_eq!(reg.next_not_before, None);
        assert!(pm_respawn_allowed(&reg, "2026-08-03T00:01:00Z"));
    }

    #[test]
    fn crash_backoff_series_extends_floor_and_blocks_respawn() {
        // FR-003: rapid consecutive crashes extend the floor.
        let mut reg = registration("session-a");
        reg.created_at = Some("2026-08-03T00:00:00Z".to_string());
        reg.consecutive_crashes = 1;
        let respawn = apply_pm_crash_backoff(&mut reg, "2026-08-03T00:01:00Z");
        assert!(!respawn, "second crash in a series must back off");
        assert_eq!(reg.consecutive_crashes, 2);
        assert_eq!(
            reg.next_not_before.as_deref(),
            Some("2026-08-03T00:01:30Z"),
            "floor = now + 30s at series position 2"
        );
        assert!(!pm_respawn_allowed(&reg, "2026-08-03T00:01:00Z"));
        assert!(pm_respawn_allowed(&reg, "2026-08-03T00:01:30Z"));
    }

    #[test]
    fn crash_backoff_resets_series_after_healthy_uptime() {
        // FR-003: a long healthy run means the next crash starts fresh.
        let mut reg = registration("session-a");
        reg.created_at = Some("2026-08-03T00:00:00Z".to_string());
        reg.consecutive_crashes = 3;
        reg.next_not_before = Some("2026-08-03T00:05:00Z".to_string());
        let respawn = apply_pm_crash_backoff(&mut reg, "2026-08-03T00:20:00Z");
        assert!(respawn);
        assert_eq!(reg.consecutive_crashes, 1);
        assert_eq!(reg.next_not_before, None);
    }

    #[test]
    fn registered_pm_session_is_recognized_and_others_are_not() {
        // SPEC-3431 FR-009: the privileged subject is exactly the session the
        // durable registration names. Anything else — a different agent
        // session, an empty ambient id, or no PM at all — is not privileged.
        let (_dir, path) = temp_prefs_path();
        assert!(
            !session_is_registered_pm(&path, "session-a"),
            "no registration means no privilege"
        );

        try_register_pm(&path, registration("session-a"), |_| false).expect("register");
        assert!(session_is_registered_pm(&path, "session-a"));
        assert!(!session_is_registered_pm(&path, "session-b"));
        assert!(!session_is_registered_pm(&path, ""));
        assert!(
            !session_is_registered_pm(&path, " session-a "),
            "untrimmed ids must not match"
        );

        deregister_pm(&path, "session-a").expect("deregister");
        assert!(
            !session_is_registered_pm(&path, "session-a"),
            "a closed PM loses its privilege immediately"
        );
    }

    #[test]
    fn pm_registration_never_touches_issue_monitor_slot_accounting() {
        // SPEC-3431 T-060 (FR-014): registering a PM must leave the Issue
        // Monitor's implementation-slot ledger untouched. The stores are
        // structurally separate files; this test pins that separation so a
        // future coupling has to break it consciously.
        let (_dir, path) = temp_prefs_path();
        let monitor = crate::IssueMonitorState::new(crate::IssueMonitorConfig::default());
        let before = monitor.active_count();

        try_register_pm(&path, registration("session-a"), |_| true).expect("register");

        assert_eq!(monitor.active_count(), before);
        assert_eq!(monitor.active_count(), 0);
    }
}
