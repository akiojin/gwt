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
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Issue #3632 FR-1/FR-2/FR-3 (user ruling 2026-08-17): how every prompt gwt
/// injects into the resident PM must end.
///
/// Three separate injection points drive a PM cycle — the delta wake, the
/// periodic wake, and the Stop-gate forced continuation — and each used to
/// phrase the reporting expectation itself. Injected text outranks the gwt-pm
/// skill body for the reading agent, so those three sentences, not the
/// milestone-only cadence in `gwt_skills::pm_guidance`, decided what the PM
/// actually did: both wakes closed with a flat "report the milestone digest"
/// and produced a digest on every tick, change or no change.
///
/// One canonical clause, used verbatim by all three, keeps the wordings from
/// drifting apart again. It constrains the *report* and never the cycle: the
/// reconcile still runs in full, and the wake that drove it is recorded in
/// `pm-loop.json`'s `last_wake_at`, which is how a silent-but-alive loop stays
/// distinguishable from a dead one without a keepalive line in the
/// conversation (FR-4).
/// Issue #3868 AC-3: a red, conflicted, or escalation-due open PR is never a
/// no-change cycle. Kept terse on purpose — this clause rides the PTY wake
/// prompts, which must stay under the 1024-byte canonical queue (#3825).
pub const PM_CYCLE_REPORTING_CLAUSE: &str =
    "Report a digest only for a milestone or an escalation; end the cycle with no user-facing \
     output only if nothing changed and no open PR is CI-RED, CONFLICTED, or escalation_due.";

/// Issue #3776 / SPEC-3431 FR-148: compact reminder shared by the delta
/// wake, periodic wake, and Stop-gate continuation. The generated gwt-pm
/// guidance owns the detailed timeout, retry, readback, and lifecycle rules;
/// this clause only prevents injected prompts from silently restoring direct
/// long-running execution.
pub const PM_GWTD_EXECUTION_CLAUSE: &str =
    "Keep the PM turn responsive: run only short read-only gwtd operations directly with the \
     contract's 10-second outer deadline. Delegate `daemon.subscribe`, batch mutations, repeated \
     `pane.read`, and every long-running or hang-risk operation to exactly one background task or \
     in-session sub-agent; collect the result only from its task-completion notification, and \
     never duplicate an operation while it is pending.";

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
/// writes the `.claude` and `.codex` mirrors. Grok consumes the existing
/// Claude-compatible target rather than inventing a third mirror. Launching
/// the PM as any other agent would hand it a prompt that resolves to nothing —
/// the exact failure the T-052 materialization fix removed, reintroduced
/// through configuration.
pub const PM_SUPPORTED_AGENTS: &[&str] = &["claude", "codex", "grok"];

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
            Some(profile) if pm_agent_is_supported(&profile.agent_id) => {
                profile.clone().normalized()
            }
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
    /// Canonicalize user-entered optional tuning without changing the agent
    /// identity or the legacy optional schema. Blank values mean provider
    /// defaults; non-blank values are persisted and launched without their UI
    /// padding.
    pub fn normalized(mut self) -> Self {
        let normalize = |value: Option<String>| {
            value.and_then(|value| {
                let trimmed = value.trim();
                (!trimmed.is_empty()).then(|| trimmed.to_string())
            })
        };
        self.model = normalize(self.model);
        self.reasoning = normalize(self.reasoning);
        if self.agent_id == "grok"
            && self
                .reasoning
                .as_deref()
                .is_some_and(|effort| effort.eq_ignore_ascii_case("auto"))
        {
            self.reasoning = None;
        }
        self
    }

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PmWorktreeFreshnessState {
    Fresh,
    Stale,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PmWorktreeTargetObservation {
    Fresh,
    Cached,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PmWorktreeRefreshFailureStage {
    Fetch,
    Inspect,
    LocalWork,
    Repoint,
    ManagedAssets,
    ScratchMigration,
}

/// Durable observation of how closely the PM worktree follows its base ref.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PmWorktreeFreshness {
    pub state: PmWorktreeFreshnessState,
    pub base_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_sha: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_sha: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub behind: Option<u64>,
    pub target_observation: PmWorktreeTargetObservation,
    pub checked_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_stage: Option<PmWorktreeRefreshFailureStage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PmPrefs {
    #[serde(default)]
    pub registration: Option<PmRegistration>,
    #[serde(default)]
    pub settings: PmSettings,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_freshness: Option<PmWorktreeFreshness>,
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

/// Project-state directory for PM-authored scratch notes.
pub fn pm_scratch_dir_for_repo_path(repo_path: &Path) -> PathBuf {
    gwt_core::paths::gwt_project_dir_for_repo_path(repo_path).join("project-state/pm-scratch")
}

/// Create the project-scoped PM scratch directory without following a
/// symlink/reparse point at any owned path component.
///
/// The caller prepares the canonical project directory itself; this helper
/// verifies that boundary, then creates `project-state` and `pm-scratch` one
/// level at a time and rechecks each node before descending into it.
pub fn ensure_pm_scratch_dir_for_repo_path(repo_path: &Path) -> io::Result<PathBuf> {
    let project_dir = gwt_core::paths::gwt_project_dir_for_repo_path(repo_path);
    require_real_pm_scratch_directory(&project_dir, "project directory")?;
    let project_state = project_dir.join("project-state");
    ensure_real_pm_scratch_directory(&project_state, "project-state directory")?;
    let scratch = project_state.join("pm-scratch");
    ensure_real_pm_scratch_directory(&scratch, "scratch root")?;
    Ok(scratch)
}

/// Resolve PM scratch only from a structurally canonical PM worktree.
pub fn pm_scratch_dir_for_pm_worktree(worktree: &Path) -> Option<PathBuf> {
    if !is_pm_worktree(worktree) {
        return None;
    }
    Some(
        worktree
            .parent()?
            .parent()?
            .join("project-state/pm-scratch"),
    )
}

const LEGACY_PM_SCRATCH_PATHS: [&str; 3] = ["tasks/todo.md", "tasks/pm-notes.md", "pm-notes.md"];

fn legacy_pm_scratch_has_local_state(worktree: &Path, relative: &str) -> io::Result<bool> {
    // Older installs and unit fixtures may contain the pre-externalization
    // directory without Git metadata. Those files are legacy scratch by
    // definition and retain the established migration behavior.
    if !worktree.join(".git").exists() {
        return Ok(true);
    }
    let output = gwt_core::process::run_git_logged(
        &[
            "status",
            "--porcelain=v1",
            "--ignored",
            "--untracked-files=all",
            "--",
            relative,
        ],
        Some(worktree),
    )
    .map_err(|error| {
        io::Error::other(format!(
            "inspect legacy PM scratch Git state for {relative} at {}: {error}",
            worktree.display()
        ))
    })?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "inspect legacy PM scratch Git state for {relative} at {}: {}",
            worktree.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(!output.stdout.is_empty())
}

fn tracked_legacy_pm_scratch_with_local_state(worktree: &Path) -> io::Result<Vec<&'static str>> {
    if !worktree.join(".git").exists() {
        return Ok(Vec::new());
    }
    let mut tracked = Vec::new();
    for relative in LEGACY_PM_SCRATCH_PATHS {
        if !worktree.join(relative).exists()
            || !legacy_pm_scratch_has_local_state(worktree, relative)?
        {
            continue;
        }
        let output = gwt_core::process::run_git_logged(
            &["ls-files", "--error-unmatch", "--", relative],
            Some(worktree),
        )?;
        if output.status.success() {
            tracked.push(relative);
        }
    }
    Ok(tracked)
}

fn restore_tracked_pm_scratch_after_migration(worktree: &Path, tracked: &[&str]) -> io::Result<()> {
    for relative in tracked {
        let output = gwt_core::process::run_git_logged(
            &["checkout", "HEAD", "--", relative],
            Some(worktree),
        )?;
        if !output.status.success() {
            return Err(io::Error::other(format!(
                "restore tracked project content after PM scratch migration for {relative}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
    }
    Ok(())
}

fn reject_tracked_pm_scratch_with_index_changes(
    worktree: &Path,
    tracked: &[&str],
) -> io::Result<()> {
    for relative in tracked {
        let output = gwt_core::process::run_git_logged(
            &["diff", "--cached", "--quiet", "--exit-code", "--", relative],
            Some(worktree),
        )?;
        match output.status.code() {
            Some(0) => {}
            Some(1) => {
                return Err(io::Error::other(format!(
                    "refusing to externalize tracked legacy PM scratch with staged changes at {relative}; the index and working tree may contain distinct local versions"
                )));
            }
            _ => {
                return Err(io::Error::other(format!(
                    "inspect staged legacy PM scratch state for {relative} at {}: {}",
                    worktree.display(),
                    String::from_utf8_lossy(&output.stderr).trim()
                )));
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PmScratchFileFingerprint {
    content_len: u64,
    content_sha256: [u8; 32],
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(windows)]
    volume_serial_number: u32,
    #[cfg(windows)]
    file_index: u64,
}

impl PmScratchFileFingerprint {
    fn has_same_content(&self, other: &Self) -> bool {
        self.content_len == other.content_len && self.content_sha256 == other.content_sha256
    }
}

#[derive(Debug, Clone)]
struct StagedPmScratchFile {
    path: PathBuf,
    fingerprint: PmScratchFileFingerprint,
}

#[derive(Debug)]
struct PmScratchMigration {
    relative: &'static str,
    source: PathBuf,
    destination: PathBuf,
    source_fingerprint: PmScratchFileFingerprint,
    staged_fingerprint: Option<PmScratchFileFingerprint>,
}

fn pm_scratch_path_error(error: io::Error, action: &str, path: &Path) -> io::Error {
    io::Error::new(
        error.kind(),
        format!("{action} at {}: {error}", path.display()),
    )
}

#[cfg(windows)]
fn windows_pm_scratch_open_file_identity(
    file: &fs::File,
    path: &Path,
    action: &str,
) -> io::Result<(u32, u64)> {
    use std::os::windows::io::AsRawHandle;

    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x0010;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;

    #[repr(C)]
    #[derive(Default)]
    struct WindowsFileTime {
        low_date_time: u32,
        high_date_time: u32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct WindowsByHandleFileInformation {
        file_attributes: u32,
        creation_time: WindowsFileTime,
        last_access_time: WindowsFileTime,
        last_write_time: WindowsFileTime,
        volume_serial_number: u32,
        file_size_high: u32,
        file_size_low: u32,
        number_of_links: u32,
        file_index_high: u32,
        file_index_low: u32,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFileInformationByHandle(
            file: *mut std::ffi::c_void,
            information: *mut WindowsByHandleFileInformation,
        ) -> i32;
    }

    let mut information = WindowsByHandleFileInformation::default();
    // SAFETY: `file` owns a valid Windows handle for the duration of the call,
    // and `information` is writable storage matching the Win32 structure.
    let succeeded =
        unsafe { GetFileInformationByHandle(file.as_raw_handle().cast(), &mut information) };
    if succeeded == 0 {
        let error = io::Error::last_os_error();
        return Err(io::Error::new(
            error.kind(),
            format!(
                "{action} identity unavailable for {}: {error}",
                path.display()
            ),
        ));
    }
    if information.file_attributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT) != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "{action} identity unavailable because the handle is not a regular non-reparse file: {}",
                path.display()
            ),
        ));
    }
    Ok((
        information.volume_serial_number,
        (u64::from(information.file_index_high) << 32) | u64::from(information.file_index_low),
    ))
}

#[cfg(windows)]
fn windows_pm_scratch_metadata_is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn windows_pm_scratch_metadata_is_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

fn pm_scratch_open_file_fingerprint(
    file: &mut fs::File,
    path: &Path,
    action: &str,
) -> io::Result<PmScratchFileFingerprint> {
    let opened_metadata = file
        .metadata()
        .map_err(|error| pm_scratch_path_error(error, action, path))?;
    if !opened_metadata.file_type().is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{action} opened a non-regular file: {}", path.display()),
        ));
    }
    #[cfg(windows)]
    let opened_windows_identity = windows_pm_scratch_open_file_identity(file, path, action)?;

    file.seek(SeekFrom::Start(0))
        .map_err(|error| pm_scratch_path_error(error, action, path))?;
    let mut hasher = Sha256::new();
    let mut content_len = 0_u64;
    let mut buffer = [0_u8; 8192];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| pm_scratch_path_error(error, action, path))?;
        if read == 0 {
            break;
        }
        content_len = content_len.checked_add(read as u64).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{action} exceeded the supported size: {}", path.display()),
            )
        })?;
        hasher.update(&buffer[..read]);
    }
    let final_metadata = file
        .metadata()
        .map_err(|error| pm_scratch_path_error(error, action, path))?;
    if opened_metadata.len() != content_len || final_metadata.len() != content_len {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{action} changed while being read: {}", path.display()),
        ));
    }

    #[cfg(windows)]
    let (volume_serial_number, file_index) = {
        let final_identity = windows_pm_scratch_open_file_identity(file, path, action)?;
        if final_identity != opened_windows_identity {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "{action} identity changed while being read: {}",
                    path.display()
                ),
            ));
        }
        opened_windows_identity
    };

    Ok(PmScratchFileFingerprint {
        content_len,
        content_sha256: hasher.finalize().into(),
        #[cfg(unix)]
        device: {
            use std::os::unix::fs::MetadataExt;
            opened_metadata.dev()
        },
        #[cfg(unix)]
        inode: {
            use std::os::unix::fs::MetadataExt;
            opened_metadata.ino()
        },
        #[cfg(windows)]
        volume_serial_number,
        #[cfg(windows)]
        file_index,
    })
}

fn pm_scratch_file_fingerprint(path: &Path, action: &str) -> io::Result<PmScratchFileFingerprint> {
    let path_metadata =
        fs::symlink_metadata(path).map_err(|error| pm_scratch_path_error(error, action, path))?;
    if !path_metadata.file_type().is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{action} requires a regular file: {}", path.display()),
        ));
    }
    #[cfg(windows)]
    if windows_pm_scratch_metadata_is_reparse_point(&path_metadata) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "{action} identity unavailable for a reparse-point file: {}",
                path.display()
            ),
        ));
    }

    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;

        const FILE_SHARE_READ: u32 = 0x0000_0001;
        const FILE_SHARE_WRITE: u32 = 0x0000_0002;
        const FILE_SHARE_DELETE: u32 = 0x0000_0004;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let mut file = options
        .open(path)
        .map_err(|error| pm_scratch_path_error(error, action, path))?;
    let fingerprint = pm_scratch_open_file_fingerprint(&mut file, path, action)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let current_metadata = fs::symlink_metadata(path)
            .map_err(|error| pm_scratch_path_error(error, action, path))?;
        if path_metadata.dev() != fingerprint.device
            || path_metadata.ino() != fingerprint.inode
            || !current_metadata.file_type().is_file()
            || current_metadata.dev() != fingerprint.device
            || current_metadata.ino() != fingerprint.inode
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{action} observed a replaced file: {}", path.display()),
            ));
        }
    }

    #[cfg(windows)]
    {
        let current_metadata = fs::symlink_metadata(path)
            .map_err(|error| pm_scratch_path_error(error, action, path))?;
        if !current_metadata.file_type().is_file()
            || windows_pm_scratch_metadata_is_reparse_point(&current_metadata)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{action} observed a replaced file: {}", path.display()),
            ));
        }
        let current_file = options
            .open(path)
            .map_err(|error| pm_scratch_path_error(error, action, path))?;
        let current_identity = windows_pm_scratch_open_file_identity(&current_file, path, action)?;
        let opened_identity = (fingerprint.volume_serial_number, fingerprint.file_index);
        if current_identity != opened_identity {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{action} observed a replaced file: {}", path.display()),
            ));
        }
    }

    Ok(fingerprint)
}

fn optional_real_pm_scratch_directory(path: &Path, role: &str) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata)
            if metadata.file_type().is_dir()
                && !windows_pm_scratch_metadata_is_reparse_point(&metadata) =>
        {
            Ok(true)
        }
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "PM scratch {role} must be a real directory, not a symlink or other node: {}",
                path.display()
            ),
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(pm_scratch_path_error(error, role, path)),
    }
}

fn require_real_pm_scratch_directory(path: &Path, role: &str) -> io::Result<()> {
    if optional_real_pm_scratch_directory(path, role)? {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("PM scratch {role} does not exist: {}", path.display()),
        ))
    }
}

fn ensure_real_pm_scratch_directory(path: &Path, role: &str) -> io::Result<()> {
    if optional_real_pm_scratch_directory(path, role)? {
        return Ok(());
    }
    fs::create_dir(path).map_err(|error| pm_scratch_path_error(error, role, path))?;
    require_real_pm_scratch_directory(path, role)
}

#[derive(Debug)]
struct PmScratchQuarantine {
    directory: PathBuf,
    node: PathBuf,
}

fn create_pm_scratch_quarantine(path: &Path) -> io::Result<PmScratchQuarantine> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "PM scratch quarantine target has no parent: {}",
                path.display()
            ),
        )
    })?;
    for _ in 0..8 {
        let directory = parent.join(format!(
            ".gwt-pm-scratch-quarantine-{}",
            uuid::Uuid::new_v4().simple()
        ));
        #[cfg_attr(not(unix), allow(unused_mut))]
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        match builder.create(&directory) {
            Ok(()) => {
                // The node lives inside a freshly created UUID directory.
                // No rename target existed, so `rename` cannot replace a
                // pre-existing caller node during the quarantine move.
                return Ok(PmScratchQuarantine {
                    node: directory.join("owned-node"),
                    directory,
                });
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(pm_scratch_path_error(
                    error,
                    "create PM scratch quarantine directory",
                    &directory,
                ));
            }
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!(
            "could not allocate a unique PM scratch quarantine beside {}",
            path.display()
        ),
    ))
}

fn remove_empty_pm_scratch_quarantine(quarantine: &PmScratchQuarantine) -> io::Result<()> {
    fs::remove_dir(&quarantine.directory).map_err(|error| {
        pm_scratch_path_error(
            error,
            "remove empty PM scratch quarantine directory",
            &quarantine.directory,
        )
    })?;
    sync_parent_directory(&quarantine.directory).map_err(|error| {
        pm_scratch_path_error(
            error,
            "sync PM scratch quarantine parent",
            &quarantine.directory,
        )
    })
}

fn restore_quarantined_pm_scratch_no_replace_with_sync<O, Q>(
    quarantine: &PmScratchQuarantine,
    original: &Path,
    quarantined_fingerprint: Option<&PmScratchFileFingerprint>,
    original_parent_sync: O,
    quarantine_directory_sync: Q,
) -> io::Result<()>
where
    O: FnOnce(&Path) -> io::Result<()>,
    Q: FnOnce(&Path) -> io::Result<()>,
{
    fs::hard_link(&quarantine.node, original).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "could not create PM scratch original link {} from quarantine {}; inspect the quarantine path: {error}",
                original.display(),
                quarantine.node.display(),
            ),
        )
    })?;
    original_parent_sync(original).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "PM scratch original link was created at {} from quarantine {}; original-parent durability sync failed: {error}",
                original.display(),
                quarantine.node.display()
            ),
        )
    })?;

    let Some(expected) = quarantined_fingerprint else {
        return Ok(());
    };
    let restored =
        pm_scratch_file_fingerprint(original, "verify no-replace PM scratch quarantine restore")?;
    let quarantined = pm_scratch_file_fingerprint(
        &quarantine.node,
        "reverify PM scratch quarantine before cleanup",
    )?;
    if restored != *expected || quarantined != *expected {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "PM scratch quarantine restore identity changed; cleanup stopped, inspect original {} and quarantine {}",
                original.display(),
                quarantine.node.display()
            ),
        ));
    }

    fs::remove_file(&quarantine.node).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "PM scratch original link was created at {}, but verified quarantine link removal failed at {}: {error}",
                original.display(),
                quarantine.node.display(),
            ),
        )
    })?;
    quarantine_directory_sync(&quarantine.node).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "PM scratch original link was created at {}, quarantine link was removed at {}, but quarantine-directory durability sync failed: {error}",
                original.display(),
                quarantine.node.display()
            ),
        )
    })?;
    remove_empty_pm_scratch_quarantine(quarantine)
}

fn restore_quarantined_pm_scratch_no_replace(
    quarantine: &PmScratchQuarantine,
    original: &Path,
    quarantined_fingerprint: Option<&PmScratchFileFingerprint>,
) -> io::Result<()> {
    restore_quarantined_pm_scratch_no_replace_with_sync(
        quarantine,
        original,
        quarantined_fingerprint,
        sync_parent_directory,
        sync_parent_directory,
    )
}

