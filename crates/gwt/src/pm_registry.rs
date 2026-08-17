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
    pub target_session_id: String,
    pub body_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmDeliveryPrepareOutcome {
    Prepared,
    Existing(PmDeliveryReceiptStatus),
}

pub fn pm_delivery_prompt_sha256(prompt: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(prompt.as_bytes()))
}

pub fn protected_pm_delivery_prompt(operation_id: &str, body: &str) -> io::Result<String> {
    if uuid::Uuid::parse_str(operation_id)
        .ok()
        .is_none_or(|parsed| parsed.hyphenated().to_string() != operation_id)
        || body.is_empty()
        || body.contains(['\r', '\n'])
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "PM delivery prompt identity must be one non-empty line",
        ));
    }
    let body_sha256 = pm_delivery_prompt_sha256(body);
    Ok(format!(
        "{body} [gwt-delivery:{operation_id}:{body_sha256}]\r"
    ))
}

pub fn parse_protected_pm_delivery_prompt(prompt: &str) -> Option<(String, String)> {
    let prompt = prompt.strip_suffix('\r').unwrap_or(prompt);
    let marker_start = prompt.rfind(" [gwt-delivery:")?;
    let body = &prompt[..marker_start];
    let marker = prompt[marker_start..]
        .strip_prefix(" [gwt-delivery:")?
        .strip_suffix(']')?;
    let (operation_id, body_sha256) = marker.split_once(':')?;
    if uuid::Uuid::parse_str(operation_id)
        .ok()
        .is_none_or(|parsed| parsed.hyphenated().to_string() != operation_id)
        || !is_canonical_sha256(body_sha256)
        || pm_delivery_prompt_sha256(body) != body_sha256
    {
        return None;
    }
    Some((operation_id.to_string(), body_sha256.to_string()))
}

fn is_canonical_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Discard only a torn final JSONL row. A malformed complete row remains a
/// hard error so corruption cannot silently erase an accepted operation.
fn repair_pm_delivery_receipt_tail(path: &Path) -> io::Result<()> {
    let existing = match fs::read(path) {
        Ok(existing) => existing,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if existing.is_empty() {
        return Ok(());
    }
    if existing.ends_with(b"\n") {
        return Ok(());
    }
    let row_start = existing
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |position| position + 1);
    let final_row = &existing[row_start..];
    if serde_json::from_slice::<PmDeliveryReceipt>(final_row).is_ok() {
        let mut file = fs::OpenOptions::new().append(true).open(path)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        return sync_parent_directory(path);
    }
    let file = fs::OpenOptions::new().write(true).open(path)?;
    file.set_len(row_start as u64)?;
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

pub fn load_pm_delivery_receipts(path: &Path) -> io::Result<Vec<PmDeliveryReceipt>> {
    with_pm_prefs_lock(path, || load_pm_delivery_receipts_unlocked(path))
}

fn validated_pm_delivery_operation<'a>(
    receipts: &'a [PmDeliveryReceipt],
    operation_id: &str,
) -> io::Result<Vec<&'a PmDeliveryReceipt>> {
    let operation = receipts
        .iter()
        .filter(|receipt| receipt.operation_id == operation_id)
        .collect::<Vec<_>>();
    let Some(prepared) = operation.first().copied() else {
        return Ok(operation);
    };
    if prepared.status != PmDeliveryReceiptStatus::Prepared {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "PM delivery operation does not begin with Prepared",
        ));
    }
    let mut latest = PmDeliveryReceiptStatus::Prepared;
    for receipt in &operation {
        if receipt.principal_session_id != prepared.principal_session_id
            || receipt.target_window_id != prepared.target_window_id
            || receipt.target_session_id != prepared.target_session_id
            || receipt.body_sha256 != prepared.body_sha256
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "PM delivery operation identity conflicts across durable receipts",
            ));
        }
        let transition_is_valid = match (latest, receipt.status) {
            (PmDeliveryReceiptStatus::Prepared, PmDeliveryReceiptStatus::Prepared) => true,
            (PmDeliveryReceiptStatus::Prepared, terminal)
                if terminal != PmDeliveryReceiptStatus::Prepared =>
            {
                true
            }
            (PmDeliveryReceiptStatus::Ambiguous, PmDeliveryReceiptStatus::Verified) => true,
            _ => false,
        };
        if !transition_is_valid {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "PM delivery operation has an invalid durable state transition",
            ));
        }
        latest = receipt.status;
    }
    Ok(operation)
}