fn quarantine_and_remove_owned_pm_scratch_file_with_restore<R>(
    path: &Path,
    expected: &PmScratchFileFingerprint,
    action: &str,
    mut restore: R,
) -> io::Result<()>
where
    R: FnMut(&PmScratchQuarantine, &Path, Option<&PmScratchFileFingerprint>) -> io::Result<()>,
{
    let quarantine = create_pm_scratch_quarantine(path)?;
    if let Err(error) = fs::rename(path, &quarantine.node) {
        let cleanup_error = remove_empty_pm_scratch_quarantine(&quarantine).err();
        return Err(io::Error::new(
            error.kind(),
            match cleanup_error {
                Some(cleanup_error) => format!(
                    "{action} by atomic quarantine rename from {} to {}: {error}; empty quarantine cleanup also failed: {cleanup_error}",
                    path.display(),
                    quarantine.node.display()
                ),
                None => format!(
                    "{action} by atomic quarantine rename from {} to {}: {error}",
                    path.display(),
                    quarantine.node.display()
                ),
            },
        ));
    }

    let sync_result =
        sync_parent_directory(path).and_then(|()| sync_parent_directory(&quarantine.node));
    if let Err(error) = sync_result {
        let quarantined_fingerprint = pm_scratch_file_fingerprint(
            &quarantine.node,
            "fingerprint PM scratch quarantine after sync failure",
        )
        .ok();
        let restore_error = restore(&quarantine, path, quarantined_fingerprint.as_ref()).err();
        return Err(io::Error::new(
            error.kind(),
            match restore_error {
                Some(restore_error) => format!(
                    "{action} quarantine durability sync failed after moving original {} to quarantine {}; restore attempt reported: {restore_error}; inspect both paths: {error}",
                    path.display(),
                    quarantine.node.display()
                ),
                None if quarantined_fingerprint.is_some() => format!(
                    "{action} quarantine sync failed for {}; original path restored: {error}",
                    path.display()
                ),
                None => format!(
                    "{action} quarantine sync failed for {}; recovery created the original link and did not remove the unverified quarantine node at {}: {error}",
                    path.display(),
                    quarantine.node.display()
                ),
            },
        ));
    }

    let quarantined = match pm_scratch_file_fingerprint(
        &quarantine.node,
        "fingerprint quarantined PM scratch node before removal",
    ) {
        Ok(fingerprint) => fingerprint,
        Err(error) => {
            let restore_error = restore(&quarantine, path, None).err();
            return Err(io::Error::new(
                error.kind(),
                match restore_error {
                    Some(restore_error) => format!(
                        "{action} could not verify quarantine {} for original {}; restore attempt reported: {restore_error}; inspect both paths: {error}",
                        quarantine.node.display(),
                        path.display()
                    ),
                    None => format!(
                        "{action} could not verify quarantine {} for original {}; recovery created the original link and did not remove the quarantine node: {error}",
                        quarantine.node.display(),
                        path.display()
                    ),
                },
            ));
        }
    };
    if quarantined != *expected {
        let restore_error = restore(&quarantine, path, Some(&quarantined)).err();
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            match restore_error {
                Some(restore_error) => format!(
                    "{action} quarantined a replaced node from original {} to {}; restore attempt reported: {restore_error}; inspect both paths",
                    path.display(),
                    quarantine.node.display()
                ),
                None => format!(
                    "{action} refused a replaced node and restored it without replacement at {}",
                    path.display()
                ),
            },
        ));
    }

    fs::remove_file(&quarantine.node).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "{action} verified quarantine link removal failed at {}; inspect the quarantine path: {error}",
                quarantine.node.display(),
            ),
        )
    })?;
    sync_parent_directory(&quarantine.node).map_err(|error| {
        pm_scratch_path_error(
            error,
            &format!("{action} sync quarantine directory"),
            &quarantine.node,
        )
    })?;
    remove_empty_pm_scratch_quarantine(&quarantine)
}

fn quarantine_and_remove_owned_pm_scratch_file(
    path: &Path,
    expected: &PmScratchFileFingerprint,
    action: &str,
) -> io::Result<()> {
    quarantine_and_remove_owned_pm_scratch_file_with_restore(
        path,
        expected,
        action,
        restore_quarantined_pm_scratch_no_replace,
    )
}

fn durably_copy_pm_scratch_no_replace_with_ops<F, C, S, P>(
    source: &Path,
    destination: &Path,
    before_create: F,
    copy: C,
    destination_sync: S,
    parent_sync: P,
) -> io::Result<PmScratchFileFingerprint>
where
    F: FnOnce() -> io::Result<()>,
    C: FnOnce(&mut fs::File, &mut fs::File) -> io::Result<u64>,
    S: FnOnce(&fs::File) -> io::Result<()>,
    P: FnOnce(&Path) -> io::Result<()>,
{
    let mut source_file = fs::File::open(source)
        .map_err(|error| pm_scratch_path_error(error, "open PM scratch source", source))?;
    let source_metadata = source_file.metadata().map_err(|error| {
        pm_scratch_path_error(error, "inspect opened PM scratch source", source)
    })?;
    if !source_metadata.file_type().is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "opened PM scratch source is not a regular file: {}",
                source.display()
            ),
        ));
    }

    before_create().map_err(|error| {
        pm_scratch_path_error(error, "run PM scratch before-create guard", destination)
    })?;
    let mut destination_file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "create PM scratch destination without replacement from {} to {}: {error}",
                    source.display(),
                    destination.display()
                ),
            )
        })?;

    let result = copy(&mut source_file, &mut destination_file)
        .map(|_| ())
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "copy PM scratch source {} to {}: {error}",
                    source.display(),
                    destination.display()
                ),
            )
        })
        .and_then(|()| {
            destination_sync(&destination_file).map_err(|error| {
                pm_scratch_path_error(error, "sync PM scratch destination", destination)
            })
        })
        .and_then(|()| {
            parent_sync(destination).map_err(|error| {
                pm_scratch_path_error(error, "sync PM scratch destination parent", destination)
            })
        });

    match result {
        Ok(()) => {
            let created_fingerprint = pm_scratch_open_file_fingerprint(
                &mut destination_file,
                destination,
                "fingerprint created PM scratch destination before success",
            )
            .map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!(
                        "cannot verify created PM scratch destination {} before success; no cleanup was attempted and the canonical path requires inspection: {error}",
                        destination.display()
                    ),
                )
            })?;
            drop(destination_file);
            let canonical_fingerprint = pm_scratch_file_fingerprint(
                destination,
                "fingerprint canonical PM scratch destination before success",
            )
            .map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!(
                        "cannot verify canonical PM scratch destination {} before success; no cleanup was attempted at the canonical path: {error}",
                        destination.display()
                    ),
                )
            })?;
            if canonical_fingerprint != created_fingerprint {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "canonical PM scratch destination identity or content changed before success; unknown node is preserved at {}",
                        destination.display()
                    ),
                ));
            }
            Ok(created_fingerprint)
        }
        Err(error) => {
            let owned_fingerprint = pm_scratch_open_file_fingerprint(
                &mut destination_file,
                destination,
                "fingerprint created PM scratch destination after durable copy operation failure",
            );
            drop(destination_file);
            let expected = match owned_fingerprint {
                Ok(fingerprint) => fingerprint,
                Err(fingerprint_error) => {
                    return Err(io::Error::new(
                        error.kind(),
                        format!(
                            "{error}; cannot prove ownership for PM scratch destination {}, so no cleanup was attempted and the path requires inspection: {fingerprint_error}",
                            destination.display()
                        ),
                    ));
                }
            };
            match quarantine_and_remove_owned_pm_scratch_file(
                destination,
                &expected,
                "rollback created PM scratch destination after durable copy operation failure",
            ) {
                Ok(()) => Err(error),
                Err(rollback_error) => Err(io::Error::new(
                    error.kind(),
                    format!(
                        "{error}; guarded PM scratch destination rollback also reported: {rollback_error}"
                    ),
                )),
            }
        }
    }
}

fn durably_copy_pm_scratch_no_replace_with<F>(
    source: &Path,
    destination: &Path,
    before_create: F,
) -> io::Result<PmScratchFileFingerprint>
where
    F: FnOnce() -> io::Result<()>,
{
    durably_copy_pm_scratch_no_replace_with_ops(
        source,
        destination,
        before_create,
        io::copy,
        fs::File::sync_all,
        sync_parent_directory,
    )
}

fn durably_copy_pm_scratch_no_replace(
    source: &Path,
    destination: &Path,
) -> io::Result<PmScratchFileFingerprint> {
    durably_copy_pm_scratch_no_replace_with(source, destination, || Ok(()))
}

fn rollback_owned_pm_scratch_file(staged: &StagedPmScratchFile) -> io::Result<()> {
    match fs::symlink_metadata(&staged.path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(pm_scratch_path_error(
                error,
                "inspect staged PM scratch destination for guarded rollback",
                &staged.path,
            ));
        }
        Ok(_) => {}
    }
    quarantine_and_remove_owned_pm_scratch_file(
        &staged.path,
        &staged.fingerprint,
        "rollback owned PM scratch destination",
    )
}

fn rollback_staged_pm_scratch(destinations: &[StagedPmScratchFile]) -> io::Result<()> {
    let mut failures = Vec::new();
    for destination in destinations.iter().rev() {
        if let Err(error) = rollback_owned_pm_scratch_file(destination) {
            failures.push(error.to_string());
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(io::Error::other(failures.join("; ")))
    }
}

fn pm_scratch_stage_error(
    error: io::Error,
    staged_destinations: &[StagedPmScratchFile],
) -> io::Error {
    match rollback_staged_pm_scratch(staged_destinations) {
        Ok(()) => error,
        Err(rollback_error) => io::Error::new(
            error.kind(),
            format!("{error}; PM scratch stage rollback also failed: {rollback_error}"),
        ),
    }
}

fn verify_staged_pm_scratch_destinations(migrations: &[PmScratchMigration]) -> io::Result<()> {
    for migration in migrations {
        let expected = migration.staged_fingerprint.as_ref().ok_or_else(|| {
            io::Error::other(format!(
                "missing staged PM scratch fingerprint for {}: {}",
                migration.relative,
                migration.destination.display()
            ))
        })?;
        let current = pm_scratch_file_fingerprint(
            &migration.destination,
            &format!(
                "verify staged PM scratch destination ownership for {}",
                migration.relative
            ),
        )?;
        if current != *expected {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "staged PM scratch destination was replaced for {}: {}",
                    migration.relative,
                    migration.destination.display()
                ),
            ));
        }
    }
    Ok(())
}

fn pm_scratch_error_preserving_backups(
    error: io::Error,
    migrations: &[PmScratchMigration],
    preserve_indices: &[usize],
) -> io::Error {
    let mut details = Vec::new();
    let mut rollback_failures = Vec::new();

    for (index, migration) in migrations.iter().enumerate() {
        let Some(expected) = migration.staged_fingerprint.as_ref() else {
            continue;
        };
        if preserve_indices.contains(&index) {
            match pm_scratch_file_fingerprint(
                &migration.destination,
                &format!(
                    "verify preserved PM scratch backup for {}",
                    migration.relative
                ),
            ) {
                Ok(current) if current == *expected => details.push(format!(
                    "original legacy PM scratch source backup preserved at {}",
                    migration.destination.display()
                )),
                Ok(_) => details.push(format!(
                    "preserved PM scratch backup ownership changed for {}; unknown node remains fail-closed at {}",
                    migration.relative,
                    migration.destination.display()
                )),
                Err(inspection_error) => details.push(format!(
                    "could not verify preserved PM scratch backup for {}; node remains fail-closed at {}: {inspection_error}",
                    migration.relative,
                    migration.destination.display()
                )),
            }
            continue;
        }

        if let Err(rollback_error) = rollback_owned_pm_scratch_file(&StagedPmScratchFile {
            path: migration.destination.clone(),
            fingerprint: expected.clone(),
        }) {
            rollback_failures.push(rollback_error.to_string());
        }
    }

    if details.is_empty() && rollback_failures.is_empty() {
        return error;
    }
    let mut message = error.to_string();
    if !details.is_empty() {
        message.push_str("; ");
        message.push_str(&details.join("; "));
    }
    if !rollback_failures.is_empty() {
        message.push_str("; selective PM scratch destination rollback also failed: ");
        message.push_str(&rollback_failures.join("; "));
    }
    io::Error::new(error.kind(), message)
}

fn restore_removed_pm_scratch_sources<C>(
    migrations: &[PmScratchMigration],
    committed_removals: usize,
    recovery_copy: &mut C,
) -> (Vec<usize>, Vec<String>)
where
    C: FnMut(&Path, &Path) -> io::Result<PmScratchFileFingerprint>,
{
    let mut preserve_backups = Vec::new();
    let mut failures = Vec::new();
    for (index, migration) in migrations.iter().take(committed_removals).enumerate() {
        match fs::symlink_metadata(&migration.source) {
            Ok(_) => match pm_scratch_file_fingerprint(
                &migration.source,
                &format!(
                    "fingerprint legacy PM scratch source during recovery for {}",
                    migration.relative
                ),
            ) {
                Ok(current) if current == migration.source_fingerprint => continue,
                Ok(_) => {
                    preserve_backups.push(index);
                    failures.push(format!(
                        "refusing to overwrite a replaced legacy PM scratch source during recovery for {}: {}",
                        migration.relative,
                        migration.source.display()
                    ));
                    continue;
                }
                Err(error) => {
                    preserve_backups.push(index);
                    failures.push(error.to_string());
                    continue;
                }
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                preserve_backups.push(index);
                failures.push(
                    pm_scratch_path_error(
                        error,
                        &format!(
                            "inspect legacy PM scratch source during recovery for {}",
                            migration.relative
                        ),
                        &migration.source,
                    )
                    .to_string(),
                );
                continue;
            }
        }

        let Some(expected_destination) = migration.staged_fingerprint.as_ref() else {
            preserve_backups.push(index);
            failures.push(format!(
                "cannot restore legacy PM scratch source without a staged fingerprint for {}: {}",
                migration.relative,
                migration.source.display()
            ));
            continue;
        };
        let current_destination = match pm_scratch_file_fingerprint(
            &migration.destination,
            &format!(
                "fingerprint PM scratch recovery source for {}",
                migration.relative
            ),
        ) {
            Ok(fingerprint) => fingerprint,
            Err(error) => {
                preserve_backups.push(index);
                failures.push(error.to_string());
                continue;
            }
        };
        if current_destination != *expected_destination {
            preserve_backups.push(index);
            failures.push(format!(
                "refusing to restore from a replaced PM scratch destination for {}: {}",
                migration.relative,
                migration.destination.display()
            ));
            continue;
        }

        let recovery_token = match recovery_copy(&migration.destination, &migration.source) {
            Ok(fingerprint) => fingerprint,
            Err(error) => {
                preserve_backups.push(index);
                failures.push(
                    io::Error::new(
                        error.kind(),
                        format!(
                            "restore legacy PM scratch source {} from {} to {}: {error}",
                            migration.relative,
                            migration.destination.display(),
                            migration.source.display()
                        ),
                    )
                    .to_string(),
                );
                continue;
            }
        };

        match pm_scratch_file_fingerprint(
            &migration.source,
            &format!(
                "verify restored legacy PM scratch source for {}",
                migration.relative
            ),
        ) {
            Ok(restored)
                if restored == recovery_token
                    && restored.has_same_content(&migration.source_fingerprint)
                    && restored.has_same_content(expected_destination) => {}
            Ok(_) => {
                preserve_backups.push(index);
                failures.push(format!(
                    "restored legacy PM scratch source identity or content was replaced during recovery ownership handoff for {}: {}",
                    migration.relative,
                    migration.source.display()
                ));
            }
            Err(error) => {
                preserve_backups.push(index);
                failures.push(error.to_string());
            }
        }
    }

    preserve_backups.sort_unstable();
    preserve_backups.dedup();
    (preserve_backups, failures)
}

fn pm_scratch_remove_error<C>(
    error: io::Error,
    migrations: &[PmScratchMigration],
    committed_removals: usize,
    current_candidate: Option<usize>,
    recovery_copy: &mut C,
) -> io::Error
where
    C: FnMut(&Path, &Path) -> io::Result<PmScratchFileFingerprint>,
{
    let (mut preserve_backups, recovery_failures) =
        restore_removed_pm_scratch_sources(migrations, committed_removals, recovery_copy);
    let mut recovery_details = recovery_failures;

    if let Some(index) = current_candidate {
        let migration = &migrations[index];
        let current_is_original = match pm_scratch_file_fingerprint(
            &migration.source,
            &format!(
                "reinspect current legacy PM scratch source after removal failure for {}",
                migration.relative
            ),
        ) {
            Ok(current) => {
                let matches_staged = migration
                    .staged_fingerprint
                    .as_ref()
                    .is_some_and(|staged| current.has_same_content(staged));
                current == migration.source_fingerprint && matches_staged
            }
            Err(inspection_error) => {
                recovery_details.push(inspection_error.to_string());
                false
            }
        };
        if !current_is_original {
            preserve_backups.push(index);
        }
    }

    preserve_backups.sort_unstable();
    preserve_backups.dedup();
    let error = if recovery_details.is_empty() {
        error
    } else {
        io::Error::new(
            error.kind(),
            format!(
                "{error}; PM scratch source recovery also reported: {}",
                recovery_details.join("; ")
            ),
        )
    };
    pm_scratch_error_preserving_backups(error, migrations, &preserve_backups)
}

fn migrate_legacy_pm_scratch_with_all_ops<F, R, C>(
    worktree: &Path,
    mut stage: F,
    mut before_remove: R,
    mut recovery_copy: C,
) -> io::Result<usize>
where
    F: FnMut(&Path, &Path) -> io::Result<PmScratchFileFingerprint>,
    R: FnMut(&Path) -> io::Result<()>,
    C: FnMut(&Path, &Path) -> io::Result<PmScratchFileFingerprint>,
{
    let scratch = pm_scratch_dir_for_pm_worktree(worktree).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "PM scratch migration requires a canonical PM worktree: {}",
                worktree.display()
            ),
        )
    })?;

    if !optional_real_pm_scratch_directory(worktree, "worktree")? {
        return Ok(0);
    }
    let pm_dir = worktree.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("PM worktree has no parent: {}", worktree.display()),
        )
    })?;
    let project_dir = pm_dir.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "PM worktree has no project directory: {}",
                worktree.display()
            ),
        )
    })?;
    require_real_pm_scratch_directory(project_dir, "project directory")?;
    require_real_pm_scratch_directory(pm_dir, "PM directory")?;

    let mut migrations = Vec::new();

    for relative in LEGACY_PM_SCRATCH_PATHS {
        let source = worktree.join(relative);
        let source_parent = source.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "legacy PM scratch source has no parent for {relative}: {}",
                    source.display()
                ),
            )
        })?;
        if source_parent != worktree
            && !optional_real_pm_scratch_directory(source_parent, "source parent")?
        {
            continue;
        }
        let source_metadata = match fs::symlink_metadata(&source) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(io::Error::new(
                    error.kind(),
                    format!(
                        "inspect legacy PM scratch source {relative} at {}: {error}",
                        source.display()
                    ),
                ));
            }
        };
        if !source_metadata.file_type().is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "legacy PM scratch source {relative} at {} is not a regular file",
                    source.display()
                ),
            ));
        }
        if !legacy_pm_scratch_has_local_state(worktree, relative)? {
            continue;
        }
        let destination = scratch.join(relative);
        let source_fingerprint = pm_scratch_file_fingerprint(
            &source,
            &format!("fingerprint legacy PM scratch source {relative}"),
        )?;
        migrations.push(PmScratchMigration {
            relative,
            source,
            destination,
            source_fingerprint,
            staged_fingerprint: None,
        });
    }

    if migrations.is_empty() {
        return Ok(0);
    }

    let project_state = project_dir.join("project-state");
    ensure_real_pm_scratch_directory(&project_state, "project-state directory")?;
    ensure_real_pm_scratch_directory(&scratch, "scratch root")?;
    if migrations
        .iter()
        .any(|migration| migration.relative.starts_with("tasks/"))
    {
        ensure_real_pm_scratch_directory(&scratch.join("tasks"), "scratch tasks directory")?;
    }

    for migration in &migrations {
        match fs::symlink_metadata(&migration.destination) {
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!(
                        "legacy PM scratch destination conflict for {}: {}",
                        migration.relative,
                        migration.destination.display()
                    ),
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(io::Error::new(
                    error.kind(),
                    format!(
                        "inspect legacy PM scratch destination {} at {}: {error}",
                        migration.relative,
                        migration.destination.display()
                    ),
                ));
            }
        }
    }

    let mut staged_destinations = Vec::new();
    for migration in &mut migrations {
        let fingerprint = match stage(&migration.source, &migration.destination) {
            Ok(fingerprint) => fingerprint,
            Err(error) => {
                let error = io::Error::new(
                    error.kind(),
                    format!(
                        "stage legacy PM scratch {} from {} to {}: {error}",
                        migration.relative,
                        migration.source.display(),
                        migration.destination.display()
                    ),
                );
                return Err(pm_scratch_stage_error(error, &staged_destinations));
            }
        };
        let canonical_fingerprint = match pm_scratch_file_fingerprint(
            &migration.destination,
            &format!(
                "verify staged PM scratch destination ownership handoff for {}",
                migration.relative
            ),
        ) {
            Ok(fingerprint) => fingerprint,
            Err(error) => return Err(pm_scratch_stage_error(error, &staged_destinations)),
        };
        if canonical_fingerprint != fingerprint {
            let error = io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "staged PM scratch destination identity or content was replaced during ownership handoff for {}: {}",
                    migration.relative,
                    migration.destination.display()
                ),
            );
            return Err(pm_scratch_stage_error(error, &staged_destinations));
        }
        migration.staged_fingerprint = Some(fingerprint.clone());
        staged_destinations.push(StagedPmScratchFile {
            path: migration.destination.clone(),
            fingerprint,
        });
    }

    if let Err(error) = verify_staged_pm_scratch_destinations(&migrations) {
        return Err(pm_scratch_stage_error(error, &staged_destinations));
    }
    for (index, migration) in migrations.iter().enumerate() {
        let current_source = match pm_scratch_file_fingerprint(
            &migration.source,
            &format!(
                "reinspect legacy PM scratch source {} before commit",
                migration.relative
            ),
        ) {
            Ok(fingerprint) => fingerprint,
            Err(error) => {
                return Err(pm_scratch_error_preserving_backups(
                    error,
                    &migrations,
                    &[index],
                ));
            }
        };
        let staged_fingerprint = migration
            .staged_fingerprint
            .as_ref()
            .expect("every migration is fingerprinted after successful staging");
        if current_source != migration.source_fingerprint
            || !current_source.has_same_content(staged_fingerprint)
        {
            let error = io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "refusing to remove a replaced or changed legacy PM scratch source {}: {}",
                    migration.relative,
                    migration.source.display()
                ),
            );
            return Err(pm_scratch_error_preserving_backups(
                error,
                &migrations,
                &[index],
            ));
        }
    }

    for (committed_removals, migration) in migrations.iter().enumerate() {
        if let Err(error) = verify_staged_pm_scratch_destinations(&migrations) {
            return Err(pm_scratch_remove_error(
                error,
                &migrations,
                committed_removals,
                Some(committed_removals),
                &mut recovery_copy,
            ));
        }
        let current_source = match pm_scratch_file_fingerprint(
            &migration.source,
            &format!(
                "fingerprint legacy PM scratch source {} immediately before removal",
                migration.relative
            ),
        ) {
            Ok(fingerprint) => fingerprint,
            Err(error) => {
                return Err(pm_scratch_remove_error(
                    error,
                    &migrations,
                    committed_removals,
                    Some(committed_removals),
                    &mut recovery_copy,
                ));
            }
        };
        let staged_fingerprint = migration
            .staged_fingerprint
            .as_ref()
            .expect("every migration is fingerprinted after successful staging");
        if current_source != migration.source_fingerprint
            || !current_source.has_same_content(staged_fingerprint)
        {
            let error = io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "legacy PM scratch source changed before removal for {}: {}",
                    migration.relative,
                    migration.source.display()
                ),
            );
            return Err(pm_scratch_remove_error(
                error,
                &migrations,
                committed_removals,
                Some(committed_removals),
                &mut recovery_copy,
            ));
        }

        if let Err(error) = before_remove(&migration.source) {
            let error = pm_scratch_path_error(
                error,
                &format!(
                    "run before-remove hook for migrated legacy PM scratch source {}",
                    migration.relative
                ),
                &migration.source,
            );
            return Err(pm_scratch_remove_error(
                error,
                &migrations,
                committed_removals,
                Some(committed_removals),
                &mut recovery_copy,
            ));
        }
        if let Err(error) = quarantine_and_remove_owned_pm_scratch_file(
            &migration.source,
            &migration.source_fingerprint,
            &format!(
                "commit migrated legacy PM scratch source {}",
                migration.relative
            ),
        ) {
            return Err(pm_scratch_remove_error(
                error,
                &migrations,
                committed_removals,
                Some(committed_removals),
                &mut recovery_copy,
            ));
        }
    }

    Ok(migrations.len())
}

fn migrate_legacy_pm_scratch_with_ops<F, R>(
    worktree: &Path,
    stage: F,
    before_remove: R,
) -> io::Result<usize>
where
    F: FnMut(&Path, &Path) -> io::Result<PmScratchFileFingerprint>,
    R: FnMut(&Path) -> io::Result<()>,
{
    migrate_legacy_pm_scratch_with_all_ops(
        worktree,
        stage,
        before_remove,
        durably_copy_pm_scratch_no_replace,
    )
}

fn migrate_legacy_pm_scratch_with<F>(worktree: &Path, stage: F) -> io::Result<usize>
where
    F: FnMut(&Path, &Path) -> io::Result<PmScratchFileFingerprint>,
{
    migrate_legacy_pm_scratch_with_ops(worktree, stage, |_| Ok(()))
}

/// Move the exact legacy PM note allowlist out of the worktree without
/// following symlinked source or destination directories.
pub fn migrate_legacy_pm_scratch(worktree: &Path) -> io::Result<usize> {
    migrate_legacy_pm_scratch_with(worktree, durably_copy_pm_scratch_no_replace)
}

/// Externalize locally changed tracked legacy notes, then restore the
/// repository-owned bytes from HEAD so a subsequent refresh or cleanup does
/// not mistake the migration's tracked deletion for unknown local work.
pub fn migrate_legacy_pm_scratch_preserving_project_content(worktree: &Path) -> io::Result<usize> {
    let tracked = tracked_legacy_pm_scratch_with_local_state(worktree)?;
    reject_tracked_pm_scratch_with_index_changes(worktree, &tracked)?;
    let migrated = migrate_legacy_pm_scratch(worktree)?;
    restore_tracked_pm_scratch_after_migration(worktree, &tracked)?;
    Ok(migrated)
}

pub const PM_WORKTREE_BASE_REF: &str = "origin/develop";

/// Result of one serialized PM worktree refresh attempt. A degraded outcome
/// remains launchable only when a usable worktree was materialized or
/// preserved; `freshness` explains why it could not advance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmWorktreeRefreshOutcome {
    pub worktree: PathBuf,
    pub freshness: PmWorktreeFreshness,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmWorktreeCleanupOutcome {
    Absent,
    Removed,
    RetainedLocalWork,
}

const PM_WORKTREE_IDENTITY_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PmWorktreeIdentity {
    schema_version: u32,
    project_root: PathBuf,
    git_root: PathBuf,
    worktree: PathBuf,
}

impl PmWorktreeRefreshOutcome {
    pub fn is_fresh(&self) -> bool {
        self.freshness.state == PmWorktreeFreshnessState::Fresh
    }
}

fn pm_refresh_checked_at() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn git_sha(repo: &Path, revision: &str) -> io::Result<Option<String>> {
    let output = gwt_core::process::run_git_logged(
        &["rev-parse", "--verify", &format!("{revision}^{{commit}}")],
        Some(repo),
    )
    .map_err(|error| {
        io::Error::other(format!(
            "resolve Git revision {revision:?} in {}: {error}",
            repo.display()
        ))
    })?;
    if !output.status.success() {
        return Ok(None);
    }
    let sha = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    Ok((!sha.is_empty()).then_some(sha))
}

fn detached_worktree_head_sha(worktree: &Path) -> io::Result<Option<String>> {
    let dot_git = worktree.join(".git");
    let metadata = match fs::symlink_metadata(&dot_git) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let git_dir = if metadata.file_type().is_dir() {
        dot_git
    } else if metadata.file_type().is_file() {
        let marker = fs::read_to_string(&dot_git)?;
        let raw = marker
            .trim()
            .strip_prefix("gitdir:")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "invalid linked-worktree gitdir marker: {}",
                        dot_git.display()
                    ),
                )
            })?;
        let path = PathBuf::from(raw);
        if path.is_absolute() {
            path
        } else {
            worktree.join(path)
        }
    } else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "PM worktree .git is neither a file nor directory: {}",
                dot_git.display()
            ),
        ));
    };
    let head = fs::read_to_string(git_dir.join("HEAD"))?;
    let head = head.trim();
    if head.starts_with("ref:") {
        return git_sha(worktree, "HEAD");
    }
    if matches!(head.len(), 40 | 64) && head.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Ok(Some(head.to_ascii_lowercase()));
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        format!("invalid detached PM worktree HEAD at {}", git_dir.display()),
    ))
}

fn fetch_pm_worktree_base(git_root: &Path) -> io::Result<()> {
    let output = gwt_core::process::run_git_logged(
        &[
            "fetch",
            "origin",
            "--prune",
            "+refs/heads/develop:refs/remotes/origin/develop",
        ],
        Some(git_root),
    )?;
    if output.status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "fetch PM base {PM_WORKTREE_BASE_REF}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

fn git_behind(repo: &Path, head: Option<&str>, target: Option<&str>) -> Option<u64> {
    let (Some(head), Some(target)) = (head, target) else {
        return None;
    };
    let range = format!("{head}..{target}");
    let output =
        gwt_core::process::run_git_logged(&["rev-list", "--count", &range], Some(repo)).ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout).trim().parse().ok()
}

fn pm_refresh_failure(
    git_root: &Path,
    worktree: &Path,
    target_sha: Option<String>,
    target_observation: PmWorktreeTargetObservation,
    stage: PmWorktreeRefreshFailureStage,
    reason: impl Into<String>,
) -> PmWorktreeFreshness {
    let head_sha = detached_worktree_head_sha(worktree).ok().flatten();
    let behind = git_behind(git_root, head_sha.as_deref(), target_sha.as_deref());
    let state = match (&head_sha, &target_sha) {
        (Some(head), Some(target)) if head != target => PmWorktreeFreshnessState::Stale,
        _ => PmWorktreeFreshnessState::Unknown,
    };
    PmWorktreeFreshness {
        state,
        base_ref: PM_WORKTREE_BASE_REF.to_string(),
        head_sha,
        target_sha,
        behind,
        target_observation,
        checked_at: pm_refresh_checked_at(),
        failure_stage: Some(stage),
        failure_reason: Some(reason.into()),
    }
}

fn persist_pm_worktree_freshness(
    project_dir: &Path,
    freshness: &PmWorktreeFreshness,
) -> io::Result<()> {
    let prefs_path = project_dir.join("project-state/pm.json");
    mutate_pm_prefs(&prefs_path, |prefs| {
        prefs.worktree_freshness = Some(freshness.clone());
    })?;
    Ok(())
}

fn persist_unknown_pm_worktree_failure(
    project_dir: &Path,
    worktree: &Path,
    stage: PmWorktreeRefreshFailureStage,
    reason: impl Into<String>,
) -> io::Result<PmWorktreeFreshness> {
    let freshness = PmWorktreeFreshness {
        state: PmWorktreeFreshnessState::Unknown,
        base_ref: PM_WORKTREE_BASE_REF.to_string(),
        head_sha: detached_worktree_head_sha(worktree).ok().flatten(),
        target_sha: None,
        behind: None,
        target_observation: PmWorktreeTargetObservation::Unavailable,
        checked_at: pm_refresh_checked_at(),
        failure_stage: Some(stage),
        failure_reason: Some(reason.into()),
    };
    persist_pm_worktree_freshness(project_dir, &freshness)?;
    Ok(freshness)
}

fn persist_untrusted_pm_worktree_failure(
    project_dir: &Path,
    stage: PmWorktreeRefreshFailureStage,
    reason: impl Into<String>,
) -> io::Result<PmWorktreeFreshness> {
    let freshness = PmWorktreeFreshness {
        state: PmWorktreeFreshnessState::Unknown,
        base_ref: PM_WORKTREE_BASE_REF.to_string(),
        head_sha: None,
        target_sha: None,
        behind: None,
        target_observation: PmWorktreeTargetObservation::Unavailable,
        checked_at: pm_refresh_checked_at(),
        failure_stage: Some(stage),
        failure_reason: Some(reason.into()),
    };
    persist_pm_worktree_freshness(project_dir, &freshness)?;
    Ok(freshness)
}

fn append_pm_worktree_refresh_failure_reason(
    project_dir: &Path,
    additional_reason: &str,
) -> io::Result<()> {
    let prefs_path = project_dir.join("project-state/pm.json");
    mutate_pm_prefs(&prefs_path, |prefs| {
        let freshness = prefs
            .worktree_freshness
            .get_or_insert_with(|| PmWorktreeFreshness {
                state: PmWorktreeFreshnessState::Unknown,
                base_ref: PM_WORKTREE_BASE_REF.to_string(),
                head_sha: None,
                target_sha: None,
                behind: None,
                target_observation: PmWorktreeTargetObservation::Unavailable,
                checked_at: pm_refresh_checked_at(),
                failure_stage: Some(PmWorktreeRefreshFailureStage::Inspect),
                failure_reason: None,
            });
        if freshness.failure_stage.is_none() {
            freshness.state = PmWorktreeFreshnessState::Unknown;
            freshness.head_sha = None;
            freshness.target_sha = None;
            freshness.behind = None;
            freshness.target_observation = PmWorktreeTargetObservation::Unavailable;
            freshness.failure_stage = Some(PmWorktreeRefreshFailureStage::Inspect);
        }
        freshness.failure_reason = Some(match freshness.failure_reason.take() {
            Some(existing) if !existing.is_empty() => {
                format!("{existing}; {additional_reason}")
            }
            _ => additional_reason.to_string(),
        });
    })?;
    Ok(())
}

fn pm_identity_authorizes_project_state(project_dir: &Path, worktree: &Path) -> bool {
    load_pm_worktree_identity(project_dir)
        .ok()
        .flatten()
        .is_some_and(|identity| {
            let identity_project_dir =
                gwt_core::paths::gwt_project_dir_for_repo_path(&identity.project_root);
            same_canonical_path(project_dir, &identity_project_dir)
                && same_canonical_path(worktree, &identity.worktree)
        })
}

fn git_sha_observation(repo: &Path, target: &str) -> (Option<String>, Option<String>) {
    match git_sha(repo, target) {
        Ok(value) => (value, None),
        Err(error) => (None, Some(error.to_string())),
    }
}

fn is_nested_bare_layout_root(path: &Path) -> bool {
    if fs::symlink_metadata(path.join(".git")).is_ok() {
        return false;
    }
    gwt_core::repo_hash::child_bare_repositories(path).len() == 1
}

fn normalize_previous_generated_hook_configs(worktree: &Path) -> io::Result<()> {
    normalize_previous_generated_hook_configs_guarded(worktree, None)
}

fn normalize_previous_generated_hook_configs_guarded(
    worktree: &Path,
    mut snapshot: Option<&mut PmGeneratedHookConfigSnapshot>,
) -> io::Result<()> {
    for relative in [".claude/settings.local.json", ".codex/hooks.json"] {
        let path = worktree.join(relative);
        let guarded = snapshot
            .as_deref()
            .is_some_and(|snapshot| snapshot.contains(&path));
        if guarded {
            snapshot
                .as_deref()
                .expect("guarded snapshot")
                .verify_captured_ownership(&path)?;
        } else if !crate::managed_assets::managed_hook_config_is_disposable(worktree, relative) {
            continue;
        }
        let cached = gwt_core::process::run_git_logged(
            &["diff", "--cached", "--quiet", "--exit-code", "--", relative],
            Some(worktree),
        )?;
        match cached.status.code() {
            Some(0) => {}
            Some(1) => {
                if guarded {
                    snapshot
                        .as_deref_mut()
                        .expect("guarded snapshot")
                        .record_normalized_state(&path)?;
                }
                continue;
            }
            _ => {
                return Err(io::Error::other(format!(
                    "inspect staged generated hook config {relative}: {}",
                    String::from_utf8_lossy(&cached.stderr).trim()
                )));
            }
        }
        let tracked = gwt_core::process::run_git_logged(
            &["ls-files", "--error-unmatch", "--", relative],
            Some(worktree),
        )?;
        match tracked.status.code() {
            Some(0) => {
                let restore = gwt_core::process::run_git_logged(
                    &["checkout", "--", relative],
                    Some(worktree),
                )?;
                if !restore.status.success() {
                    return Err(io::Error::other(format!(
                        "restore prior generated hook config {relative}: {}",
                        String::from_utf8_lossy(&restore.stderr).trim()
                    )));
                }
            }
            Some(1) => fs::remove_file(&path)?,
            _ => {
                return Err(io::Error::other(format!(
                    "inspect tracked generated hook config {relative}: {}",
                    String::from_utf8_lossy(&tracked.stderr).trim()
                )));
            }
        }
        if guarded {
            snapshot
                .as_deref_mut()
                .expect("guarded snapshot")
                .record_normalized_state(&path)?;
        }
    }
    Ok(())
}

struct PmGeneratedHookConfigSnapshot {
    entries: Vec<PmGeneratedHookConfigSnapshotEntry>,
}

struct PmGeneratedHookConfigSnapshotEntry {
    path: PathBuf,
    contents: Vec<u8>,
    permissions: fs::Permissions,
    normalized_state: Option<PmGeneratedHookNodeState>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum PmGeneratedHookNodeState {
    Absent,
    Regular(Vec<u8>),
}

impl PmGeneratedHookConfigSnapshot {
    fn capture(worktree: &Path) -> io::Result<Self> {
        let mut entries = Vec::new();
        for relative in [".claude/settings.local.json", ".codex/hooks.json"] {
            if !crate::managed_assets::managed_hook_config_is_disposable(worktree, relative) {
                continue;
            }
            let path = worktree.join(relative);
            require_regular_pm_worktree_control_file(&path, "generated PM hook config")?;
            let metadata = fs::symlink_metadata(&path)?;
            entries.push(PmGeneratedHookConfigSnapshotEntry {
                contents: fs::read(&path)?,
                permissions: metadata.permissions(),
                path,
                normalized_state: None,
            });
        }
        Ok(Self { entries })
    }

    fn contains(&self, path: &Path) -> bool {
        self.entries.iter().any(|entry| entry.path == path)
    }

    fn verify_captured_ownership(&self, path: &Path) -> io::Result<()> {
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.path == path)
            .ok_or_else(|| io::Error::other("generated PM hook snapshot entry is missing"))?;
        let current = pm_generated_hook_node_state(path)?;
        if current == PmGeneratedHookNodeState::Regular(entry.contents.clone()) {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "generated PM hook config ownership changed after capture: {}",
                    path.display()
                ),
            ))
        }
    }

    fn record_normalized_state(&mut self, path: &Path) -> io::Result<()> {
        let state = pm_generated_hook_node_state(path)?;
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry.path == path)
            .ok_or_else(|| io::Error::other("generated PM hook snapshot entry is missing"))?;
        entry.normalized_state = Some(state);
        Ok(())
    }

    fn restore(self) -> io::Result<()> {
        let mut failures = Vec::new();
        for entry in self.entries {
            let scratch = unique_pm_scratch_path(&entry.path);
            let prepare = (|| {
                let mut file = fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&scratch)?;
                file.write_all(&entry.contents)?;
                file.sync_all()?;
                fs::set_permissions(&scratch, entry.permissions)
            })();
            if let Err(error) = prepare {
                failures.push(format!(
                    "prepare {}: {error}; partial recovery retained at {}",
                    entry.path.display(),
                    scratch.display()
                ));
                continue;
            }

            let current = match pm_generated_hook_node_state(&entry.path) {
                Ok(current) => current,
                Err(error) => {
                    failures.push(format!(
                        "inspect {}: {error}; recovery retained at {}",
                        entry.path.display(),
                        scratch.display()
                    ));
                    continue;
                }
            };
            let captured = PmGeneratedHookNodeState::Regular(entry.contents.clone());
            let Some(expected) = entry.normalized_state else {
                if current == captured {
                    let _ = fs::remove_file(&scratch);
                    continue;
                }
                failures.push(format!(
                    "refuse to restore {} after ownership changed before normalization; recovery retained at {}",
                    entry.path.display(),
                    scratch.display()
                ));
                continue;
            };
            if current != expected {
                failures.push(format!(
                    "refuse to restore {} after its normalized state changed; recovery retained at {}",
                    entry.path.display(),
                    scratch.display()
                ));
                continue;
            }
            if current == captured {
                let _ = fs::remove_file(&scratch);
                continue;
            }
            let quarantine = matches!(current, PmGeneratedHookNodeState::Regular(_))
                .then(|| unique_pm_scratch_path(&entry.path));
            if let Some(quarantine) = quarantine.as_ref() {
                if let Err(error) = fs::rename(&entry.path, quarantine) {
                    failures.push(format!(
                        "quarantine {}: {error}; recovery retained at {}",
                        entry.path.display(),
                        scratch.display()
                    ));
                    continue;
                }
            }
            if let Err(error) = fs::rename(&scratch, &entry.path) {
                let put_back = quarantine
                    .as_ref()
                    .map_or(Ok(()), |quarantine| fs::rename(quarantine, &entry.path));
                failures.push(format!(
                    "restore {}: {error}; changed-tree recovery={put_back:?} at {}; prior config retained at {}",
                    entry.path.display(),
                    quarantine
                        .as_deref()
                        .unwrap_or_else(|| Path::new("<no quarantine>"))
                        .display(),
                    scratch.display()
                ));
                continue;
            }
            if let Some(quarantine) = quarantine {
                if let Err(error) = fs::remove_file(&quarantine) {
                    failures.push(format!(
                        "remove generated PM hook rollback quarantine {}: {error}",
                        quarantine.display()
                    ));
                }
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "restore prior generated PM hook configs: {}",
                failures.join("; ")
            )))
        }
    }
}