pub fn pm_delivery_receipt_for_operation(
    path: &Path,
    operation_id: &str,
) -> io::Result<Option<PmDeliveryReceipt>> {
    with_pm_prefs_lock(path, || {
        let receipts = load_pm_delivery_receipts_unlocked(path)?;
        Ok(validated_pm_delivery_operation(&receipts, operation_id)?
            .last()
            .copied()
            .cloned())
    })
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
    with_pm_prefs_lock(path, || {
        let receipts = load_pm_delivery_receipts_unlocked(path)?;
        let existing = validated_pm_delivery_operation(&receipts, &prepared.operation_id)?;
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
    reason: Option<&str>,
) -> io::Result<PmDeliveryReceiptStatus> {
    if status == PmDeliveryReceiptStatus::Prepared {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "PM delivery terminal receipt cannot be Prepared",
        ));
    }
    with_pm_prefs_lock(path, || {
        let receipts = load_pm_delivery_receipts_unlocked(path)?;
        let operation = validated_pm_delivery_operation(&receipts, operation_id)?;
        let Some(prepared) = operation.first().copied() else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "PM delivery Prepared receipt is missing",
            ));
        };
        if prepared.target_session_id != target_session_id || prepared.body_sha256 != body_sha256 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "PM delivery acknowledgement does not match its Prepared receipt",
            ));
        }
        if let Some(existing) = operation
            .iter()
            .rev()
            .copied()
            .find(|receipt| receipt.status != PmDeliveryReceiptStatus::Prepared)
        {
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
                    ))
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
            reason: reason.map(str::to_string),
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

pub fn load_pm_loop_state(path: &Path) -> io::Result<PmLoopState> {
    match fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str(&raw)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(PmLoopState::default()),
        Err(error) => Err(error),
    }
}

pub fn save_pm_loop_state(path: &Path, state: &PmLoopState) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let scratch = unique_pm_scratch_path(path);
    fs::write(&scratch, serde_json::to_string_pretty(state)?)?;
    fs::rename(&scratch, path)
}

/// Whether `path` is some project's canonical PM worktree.
///
/// Gates PM-only managed assets, so it anchors on the full shape
/// `<gwt projects dir>/<repo hash>/pm/worktree` rather than the trailing two
/// segments alone — a production branch literally named `pm/worktree` would
/// otherwise be handed the PM operating contract.
pub fn is_pm_worktree(path: &Path) -> bool {
    pm_worktree_store_dir(path).is_some()
}

/// The project store that owns `path`, when `path` is that store's PM
/// worktree: `<gwt projects dir>/<repo hash>` for
/// `<gwt projects dir>/<repo hash>/pm/worktree`.
///
/// Issue #3607 needs the owning store, not just the yes/no shape test, because
/// a PM worktree from *another* store is exactly what session restore must
/// refuse (AC-3).
pub fn pm_worktree_store_dir(path: &Path) -> Option<PathBuf> {
    let pm_dir = path.parent()?;
    if path.file_name() != Some(std::ffi::OsStr::new("worktree"))
        || pm_dir.file_name() != Some(std::ffi::OsStr::new("pm"))
    {
        return None;
    }
    let project_dir = pm_dir.parent()?;
    (project_dir.parent()? == gwt_core::paths::gwt_projects_dir())
        .then(|| project_dir.to_path_buf())
}