fn pm_generated_hook_node_state(path: &Path) -> io::Result<PmGeneratedHookNodeState> {
    match fs::symlink_metadata(path) {
        Ok(_) => {
            require_regular_pm_worktree_control_file(path, "generated PM hook rollback target")?;
            Ok(PmGeneratedHookNodeState::Regular(fs::read(path)?))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            Ok(PmGeneratedHookNodeState::Absent)
        }
        Err(error) => Err(error),
    }
}

fn finish_degraded_pm_refresh(
    git_root: &Path,
    project_dir: &Path,
    worktree: &Path,
    freshness: PmWorktreeFreshness,
) -> io::Result<PmWorktreeRefreshOutcome> {
    if let Err(error) = crate::managed_assets::refresh_managed_gwt_assets_for_pm_worktree(worktree)
    {
        let original = freshness
            .failure_stage
            .map(|stage| format!(" after {stage:?} degradation"))
            .unwrap_or_default();
        let asset_failure = pm_refresh_failure(
            git_root,
            worktree,
            freshness.target_sha.clone(),
            freshness.target_observation,
            PmWorktreeRefreshFailureStage::ManagedAssets,
            format!("managed asset refresh failed{original}: {error}"),
        );
        persist_pm_worktree_freshness(project_dir, &asset_failure)?;
        return Err(error);
    }
    persist_pm_worktree_freshness(project_dir, &freshness)?;
    Ok(PmWorktreeRefreshOutcome {
        worktree: worktree.to_path_buf(),
        freshness,
    })
}

fn with_pm_worktree_refresh_lock<T>(
    project_dir: &Path,
    operation: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    let project_state = project_dir.join("project-state");
    ensure_real_pm_scratch_directory(&project_state, "project-state directory")?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(project_state.join("pm-refresh.lock"))?;
    gwt_core::operation_deadline::lock_exclusive(&lock)?;
    let result = operation();
    let unlock = FileExt::unlock(&lock);
    match (result, unlock) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
    }
}

fn ensure_existing_pm_worktree_owned_by_git_root(
    expected_git_root: &Path,
    worktree: &Path,
) -> io::Result<()> {
    match fs::symlink_metadata(worktree.join(".git")) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    }
    require_real_pm_scratch_directory(worktree, "PM worktree")?;
    let actual_git_root = gwt_git::worktree::main_worktree_root(worktree)
        .map_err(|error| io::Error::other(error.to_string()))?;
    if same_canonical_path(expected_git_root, &actual_git_root) {
        return validate_linked_pm_worktree_marker(expected_git_root, worktree);
    }
    Err(io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!(
            "PM worktree Git-root ownership mismatch: {} belongs to {}, expected {}",
            worktree.display(),
            actual_git_root.display(),
            expected_git_root.display()
        ),
    ))
}

fn require_regular_pm_worktree_control_file(path: &Path, label: &str) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{label} must be a regular file: {}", path.display()),
        ));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;

        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{label} must not be a reparse point: {}", path.display()),
            ));
        }
    }
    Ok(())
}

fn parse_pm_worktree_control_path(path: &Path, prefix: Option<&str>) -> io::Result<PathBuf> {
    require_regular_pm_worktree_control_file(path, "PM linked-worktree control file")?;
    let raw = fs::read_to_string(path)?;
    let value = match prefix {
        Some(prefix) => raw.trim().strip_prefix(prefix).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "PM linked-worktree marker has an invalid format: {}",
                    path.display()
                ),
            )
        })?,
        None => raw.trim(),
    };
    if value.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "PM linked-worktree control path is empty: {}",
                path.display()
            ),
        ));
    }
    let value = PathBuf::from(value);
    Ok(if value.is_absolute() {
        value
    } else {
        path.parent().unwrap_or_else(|| Path::new(".")).join(value)
    })
}

fn git_common_dir(git_root: &Path) -> io::Result<PathBuf> {
    let output =
        gwt_core::process::run_git_logged(&["rev-parse", "--git-common-dir"], Some(git_root))?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "resolve PM Git common directory: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let common = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    if common.as_os_str().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "resolve PM Git common directory: Git returned an empty path",
        ));
    }
    Ok(if common.is_absolute() {
        common
    } else {
        git_root.join(common)
    })
}

fn git_repository_is_bare(git_root: &Path) -> io::Result<bool> {
    let output =
        gwt_core::process::run_git_logged(&["rev-parse", "--is-bare-repository"], Some(git_root))?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "inspect whether PM Git root is bare: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    match String::from_utf8_lossy(&output.stdout).trim() {
        "true" => Ok(true),
        "false" => Ok(false),
        value => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Git returned an invalid bare-repository flag: {value:?}"),
        )),
    }
}

fn legacy_pm_project_root_for_identity(
    git_root: &Path,
    project_dir: &Path,
) -> io::Result<Option<PathBuf>> {
    let mut candidates = vec![git_root.to_path_buf()];
    if git_repository_is_bare(git_root)? {
        if let Some(parent) = git_root.parent() {
            candidates.push(parent.to_path_buf());
        }
    }
    Ok(candidates.into_iter().find(|candidate| {
        let candidate_project_dir = gwt_core::paths::gwt_project_dir_for_repo_path(candidate);
        same_canonical_path(project_dir, &candidate_project_dir)
    }))
}

fn validate_linked_pm_worktree_marker(git_root: &Path, worktree: &Path) -> io::Result<()> {
    let dot_git = worktree.join(".git");
    let admin_dir = parse_pm_worktree_control_path(&dot_git, Some("gitdir: "))?;
    require_real_pm_scratch_directory(&admin_dir, "PM linked-worktree admin directory")?;
    let common_dir = git_common_dir(git_root)?;
    let worktrees_dir = common_dir.join("worktrees");
    let canonical_admin = dunce::canonicalize(&admin_dir)?;
    let canonical_worktrees = dunce::canonicalize(&worktrees_dir)?;
    if canonical_admin.parent() != Some(canonical_worktrees.as_path()) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "PM linked-worktree admin directory is outside the expected Git common directory: {}",
                admin_dir.display()
            ),
        ));
    }
    let backpointer = parse_pm_worktree_control_path(&admin_dir.join("gitdir"), None)?;
    if !same_canonical_path(&backpointer, &dot_git) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "PM linked-worktree admin backpointer does not authorize {}",
                worktree.display()
            ),
        ));
    }
    let registered = gwt_git::WorktreeManager::new(git_root)
        .list()
        .map_err(|error| io::Error::other(error.to_string()))?
        .into_iter()
        .filter(|entry| same_canonical_path(&entry.path, worktree))
        .count();
    if registered != 1 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "PM worktree must have exactly one Git registration, found {registered}: {}",
                worktree.display()
            ),
        ));
    }
    Ok(())
}

fn refresh_pm_worktree_at_locked(
    git_root: &Path,
    // Fetch and worktree administration belong to the shared main Git root,
    // while a remote-less first spawn first preserves the calling checkout's
    // local HEAD (which may be a linked worktree at a different commit). A
    // workspace-home caller is not itself a Git directory, so that layout
    // falls back once more to the resolved bare Git root.
    fallback_head_root: &Path,
    project_dir: &Path,
    worktree: &Path,
) -> io::Result<PmWorktreeRefreshOutcome> {
    let manager = gwt_git::WorktreeManager::new(git_root);
    let existed = worktree.join(".git").exists();
    if worktree.exists() && !existed {
        let (cached_target, inspect_error) = git_sha_observation(git_root, PM_WORKTREE_BASE_REF);
        let observation = if cached_target.is_some() {
            PmWorktreeTargetObservation::Cached
        } else {
            PmWorktreeTargetObservation::Unavailable
        };
        let mut reason = format!(
            "canonical PM worktree path exists without Git metadata: {}",
            worktree.display()
        );
        if let Some(error) = inspect_error {
            reason.push_str(&format!("; cached target inspection also failed: {error}"));
        }
        let freshness = pm_refresh_failure(
            git_root,
            worktree,
            cached_target,
            observation,
            PmWorktreeRefreshFailureStage::Inspect,
            reason,
        );
        persist_pm_worktree_freshness(project_dir, &freshness)?;
        return Err(io::Error::other(
            freshness.failure_reason.clone().unwrap_or_default(),
        ));
    }

    let mut generated_hook_snapshot = if existed {
        match PmGeneratedHookConfigSnapshot::capture(worktree) {
            Ok(snapshot) => Some(snapshot),
            Err(error) => {
                persist_unknown_pm_worktree_failure(
                    project_dir,
                    worktree,
                    PmWorktreeRefreshFailureStage::Inspect,
                    error.to_string(),
                )?;
                return Err(error);
            }
        }
    } else {
        None
    };

    let refresh = (|| {
        if existed {
            if let Err(error) = normalize_previous_generated_hook_configs_guarded(
                worktree,
                generated_hook_snapshot.as_mut(),
            ) {
                let (cached_target, inspect_error) =
                    git_sha_observation(git_root, PM_WORKTREE_BASE_REF);
                let observation = if cached_target.is_some() {
                    PmWorktreeTargetObservation::Cached
                } else {
                    PmWorktreeTargetObservation::Unavailable
                };
                let mut reason = error.to_string();
                if let Some(error) = inspect_error {
                    reason.push_str(&format!("; cached target inspection also failed: {error}"));
                }
                let freshness = pm_refresh_failure(
                    git_root,
                    worktree,
                    cached_target,
                    observation,
                    PmWorktreeRefreshFailureStage::Inspect,
                    reason,
                );
                if error.kind() == io::ErrorKind::PermissionDenied {
                    persist_pm_worktree_freshness(project_dir, &freshness)?;
                    return Err(error);
                }
                return finish_degraded_pm_refresh(git_root, project_dir, worktree, freshness);
            }
            if let Err(error) = migrate_legacy_pm_scratch_preserving_project_content(worktree) {
                let (cached_target, inspect_error) =
                    git_sha_observation(git_root, PM_WORKTREE_BASE_REF);
                let observation = if cached_target.is_some() {
                    PmWorktreeTargetObservation::Cached
                } else {
                    PmWorktreeTargetObservation::Unavailable
                };
                let mut reason = error.to_string();
                if let Some(error) = inspect_error {
                    reason.push_str(&format!("; cached target inspection also failed: {error}"));
                }
                let freshness = pm_refresh_failure(
                    git_root,
                    worktree,
                    cached_target,
                    observation,
                    PmWorktreeRefreshFailureStage::ScratchMigration,
                    reason,
                );
                return finish_degraded_pm_refresh(git_root, project_dir, worktree, freshness);
            }
        }

        if let Err(error) = fetch_pm_worktree_base(git_root) {
            let (cached_target, inspect_error) =
                git_sha_observation(git_root, PM_WORKTREE_BASE_REF);
            let observation = if cached_target.is_some() {
                PmWorktreeTargetObservation::Cached
            } else {
                PmWorktreeTargetObservation::Unavailable
            };
            let mut failure_reason = error.to_string();
            if let Some(error) = inspect_error {
                failure_reason
                    .push_str(&format!("; cached target inspection also failed: {error}"));
            }
            if !existed {
                let (mut local_head, mut local_head_inspect_error) = if cached_target.is_none() {
                    git_sha_observation(fallback_head_root, "HEAD")
                } else {
                    (None, None)
                };
                if cached_target.is_none()
                    && local_head.is_none()
                    && is_nested_bare_layout_root(fallback_head_root)
                {
                    let (git_root_head, git_root_inspect_error) =
                        git_sha_observation(git_root, "HEAD");
                    local_head = git_root_head;
                    local_head_inspect_error = git_root_inspect_error.or(local_head_inspect_error);
                }
                let materialization_sha = cached_target.as_deref().or(local_head.as_deref());
                let Some(materialization_sha) = materialization_sha else {
                    failure_reason.push_str("; local HEAD is unavailable for degraded startup");
                    if let Some(error) = local_head_inspect_error {
                        failure_reason
                            .push_str(&format!("; local HEAD inspection also failed: {error}"));
                    }
                    let freshness = pm_refresh_failure(
                        git_root,
                        worktree,
                        None,
                        observation,
                        PmWorktreeRefreshFailureStage::Fetch,
                        failure_reason,
                    );
                    persist_pm_worktree_freshness(project_dir, &freshness)?;
                    return Err(io::Error::other(
                        freshness.failure_reason.clone().unwrap_or_default(),
                    ));
                };
                if cached_target.is_none() {
                    failure_reason.push_str(&format!(
                        "; materialized local HEAD {materialization_sha} because the remote target was unavailable"
                    ));
                }
                if let Some(parent) = worktree.parent() {
                    if let Err(create_error) = fs::create_dir_all(parent) {
                        let freshness = pm_refresh_failure(
                            git_root,
                            worktree,
                            cached_target,
                            observation,
                            PmWorktreeRefreshFailureStage::Repoint,
                            create_error.to_string(),
                        );
                        persist_pm_worktree_freshness(project_dir, &freshness)?;
                        return Err(create_error);
                    }
                }
                if let Err(create_error) = manager.create_detached(materialization_sha, worktree) {
                    let freshness = pm_refresh_failure(
                        git_root,
                        worktree,
                        cached_target,
                        observation,
                        PmWorktreeRefreshFailureStage::Repoint,
                        create_error.to_string(),
                    );
                    persist_pm_worktree_freshness(project_dir, &freshness)?;
                    return Err(io::Error::other(create_error.to_string()));
                }
            }
            let freshness = pm_refresh_failure(
                git_root,
                worktree,
                cached_target,
                observation,
                PmWorktreeRefreshFailureStage::Fetch,
                failure_reason,
            );
            return finish_degraded_pm_refresh(git_root, project_dir, worktree, freshness);
        }

        let (target_sha, target_inspect_error) =
            git_sha_observation(git_root, PM_WORKTREE_BASE_REF);
        let Some(target_sha) = target_sha else {
            let reason = target_inspect_error
                .unwrap_or_else(|| format!("{} is unavailable after fetch", PM_WORKTREE_BASE_REF));
            let freshness = pm_refresh_failure(
                git_root,
                worktree,
                None,
                PmWorktreeTargetObservation::Unavailable,
                PmWorktreeRefreshFailureStage::Inspect,
                reason,
            );
            if existed {
                return finish_degraded_pm_refresh(git_root, project_dir, worktree, freshness);
            }
            persist_pm_worktree_freshness(project_dir, &freshness)?;
            return Err(io::Error::other(
                freshness.failure_reason.clone().unwrap_or_default(),
            ));
        };

        let old_head = if existed {
            match detached_worktree_head_sha(worktree) {
                Ok(head) => head,
                Err(error) => {
                    let freshness = pm_refresh_failure(
                        git_root,
                        worktree,
                        Some(target_sha),
                        PmWorktreeTargetObservation::Fresh,
                        PmWorktreeRefreshFailureStage::Inspect,
                        error.to_string(),
                    );
                    return finish_degraded_pm_refresh(git_root, project_dir, worktree, freshness);
                }
            }
        } else {
            None
        };
        if existed {
            let safety = manager
                .detached_repoint_safety(worktree)
                .map_err(|error| io::Error::other(error.to_string()));
            let failure = match safety {
                Ok(gwt_git::worktree::DetachedRepointSafety::Ready) => None,
                Ok(gwt_git::worktree::DetachedRepointSafety::SymbolicHead { branch }) => Some((
                    PmWorktreeRefreshFailureStage::Inspect,
                    format!("PM worktree HEAD is symbolic ({branch})"),
                )),
                Ok(gwt_git::worktree::DetachedRepointSafety::TrackedOrIndexChanges) => Some((
                    PmWorktreeRefreshFailureStage::LocalWork,
                    "PM worktree has tracked or index changes".to_string(),
                )),
                Ok(gwt_git::worktree::DetachedRepointSafety::DetachedOnlyCommit) => Some((
                    PmWorktreeRefreshFailureStage::LocalWork,
                    "PM worktree has a detached-only commit".to_string(),
                )),
                Err(error) => Some((PmWorktreeRefreshFailureStage::Inspect, error.to_string())),
            };
            if let Some((stage, reason)) = failure {
                let freshness = pm_refresh_failure(
                    git_root,
                    worktree,
                    Some(target_sha),
                    PmWorktreeTargetObservation::Fresh,
                    stage,
                    reason,
                );
                return finish_degraded_pm_refresh(git_root, project_dir, worktree, freshness);
            }
        }
        if let Some(head) = old_head.as_deref().filter(|head| *head != target_sha) {
            if let Err(error) = manager.repoint_detached(worktree, &target_sha) {
                let freshness = pm_refresh_failure(
                    git_root,
                    worktree,
                    Some(target_sha),
                    PmWorktreeTargetObservation::Fresh,
                    PmWorktreeRefreshFailureStage::Repoint,
                    error.to_string(),
                );
                return finish_degraded_pm_refresh(git_root, project_dir, worktree, freshness);
            }
            debug_assert!(!head.is_empty());
        } else if !existed {
            if let Some(parent) = worktree.parent() {
                if let Err(error) = fs::create_dir_all(parent) {
                    let freshness = pm_refresh_failure(
                        git_root,
                        worktree,
                        Some(target_sha),
                        PmWorktreeTargetObservation::Fresh,
                        PmWorktreeRefreshFailureStage::Repoint,
                        error.to_string(),
                    );
                    persist_pm_worktree_freshness(project_dir, &freshness)?;
                    return Err(error);
                }
            }
            if let Err(error) = manager.create_detached(&target_sha, worktree) {
                let freshness = pm_refresh_failure(
                    git_root,
                    worktree,
                    Some(target_sha),
                    PmWorktreeTargetObservation::Fresh,
                    PmWorktreeRefreshFailureStage::Repoint,
                    error.to_string(),
                );
                persist_pm_worktree_freshness(project_dir, &freshness)?;
                return Err(io::Error::other(error.to_string()));
            }
        }

        if let Err(error) =
            crate::managed_assets::refresh_managed_gwt_assets_for_pm_worktree(worktree)
        {
            let mut failure_reason = error.to_string();
            if let Some(old_head) = old_head
                .as_deref()
                .filter(|old_head| *old_head != target_sha)
            {
                let rollback = normalize_previous_generated_hook_configs(worktree)
                    .map_err(|error| error.to_string())
                    .and_then(|()| {
                        manager
                            .repoint_detached(worktree, old_head)
                            .map_err(|error| error.to_string())
                    });
                if let Err(rollback_error) = rollback {
                    failure_reason.push_str(&format!(
                    "; restoring prior PM worktree HEAD {old_head} also failed: {rollback_error}"
                ));
                }
            }
            let freshness = pm_refresh_failure(
                git_root,
                worktree,
                Some(target_sha),
                PmWorktreeTargetObservation::Fresh,
                PmWorktreeRefreshFailureStage::ManagedAssets,
                &failure_reason,
            );
            persist_pm_worktree_freshness(project_dir, &freshness)?;
            return Err(io::Error::other(failure_reason));
        }

        let observed_head = match detached_worktree_head_sha(worktree) {
            Ok(head) => head,
            Err(error) => {
                let freshness = pm_refresh_failure(
                    git_root,
                    worktree,
                    Some(target_sha),
                    PmWorktreeTargetObservation::Fresh,
                    PmWorktreeRefreshFailureStage::Inspect,
                    error.to_string(),
                );
                persist_pm_worktree_freshness(project_dir, &freshness)?;
                return Ok(PmWorktreeRefreshOutcome {
                    worktree: worktree.to_path_buf(),
                    freshness,
                });
            }
        };
        if observed_head.as_deref() != Some(target_sha.as_str()) {
            let freshness = pm_refresh_failure(
                git_root,
                worktree,
                Some(target_sha),
                PmWorktreeTargetObservation::Fresh,
                PmWorktreeRefreshFailureStage::Inspect,
                "PM worktree HEAD changed before freshness could be committed",
            );
            persist_pm_worktree_freshness(project_dir, &freshness)?;
            return Ok(PmWorktreeRefreshOutcome {
                worktree: worktree.to_path_buf(),
                freshness,
            });
        }

        let freshness = PmWorktreeFreshness {
            state: PmWorktreeFreshnessState::Fresh,
            base_ref: PM_WORKTREE_BASE_REF.to_string(),
            head_sha: Some(target_sha.clone()),
            target_sha: Some(target_sha),
            behind: Some(0),
            target_observation: PmWorktreeTargetObservation::Fresh,
            checked_at: pm_refresh_checked_at(),
            failure_stage: None,
            failure_reason: None,
        };
        persist_pm_worktree_freshness(project_dir, &freshness)?;
        Ok(PmWorktreeRefreshOutcome {
            worktree: worktree.to_path_buf(),
            freshness,
        })
    })();

    match (refresh, generated_hook_snapshot) {
        (Ok(outcome), _) => Ok(outcome),
        (Err(error), Some(snapshot)) => match snapshot.restore() {
            Ok(()) => Err(error),
            Err(restore_error) => {
                let restore_reason = format!(
                    "restoring prior generated PM hook configs also failed: {restore_error}"
                );
                let persistence =
                    append_pm_worktree_refresh_failure_reason(project_dir, &restore_reason);
                let persistence_suffix = persistence
                    .err()
                    .map(|persist_error| {
                        format!("; persisting rollback failure also failed: {persist_error}")
                    })
                    .unwrap_or_default();
                Err(io::Error::other(format!(
                    "{error}; {restore_reason}{persistence_suffix}"
                )))
            }
        },
        (Err(error), None) => Err(error),
    }
}