/// Issue #3607 AC-3: whether restoring a session rooted at `session_worktree`
/// into the store at `own_project_dir` would resurrect a *different* store's
/// PM.
///
/// The stopped store in the incident was not even open in the app, yet its PM
/// came back: the current store's `workspace.json` still held a window whose
/// Session pointed at the other store's `pm/worktree`. Restore reached it
/// through the window's session id alone, so nothing on that path ever
/// compared the two stores. Dropping the `auto_start` flag cannot close this —
/// restore never consults a registration.
///
/// Ordinary work worktrees are unaffected: only a path shaped like some
/// store's PM worktree can be foreign here.
pub fn is_foreign_pm_worktree(session_worktree: &Path, own_project_dir: &Path) -> bool {
    let Some(store_dir) = pm_worktree_store_dir(session_worktree) else {
        return false;
    };
    !paths_are_same_store(&store_dir, own_project_dir)
}

fn paths_are_same_store(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    let canonical = |path: &Path| dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    canonical(left) == canonical(right)
}

/// A PM registration together with the project store holding it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmStoreRegistration {
    /// `<gwt projects dir>/<repo hash>`.
    pub project_dir: PathBuf,
    /// `<project_dir>/project-state/pm.json`.
    pub prefs_path: PathBuf,
    pub registration: PmRegistration,
}

/// Issue #3607 AC-1: the identity PM uniqueness is judged on.
///
/// `~/.gwt/projects/<hash>` is a *store* scope. One repository can own two
/// stores once scope resolution splits (#3466), and each store then keeps its
/// own `pm.json` with its own `auto_start` — neither can see the other, so
/// "one PM per project" silently became "one PM per store". The git common dir
/// is the scope that does not split.
pub fn pm_repository_key(path: &Path) -> Option<PathBuf> {
    gwt_core::repo_hash::repository_common_dir(path)
}

/// Every PM registration on this machine belonging to `repository_key`,
/// ordered by store directory so callers and diagnostics are deterministic.
///
/// A registration whose worktree no longer resolves to a repository is skipped:
/// its PM cannot be running, so it must not block a fresh one.
pub fn pm_registrations_for_repository(repository_key: &Path) -> Vec<PmStoreRegistration> {
    let projects_dir = gwt_core::paths::gwt_projects_dir();
    let Ok(entries) = fs::read_dir(&projects_dir) else {
        return Vec::new();
    };
    let mut project_dirs: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    project_dirs.sort();

    project_dirs
        .into_iter()
        .filter_map(|project_dir| {
            let prefs_path = project_dir.join("project-state/pm.json");
            let registration = load_pm_prefs(&prefs_path).ok()?.registration?;
            let worktree = PathBuf::from(&registration.worktree_path);
            (pm_repository_key(&worktree).as_deref() == Some(repository_key)).then_some(
                PmStoreRegistration {
                    project_dir,
                    prefs_path,
                    registration,
                },
            )
        })
        .collect()
}

/// Issue #3607 AC-5: clear the registration naming `session_id` from whichever
/// store in `repository_key` holds it.
///
/// Addressing by session id rather than by store is what makes an orphan
/// stoppable: the PM asking for the stop only knows the id it saw in
/// `pm.status`, not which of the split stores the orphan registered in.
/// Returns the record that was cleared, or `None` when no store in this
/// repository registered that session.
pub fn stop_pm_registration_in_repository(
    repository_key: &Path,
    session_id: &str,
) -> Option<PmStoreRegistration> {
    let target = pm_registrations_for_repository(repository_key)
        .into_iter()
        .find(|record| record.registration.session_id == session_id)?;
    match deregister_pm(&target.prefs_path, session_id) {
        Ok((_, true)) => Some(target),
        Ok((_, false)) => None,
        Err(error) => {
            tracing::warn!(
                path = %target.prefs_path.display(),
                %error,
                "failed to clear the PM registration"
            );
            None
        }
    }
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
        gwt_core::operation_deadline::ensure_remaining("PM prefs durable rename")?;
        fs::rename(&scratch_path, path)?;
        sync_parent_directory(path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&scratch_path);
    }
    result
}