fn same_canonical_path(left: &Path, right: &Path) -> bool {
    match (dunce::canonicalize(left), dunce::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

fn pm_worktree_identity_path(project_dir: &Path) -> PathBuf {
    project_dir.join("project-state/pm-worktree-identity.json")
}

fn require_regular_pm_identity_file(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "PM worktree identity is not a regular file: {}",
                path.display()
            ),
        ));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;

        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "PM worktree identity must not be a reparse point: {}",
                    path.display()
                ),
            ));
        }
    }
    Ok(())
}

fn load_pm_worktree_identity(project_dir: &Path) -> io::Result<Option<PmWorktreeIdentity>> {
    let path = pm_worktree_identity_path(project_dir);
    match require_regular_pm_identity_file(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    }
    let identity: PmWorktreeIdentity = serde_json::from_slice(&fs::read(&path)?)?;
    if identity.schema_version != PM_WORKTREE_IDENTITY_SCHEMA_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "unsupported PM worktree identity schema {} at {}",
                identity.schema_version,
                path.display()
            ),
        ));
    }
    Ok(Some(identity))
}

fn save_pm_worktree_identity(
    project_dir: &Path,
    project_root: &Path,
    git_root: &Path,
    worktree: &Path,
) -> io::Result<()> {
    let project_state = project_dir.join("project-state");
    require_real_pm_scratch_directory(&project_state, "project-state directory")?;
    let path = pm_worktree_identity_path(project_dir);
    match require_regular_pm_identity_file(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let identity = PmWorktreeIdentity {
        schema_version: PM_WORKTREE_IDENTITY_SCHEMA_VERSION,
        project_root: dunce::canonicalize(project_root)
            .unwrap_or_else(|_| project_root.to_path_buf()),
        git_root: dunce::canonicalize(git_root).unwrap_or_else(|_| git_root.to_path_buf()),
        worktree: worktree.to_path_buf(),
    };
    let scratch = unique_pm_scratch_path(&path);
    fs::write(&scratch, serde_json::to_vec_pretty(&identity)?)?;
    fs::rename(scratch, path)
}

fn persist_preflight_failure_if_safe(
    project_dir: &Path,
    worktree: &Path,
    stage: PmWorktreeRefreshFailureStage,
    error: &io::Error,
) {
    let project_state = project_dir.join("project-state");
    if require_real_pm_scratch_directory(project_dir, "project directory").is_ok()
        && require_real_pm_scratch_directory(&project_state, "project-state directory").is_ok()
    {
        let _ =
            persist_unknown_pm_worktree_failure(project_dir, worktree, stage, error.to_string());
    }
}

fn persist_scratch_preflight_failure_if_safe(
    project_dir: &Path,
    worktree: &Path,
    error: &io::Error,
) {
    persist_preflight_failure_if_safe(
        project_dir,
        worktree,
        PmWorktreeRefreshFailureStage::ScratchMigration,
        error,
    );
}

fn persist_untrusted_inspection_failure_if_safe(project_dir: &Path, error: &io::Error) {
    let project_state = project_dir.join("project-state");
    if require_real_pm_scratch_directory(project_dir, "project directory").is_ok()
        && require_real_pm_scratch_directory(&project_state, "project-state directory").is_ok()
    {
        let _ = persist_untrusted_pm_worktree_failure(
            project_dir,
            PmWorktreeRefreshFailureStage::Inspect,
            error.to_string(),
        );
    }
}

/// Refresh the canonical resident PM worktree before a process starts or
/// resumes. This is the repo-root entrypoint used by AppRuntime.
pub fn refresh_pm_worktree_for_repo_path(repo_path: &Path) -> io::Result<PmWorktreeRefreshOutcome> {
    let project_dir = gwt_core::paths::gwt_project_dir_for_repo_path(repo_path);
    let worktree = project_dir.join("pm/worktree");
    let git_root = match gwt_git::worktree::main_worktree_root(repo_path) {
        Ok(git_root) => git_root,
        Err(error) => {
            let error = io::Error::other(error.to_string());
            persist_untrusted_inspection_failure_if_safe(&project_dir, &error);
            return Err(error);
        }
    };
    fs::create_dir_all(&project_dir)?;
    with_pm_worktree_refresh_lock(&project_dir, || {
        if let Err(error) = ensure_existing_pm_worktree_owned_by_git_root(&git_root, &worktree) {
            let reason = error.to_string();
            persist_untrusted_pm_worktree_failure(
                &project_dir,
                PmWorktreeRefreshFailureStage::Inspect,
                &reason,
            )
            .map_err(|persist_error| {
                io::Error::other(format!(
                    "{reason}; persist PM inspection failure: {persist_error}"
                ))
            })?;
            return Err(error);
        }
        if let Err(error) = ensure_pm_scratch_dir_for_repo_path(repo_path) {
            persist_scratch_preflight_failure_if_safe(&project_dir, &worktree, &error);
            return Err(error);
        }
        if let Err(error) = save_pm_worktree_identity(&project_dir, repo_path, &git_root, &worktree)
        {
            let reason = error.to_string();
            persist_unknown_pm_worktree_failure(
                &project_dir,
                &worktree,
                PmWorktreeRefreshFailureStage::Inspect,
                &reason,
            )
            .map_err(|persist_error| {
                io::Error::other(format!(
                    "{reason}; persist PM identity storage failure: {persist_error}"
                ))
            })?;
            return Err(error);
        }
        refresh_pm_worktree_at_locked(&git_root, repo_path, &project_dir, &worktree)
    })
}

/// Serialize PM worktree cleanup with every refresh boundary. The caller
/// supplies the higher-layer classifier for generated merged hook configs;
/// all unknown local work remains a fail-closed retention signal.
pub fn cleanup_pm_worktree_for_repo_path<F>(
    repo_path: &Path,
    extra_disposable: F,
) -> io::Result<PmWorktreeCleanupOutcome>
where
    F: Fn(&Path, &str) -> bool,
{
    let project_dir = gwt_core::paths::gwt_project_dir_for_repo_path(repo_path);
    let worktree = project_dir.join("pm/worktree");
    let git_root = match gwt_git::worktree::main_worktree_root(repo_path) {
        Ok(git_root) => git_root,
        Err(error) => {
            let error = io::Error::other(error.to_string());
            persist_untrusted_inspection_failure_if_safe(&project_dir, &error);
            return Err(error);
        }
    };
    fs::create_dir_all(&project_dir)?;
    with_pm_worktree_refresh_lock(&project_dir, || {
        if let Err(error) = ensure_existing_pm_worktree_owned_by_git_root(&git_root, &worktree) {
            let reason = error.to_string();
            persist_untrusted_pm_worktree_failure(
                &project_dir,
                PmWorktreeRefreshFailureStage::Inspect,
                &reason,
            )
            .map_err(|persist_error| {
                io::Error::other(format!(
                    "{reason}; persist PM cleanup inspection failure: {persist_error}"
                ))
            })?;
            return Err(error);
        }
        if !worktree.exists() {
            return Ok(PmWorktreeCleanupOutcome::Absent);
        }
        if let Err(error) = migrate_legacy_pm_scratch_preserving_project_content(&worktree) {
            let (target_sha, inspect_error) = git_sha_observation(&git_root, PM_WORKTREE_BASE_REF);
            let observation = if target_sha.is_some() {
                PmWorktreeTargetObservation::Cached
            } else {
                PmWorktreeTargetObservation::Unavailable
            };
            let mut reason = error.to_string();
            if let Some(error) = inspect_error {
                reason.push_str(&format!("; cached target inspection also failed: {error}"));
            }
            let freshness = pm_refresh_failure(
                &git_root,
                &worktree,
                target_sha,
                observation,
                PmWorktreeRefreshFailureStage::ScratchMigration,
                reason,
            );
            persist_pm_worktree_freshness(&project_dir, &freshness)?;
            return Err(error);
        }
        let manager = gwt_git::WorktreeManager::new(git_root.clone());
        if manager
            .ephemeral_worktree_has_local_work_with(&worktree, |entry| {
                extra_disposable(&worktree, entry)
            })
            .map_err(|error| io::Error::other(error.to_string()))?
        {
            return Ok(PmWorktreeCleanupOutcome::RetainedLocalWork);
        }
        manager
            .remove_force_twice(&worktree)
            .map_err(|error| io::Error::other(error.to_string()))?;
        Ok(PmWorktreeCleanupOutcome::Removed)
    })
}

/// Refresh a running resident PM at an explicit pre-turn or completed-Stop
/// boundary. Structural path validation prevents ordinary worktrees named
/// `pm/worktree` from gaining this authority.
pub fn refresh_pm_worktree_at_safe_boundary(
    worktree: &Path,
) -> io::Result<Option<PmWorktreeRefreshOutcome>> {
    if !is_pm_worktree(worktree) {
        return Ok(None);
    }
    let Some(project_dir) = worktree.parent().and_then(Path::parent) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "canonical PM worktree has no project directory: {}",
                worktree.display()
            ),
        ));
    };
    require_real_pm_scratch_directory(project_dir, "project directory")?;
    require_real_pm_scratch_directory(worktree, "PM worktree")?;
    with_pm_worktree_refresh_lock(project_dir, || {
        let git_root = match gwt_git::worktree::main_worktree_root(worktree) {
            Ok(git_root) => git_root,
            Err(error) => {
                let error = io::Error::other(error.to_string());
                if pm_identity_authorizes_project_state(project_dir, worktree) {
                    persist_untrusted_pm_worktree_failure(
                        project_dir,
                        PmWorktreeRefreshFailureStage::Inspect,
                        error.to_string(),
                    )?;
                }
                return Err(error);
            }
        };
        if let Err(error) = ensure_existing_pm_worktree_owned_by_git_root(&git_root, worktree) {
            if pm_identity_authorizes_project_state(project_dir, worktree) {
                persist_untrusted_pm_worktree_failure(
                    project_dir,
                    PmWorktreeRefreshFailureStage::Inspect,
                    error.to_string(),
                )?;
            }
            return Err(error);
        }
        let root_derived_project_dir = gwt_core::paths::gwt_project_dir_for_repo_path(&git_root);
        let identity = match load_pm_worktree_identity(project_dir) {
            Ok(identity) => identity,
            Err(error) => {
                if same_canonical_path(project_dir, &root_derived_project_dir) {
                    let reason = error.to_string();
                    persist_unknown_pm_worktree_failure(
                        project_dir,
                        worktree,
                        PmWorktreeRefreshFailureStage::Inspect,
                        &reason,
                    )
                    .map_err(|persist_error| {
                        io::Error::other(format!(
                            "{reason}; persist PM identity inspection failure: {persist_error}"
                        ))
                    })?;
                }
                return Err(error);
            }
        };
        let identity_matches = match identity {
            Some(identity) => {
                let identity_project_dir =
                    gwt_core::paths::gwt_project_dir_for_repo_path(&identity.project_root);
                same_canonical_path(project_dir, &identity_project_dir)
                    && same_canonical_path(&git_root, &identity.git_root)
                    && same_canonical_path(worktree, &identity.worktree)
            }
            None => {
                let legacy_project_root =
                    legacy_pm_project_root_for_identity(&git_root, project_dir)?;
                let Some(legacy_project_root) = legacy_project_root else {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        format!(
                            "PM worktree has no identity and no authorized legacy project root: {}",
                            worktree.display()
                        ),
                    ));
                };
                let expected_worktree = project_dir.join("pm/worktree");
                if !same_canonical_path(worktree, &expected_worktree) {
                    false
                } else if let Err(error) = save_pm_worktree_identity(
                    project_dir,
                    &legacy_project_root,
                    &git_root,
                    worktree,
                ) {
                    let reason = error.to_string();
                    persist_unknown_pm_worktree_failure(
                        project_dir,
                        worktree,
                        PmWorktreeRefreshFailureStage::Inspect,
                        &reason,
                    )
                    .map_err(|persist_error| {
                        io::Error::other(format!(
                            "{reason}; persist legacy PM identity migration failure: {persist_error}"
                        ))
                    })?;
                    return Err(error);
                } else {
                    true
                }
            }
        };
        if !identity_matches {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "PM worktree project identity mismatch: {} does not authorize Git root {}",
                    worktree.display(),
                    git_root.display()
                ),
            ));
        }
        let project_state = project_dir.join("project-state");
        ensure_real_pm_scratch_directory(&project_state, "project-state directory")?;
        if let Err(error) = ensure_real_pm_scratch_directory(
            &project_state.join("pm-scratch"),
            "PM scratch directory",
        ) {
            persist_scratch_preflight_failure_if_safe(project_dir, worktree, &error);
            return Err(error);
        }
        refresh_pm_worktree_at_locked(&git_root, worktree, project_dir, worktree).map(Some)
    })
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

/// [`is_pm_worktree`] for callers holding an already-canonicalized path.
///
/// The Work mutation paths canonicalize every path they touch, while
/// [`gwt_projects_dir`](gwt_core::paths::gwt_projects_dir) is built from `HOME`
/// verbatim. When `HOME` traverses a symlink — macOS `/var` -> `/private/var`
/// is the everyday case under a temporary home — the two spellings disagree
/// and the plain shape test rejects a genuine PM worktree. Comparing both
/// spellings keeps the answer about the path, not about how it was spelled.
pub fn is_canonical_pm_worktree(path: &Path) -> bool {
    if is_pm_worktree(path) {
        return true;
    }
    let Some(canonical_projects_dir) =
        dunce::canonicalize(gwt_core::paths::gwt_projects_dir()).ok()
    else {
        return false;
    };
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
        .is_some_and(|projects_dir| projects_dir == canonical_projects_dir)
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_freshness: Option<PmWorktreeFreshness>,
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
        worktree_freshness: prefs.worktree_freshness.clone(),
        repository_registrations: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn generated_hook_snapshot_restore_rejects_a_replaced_symlink_target() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().expect("tempdir");
        let worktree = dir.path().join("worktree");
        let codex = worktree.join(".codex");
        fs::create_dir_all(&codex).expect("Codex config directory");
        let hook = codex.join("hooks.json");
        fs::write(&hook, b"{}\n").expect("prior generated-only hook config");
        assert!(crate::managed_assets::managed_hook_config_is_disposable(
            &worktree,
            ".codex/hooks.json"
        ));
        let snapshot = PmGeneratedHookConfigSnapshot::capture(&worktree)
            .expect("capture regular generated hook config");
        let external = dir.path().join("external.json");
        fs::write(&external, b"external content must remain\n").expect("external config");
        fs::remove_file(&hook).expect("replace captured hook");
        symlink(&external, &hook).expect("indirect rollback target");

        let error = snapshot
            .restore()
            .expect_err("rollback must reject a replaced symlink target");

        assert_eq!(
            fs::read(&external).expect("external config remains"),
            b"external content must remain\n"
        );
        assert!(
            fs::symlink_metadata(&hook)
                .expect("rollback target remains")
                .file_type()
                .is_symlink(),
            "rollback must not replace or follow the indirect target"
        );
        let recovery = fs::read_dir(&codex)
            .expect("scan recovery directory")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with(".hooks.json.tmp-"))
            })
            .expect("prior bytes remain in a same-parent recovery file");
        assert_eq!(fs::read(&recovery).expect("recovery bytes"), b"{}\n");
        assert!(error.to_string().contains(&recovery.display().to_string()));
    }

    #[test]
    fn generated_hook_snapshot_restore_preserves_a_replaced_regular_user_config() {
        let dir = tempfile::tempdir().expect("tempdir");
        let worktree = dir.path().join("worktree");
        let codex = worktree.join(".codex");
        fs::create_dir_all(&codex).expect("Codex config directory");
        let hook = codex.join("hooks.json");
        fs::write(&hook, b"{}\n").expect("prior generated-only hook config");
        let snapshot = PmGeneratedHookConfigSnapshot::capture(&worktree)
            .expect("capture regular generated hook config");
        let user_config = br#"{"hooks":{},"user-owned":true}
"#;
        fs::write(&hook, user_config).expect("replace with regular user config");

        let error = snapshot
            .restore()
            .expect_err("rollback must reject a replaced regular user config");

        assert_eq!(
            fs::read(&hook).expect("user config remains"),
            user_config,
            "ownership mismatch must preserve the newer regular file"
        );
        let recovery = fs::read_dir(&codex)
            .expect("scan recovery directory")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with(".hooks.json.tmp-"))
            })
            .expect("prior bytes remain in a same-parent recovery file");
        assert_eq!(fs::read(&recovery).expect("recovery bytes"), b"{}\n");
        assert!(error.to_string().contains(&recovery.display().to_string()));
    }

    #[test]
    fn generated_hook_rollback_failure_is_durable_in_the_typed_status_snapshot() {
        let dir = tempfile::tempdir().expect("tempdir");
        let project_dir = dir.path().join("project");
        let prefs_path = project_dir.join("project-state/pm.json");
        let checked_at = "2026-08-29T00:00:00Z".to_string();
        mutate_pm_prefs(&prefs_path, |prefs| {
            prefs.worktree_freshness = Some(PmWorktreeFreshness {
                state: PmWorktreeFreshnessState::Unknown,
                base_ref: PM_WORKTREE_BASE_REF.to_string(),
                head_sha: None,
                target_sha: Some("target".to_string()),
                behind: None,
                target_observation: PmWorktreeTargetObservation::Fresh,
                checked_at: checked_at.clone(),
                failure_stage: Some(PmWorktreeRefreshFailureStage::ManagedAssets),
                failure_reason: Some("managed asset refresh failed".to_string()),
            });
        })
        .expect("seed typed refresh failure");
        let recovery = project_dir.join("pm/worktree/.codex/.hooks.json.tmp-recovery");
        append_pm_worktree_refresh_failure_reason(
            &project_dir,
            &format!("rollback recovery retained at {}", recovery.display()),
        )
        .expect("append durable rollback failure");

        let prefs = load_pm_prefs(&prefs_path).expect("read back PM prefs");
        let status = pm_status_report(&prefs, |_| false);
        let freshness = status
            .worktree_freshness
            .expect("pm.status freshness snapshot");
        assert_eq!(freshness.state, PmWorktreeFreshnessState::Unknown);
        assert_eq!(
            freshness.failure_stage,
            Some(PmWorktreeRefreshFailureStage::ManagedAssets),
            "outer rollback failure must not erase the typed primary stage"
        );
        assert_eq!(freshness.checked_at, checked_at);
        let reason = freshness.failure_reason.expect("stable failure reason");
        assert!(reason.contains("managed asset refresh failed"), "{reason}");
        assert!(reason.contains(&recovery.display().to_string()), "{reason}");
    }

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

    fn pm_scratch_quarantine_residue(root: &Path) -> Vec<PathBuf> {
        let mut pending = vec![root.to_path_buf()];
        let mut residue = Vec::new();
        while let Some(directory) = pending.pop() {
            let metadata = match fs::symlink_metadata(&directory) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => panic!(
                    "inspect quarantine scan root {}: {error}",
                    directory.display()
                ),
            };
            if !metadata.file_type().is_dir() {
                continue;
            }
            for entry in fs::read_dir(&directory).expect("read quarantine scan directory") {
                let entry = entry.expect("quarantine scan entry");
                let path = entry.path();
                if entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".gwt-pm-scratch-quarantine-")
                {
                    residue.push(path);
                } else if entry
                    .file_type()
                    .expect("quarantine scan entry type")
                    .is_dir()
                {
                    pending.push(path);
                }
            }
        }
        residue.sort();
        residue
    }

    fn assert_no_pm_scratch_quarantine_residue(root: &Path) {
        let residue = pm_scratch_quarantine_residue(root);
        assert!(
            residue.is_empty(),
            "PM scratch quarantine residue must be cleaned up: {residue:?}"
        );
    }

    fn run_pm_scratch_git(worktree: &Path, args: &[&str]) -> std::process::Output {
        let output = gwt_core::process::run_git_logged(args, Some(worktree))
            .expect("run git for PM scratch fixture");
        assert!(
            output.status.success(),
            "git {args:?} failed for PM scratch fixture: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }

    fn commit_tracked_pm_scratch(worktree: &Path, relative: &str, contents: &[u8]) {
        fs::create_dir_all(worktree.join("tasks")).expect("create tracked scratch parent");
        run_pm_scratch_git(worktree, &["init"]);
        fs::write(worktree.join(relative), contents).expect("write tracked scratch fixture");
        run_pm_scratch_git(worktree, &["add", "--", relative]);
        run_pm_scratch_git(
            worktree,
            &[
                "-c",
                "user.name=gwt test",
                "-c",
                "user.email=gwt-test@example.invalid",
                "commit",
                "-m",
                "tracked PM scratch fixture",
            ],
        );
    }

    #[cfg(windows)]
    #[test]
    fn pm_scratch_file_fingerprint_distinguishes_same_content_file_identities() {
        let dir = tempfile::tempdir().expect("tempdir");
        let first = dir.path().join("first.md");
        let second = dir.path().join("second.md");
        fs::write(&first, b"same bytes").expect("write first file");
        fs::write(&second, b"same bytes").expect("write second file");

        match (
            pm_scratch_file_fingerprint(&first, "fingerprint first Windows file"),
            pm_scratch_file_fingerprint(&second, "fingerprint second Windows file"),
        ) {
            (Ok(first_fingerprint), Ok(second_fingerprint)) => assert_ne!(
                first_fingerprint, second_fingerprint,
                "same-content files must remain distinct by volume/file identity"
            ),
            (Err(error), _) | (_, Err(error)) => assert!(
                error.to_string().contains("identity"),
                "identity retrieval may fail closed only with an explicit identity error: {error}"
            ),
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
                    ..PmPrefs::default()
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
            ..PmPrefs::default()
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

    /// SPEC-3431 FR-119/FR-120 / T-484: Grok uses the existing
    /// Claude-compatible managed target, but its PM identity and launch
    /// tuning remain Grok-specific in durable project settings. Older prefs
    /// may omit every optional tuning value and must still deserialize.
    #[test]
    fn grok_launch_profile_is_supported_and_preserves_optional_launch_tuning() {
        let (_dir, path) = temp_prefs_path();
        let configured = PmLaunchProfile {
            agent_id: "grok".to_string(),
            model: Some("grok-4.20-beta".to_string()),
            reasoning: Some("xhigh".to_string()),
            version: None,
        };
        let prefs = PmPrefs {
            settings: PmSettings {
                launch_profile: Some(configured.clone()),
                ..PmSettings::default()
            },
            ..PmPrefs::default()
        };

        assert!(
            pm_agent_is_supported("grok"),
            "Grok is a valid PM agent through the Claude-compatible managed target"
        );
        assert_eq!(prefs.settings.launch_profile_or_default(), configured);

        save_pm_prefs(&path, &prefs).expect("save Grok PM profile");
        let reloaded = load_pm_prefs(&path).expect("reload Grok PM profile");
        assert_eq!(
            reloaded.settings.launch_profile,
            prefs.settings.launch_profile
        );

        let legacy: PmPrefs = serde_json::from_str(
            r#"{"settings":{"auto_start":true,"launch_profile":{"agent_id":"grok"}}}"#,
        )
        .expect("legacy profile with omitted optional tuning remains readable");
        assert_eq!(
            legacy.settings.launch_profile_or_default(),
            PmLaunchProfile {
                agent_id: "grok".to_string(),
                model: None,
                reasoning: None,
                version: None,
            }
        );
    }

    #[test]
    fn launch_profile_normalizes_model_and_reasoning_whitespace() {
        let settings = PmSettings {
            launch_profile: Some(PmLaunchProfile {
                agent_id: "codex".to_string(),
                model: Some("  gpt-5.6  ".to_string()),
                reasoning: Some("  xhigh  ".to_string()),
                version: None,
            }),
            ..PmSettings::default()
        };
        let normalized = settings.launch_profile_or_default();
        assert_eq!(normalized.model.as_deref(), Some("gpt-5.6"));
        assert_eq!(normalized.reasoning.as_deref(), Some("xhigh"));

        let blank = PmSettings {
            launch_profile: Some(PmLaunchProfile {
                agent_id: "codex".to_string(),
                model: Some(" \t ".to_string()),
                reasoning: Some("  ".to_string()),
                version: None,
            }),
            ..PmSettings::default()
        }
        .launch_profile_or_default();
        assert_eq!(blank.model, None);
        assert_eq!(blank.reasoning, None);

        let grok_auto = PmSettings {
            launch_profile: Some(PmLaunchProfile {
                agent_id: "grok".to_string(),
                model: None,
                reasoning: Some(" AUTO ".to_string()),
                version: None,
            }),
            ..PmSettings::default()
        }
        .launch_profile_or_default();
        assert_eq!(grok_auto.reasoning, None);
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
    fn pm_scratch_paths_use_project_state_and_only_accept_the_canonical_pm_worktree() {
        let home = tempfile::tempdir().expect("tempdir");
        let _home_guard = gwt_core::test_support::ScopedGwtHome::set(home.path());

        let repo = Path::new("/tmp/pm-scratch-repo");
        let expected =
            gwt_core::paths::gwt_project_dir_for_repo_path(repo).join("project-state/pm-scratch");
        let worktree = pm_worktree_path_for_repo_path(repo);

        assert_eq!(pm_scratch_dir_for_repo_path(repo), expected);
        assert_eq!(
            pm_scratch_dir_for_pm_worktree(&worktree),
            Some(expected.clone())
        );
        assert_eq!(
            pm_scratch_dir_for_pm_worktree(
                worktree
                    .parent()
                    .expect("pm directory")
                    .join("worktree-copy")
                    .as_path()
            ),
            None,
            "a sibling of the canonical PM worktree must not resolve scratch"
        );
        assert_eq!(
            pm_scratch_dir_for_pm_worktree(Path::new("/tmp/project/pm/worktree")),
            None,
            "a production branch path named pm/worktree must not resolve scratch"
        );
    }

    #[test]
    fn pm_scratch_migration_preserves_known_notes_and_leaves_unknown_files_untouched() {
        let home = tempfile::tempdir().expect("tempdir");
        let _home_guard = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let repo = Path::new("/tmp/pm-scratch-migration-repo");
        let worktree = pm_worktree_path_for_repo_path(repo);
        let scratch = pm_scratch_dir_for_repo_path(repo);

        fs::create_dir_all(worktree.join("tasks")).expect("create legacy tasks directory");
        let known = [
            ("tasks/todo.md", b"todo contents\n".as_slice()),
            ("tasks/pm-notes.md", b"task note contents\n".as_slice()),
            ("pm-notes.md", b"root note contents\n".as_slice()),
        ];
        for (relative, contents) in known {
            fs::write(worktree.join(relative), contents).expect("write legacy scratch");
        }
        fs::write(worktree.join("tasks/unknown.md"), b"unknown task note\n")
            .expect("write unknown task file");
        fs::write(worktree.join("unknown.txt"), b"unknown root file\n")
            .expect("write unknown root file");

        assert_eq!(
            migrate_legacy_pm_scratch(&worktree).expect("migrate legacy PM scratch"),
            3
        );

        for (relative, contents) in known {
            assert_eq!(
                fs::read(scratch.join(relative)).expect("read migrated scratch"),
                contents
            );
            assert!(
                !worktree.join(relative).exists(),
                "migrated source must be removed: {relative}"
            );
        }
        assert_eq!(
            fs::read(worktree.join("tasks/unknown.md")).expect("unknown task file remains"),
            b"unknown task note\n"
        );
        assert_eq!(
            fs::read(worktree.join("unknown.txt")).expect("unknown root file remains"),
            b"unknown root file\n"
        );
        assert!(!scratch.join("tasks/unknown.md").exists());
        assert!(!scratch.join("unknown.txt").exists());
        assert_no_pm_scratch_quarantine_residue(home.path());
    }

    #[test]
    fn pm_scratch_migration_keeps_clean_tracked_allowlist_file_in_worktree() {
        let home = tempfile::tempdir().expect("tempdir");
        let _home_guard = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let repo = Path::new("/tmp/pm-scratch-clean-tracked-repo");
        let worktree = pm_worktree_path_for_repo_path(repo);
        let scratch = pm_scratch_dir_for_repo_path(repo);
        let relative = "tasks/todo.md";
        let committed_contents = b"tracked project task\n";
        commit_tracked_pm_scratch(&worktree, relative, committed_contents);
        let status = run_pm_scratch_git(&worktree, &["status", "--porcelain", "--", relative]);
        assert!(
            status.stdout.is_empty(),
            "tracked allowlist fixture must be clean before migration: {}",
            String::from_utf8_lossy(&status.stdout)
        );

        assert_eq!(
            migrate_legacy_pm_scratch(&worktree).expect("inspect clean tracked PM scratch"),
            0,
            "an allowlisted path that is clean in HEAD is project content, not legacy PM scratch"
        );
        assert_eq!(
            fs::read(worktree.join(relative)).expect("clean tracked file remains"),
            committed_contents
        );
        assert!(
            !scratch.join(relative).exists(),
            "clean tracked project content must not be copied into external PM scratch"
        );
        assert_no_pm_scratch_quarantine_residue(home.path());
    }

    #[test]
    fn pm_scratch_migration_moves_modified_tracked_allowlist_file_to_external_scratch() {
        let home = tempfile::tempdir().expect("tempdir");
        let _home_guard = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let repo = Path::new("/tmp/pm-scratch-modified-tracked-repo");
        let worktree = pm_worktree_path_for_repo_path(repo);
        let scratch = pm_scratch_dir_for_repo_path(repo);
        let relative = "tasks/todo.md";
        commit_tracked_pm_scratch(&worktree, relative, b"tracked project task\n");
        let modified_contents = b"PM-local task update\n";
        fs::write(worktree.join(relative), modified_contents)
            .expect("modify tracked allowlist fixture");
        let status = run_pm_scratch_git(&worktree, &["status", "--porcelain", "--", relative]);
        assert_eq!(
            String::from_utf8_lossy(&status.stdout),
            " M tasks/todo.md\n",
            "positive fixture must be a locally modified tracked allowlist file"
        );

        assert_eq!(
            migrate_legacy_pm_scratch(&worktree).expect("migrate modified tracked PM scratch"),
            1
        );
        assert!(
            !worktree.join(relative).exists(),
            "locally modified tracked legacy scratch must be removed after durable migration"
        );
        assert_eq!(
            fs::read(scratch.join(relative)).expect("read migrated modified scratch"),
            modified_contents
        );
        assert_no_pm_scratch_quarantine_residue(home.path());
    }

    #[test]
    fn preserving_pm_scratch_migration_rejects_staged_and_unstaged_versions_without_mutation() {
        let home = tempfile::tempdir().expect("tempdir");
        let _home_guard = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let repo = Path::new("/tmp/pm-scratch-staged-and-unstaged-repo");
        let worktree = pm_worktree_path_for_repo_path(repo);
        let scratch = pm_scratch_dir_for_repo_path(repo);
        let relative = "tasks/todo.md";
        commit_tracked_pm_scratch(&worktree, relative, b"tracked project task\n");
        fs::write(worktree.join(relative), b"staged PM-local update\n")
            .expect("write staged legacy scratch fixture");
        run_pm_scratch_git(&worktree, &["add", "--", relative]);
        let index_before = run_pm_scratch_git(&worktree, &["write-tree"]).stdout;
        fs::write(worktree.join(relative), b"unstaged PM-local update\n")
            .expect("write unstaged legacy scratch fixture");
        let working_bytes_before = fs::read(worktree.join(relative)).expect("read working bytes");

        migrate_legacy_pm_scratch_preserving_project_content(&worktree)
            .expect_err("two distinct local versions cannot be represented by one scratch file");

        assert_eq!(
            run_pm_scratch_git(&worktree, &["write-tree"]).stdout,
            index_before,
            "the staged version must remain in the index"
        );
        assert_eq!(
            fs::read(worktree.join(relative)).expect("read preserved working bytes"),
            working_bytes_before,
            "the unstaged version must remain in the worktree"
        );
        assert!(
            !scratch.join(relative).exists(),
            "a rejected migration must not publish a partial scratch copy"
        );
    }

    #[test]
    fn pm_scratch_migration_collision_preserves_source_and_destination() {
        let home = tempfile::tempdir().expect("tempdir");
        let _home_guard = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let repo = Path::new("/tmp/pm-scratch-collision-repo");
        let worktree = pm_worktree_path_for_repo_path(repo);
        let scratch = pm_scratch_dir_for_repo_path(repo);
        let known = [
            ("tasks/todo.md", b"todo source\n".as_slice()),
            ("tasks/pm-notes.md", b"notes source\n".as_slice()),
            ("pm-notes.md", b"root source\n".as_slice()),
        ];
        for (relative, contents) in known {
            let source = worktree.join(relative);
            fs::create_dir_all(source.parent().expect("source parent"))
                .expect("create source parent");
            fs::write(source, contents).expect("write source");
        }
        let collision = scratch.join("tasks/pm-notes.md");
        fs::create_dir_all(collision.parent().expect("destination parent"))
            .expect("create destination parent");
        fs::write(&collision, b"existing destination\n").expect("write destination");

        migrate_legacy_pm_scratch(&worktree).expect_err("a destination collision must fail closed");
        for (relative, contents) in known {
            assert_eq!(
                fs::read(worktree.join(relative)).expect("collision source remains"),
                contents,
                "all legacy sources must remain unchanged: {relative}"
            );
            if relative != "tasks/pm-notes.md" {
                assert!(
                    !scratch.join(relative).exists(),
                    "migration must not partially move another source: {relative}"
                );
            }
        }
        assert_eq!(
            fs::read(&collision).expect("collision destination remains"),
            b"existing destination\n"
        );
    }

    #[test]
    fn pm_scratch_staging_failure_rolls_back_destinations_and_preserves_all_sources() {
        let home = tempfile::tempdir().expect("tempdir");
        let _home_guard = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let repo = Path::new("/tmp/pm-scratch-stage-failure-repo");
        let worktree = pm_worktree_path_for_repo_path(repo);
        let scratch = pm_scratch_dir_for_repo_path(repo);
        let known = [
            ("tasks/todo.md", b"todo source\n".as_slice()),
            ("tasks/pm-notes.md", b"notes source\n".as_slice()),
            ("pm-notes.md", b"root source\n".as_slice()),
        ];
        for (relative, contents) in known {
            let source = worktree.join(relative);
            fs::create_dir_all(source.parent().expect("source parent"))
                .expect("create source parent");
            fs::write(source, contents).expect("write source");
        }

        let mut stages = 0;
        migrate_legacy_pm_scratch_with(&worktree, |source: &Path, destination: &Path| {
            stages += 1;
            if stages == 2 {
                assert_eq!(
                    fs::read(scratch.join("tasks/todo.md")).expect("first staged destination"),
                    b"todo source\n",
                    "the first destination must be durably staged before the injected failure"
                );
                assert_eq!(
                    fs::read(worktree.join("tasks/todo.md")).expect("first source remains staged"),
                    b"todo source\n",
                    "staging must not remove a source"
                );
                return Err(io::Error::other("injected second-stage failure"));
            }
            durably_copy_pm_scratch_no_replace(source, destination)
        })
        .expect_err("a staging failure must fail the whole migration");
        assert_eq!(stages, 2, "the failure must occur on the second stage");

        for (relative, contents) in known {
            assert_eq!(
                fs::read(worktree.join(relative)).expect("source remains after rollback"),
                contents,
                "all sources must remain unchanged after staging rollback: {relative}"
            );
            assert!(
                !scratch.join(relative).exists(),
                "every destination created by this attempt must be rolled back: {relative}"
            );
        }
        assert_no_pm_scratch_quarantine_residue(home.path());
    }

    #[test]
    fn pm_scratch_remove_failure_restores_sources_and_rolls_back_destinations() {
        let home = tempfile::tempdir().expect("tempdir");
        let _home_guard = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let repo = Path::new("/tmp/pm-scratch-remove-failure-repo");
        let worktree = pm_worktree_path_for_repo_path(repo);
        let scratch = pm_scratch_dir_for_repo_path(repo);
        let known = [
            ("tasks/todo.md", b"todo source\n".as_slice()),
            ("tasks/pm-notes.md", b"notes source\n".as_slice()),
            ("pm-notes.md", b"root source\n".as_slice()),
        ];
        for (relative, contents) in known {
            let source = worktree.join(relative);
            fs::create_dir_all(source.parent().expect("source parent"))
                .expect("create source parent");
            fs::write(source, contents).expect("write source");
        }

        let unknown_directory = worktree.join("tasks/unknown-directory");
        let unknown_marker = unknown_directory.join("marker.txt");
        fs::create_dir(&unknown_directory).expect("create unknown directory");
        fs::write(&unknown_marker, b"unknown marker\n").expect("write unknown marker");
        #[cfg(unix)]
        let (unknown_link, unknown_link_target) = {
            let link = worktree.join("tasks/unknown-link");
            let target = PathBuf::from("unknown-directory/marker.txt");
            std::os::unix::fs::symlink(&target, &link).expect("create unknown symlink");
            (link, target)
        };

        let first_source = worktree.join("tasks/todo.md");
        let second_source = worktree.join("tasks/pm-notes.md");
        let mut before_remove_calls = 0;
        migrate_legacy_pm_scratch_with_ops(
            &worktree,
            durably_copy_pm_scratch_no_replace,
            |source: &Path| {
                before_remove_calls += 1;
                match before_remove_calls {
                    1 => {
                        assert_eq!(source, first_source);
                        Ok(())
                    }
                    2 => {
                        assert_eq!(source, second_source);
                        assert!(
                            !first_source.exists(),
                            "the first source must have been removed before the injected failure"
                        );
                        assert!(
                            second_source.exists(),
                            "the failing remove must not delete its source"
                        );
                        Err(io::Error::other("injected second-remove failure"))
                    }
                    _ => panic!("migration must stop after the injected remove failure"),
                }
            },
        )
        .expect_err("a remove failure must fail the whole migration");
        assert_eq!(
            before_remove_calls, 2,
            "the failure must occur before the second internal remove"
        );

        for (relative, contents) in known {
            assert_eq!(
                fs::read(worktree.join(relative)).expect("source restored after remove rollback"),
                contents,
                "all sources must retain their original bytes after remove rollback: {relative}"
            );
            assert!(
                !scratch.join(relative).exists(),
                "every staged destination must be rolled back: {relative}"
            );
        }
        assert!(unknown_directory.is_dir());
        assert_eq!(
            fs::read(&unknown_marker).expect("unknown marker remains"),
            b"unknown marker\n"
        );
        #[cfg(unix)]
        {
            assert!(fs::symlink_metadata(&unknown_link)
                .expect("unknown link metadata")
                .file_type()
                .is_symlink());
            assert_eq!(
                fs::read_link(&unknown_link).expect("unknown link target"),
                unknown_link_target
            );
        }
        assert_no_pm_scratch_quarantine_residue(home.path());
    }

    #[cfg(unix)]
    #[test]
    fn pm_scratch_recovery_rejects_same_content_source_identity_handoff() {
        use std::os::unix::fs::MetadataExt;

        let home = tempfile::tempdir().expect("tempdir");
        let _home_guard = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let repo = Path::new("/tmp/pm-scratch-recovery-identity-handoff-repo");
        let worktree = pm_worktree_path_for_repo_path(repo);
        let scratch = pm_scratch_dir_for_repo_path(repo);
        let known = [
            ("tasks/todo.md", b"todo source\n".as_slice()),
            ("tasks/pm-notes.md", b"notes source\n".as_slice()),
            ("pm-notes.md", b"root source\n".as_slice()),
        ];
        for (relative, contents) in known {
            let source = worktree.join(relative);
            fs::create_dir_all(source.parent().expect("source parent"))
                .expect("create source parent");
            fs::write(source, contents).expect("write source");
        }

        let first_source = worktree.join(known[0].0);
        let second_source = worktree.join(known[1].0);
        let first_backup = scratch.join(known[0].0);
        let prepared_competitor = home.path().join("prepared-recovery-competitor.md");
        fs::write(&prepared_competitor, known[0].1).expect("write prepared competitor");
        let competitor_inode = fs::symlink_metadata(&prepared_competitor)
            .expect("prepared competitor metadata")
            .ino();
        let mut before_remove_calls = 0;
        let mut recovery_copy_calls = 0;

        let error = migrate_legacy_pm_scratch_with_all_ops(
            &worktree,
            durably_copy_pm_scratch_no_replace,
            |source: &Path| -> io::Result<()> {
                before_remove_calls += 1;
                match before_remove_calls {
                    1 => {
                        assert_eq!(source, first_source);
                        Ok(())
                    }
                    2 => {
                        assert_eq!(source, second_source);
                        assert!(!first_source.exists());
                        Err(io::Error::other("injected benign second-remove failure"))
                    }
                    _ => panic!("migration must stop at the second before-remove hook"),
                }
            },
            |backup: &Path, restored_source: &Path| -> io::Result<PmScratchFileFingerprint> {
                recovery_copy_calls += 1;
                assert_eq!(recovery_copy_calls, 1);
                assert_eq!(backup, first_backup);
                assert_eq!(restored_source, first_source);
                let fingerprint = durably_copy_pm_scratch_no_replace(backup, restored_source)?;
                assert_ne!(
                    fs::symlink_metadata(restored_source)?.ino(),
                    competitor_inode,
                    "the restored source and prepared competitor must be distinct nodes"
                );
                fs::remove_file(restored_source)?;
                fs::rename(&prepared_competitor, restored_source)?;
                Ok(fingerprint)
            },
        )
        .expect_err("recovery must reject a same-content restored source identity handoff");

        assert_eq!(before_remove_calls, 2);
        assert_eq!(recovery_copy_calls, 1);
        let error_text = error.to_string();
        assert!(
            error_text.contains("identity") || error_text.contains("replaced"),
            "the error must explain the recovery identity violation: {error_text}"
        );
        assert!(
            error_text.contains(&first_backup.display().to_string()),
            "the error must identify the preserved recovery backup: {error_text}"
        );
        assert_eq!(
            fs::read(&first_source).expect("competitor source remains"),
            known[0].1
        );
        assert_eq!(
            fs::symlink_metadata(&first_source)
                .expect("competitor source metadata")
                .ino(),
            competitor_inode
        );
        assert!(!prepared_competitor.exists());
        assert_eq!(
            fs::read(&first_backup).expect("original recovery backup remains"),
            known[0].1
        );
        for (relative, contents) in known.into_iter().skip(1) {
            assert_eq!(
                fs::read(worktree.join(relative)).expect("unaffected source remains"),
                contents
            );
            assert!(
                !scratch.join(relative).exists(),
                "unaffected backup must be rolled back: {relative}"
            );
        }
        assert_no_pm_scratch_quarantine_residue(home.path());
    }

    #[test]
    fn pm_scratch_before_remove_replacement_preserves_competitor_and_staged_backups() {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;

        let home = tempfile::tempdir().expect("tempdir");
        let _home_guard = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let repo = Path::new("/tmp/pm-scratch-before-remove-replacement-repo");
        let worktree = pm_worktree_path_for_repo_path(repo);
        let scratch = pm_scratch_dir_for_repo_path(repo);
        let known = [
            ("tasks/todo.md", b"todo source\n".as_slice()),
            ("tasks/pm-notes.md", b"notes source\n".as_slice()),
            ("pm-notes.md", b"root source\n".as_slice()),
        ];
        for (relative, contents) in known {
            let source = worktree.join(relative);
            fs::create_dir_all(source.parent().expect("source parent"))
                .expect("create source parent");
            fs::write(source, contents).expect("write source");
        }
        let unknown = worktree.join("tasks/unknown.md");
        fs::write(&unknown, b"unknown source bytes\n").expect("write unknown source");

        let first_source = worktree.join("tasks/todo.md");
        let replaced_source = worktree.join("tasks/pm-notes.md");
        let prepared_replacement = home.path().join("prepared-before-remove-replacement.md");
        fs::write(&prepared_replacement, b"replacement after precommit")
            .expect("write prepared replacement");
        #[cfg(unix)]
        let original_source_inode = fs::symlink_metadata(&replaced_source)
            .expect("original source metadata")
            .ino();
        #[cfg(unix)]
        let prepared_replacement_inode = fs::symlink_metadata(&prepared_replacement)
            .expect("prepared replacement metadata")
            .ino();
        #[cfg(unix)]
        assert_ne!(original_source_inode, prepared_replacement_inode);

        let mut before_remove_calls = 0;
        let error = migrate_legacy_pm_scratch_with_ops(
            &worktree,
            durably_copy_pm_scratch_no_replace,
            |source: &Path| {
                before_remove_calls += 1;
                match before_remove_calls {
                    1 => {
                        assert_eq!(source, first_source);
                        Ok(())
                    }
                    2 => {
                        assert_eq!(source, replaced_source);
                        fs::remove_file(source)?;
                        fs::rename(&prepared_replacement, source)?;
                        Ok(())
                    }
                    _ => panic!("migration must abort after the second source is replaced"),
                }
            },
        )
        .expect_err("a source replaced immediately before quarantine must fail closed");
        assert_eq!(before_remove_calls, 2);

        assert_eq!(
            fs::read(&first_source).expect("first source restored"),
            known[0].1,
            "the already removed first source must be restored from its staged backup"
        );
        assert_eq!(
            fs::read(&replaced_source).expect("replacement source remains"),
            b"replacement after precommit"
        );
        #[cfg(unix)]
        {
            let current_inode = fs::symlink_metadata(&replaced_source)
                .expect("replacement source metadata")
                .ino();
            assert_eq!(current_inode, prepared_replacement_inode);
            assert_ne!(current_inode, original_source_inode);
        }
        assert_eq!(
            fs::read(worktree.join(known[2].0)).expect("third source remains"),
            known[2].1,
            "the third source must remain unchanged"
        );
        let preserved_backup = scratch.join("tasks/pm-notes.md");
        assert!(
            error
                .to_string()
                .contains(&preserved_backup.display().to_string()),
            "the error must identify the preserved durable backup: {error}"
        );
        assert_eq!(
            fs::read(&preserved_backup).expect("affected source backup remains"),
            known[1].1,
            "the affected source must retain its durable staged backup"
        );
        assert!(
            !scratch.join(known[0].0).exists(),
            "the restored first source backup must be rolled back"
        );
        assert!(
            !scratch.join(known[2].0).exists(),
            "the untouched third source backup must be rolled back"
        );
        assert_eq!(
            fs::read(&unknown).expect("unknown source remains"),
            b"unknown source bytes\n"
        );
        assert!(!scratch.join("tasks/unknown.md").exists());
        assert_no_pm_scratch_quarantine_residue(home.path());
    }

    #[test]
    fn pm_scratch_source_replacement_after_staging_aborts_without_deleting_replacement() {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;

        let home = tempfile::tempdir().expect("tempdir");
        let _home_guard = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let repo = Path::new("/tmp/pm-scratch-source-replacement-repo");
        let worktree = pm_worktree_path_for_repo_path(repo);
        let scratch = pm_scratch_dir_for_repo_path(repo);
        let known = [
            ("tasks/todo.md", b"todo source\n".as_slice()),
            ("tasks/pm-notes.md", b"notes source\n".as_slice()),
            ("pm-notes.md", b"root source\n".as_slice()),
        ];
        for (relative, contents) in known {
            let source = worktree.join(relative);
            fs::create_dir_all(source.parent().expect("source parent"))
                .expect("create source parent");
            fs::write(source, contents).expect("write source");
        }
        let unknown = worktree.join("tasks/unknown.md");
        fs::write(&unknown, b"unknown source bytes\n").expect("write unknown source");

        let first_source = worktree.join("tasks/todo.md");
        let prepared_replacement = home.path().join("prepared-source-replacement.md");
        fs::write(&prepared_replacement, b"replacement unknown bytes")
            .expect("write prepared replacement");
        #[cfg(unix)]
        let original_source_inode = fs::symlink_metadata(&first_source)
            .expect("original source metadata")
            .ino();
        #[cfg(unix)]
        let prepared_replacement_inode = fs::symlink_metadata(&prepared_replacement)
            .expect("prepared replacement metadata")
            .ino();
        #[cfg(unix)]
        assert_ne!(
            original_source_inode, prepared_replacement_inode,
            "the prepared replacement must be a distinct filesystem node"
        );

        let mut stages = 0;
        let error =
            migrate_legacy_pm_scratch_with(&worktree, |source: &Path, destination: &Path| {
                let fingerprint = durably_copy_pm_scratch_no_replace(source, destination)?;
                stages += 1;
                if stages == known.len() {
                    fs::remove_file(&first_source)?;
                    fs::rename(&prepared_replacement, &first_source)?;
                }
                Ok(fingerprint)
            })
            .expect_err("replacing a source after staging must abort the migration");
        assert_eq!(stages, known.len(), "all sources must have been staged");

        assert_eq!(
            fs::read(&first_source).expect("replacement source remains"),
            b"replacement unknown bytes",
            "the replacement must never be deleted or overwritten"
        );
        assert!(
            !prepared_replacement.exists(),
            "the prepared replacement must have been renamed into the source path"
        );
        #[cfg(unix)]
        {
            let replacement_inode = fs::symlink_metadata(&first_source)
                .expect("replacement source metadata")
                .ino();
            assert_eq!(replacement_inode, prepared_replacement_inode);
            assert_ne!(replacement_inode, original_source_inode);
        }
        for (relative, contents) in known.into_iter().skip(1) {
            assert_eq!(
                fs::read(worktree.join(relative)).expect("unchanged source remains"),
                contents,
                "an untouched source must retain its original bytes: {relative}"
            );
        }
        let preserved_backup = scratch.join("tasks/todo.md");
        assert!(
            error
                .to_string()
                .contains(&preserved_backup.display().to_string()),
            "the error must identify the preserved durable backup: {error}"
        );
        assert_eq!(
            fs::read(&preserved_backup).expect("affected source backup remains"),
            known[0].1,
            "the affected source must retain its durable staged backup"
        );
        for (relative, _) in known.into_iter().skip(1) {
            assert!(
                !scratch.join(relative).exists(),
                "an unaffected source backup must be rolled back: {relative}"
            );
        }
        assert_eq!(
            fs::read(&unknown).expect("unknown source remains"),
            b"unknown source bytes\n"
        );
        assert!(!scratch.join("tasks/unknown.md").exists());
        assert_no_pm_scratch_quarantine_residue(home.path());
    }

    #[cfg(unix)]
    #[test]
    fn pm_scratch_stage_rejects_same_content_destination_identity_handoff() {
        use std::os::unix::fs::MetadataExt;

        let home = tempfile::tempdir().expect("tempdir");
        let _home_guard = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let repo = Path::new("/tmp/pm-scratch-stage-identity-handoff-repo");
        let worktree = pm_worktree_path_for_repo_path(repo);
        let scratch = pm_scratch_dir_for_repo_path(repo);
        let known = [
            ("tasks/todo.md", b"todo source\n".as_slice()),
            ("tasks/pm-notes.md", b"notes source\n".as_slice()),
            ("pm-notes.md", b"root source\n".as_slice()),
        ];
        for (relative, contents) in known {
            let source = worktree.join(relative);
            fs::create_dir_all(source.parent().expect("source parent"))
                .expect("create source parent");
            fs::write(source, contents).expect("write source");
        }

        let first_destination = scratch.join(known[0].0);
        let prepared_competitor = home.path().join("prepared-same-content-competitor.md");
        fs::write(&prepared_competitor, known[0].1).expect("write prepared competitor");
        let competitor_inode = fs::symlink_metadata(&prepared_competitor)
            .expect("prepared competitor metadata")
            .ino();
        let mut stages = 0;

        let error = migrate_legacy_pm_scratch_with(
            &worktree,
            |source: &Path, destination: &Path| -> io::Result<PmScratchFileFingerprint> {
                let fingerprint = durably_copy_pm_scratch_no_replace(source, destination)?;
                stages += 1;
                if stages == 1 {
                    assert_eq!(destination, first_destination);
                    assert_ne!(
                        fs::symlink_metadata(destination)?.ino(),
                        competitor_inode,
                        "the durable-copy node and prepared competitor must be distinct"
                    );
                    fs::remove_file(destination)?;
                    fs::rename(&prepared_competitor, destination)?;
                }
                Ok(fingerprint)
            },
        )
        .expect_err("migration must reject a same-content staged destination identity handoff");

        let error_text = error.to_string();
        assert!(
            error_text.contains("identity") || error_text.contains("replaced"),
            "the error must explain the staged identity violation: {error_text}"
        );
        assert!(
            error_text.contains(&first_destination.display().to_string()),
            "the error must identify the replaced destination: {error_text}"
        );
        for (relative, contents) in known {
            assert_eq!(
                fs::read(worktree.join(relative)).expect("source remains"),
                contents,
                "identity handoff rejection must preserve every source: {relative}"
            );
        }
        assert_eq!(
            fs::read(&first_destination).expect("competitor destination remains"),
            known[0].1
        );
        assert_eq!(
            fs::symlink_metadata(&first_destination)
                .expect("competitor destination metadata")
                .ino(),
            competitor_inode
        );
        assert!(!prepared_competitor.exists());
        for (relative, _) in known.into_iter().skip(1) {
            assert!(
                !scratch.join(relative).exists(),
                "unaffected staged destination must be rolled back: {relative}"
            );
        }
        assert_no_pm_scratch_quarantine_residue(home.path());
    }

    #[test]
    fn pm_scratch_stage_rollback_preserves_a_replaced_destination_node() {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;

        let home = tempfile::tempdir().expect("tempdir");
        let _home_guard = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let repo = Path::new("/tmp/pm-scratch-destination-replacement-repo");
        let worktree = pm_worktree_path_for_repo_path(repo);
        let scratch = pm_scratch_dir_for_repo_path(repo);
        let known = [
            ("tasks/todo.md", b"todo source\n".as_slice()),
            ("tasks/pm-notes.md", b"notes source\n".as_slice()),
            ("pm-notes.md", b"root source\n".as_slice()),
        ];
        for (relative, contents) in known {
            let source = worktree.join(relative);
            fs::create_dir_all(source.parent().expect("source parent"))
                .expect("create source parent");
            fs::write(source, contents).expect("write source");
        }

        let first_destination = scratch.join("tasks/todo.md");
        let prepared_competitor = home.path().join("prepared-destination-competitor.md");
        fs::write(&prepared_competitor, b"competitor destination bytes")
            .expect("write prepared competitor");
        #[cfg(unix)]
        let competitor_inode = fs::symlink_metadata(&prepared_competitor)
            .expect("prepared competitor metadata")
            .ino();
        #[cfg(unix)]
        let mut staged_destination_inode = None;

        let mut stages = 0;
        migrate_legacy_pm_scratch_with(&worktree, |source: &Path, destination: &Path| {
            stages += 1;
            match stages {
                1 => {
                    assert_eq!(destination, first_destination);
                    let fingerprint = durably_copy_pm_scratch_no_replace(source, destination)?;
                    #[cfg(unix)]
                    {
                        let inode = fs::symlink_metadata(destination)
                            .expect("first staged destination metadata")
                            .ino();
                        assert_ne!(
                            inode, competitor_inode,
                            "the staged destination and prepared competitor must be distinct nodes"
                        );
                        staged_destination_inode = Some(inode);
                    }
                    Ok(fingerprint)
                }
                2 => {
                    fs::remove_file(&first_destination)?;
                    fs::rename(&prepared_competitor, &first_destination)?;
                    #[cfg(unix)]
                    {
                        let replacement_inode = fs::symlink_metadata(&first_destination)
                            .expect("competitor destination metadata")
                            .ino();
                        assert_eq!(replacement_inode, competitor_inode);
                        assert_ne!(
                            replacement_inode,
                            staged_destination_inode.expect("first staged destination inode")
                        );
                    }
                    Err(io::Error::other(
                        "injected second-stage failure after destination replacement",
                    ))
                }
                _ => panic!("migration must stop after the injected staging failure"),
            }
        })
        .expect_err("a staging failure after destination replacement must abort migration");
        assert_eq!(stages, 2, "the failure must occur on the second stage");

        for (relative, contents) in known {
            assert_eq!(
                fs::read(worktree.join(relative)).expect("source remains after staging failure"),
                contents,
                "staging failure must leave every source unchanged: {relative}"
            );
        }
        assert!(fs::symlink_metadata(&first_destination)
            .expect("competitor destination remains")
            .file_type()
            .is_file());
        assert_eq!(
            fs::read(&first_destination).expect("read competitor destination"),
            b"competitor destination bytes"
        );
        assert!(
            !prepared_competitor.exists(),
            "the prepared competitor must have been renamed into the destination path"
        );
        #[cfg(unix)]
        assert_eq!(
            fs::symlink_metadata(&first_destination)
                .expect("competitor destination metadata after rollback")
                .ino(),
            competitor_inode
        );
        for (relative, _) in known.into_iter().skip(1) {
            assert!(
                !scratch.join(relative).exists(),
                "no other destination may remain after staging rollback: {relative}"
            );
        }
    }

    #[test]
    fn pm_scratch_durable_copy_rolls_back_an_owned_destination_after_copy_failure() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.md");
        let destination = dir.path().join("destination.md");
        fs::write(&source, b"source bytes\n").expect("write source");

        durably_copy_pm_scratch_no_replace_with_ops(
            &source,
            &destination,
            || Ok(()),
            |_: &mut fs::File, destination_file: &mut fs::File| -> io::Result<u64> {
                destination_file.write_all(b"partial destination")?;
                Err(io::Error::other("injected copy failure"))
            },
            |_: &fs::File| -> io::Result<()> {
                panic!("destination sync must not run after copy failure")
            },
            |_: &Path| -> io::Result<()> { panic!("parent sync must not run after copy failure") },
        )
        .expect_err("a copy failure after create must roll back the owned destination");

        assert_eq!(
            fs::read(&source).expect("source remains"),
            b"source bytes\n"
        );
        assert!(
            !destination.exists(),
            "an owned partial destination must be removed through guarded quarantine"
        );
        assert_no_pm_scratch_quarantine_residue(dir.path());
    }

    #[test]
    fn pm_scratch_durable_copy_rolls_back_an_owned_destination_after_sync_failure() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.md");
        let destination = dir.path().join("destination.md");
        fs::write(&source, b"source bytes\n").expect("write source");

        durably_copy_pm_scratch_no_replace_with_ops(
            &source,
            &destination,
            || Ok(()),
            |source_file: &mut fs::File, destination_file: &mut fs::File| -> io::Result<u64> {
                io::copy(source_file, destination_file)
            },
            |_: &fs::File| -> io::Result<()> {
                Err(io::Error::other("injected destination sync failure"))
            },
            |_: &Path| -> io::Result<()> {
                panic!("parent sync must not run after destination sync failure")
            },
        )
        .expect_err("a sync failure after create must roll back the owned destination");

        assert_eq!(
            fs::read(&source).expect("source remains"),
            b"source bytes\n"
        );
        assert!(
            !destination.exists(),
            "an owned synced destination must be removed through guarded quarantine"
        );
        assert_no_pm_scratch_quarantine_residue(dir.path());
    }

    #[cfg(unix)]
    #[test]
    fn pm_scratch_durable_copy_preserves_a_destination_replaced_inside_sync_failure() {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.md");
        let destination = dir.path().join("destination.md");
        let prepared_competitor = dir.path().join("prepared-competitor.md");
        fs::write(&source, b"source bytes\n").expect("write source");
        fs::write(&prepared_competitor, b"competitor destination bytes")
            .expect("write prepared competitor");
        #[cfg(unix)]
        let prepared_competitor_inode = fs::symlink_metadata(&prepared_competitor)
            .expect("prepared competitor metadata")
            .ino();

        durably_copy_pm_scratch_no_replace_with_ops(
            &source,
            &destination,
            || Ok(()),
            |source_file: &mut fs::File, destination_file: &mut fs::File| -> io::Result<u64> {
                io::copy(source_file, destination_file)
            },
            |destination_file: &fs::File| -> io::Result<()> {
                destination_file.sync_all()?;
                #[cfg(unix)]
                assert_ne!(
                    destination_file
                        .metadata()
                        .expect("owned destination metadata")
                        .ino(),
                    prepared_competitor_inode,
                    "owned destination and competitor must be distinct nodes"
                );
                fs::remove_file(&destination)?;
                fs::rename(&prepared_competitor, &destination)?;
                Err(io::Error::other(
                    "injected sync failure after destination replacement",
                ))
            },
            |_: &Path| -> io::Result<()> {
                panic!("parent sync must not run after destination sync failure")
            },
        )
        .expect_err("cleanup must not delete a competitor installed during sync failure");

        assert_eq!(
            fs::read(&source).expect("source remains"),
            b"source bytes\n"
        );
        assert_eq!(
            fs::read(&destination).expect("competitor destination remains"),
            b"competitor destination bytes"
        );
        assert!(!prepared_competitor.exists());
        #[cfg(unix)]
        assert_eq!(
            fs::symlink_metadata(&destination)
                .expect("competitor destination metadata")
                .ino(),
            prepared_competitor_inode
        );
        assert_no_pm_scratch_quarantine_residue(dir.path());
    }

    #[cfg(unix)]
    #[test]
    fn pm_scratch_durable_copy_rejects_success_after_destination_identity_replacement() {
        use std::os::unix::fs::MetadataExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.md");
        let destination = dir.path().join("destination.md");
        let prepared_competitor = dir.path().join("prepared-competitor.md");
        fs::write(&source, b"source bytes\n").expect("write source");
        fs::write(&prepared_competitor, b"competitor destination bytes")
            .expect("write prepared competitor");
        let prepared_competitor_inode = fs::symlink_metadata(&prepared_competitor)
            .expect("prepared competitor metadata")
            .ino();

        durably_copy_pm_scratch_no_replace_with_ops(
            &source,
            &destination,
            || Ok(()),
            |source_file: &mut fs::File, destination_file: &mut fs::File| -> io::Result<u64> {
                io::copy(source_file, destination_file)
            },
            |destination_file: &fs::File| destination_file.sync_all(),
            |path: &Path| -> io::Result<()> {
                sync_parent_directory(path)?;
                assert_ne!(
                    fs::symlink_metadata(path)?.ino(),
                    prepared_competitor_inode,
                    "owned destination and competitor must be distinct nodes"
                );
                fs::remove_file(path)?;
                fs::rename(&prepared_competitor, path)?;
                Ok(())
            },
        )
        .expect_err("a replaced canonical destination must never be reported as success");

        assert_eq!(
            fs::read(&source).expect("source remains"),
            b"source bytes\n"
        );
        assert_eq!(
            fs::read(&destination).expect("competitor destination remains"),
            b"competitor destination bytes"
        );
        assert!(!prepared_competitor.exists());
        assert_eq!(
            fs::symlink_metadata(&destination)
                .expect("competitor destination metadata")
                .ino(),
            prepared_competitor_inode
        );
        assert_no_pm_scratch_quarantine_residue(dir.path());
    }

    #[test]
    fn pm_scratch_durable_copy_missing_canonical_destination_does_not_claim_preservation() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.md");
        let destination = dir.path().join("destination.md");
        fs::write(&source, b"source bytes\n").expect("write source");

        let error = durably_copy_pm_scratch_no_replace_with_ops(
            &source,
            &destination,
            || Ok(()),
            |source_file: &mut fs::File, destination_file: &mut fs::File| -> io::Result<u64> {
                io::copy(source_file, destination_file)
            },
            |destination_file: &fs::File| destination_file.sync_all(),
            |path: &Path| -> io::Result<()> {
                sync_parent_directory(path)?;
                fs::remove_file(path)?;
                Ok(())
            },
        )
        .expect_err("a missing canonical destination must fail final inspection");

        let error_text = error.to_string();
        assert!(
            error_text.contains("cannot verify canonical PM scratch destination"),
            "the error must explain the canonical inspection failure: {error_text}"
        );
        assert!(
            error_text.contains(&destination.display().to_string()),
            "the error must identify the missing destination: {error_text}"
        );
        assert!(
            !error_text.contains("canonical destination is preserved"),
            "the error must not claim that a missing canonical destination was preserved: {error_text}"
        );
        assert!(
            !error_text.contains(&format!("data is preserved at {}", destination.display())),
            "the error must not claim that data exists at the missing path: {error_text}"
        );
        assert_eq!(
            fs::read(&source).expect("source remains"),
            b"source bytes\n"
        );
        assert!(!destination.exists());
        assert_no_pm_scratch_quarantine_residue(dir.path());
    }

    #[cfg(unix)]
    #[test]
    fn pm_scratch_quarantine_restore_sync_error_does_not_claim_deleted_node_remains() {
        let dir = tempfile::tempdir().expect("tempdir");
        let original = dir.path().join("original.md");
        let quarantine = create_pm_scratch_quarantine(&original).expect("create quarantine");
        fs::write(&quarantine.node, b"quarantined bytes\n").expect("write quarantine node");
        let expected =
            pm_scratch_file_fingerprint(&quarantine.node, "fingerprint quarantine restore fixture")
                .expect("fingerprint quarantine fixture");

        let error = restore_quarantined_pm_scratch_no_replace_with_sync(
            &quarantine,
            &original,
            Some(&expected),
            |path: &Path| sync_parent_directory(path),
            |path: &Path| -> io::Result<()> {
                assert_eq!(path, quarantine.node);
                assert!(
                    !quarantine.node.exists(),
                    "the quarantine link must be removed before its directory sync"
                );
                Err(io::Error::other(
                    "injected quarantine directory sync failure",
                ))
            },
        )
        .expect_err("a quarantine directory sync failure must remain visible");

        let error_text = error.to_string();
        assert!(
            error_text.contains(&original.display().to_string()),
            "the error must identify the restored original: {error_text}"
        );
        assert!(
            error_text.contains(&quarantine.node.display().to_string()),
            "the error must identify the quarantine path whose sync failed: {error_text}"
        );
        assert!(
            error_text.contains("sync") || error_text.contains("durability"),
            "the error must explain the durability failure: {error_text}"
        );
        assert!(
            !error_text.contains(&format!(
                "data is preserved at {}",
                quarantine.node.display()
            )),
            "the error must not claim that the deleted quarantine node holds data: {error_text}"
        );
        assert!(
            !error_text.contains("remains quarantined"),
            "the error must not claim that the deleted node remains quarantined: {error_text}"
        );
        assert_eq!(
            fs::read(&original).expect("restored original remains"),
            b"quarantined bytes\n"
        );
        assert!(!quarantine.node.exists());
        assert!(
            quarantine.directory.is_dir(),
            "the quarantine directory may remain after its sync fails"
        );
    }

    #[cfg(unix)]
    #[test]
    fn pm_scratch_quarantine_owner_mismatch_sync_error_does_not_claim_deleted_node_remains() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("target.md");
        let expected_fixture = dir.path().join("expected.md");
        fs::write(&target, b"actual target bytes\n").expect("write actual target");
        fs::write(&expected_fixture, b"different expected bytes\n")
            .expect("write expected fixture");
        let expected = pm_scratch_file_fingerprint(
            &expected_fixture,
            "fingerprint mismatched expected fixture",
        )
        .expect("fingerprint expected fixture");
        let mut quarantine_node = None;
        let mut quarantine_directory = None;

        let error = quarantine_and_remove_owned_pm_scratch_file_with_restore(
            &target,
            &expected,
            "test PM scratch ownership mismatch",
            |quarantine: &PmScratchQuarantine,
             original: &Path,
             quarantined_fingerprint: Option<&PmScratchFileFingerprint>|
             -> io::Result<()> {
                assert_eq!(original, target);
                let quarantined_fingerprint = quarantined_fingerprint
                    .expect("ownership mismatch restore must receive the actual fingerprint");
                quarantine_node = Some(quarantine.node.clone());
                quarantine_directory = Some(quarantine.directory.clone());
                restore_quarantined_pm_scratch_no_replace_with_sync(
                    quarantine,
                    original,
                    Some(quarantined_fingerprint),
                    |path: &Path| sync_parent_directory(path),
                    |path: &Path| -> io::Result<()> {
                        assert_eq!(path, quarantine.node);
                        assert!(!quarantine.node.exists());
                        Err(io::Error::other(
                            "injected outer quarantine directory sync failure",
                        ))
                    },
                )
            },
        )
        .expect_err("ownership mismatch restore sync failure must remain visible");

        let quarantine_node = quarantine_node.expect("restore closure captures quarantine node");
        let quarantine_directory =
            quarantine_directory.expect("restore closure captures quarantine directory");
        let error_text = error.to_string();
        assert!(
            error_text.contains(&target.display().to_string()),
            "the outer error must identify the restored original: {error_text}"
        );
        assert!(
            error_text.contains(&quarantine_node.display().to_string()),
            "the outer error must identify the quarantine sync path: {error_text}"
        );
        assert!(
            error_text.contains("sync") || error_text.contains("durability"),
            "the outer error must explain the durability failure: {error_text}"
        );
        assert!(
            !error_text.contains(&format!(
                "data is preserved at {}",
                quarantine_node.display()
            )),
            "the outer error must not claim deleted quarantine data remains: {error_text}"
        );
        assert!(
            !error_text.contains("remains quarantined"),
            "the outer error must not claim the deleted node remains quarantined: {error_text}"
        );
        assert_eq!(
            fs::read(&target).expect("actual target restored"),
            b"actual target bytes\n"
        );
        assert!(!quarantine_node.exists());
        assert!(quarantine_directory.is_dir());
    }

    #[test]
    fn pm_scratch_durable_copy_rolls_back_an_owned_destination_after_parent_sync_failure() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.md");
        let destination = dir.path().join("destination.md");
        fs::write(&source, b"source bytes\n").expect("write source");

        durably_copy_pm_scratch_no_replace_with_ops(
            &source,
            &destination,
            || Ok(()),
            |source_file: &mut fs::File, destination_file: &mut fs::File| -> io::Result<u64> {
                io::copy(source_file, destination_file)
            },
            |destination_file: &fs::File| destination_file.sync_all(),
            |_: &Path| -> io::Result<()> {
                Err(io::Error::other("injected destination parent sync failure"))
            },
        )
        .expect_err("a parent sync failure must roll back the owned destination");

        assert_eq!(
            fs::read(&source).expect("source remains"),
            b"source bytes\n"
        );
        assert!(!destination.exists());
        assert_no_pm_scratch_quarantine_residue(dir.path());
    }

    #[test]
    fn pm_scratch_durable_copy_rejects_an_existing_destination_without_mutation() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.md");
        let destination = dir.path().join("destination.md");
        fs::write(&source, b"source bytes\n").expect("write source");
        fs::write(&destination, b"destination bytes\n").expect("write destination");

        durably_copy_pm_scratch_no_replace(&source, &destination)
            .expect_err("an existing destination must be rejected atomically");

        assert_eq!(
            fs::read(&source).expect("source remains"),
            b"source bytes\n"
        );
        assert_eq!(
            fs::read(&destination).expect("destination remains"),
            b"destination bytes\n"
        );
    }

    #[test]
    fn pm_scratch_durable_copy_rejects_a_destination_created_immediately_before_create_new() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.md");
        let destination = dir.path().join("destination.md");
        fs::write(&source, b"source bytes\n").expect("write source");

        durably_copy_pm_scratch_no_replace_with(&source, &destination, || {
            fs::write(&destination, b"competitor bytes\n")
        })
        .expect_err("create_new must reject a destination won by a competitor");

        assert_eq!(
            fs::read(&source).expect("source remains"),
            b"source bytes\n"
        );
        assert_eq!(
            fs::read(&destination).expect("competitor destination remains"),
            b"competitor bytes\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn pm_scratch_migration_rejects_a_symlinked_source_parent() {
        let home = tempfile::tempdir().expect("tempdir");
        let _home_guard = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let repo = Path::new("/tmp/pm-scratch-source-symlink-repo");
        let worktree = pm_worktree_path_for_repo_path(repo);
        let scratch = pm_scratch_dir_for_repo_path(repo);
        let external = home.path().join("external-source");
        let tasks_link = worktree.join("tasks");
        let original_target = external.clone();
        fs::create_dir_all(&worktree).expect("create worktree");
        fs::create_dir_all(&external).expect("create external source directory");
        fs::write(external.join("todo.md"), b"external source\n").expect("write external source");
        std::os::unix::fs::symlink(&external, &tasks_link).expect("symlink source parent");

        migrate_legacy_pm_scratch(&worktree)
            .expect_err("a symlinked legacy source parent must be rejected");

        assert_eq!(
            fs::read(external.join("todo.md")).expect("external source remains"),
            b"external source\n"
        );
        assert!(!scratch.join("tasks/todo.md").exists());
        assert!(fs::symlink_metadata(&tasks_link)
            .expect("source parent link metadata")
            .file_type()
            .is_symlink());
        assert_eq!(
            fs::read_link(&tasks_link).expect("source parent link target"),
            original_target
        );
    }

    #[cfg(unix)]
    #[test]
    fn pm_scratch_migration_rejects_a_symlinked_destination_parent() {
        let home = tempfile::tempdir().expect("tempdir");
        let _home_guard = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let repo = Path::new("/tmp/pm-scratch-destination-symlink-repo");
        let worktree = pm_worktree_path_for_repo_path(repo);
        let scratch = pm_scratch_dir_for_repo_path(repo);
        let source = worktree.join("tasks/todo.md");
        let external = home.path().join("external-destination");
        let tasks_link = scratch.join("tasks");
        let original_target = external.clone();
        fs::create_dir_all(source.parent().expect("source parent")).expect("create source parent");
        fs::write(&source, b"source bytes\n").expect("write source");
        fs::create_dir_all(&scratch).expect("create scratch root");
        fs::create_dir_all(&external).expect("create external destination directory");
        std::os::unix::fs::symlink(&external, &tasks_link).expect("symlink destination parent");

        migrate_legacy_pm_scratch(&worktree)
            .expect_err("a symlinked scratch destination parent must be rejected");

        assert_eq!(
            fs::read(&source).expect("source remains"),
            b"source bytes\n"
        );
        assert!(!external.join("todo.md").exists());
        assert!(fs::symlink_metadata(&tasks_link)
            .expect("destination parent link metadata")
            .file_type()
            .is_symlink());
        assert_eq!(
            fs::read_link(&tasks_link).expect("destination parent link target"),
            original_target
        );
    }

    #[cfg(unix)]
    #[test]
    fn pm_scratch_migration_rejects_a_symlinked_scratch_root() {
        let home = tempfile::tempdir().expect("tempdir");
        let _home_guard = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let repo = Path::new("/tmp/pm-scratch-root-symlink-repo");
        let worktree = pm_worktree_path_for_repo_path(repo);
        let scratch = pm_scratch_dir_for_repo_path(repo);
        let source = worktree.join("pm-notes.md");
        let external = home.path().join("external-scratch-root");
        let original_target = external.clone();
        fs::create_dir_all(&worktree).expect("create worktree");
        fs::write(&source, b"source bytes\n").expect("write source");
        fs::create_dir_all(scratch.parent().expect("scratch parent"))
            .expect("create scratch parent");
        fs::create_dir_all(&external).expect("create external scratch directory");
        std::os::unix::fs::symlink(&external, &scratch).expect("symlink scratch root");

        migrate_legacy_pm_scratch(&worktree)
            .expect_err("a symlinked scratch root must be rejected");

        assert_eq!(
            fs::read(&source).expect("source remains"),
            b"source bytes\n"
        );
        assert!(!external.join("pm-notes.md").exists());
        assert!(fs::symlink_metadata(&scratch)
            .expect("scratch root link metadata")
            .file_type()
            .is_symlink());
        assert_eq!(
            fs::read_link(&scratch).expect("scratch root link target"),
            original_target
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
    fn pm_worktree_freshness_roundtrips_typed_states_and_legacy_json_defaults_to_none() {
        for legacy in [
            serde_json::json!({}),
            serde_json::json!({"settings": {"auto_start": false}}),
        ] {
            let prefs: PmPrefs = serde_json::from_value(legacy).expect("parse legacy PM prefs");
            assert_eq!(prefs.worktree_freshness, None);
        }

        let cases = [
            serde_json::json!({
                "state": "fresh",
                "base_ref": "origin/develop",
                "head_sha": "aaaaaaaa",
                "target_sha": "aaaaaaaa",
                "behind": 0,
                "target_observation": "fresh",
                "checked_at": "2026-08-14T00:00:00Z"
            }),
            serde_json::json!({
                "state": "stale",
                "base_ref": "origin/develop",
                "head_sha": "aaaaaaaa",
                "target_sha": "bbbbbbbb",
                "behind": 1,
                "target_observation": "cached",
                "checked_at": "2026-08-14T00:01:00Z",
                "failure_stage": "fetch",
                "failure_reason": "network unavailable"
            }),
            serde_json::json!({
                "state": "unknown",
                "base_ref": "origin/develop",
                "head_sha": "aaaaaaaa",
                "target_observation": "unavailable",
                "checked_at": "2026-08-14T00:02:00Z",
                "failure_stage": "inspect",
                "failure_reason": "target ref unavailable"
            }),
        ];

        for expected in cases {
            let (_dir, path) = temp_prefs_path();
            let prefs: PmPrefs = serde_json::from_value(serde_json::json!({
                "worktree_freshness": expected.clone()
            }))
            .expect("deserialize typed PM worktree freshness");
            let freshness: &PmWorktreeFreshness = prefs
                .worktree_freshness
                .as_ref()
                .expect("freshness is present");
            let serialized = serde_json::to_value(freshness).expect("serialize freshness");
            for field in [
                "state",
                "base_ref",
                "head_sha",
                "target_sha",
                "behind",
                "target_observation",
                "checked_at",
                "failure_stage",
                "failure_reason",
            ] {
                match expected.get(field) {
                    Some(expected_value) => assert_eq!(
                        serialized.get(field),
                        Some(expected_value),
                        "present freshness field must round-trip exactly: {field}"
                    ),
                    None => assert!(
                        serialized.get(field).is_none_or(serde_json::Value::is_null),
                        "absent optional freshness field must stay absent or null: {field}"
                    ),
                }
            }

            save_pm_prefs(&path, &prefs).expect("persist freshness");
            assert_eq!(
                load_pm_prefs(&path).expect("reload freshness"),
                prefs,
                "PmPrefs must durably round-trip typed freshness"
            );
        }
    }

    #[test]
    fn pm_worktree_freshness_enums_use_snake_case_and_reject_unknown_values() {
        for (state, expected) in [
            (PmWorktreeFreshnessState::Fresh, "fresh"),
            (PmWorktreeFreshnessState::Stale, "stale"),
            (PmWorktreeFreshnessState::Unknown, "unknown"),
        ] {
            assert_eq!(
                serde_json::to_value(state).expect("serialize freshness state"),
                serde_json::json!(expected)
            );
            assert_eq!(
                serde_json::from_value::<PmWorktreeFreshnessState>(serde_json::json!(expected))
                    .expect("deserialize freshness state"),
                state
            );
        }
        assert!(
            serde_json::from_value::<PmWorktreeFreshnessState>(serde_json::json!("newer")).is_err()
        );

        for (observation, expected) in [
            (PmWorktreeTargetObservation::Fresh, "fresh"),
            (PmWorktreeTargetObservation::Cached, "cached"),
            (PmWorktreeTargetObservation::Unavailable, "unavailable"),
        ] {
            assert_eq!(
                serde_json::to_value(observation).expect("serialize target observation"),
                serde_json::json!(expected)
            );
            assert_eq!(
                serde_json::from_value::<PmWorktreeTargetObservation>(serde_json::json!(expected))
                    .expect("deserialize target observation"),
                observation
            );
        }
        assert!(
            serde_json::from_value::<PmWorktreeTargetObservation>(serde_json::json!("newer"))
                .is_err()
        );

        for (stage, expected) in [
            (PmWorktreeRefreshFailureStage::Fetch, "fetch"),
            (PmWorktreeRefreshFailureStage::Inspect, "inspect"),
            (PmWorktreeRefreshFailureStage::LocalWork, "local_work"),
            (PmWorktreeRefreshFailureStage::Repoint, "repoint"),
            (
                PmWorktreeRefreshFailureStage::ManagedAssets,
                "managed_assets",
            ),
            (
                PmWorktreeRefreshFailureStage::ScratchMigration,
                "scratch_migration",
            ),
        ] {
            assert_eq!(
                serde_json::to_value(stage).expect("serialize refresh failure stage"),
                serde_json::json!(expected)
            );
            assert_eq!(
                serde_json::from_value::<PmWorktreeRefreshFailureStage>(serde_json::json!(
                    expected
                ))
                .expect("deserialize refresh failure stage"),
                stage
            );
        }
        assert!(
            serde_json::from_value::<PmWorktreeRefreshFailureStage>(serde_json::json!("newer"))
                .is_err()
        );
    }

    #[test]
    fn pm_worktree_freshness_propagates_to_status_independently_of_session_stale_hint() {
        let mut prefs: PmPrefs = serde_json::from_value(serde_json::json!({
            "worktree_freshness": {
                "state": "stale",
                "base_ref": "origin/develop",
                "head_sha": "aaaaaaaa",
                "target_sha": "bbbbbbbb",
                "behind": 1,
                "target_observation": "cached",
                "checked_at": "2026-08-14T00:01:00Z",
                "failure_stage": "fetch",
                "failure_reason": "network unavailable"
            }
        }))
        .expect("deserialize stale freshness");
        prefs.registration = Some(registration("live-session"));

        let report = pm_status_report(&prefs, |session_id| {
            assert_eq!(session_id, "live-session");
            true
        });
        assert_eq!(report.worktree_freshness, prefs.worktree_freshness);
        assert_eq!(report.stale_hint, Some(false));
        let freshness: &PmWorktreeFreshness = report
            .worktree_freshness
            .as_ref()
            .expect("status propagates freshness");
        let serialized = serde_json::to_value(freshness).expect("serialize status freshness");
        assert_eq!(serialized["state"], "stale");
        assert_eq!(serialized["target_observation"], "cached");
        assert_eq!(serialized["behind"], 1);
    }

    #[test]
    fn pm_worktree_freshness_stays_fresh_when_the_session_record_is_stale() {
        let mut prefs: PmPrefs = serde_json::from_value(serde_json::json!({
            "worktree_freshness": {
                "state": "fresh",
                "base_ref": "origin/develop",
                "head_sha": "aaaaaaaa",
                "target_sha": "aaaaaaaa",
                "behind": 0,
                "target_observation": "fresh",
                "checked_at": "2026-08-14T00:03:00Z"
            }
        }))
        .expect("deserialize fresh worktree state");
        prefs.registration = Some(registration("missing-session"));

        let report = pm_status_report(&prefs, |session_id| {
            assert_eq!(session_id, "missing-session");
            false
        });

        assert_eq!(report.worktree_freshness, prefs.worktree_freshness);
        assert_eq!(report.stale_hint, Some(true));
        let freshness = report
            .worktree_freshness
            .as_ref()
            .expect("status propagates fresh worktree state");
        assert_eq!(freshness.state, PmWorktreeFreshnessState::Fresh);
    }

    #[test]
    fn status_report_without_registration_omits_liveness_fields() {
        let prefs = PmPrefs {
            registration: None,
            settings: PmSettings {
                auto_start: false,
                ..PmSettings::default()
            },
            ..PmPrefs::default()
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