fn with_pm_prefs_lock<T>(path: &Path, operation: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
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

/// SPEC-3431 FR-032 (Issue #3477): is `session_id` this project's registered
/// PM, running in this project's canonical PM worktree?
///
/// The Work mutation paths use this to grant a branchless identity to the one
/// Session that has no branch by design. It is stricter than
/// [`session_is_registered_pm`] on purpose: PM privilege over conversational
/// operations only needs the subject, but rewriting Work state also needs the
/// *container* to be the exact worktree this project's PM was given. Three
/// facts must agree, and any disagreement is not privileged (fail-closed):
///
/// 1. the durable registration names `session_id`,
/// 2. the registration's worktree is `worktree`, and
/// 3. `worktree` is the canonical PM worktree derived from
///    `project_state_root` itself.
///
/// (3) is what keeps a stale registration pointing at some other directory —
/// or a foreign project's PM path — from authorizing anything here. Paths are
/// compared canonically because callers hand us an already-canonicalized
/// worktree while the derived and stored paths may still contain symlinks.
pub fn registered_pm_worktree_authority(
    project_state_root: &Path,
    session_id: &str,
    worktree: &Path,
) -> bool {
    if session_id.is_empty() {
        return false;
    }
    let canonical = |path: &Path| dunce::canonicalize(path).ok();
    let Some(worktree) = canonical(worktree) else {
        return false;
    };
    if canonical(&pm_worktree_path_for_repo_path(project_state_root)).as_ref() != Some(&worktree) {
        return false;
    }
    load_pm_prefs(&pm_prefs_path_for_repo_path(project_state_root)).is_ok_and(|prefs| {
        prefs.registration.is_some_and(|registration| {
            registration.session_id == session_id
                && canonical(Path::new(&registration.worktree_path)).as_ref() == Some(&worktree)
        })
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
    /// Issue #3607: every PM registration in this *repository*, including ones
    /// held by another project store.
    ///
    /// `registration` above is this store's only, which is exactly the blind
    /// spot that let a second PM run unnoticed. A PM cannot ask another store
    /// to identify itself, so the orphan's session id has to be discoverable
    /// here — it is the input `pm.stop` needs.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub repository_registrations: Vec<PmRepositoryRegistrationView>,
}

/// One row of [`PmStatusReport::repository_registrations`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PmRepositoryRegistrationView {
    pub project_dir: String,
    pub session_id: String,
    pub agent_id: String,
    pub worktree_path: String,
    /// Whether this row is the store the report was asked about. A `false` row
    /// is a PM this store cannot see through its own `pm.json`.
    pub is_current_store: bool,
}

/// Build the repository-scoped rows for `repo_path`'s report.
pub fn pm_repository_registration_views(repo_path: &Path) -> Vec<PmRepositoryRegistrationView> {
    let Some(repository_key) = pm_repository_key(repo_path) else {
        return Vec::new();
    };
    let own_project_dir = gwt_core::paths::gwt_project_dir_for_repo_path(repo_path);
    pm_registrations_for_repository(&repository_key)
        .into_iter()
        .map(|record| PmRepositoryRegistrationView {
            is_current_store: record.project_dir == own_project_dir,
            project_dir: record.project_dir.display().to_string(),
            session_id: record.registration.session_id,
            agent_id: record.registration.agent_id,
            worktree_path: record.registration.worktree_path,
        })
        .collect()
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
        repository_registrations: Vec::new(),
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

    /// Issue #3607: reproduce the observed split — one repository whose
    /// `pm/worktree` linked worktrees live in two different project stores.
    ///
    /// The `.git` file / `commondir` pair is written by hand rather than by
    /// `git worktree add` so the fixture stays hermetic and fast; it is the
    /// exact on-disk shape git itself materializes.
    struct SplitStoreFixture {
        _home: tempfile::TempDir,
        _repo_dir: tempfile::TempDir,
        repo: PathBuf,
        stores: Vec<PathBuf>,
        _home_guard: gwt_core::test_support::ScopedGwtHome,
    }

    impl SplitStoreFixture {
        fn new(store_hashes: &[&str]) -> Self {
            let home = tempfile::tempdir().expect("home");
            let home_guard = gwt_core::test_support::ScopedGwtHome::set(home.path());
            let repo_dir = tempfile::tempdir().expect("repo dir");
            let repo = repo_dir.path().join("repo");
            let git_dir = repo.join(".git");
            fs::create_dir_all(git_dir.join("worktrees")).expect("git dir");
            fs::write(git_dir.join("HEAD"), "ref: refs/heads/main\n").expect("HEAD");
            fs::write(git_dir.join("config"), "[core]\n\tbare = false\n").expect("config");

            let stores = store_hashes
                .iter()
                .map(|hash| {
                    let project_dir = gwt_core::paths::gwt_project_dir(
                        &gwt_core::repo_hash::compute_repo_hash(hash),
                    );
                    // Use the caller's literal hash for the directory name so
                    // assertions can name the store the way the incident report
                    // does.
                    let project_dir = project_dir.with_file_name(hash);
                    let worktree = project_dir.join("pm/worktree");
                    fs::create_dir_all(&worktree).expect("pm worktree");
                    let admin = git_dir.join("worktrees").join(hash);
                    fs::create_dir_all(&admin).expect("worktree admin dir");
                    fs::write(admin.join("commondir"), "../..\n").expect("commondir");
                    fs::write(
                        worktree.join(".git"),
                        format!("gitdir: {}\n", admin.display()),
                    )
                    .expect(".git file");
                    project_dir
                })
                .collect();

            Self {
                _home: home,
                _repo_dir: repo_dir,
                repo,
                stores,
                _home_guard: home_guard,
            }
        }

        fn register(&self, store_index: usize, session_id: &str) -> PathBuf {
            let project_dir = &self.stores[store_index];
            let prefs_path = project_dir.join("project-state/pm.json");
            let mut candidate = registration(session_id);
            candidate.worktree_path = project_dir.join("pm/worktree").display().to_string();
            save_pm_prefs(
                &prefs_path,
                &PmPrefs {
                    registration: Some(candidate),
                    settings: PmSettings::default(),
                },
            )
            .expect("save prefs");
            prefs_path
        }
    }

    /// AC-1 / AC-4: with two split stores holding a registration each, the
    /// repository-scoped view must see both. Store-scoped uniqueness cannot —
    /// that blindness is what let two PMs run against one repository.
    #[test]
    fn repository_scoped_scan_sees_registrations_in_every_split_store() {
        let fixture = SplitStoreFixture::new(&["99a8660247f5bc49", "b19aac38305901f5"]);
        fixture.register(0, "fedf798b-current");
        fixture.register(1, "b0801016-orphan");

        let key = pm_repository_key(&fixture.repo).expect("repository key");
        let found = pm_registrations_for_repository(&key);

        let sessions: Vec<&str> = found
            .iter()
            .map(|record| record.registration.session_id.as_str())
            .collect();
        assert_eq!(
            sessions.len(),
            2,
            "both split stores belong to one repository: {sessions:?}"
        );
        assert!(sessions.contains(&"fedf798b-current"));
        assert!(sessions.contains(&"b0801016-orphan"));
    }

    /// The repository key must also be derivable from a PM worktree, because
    /// that is all a registration records about its repository.
    #[test]
    fn repository_key_from_a_pm_worktree_matches_the_repository() {
        let fixture = SplitStoreFixture::new(&["store-a", "store-b"]);
        let expected = pm_repository_key(&fixture.repo).expect("repository key");
        for store in &fixture.stores {
            assert_eq!(
                pm_repository_key(&store.join("pm/worktree")).as_ref(),
                Some(&expected),
                "every store's PM worktree belongs to the one repository"
            );
        }
    }

    /// Registrations for an unrelated repository must never be pulled in.
    #[test]
    fn repository_scoped_scan_excludes_other_repositories() {
        let fixture = SplitStoreFixture::new(&["store-a"]);
        fixture.register(0, "mine");

        let unrelated = tempfile::tempdir().expect("unrelated");
        let unrelated_repo = unrelated.path().join("repo");
        fs::create_dir_all(unrelated_repo.join(".git")).expect("git dir");
        fs::write(unrelated_repo.join(".git/HEAD"), "ref: refs/heads/main\n").expect("HEAD");

        let key = pm_repository_key(&unrelated_repo).expect("repository key");
        assert!(
            pm_registrations_for_repository(&key).is_empty(),
            "another repository's PM must not appear in this repository's scan"
        );
    }

    /// AC-3: session restore must be able to tell "this session's worktree is
    /// another store's PM worktree" without loading any registration.
    #[test]
    fn foreign_pm_worktree_is_recognised_across_stores() {
        let fixture = SplitStoreFixture::new(&["current", "orphan"]);
        let current = &fixture.stores[0];
        let orphan_pm_worktree = fixture.stores[1].join("pm/worktree");

        assert_eq!(
            pm_worktree_store_dir(&orphan_pm_worktree).as_ref(),
            Some(&fixture.stores[1]),
            "a PM worktree names the store that owns it"
        );
        assert!(
            is_foreign_pm_worktree(&orphan_pm_worktree, current),
            "the orphan store's PM worktree is foreign to the current store"
        );
        assert!(
            !is_foreign_pm_worktree(&current.join("pm/worktree"), current),
            "the current store's own PM worktree must still restore"
        );
        assert!(
            !is_foreign_pm_worktree(Path::new("/tmp/ordinary/worktree"), current),
            "an ordinary work worktree is not a PM worktree at all"
        );
    }

    /// AC-5: the orphan's registration is cleared from whichever store holds
    /// it, addressed only by session id.
    #[test]
    fn repository_scoped_stop_clears_the_orphans_registration() {
        let fixture = SplitStoreFixture::new(&["current", "orphan"]);
        let current_prefs = fixture.register(0, "live-pm");
        let orphan_prefs = fixture.register(1, "orphan-pm");

        let key = pm_repository_key(&fixture.repo).expect("repository key");
        let stopped = stop_pm_registration_in_repository(&key, "orphan-pm").expect("stop");

        assert_eq!(stopped.registration.session_id, "orphan-pm");
        assert_eq!(stopped.prefs_path, orphan_prefs);
        assert_eq!(
            load_pm_prefs(&orphan_prefs)
                .expect("load orphan")
                .registration,
            None,
            "the orphan registration is gone"
        );
        assert!(
            load_pm_prefs(&current_prefs)
                .expect("load current")
                .registration
                .is_some(),
            "stopping the orphan must not disturb the live PM"
        );
    }

    /// Stopping a session that is not a PM of this repository must fail rather
    /// than silently succeed — the caller has to learn it named the wrong one.
    #[test]
    fn repository_scoped_stop_refuses_an_unknown_session() {
        let fixture = SplitStoreFixture::new(&["current"]);
        fixture.register(0, "live-pm");
        let key = pm_repository_key(&fixture.repo).expect("repository key");

        assert!(stop_pm_registration_in_repository(&key, "no-such-session").is_none());
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

    #[test]
    fn delivery_receipt_is_write_ahead_idempotent_and_body_free() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir
            .path()
            .join("project-state")
            .join("pm-delivery-receipts.jsonl");
        let operation_id = "72fc3cd4-ad49-43e3-bf3d-d791357643a3";
        let body = "report exact status";
        let prepared = PmDeliveryReceipt {
            operation_id: operation_id.to_string(),
            recorded_at: "2026-08-13T00:00:00Z".to_string(),
            status: PmDeliveryReceiptStatus::Prepared,
            principal_session_id: "pm-session".to_string(),
            target_window_id: "tab-1::agent-1".to_string(),
            target_session_id: "agent-session".to_string(),
            body_sha256: pm_delivery_prompt_sha256(body),
            reason: None,
        };

        assert_eq!(
            prepare_pm_delivery_receipt(&path, &prepared).expect("prepare"),
            PmDeliveryPrepareOutcome::Prepared
        );
        assert_eq!(
            prepare_pm_delivery_receipt(&path, &prepared).expect("idempotent prepare"),
            PmDeliveryPrepareOutcome::Existing(PmDeliveryReceiptStatus::Prepared)
        );

        let mut conflicting = prepared.clone();
        conflicting.body_sha256 = pm_delivery_prompt_sha256("different body");
        assert!(prepare_pm_delivery_receipt(&path, &conflicting).is_err());

        assert_eq!(
            finish_pm_delivery_receipt(
                &path,
                operation_id,
                "agent-session",
                &prepared.body_sha256,
                PmDeliveryReceiptStatus::Verified,
                None,
            )
            .expect("verify"),
            PmDeliveryReceiptStatus::Verified
        );
        assert_eq!(
            prepare_pm_delivery_receipt(&path, &prepared).expect("terminal replay"),
            PmDeliveryPrepareOutcome::Existing(PmDeliveryReceiptStatus::Verified)
        );
        assert!(load_pm_delivery_receipts(&path)
            .expect("load receipts")
            .iter()
            .any(|receipt| receipt.status == PmDeliveryReceiptStatus::Verified));
        assert!(!fs::read_to_string(path)
            .expect("receipt file")
            .contains(body));
    }

    #[test]
    fn delivery_receipt_tail_repair_preserves_a_complete_row_without_newline() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("pm-delivery-receipts.jsonl");
        let receipt = PmDeliveryReceipt {
            operation_id: "72fc3cd4-ad49-43e3-bf3d-d791357643a4".to_string(),
            recorded_at: "2026-08-13T00:00:00Z".to_string(),
            status: PmDeliveryReceiptStatus::Prepared,
            principal_session_id: "pm-session".to_string(),
            target_window_id: "tab-1::agent-1".to_string(),
            target_session_id: "agent-session".to_string(),
            body_sha256: pm_delivery_prompt_sha256("body"),
            reason: None,
        };
        fs::write(
            &path,
            serde_json::to_vec(&receipt).expect("serialize receipt"),
        )
        .expect("write complete row without newline");

        assert_eq!(
            load_pm_delivery_receipts(&path).expect("repair complete row"),
            vec![receipt]
        );
    }

    #[test]
    fn delivery_receipt_rejects_a_terminal_row_with_conflicting_identity() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("pm-delivery-receipts.jsonl");
        let operation_id = "72fc3cd4-ad49-43e3-bf3d-d791357643a5";
        let prepared = PmDeliveryReceipt {
            operation_id: operation_id.to_string(),
            recorded_at: "2026-08-13T00:00:00Z".to_string(),
            status: PmDeliveryReceiptStatus::Prepared,
            principal_session_id: "pm-session".to_string(),
            target_window_id: "tab-1::agent-1".to_string(),
            target_session_id: "agent-session".to_string(),
            body_sha256: pm_delivery_prompt_sha256("body"),
            reason: None,
        };
        let mut forged = prepared.clone();
        forged.status = PmDeliveryReceiptStatus::Verified;
        forged.target_window_id = "tab-1::agent-2".to_string();
        let rows = format!(
            "{}\n{}\n",
            serde_json::to_string(&prepared).expect("serialize Prepared"),
            serde_json::to_string(&forged).expect("serialize forged terminal")
        );
        fs::write(&path, rows).expect("write forged receipt log");

        let error = prepare_pm_delivery_receipt(&path, &prepared)
            .expect_err("terminal identity conflict must fail closed");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}
