//! SPEC #3576: `verify.lease.*` — host-wide serialization of heavy
//! verification runs.
//!
//! Heavy verification (`cargo test --all-features`, `cargo llvm-cov`, headed
//! Playwright, `verify.run`) contends for host CPU, so it claims the same
//! host-wide heavy lease that Project Index jobs already use
//! ([`gwt_core::index_coordinator`]). Nothing here invents a second exclusion
//! mechanism; verification simply becomes another claimant under the existing
//! lock order (target job -> heavy).
//!
//! A lease has to outlive the invocation that asked for it, and its liveness
//! must stay a kernel fact rather than a PID guess — PID probing is exactly
//! the false signal that broke the manual Board token protocol. Those two
//! requirements together mean the lease needs a process to live in, so
//! `verify.lease.acquire` spawns a detached `verify.lease.hold` holder that
//! keeps both kernel locks and parks. Consequences that fall out of that:
//!
//! - acquisition is atomic, because the heavy kernel lock decides it (FR-8);
//! - a killed or crashed holder releases immediately, because the kernel
//!   drops its locks (T-IDX-383 / AC-3 is preserved);
//! - TTL bounds crash residue, because the holder self-terminates (AC-4);
//! - a running command is never interrupted: release happens after the holder
//!   observes the request, not by signalling the workload (AC-Y3).

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use gwt_core::index_coordinator::{
    coordinator_root, HeavyLeaseStatus, IndexCoordinator, JobAdmission, JobPriority, TargetKey,
};
use gwt_core::paths::{project_scope_hash, resolve_current_worktree_root};
use gwt_core::worktree_hash::compute_worktree_hash;
use gwt_github::{client::ApiError, SpecOpsError};
use serde::{Deserialize, Serialize};

use crate::cli::CliEnv;

/// PM operational value: 45 minutes covered every observed heavy matrix.
pub const DEFAULT_TTL_MINUTES: u64 = 45;
/// Upper bound so a typo cannot park the host for a day.
const MAX_TTL_MINUTES: u64 = 12 * 60;
const CONTROL_DIR: &str = "verification.control";
const OUTCOME_FILE: &str = "outcome.json";
const RELEASE_FILE: &str = "release";
const EXTEND_FILE: &str = "extend-to-ms";
/// The holder is parked, so this poll only decides how fast it reacts to a
/// release request — it never watches another process.
const HOLDER_POLL: Duration = Duration::from_millis(100);
/// Bounds process startup only; the acquisition attempt itself never blocks.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);
const CONTROL_ACK_TIMEOUT: Duration = Duration::from_secs(15);
const CONTROL_POLL: Duration = Duration::from_millis(50);
/// A contended attempt answers immediately with the current holder instead of
/// queueing, so no agent ever sits in a wait loop (US-1 / FR-3).
const NON_BLOCKING: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerificationLeaseCommand {
    Acquire {
        ttl_minutes: u64,
        reason: Option<String>,
    },
    Release {
        lease_id: String,
        reason: Option<String>,
    },
    Extend {
        lease_id: String,
        ttl_minutes: u64,
    },
    Status,
    /// Internal: the foreground holder spawned by `Acquire`. Not intended for
    /// direct use; it blocks until released or until its TTL lapses.
    Hold {
        ttl_minutes: u64,
        control: PathBuf,
        reason: Option<String>,
    },
}

pub(super) fn run<E: CliEnv>(
    env: &mut E,
    command: VerificationLeaseCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    match command {
        VerificationLeaseCommand::Status => {
            render(out, "held", "free", &status()?);
            Ok(0)
        }
        VerificationLeaseCommand::Acquire {
            ttl_minutes,
            reason,
        } => acquire(env, ttl_minutes, reason, out),
        VerificationLeaseCommand::Release { lease_id, reason } => {
            release(&lease_id, reason.as_deref(), out)
        }
        VerificationLeaseCommand::Extend {
            lease_id,
            ttl_minutes,
        } => extend(&lease_id, ttl_minutes, out),
        VerificationLeaseCommand::Hold {
            ttl_minutes,
            control,
            reason,
        } => hold(env, ttl_minutes, &control, reason.as_deref()),
    }
}

pub fn validate_ttl_minutes(ttl_minutes: u64) -> Result<Duration, SpecOpsError> {
    if ttl_minutes == 0 || ttl_minutes > MAX_TTL_MINUTES {
        return Err(unexpected(format!(
            "ttl_minutes must be between 1 and {MAX_TTL_MINUTES}, got {ttl_minutes}"
        )));
    }
    Ok(Duration::from_secs(ttl_minutes * 60))
}

// ---------------------------------------------------------------------------
// Operations
// ---------------------------------------------------------------------------

fn acquire<E: CliEnv>(
    env: &mut E,
    ttl_minutes: u64,
    reason: Option<String>,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    validate_ttl_minutes(ttl_minutes)?;
    let root = coordinator_root();
    let control = root
        .join(CONTROL_DIR)
        .join(uuid::Uuid::new_v4().to_string());
    fs::create_dir_all(&control).map_err(|err| {
        unexpected(format!(
            "failed to prepare the lease control directory {}: {err}",
            control.display()
        ))
    })?;

    spawn_holder(env, ttl_minutes, &control, reason.as_deref())?;
    let outcome = await_outcome(&control)?;
    if let Some(error) = &outcome.error {
        let _ = fs::remove_dir_all(&control);
        return Err(unexpected(format!(
            "verification lease holder failed: {error}"
        )));
    }
    if outcome.granted {
        out.push_str("verification lease: granted\n");
    } else {
        // A refusal names the *current* holder, so leaving this directory
        // behind would make it answer to that holder's lease id.
        let _ = fs::remove_dir_all(&control);
        out.push_str("verification lease: unavailable\n");
    }
    push_status_fields(out, &outcome.status);
    if !outcome.granted {
        out.push_str(
            "note: the current holder finishes its run before the lease is released; \
             re-run verify.lease.acquire after it reports done\n",
        );
    }
    Ok(0)
}

fn release(lease_id: &str, reason: Option<&str>, out: &mut String) -> Result<i32, SpecOpsError> {
    let control = control_dir_for(lease_id).ok_or_else(|| missing_lease(lease_id))?;
    fs::write(control.join(RELEASE_FILE), reason.unwrap_or("").as_bytes())
        .map_err(|err| unexpected(format!("failed to signal release for {lease_id}: {err}")))?;
    await_settled(lease_id)?;
    out.push_str("verification lease: released\n");
    out.push_str(&format!("lease_id: {lease_id}\n"));
    if let Some(reason) = reason {
        out.push_str(&format!("reason: {reason}\n"));
    }
    push_status_fields(out, &status()?);
    Ok(0)
}

fn extend(lease_id: &str, ttl_minutes: u64, out: &mut String) -> Result<i32, SpecOpsError> {
    let ttl = validate_ttl_minutes(ttl_minutes)?;
    let control = control_dir_for(lease_id).ok_or_else(|| missing_lease(lease_id))?;
    let target = now_ms().saturating_add(ttl.as_millis() as u64);
    fs::write(control.join(EXTEND_FILE), target.to_string().as_bytes())
        .map_err(|err| unexpected(format!("failed to request extend for {lease_id}: {err}")))?;

    // The republished ticket is the acknowledgement: only the holder writes
    // it, so seeing the new deadline proves the holder applied the request.
    let deadline = Instant::now() + CONTROL_ACK_TIMEOUT;
    loop {
        let status = status()?;
        if status.lease_id.as_deref() == Some(lease_id) && status.expires_at_ms == Some(target) {
            out.push_str("verification lease: extended\n");
            out.push_str(&format!("ttl_minutes: {ttl_minutes}\n"));
            push_status_fields(out, &status);
            return Ok(0);
        }
        if Instant::now() >= deadline {
            return Err(unexpected(format!(
                "verification lease {lease_id} did not apply the extend request within {}s",
                CONTROL_ACK_TIMEOUT.as_secs()
            )));
        }
        std::thread::sleep(CONTROL_POLL);
    }
}

/// Foreground holder: take both kernel locks, publish the outcome, then park
/// until released or expired.
fn hold<E: CliEnv>(
    env: &mut E,
    ttl_minutes: u64,
    control: &Path,
    reason: Option<&str>,
) -> Result<i32, SpecOpsError> {
    let ttl = validate_ttl_minutes(ttl_minutes)?;
    let key = verification_key(env)?;
    let coordinator = open_coordinator()?;

    let admission = coordinator
        .request_job(&key, JobPriority::ManualRebuild, NON_BLOCKING)
        .map_err(|err| unexpected(format!("verification job admission failed: {err}")))?;
    let guard = match admission {
        JobAdmission::Owner(guard) => guard,
        JobAdmission::Joined(waiter) => {
            // Another verification run already owns this exact worktree.
            drop(waiter);
            publish_outcome(control, &LeaseOutcome::refused(status()?));
            return Ok(0);
        }
    };
    let mut lease = match guard.acquire_heavy_with_ttl(NON_BLOCKING, ttl) {
        Ok(lease) => lease,
        Err(_) => {
            publish_outcome(control, &LeaseOutcome::refused(status()?));
            return Ok(0);
        }
    };
    if let Some(reason) = reason {
        let _ = fs::write(control.join("reason"), reason.as_bytes());
    }
    publish_outcome(control, &LeaseOutcome::granted(status()?));

    let release_path = control.join(RELEASE_FILE);
    let extend_path = control.join(EXTEND_FILE);
    loop {
        if release_path.exists() {
            break;
        }
        if let Some(expires_at_ms) = read_extend_request(&extend_path) {
            let _ = lease.extend_until(expires_at_ms);
            let _ = fs::remove_file(&extend_path);
        }
        if lease.is_expired() {
            break;
        }
        std::thread::sleep(HOLDER_POLL);
    }
    // `release` records `expired` with a reason when the TTL lapsed and
    // `released` otherwise, so the ledger distinguishes the two exits.
    let _ = lease.release();
    let _ = guard.complete(gwt_core::index_coordinator::JobOutcome::Completed);
    let _ = fs::remove_dir_all(control);
    Ok(0)
}

// ---------------------------------------------------------------------------
// Holder handshake
// ---------------------------------------------------------------------------

fn spawn_holder<E: CliEnv>(
    env: &mut E,
    ttl_minutes: u64,
    control: &Path,
    reason: Option<&str>,
) -> Result<(), SpecOpsError> {
    let exe = std::env::current_exe()
        .map_err(|err| unexpected(format!("cannot resolve the gwtd binary path: {err}")))?;
    let envelope = serde_json::json!({
        "schema_version": 1,
        "operation": "verify.lease.hold",
        "params": {
            "ttl_minutes": ttl_minutes,
            "control": control.to_string_lossy(),
            "reason": reason,
        }
    })
    .to_string();

    let mut child = gwt_core::process::hidden_command(exe)
        .current_dir(env.repo_path())
        .stdin(Stdio::piped())
        // The holder must not inherit our stdout/stderr: the caller reads our
        // output to EOF, and an inherited pipe would keep it open for the
        // whole lease.
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| unexpected(format!("failed to spawn the lease holder: {err}")))?;
    child
        .stdin
        .take()
        .ok_or_else(|| unexpected("lease holder stdin unavailable".to_string()))?
        .write_all(envelope.as_bytes())
        .map_err(|err| unexpected(format!("failed to hand the holder its request: {err}")))?;
    // Deliberately not awaited: the holder outlives this invocation.
    Ok(())
}

fn await_outcome(control: &Path) -> Result<LeaseOutcome, SpecOpsError> {
    let path = control.join(OUTCOME_FILE);
    let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
    loop {
        if let Some(outcome) = read_json::<LeaseOutcome>(&path) {
            return Ok(outcome);
        }
        if Instant::now() >= deadline {
            let _ = fs::remove_dir_all(control);
            return Err(unexpected(format!(
                "the verification lease holder did not answer within {}s",
                HANDSHAKE_TIMEOUT.as_secs()
            )));
        }
        std::thread::sleep(CONTROL_POLL);
    }
}

fn await_settled(lease_id: &str) -> Result<(), SpecOpsError> {
    let deadline = Instant::now() + CONTROL_ACK_TIMEOUT;
    loop {
        let status = status()?;
        if !status.held || status.lease_id.as_deref() != Some(lease_id) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(unexpected(format!(
                "verification lease {lease_id} was still held {}s after the release request",
                CONTROL_ACK_TIMEOUT.as_secs()
            )));
        }
        std::thread::sleep(CONTROL_POLL);
    }
}

/// Locate the control directory of a live lease. At most one lease is held
/// host-wide, so this scan sees one candidate in practice. Only a *granted*
/// outcome may answer: a refusal snapshot names the lease it lost to, so
/// matching on the lease id alone would route release and extend requests to
/// a directory with nobody listening.
fn control_dir_for(lease_id: &str) -> Option<PathBuf> {
    fs::read_dir(coordinator_root().join(CONTROL_DIR))
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .find(|dir| {
            read_json::<LeaseOutcome>(&dir.join(OUTCOME_FILE)).is_some_and(|outcome| {
                outcome.granted && outcome.status.lease_id.as_deref() == Some(lease_id)
            })
        })
}

fn publish_outcome(control: &Path, outcome: &LeaseOutcome) {
    let Ok(payload) = serde_json::to_vec(outcome) else {
        return;
    };
    let tmp = control.join(".outcome.tmp");
    if fs::write(&tmp, payload).is_ok() {
        let _ = fs::rename(&tmp, control.join(OUTCOME_FILE));
    }
}

fn read_extend_request(path: &Path) -> Option<u64> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Mirrors [`HeavyLeaseStatus`] so the holder can hand a race-free snapshot to
/// the invocation that spawned it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct LeaseStatusSnapshot {
    held: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lease_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    owner_pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    acquired_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expires_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    remaining_ms: Option<u64>,
    #[serde(default)]
    expired: bool,
    #[serde(default)]
    pending: usize,
}

impl From<HeavyLeaseStatus> for LeaseStatusSnapshot {
    fn from(status: HeavyLeaseStatus) -> Self {
        Self {
            held: status.held,
            lease_id: status.lease_id,
            target: status.target,
            owner_pid: status.owner.map(|owner| owner.pid),
            acquired_at_ms: status.acquired_at_ms,
            expires_at_ms: status.expires_at_ms,
            remaining_ms: status.remaining_ms,
            expired: status.expired,
            pending: status.pending,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct LeaseOutcome {
    granted: bool,
    #[serde(flatten)]
    status: LeaseStatusSnapshot,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl LeaseOutcome {
    fn granted(status: LeaseStatusSnapshot) -> Self {
        Self {
            granted: true,
            status,
            error: None,
        }
    }

    fn refused(status: LeaseStatusSnapshot) -> Self {
        Self {
            granted: false,
            status,
            error: None,
        }
    }
}

fn status() -> Result<LeaseStatusSnapshot, SpecOpsError> {
    Ok(open_coordinator()?
        .heavy_lease_status()
        .map_err(|err| unexpected(format!("failed to read the verification lease: {err}")))?
        .into())
}

fn open_coordinator() -> Result<IndexCoordinator, SpecOpsError> {
    IndexCoordinator::open_default()
        .map_err(|err| unexpected(format!("verification lease coordinator unavailable: {err}")))
}

fn verification_key<E: CliEnv>(env: &mut E) -> Result<TargetKey, SpecOpsError> {
    let worktree = resolve_current_worktree_root(env.repo_path());
    let worktree_hash = compute_worktree_hash(&worktree)
        .map_err(|err| unexpected(format!("failed to identify the current worktree: {err}")))?;
    Ok(TargetKey::verification(
        project_scope_hash(&worktree).as_str(),
        worktree_hash.as_str(),
    ))
}

fn render(out: &mut String, held_label: &str, free_label: &str, status: &LeaseStatusSnapshot) {
    let label = if status.held { held_label } else { free_label };
    out.push_str(&format!("verification lease: {label}\n"));
    push_status_fields(out, status);
}

fn push_status_fields(out: &mut String, status: &LeaseStatusSnapshot) {
    if let Some(lease_id) = &status.lease_id {
        out.push_str(&format!("lease_id: {lease_id}\n"));
    }
    if let Some(target) = &status.target {
        out.push_str(&format!("target: {target}\n"));
    }
    if let Some(pid) = status.owner_pid {
        out.push_str(&format!("owner_pid: {pid}\n"));
    }
    if let Some(at) = status.acquired_at_ms {
        out.push_str(&format!("acquired_at_ms: {at}\n"));
    }
    if let Some(at) = status.expires_at_ms {
        out.push_str(&format!("expires_at_ms: {at}\n"));
    }
    if let Some(remaining) = status.remaining_ms {
        out.push_str(&format!("remaining_ms: {remaining}\n"));
    }
    if status.held {
        out.push_str(&format!("expired: {}\n", status.expired));
    }
    out.push_str(&format!("pending: {}\n", status.pending));
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis() as u64)
        .unwrap_or(0)
}

fn missing_lease(lease_id: &str) -> SpecOpsError {
    unexpected(format!(
        "no live verification lease {lease_id} — check `verify.lease.status`; \
         a holder that died has already released the lease"
    ))
}

fn unexpected(message: String) -> SpecOpsError {
    SpecOpsError::from(ApiError::Unexpected(message))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ttl_bounds_are_enforced() {
        assert!(validate_ttl_minutes(0).is_err());
        assert!(validate_ttl_minutes(MAX_TTL_MINUTES + 1).is_err());
        assert_eq!(
            validate_ttl_minutes(DEFAULT_TTL_MINUTES).unwrap(),
            Duration::from_secs(45 * 60)
        );
    }

    #[test]
    fn free_status_renders_without_holder_fields() {
        let mut out = String::new();
        render(&mut out, "held", "free", &LeaseStatusSnapshot::default());
        assert_eq!(out, "verification lease: free\npending: 0\n");
    }

    #[test]
    fn held_status_renders_the_holder_and_remaining_ttl() {
        let mut out = String::new();
        render(
            &mut out,
            "held",
            "free",
            &LeaseStatusSnapshot {
                held: true,
                lease_id: Some("lease-1".to_string()),
                target: Some("repo--verification--wt".to_string()),
                owner_pid: Some(4242),
                acquired_at_ms: Some(1_000),
                expires_at_ms: Some(61_000),
                remaining_ms: Some(60_000),
                expired: false,
                pending: 2,
            },
        );
        assert_eq!(
            out,
            "verification lease: held\n\
             lease_id: lease-1\n\
             target: repo--verification--wt\n\
             owner_pid: 4242\n\
             acquired_at_ms: 1000\n\
             expires_at_ms: 61000\n\
             remaining_ms: 60000\n\
             expired: false\n\
             pending: 2\n"
        );
    }

    #[test]
    fn outcome_round_trips_through_the_control_file() {
        let dir = tempfile::tempdir().expect("control dir");
        let outcome = LeaseOutcome::refused(LeaseStatusSnapshot {
            held: true,
            lease_id: Some("lease-9".to_string()),
            remaining_ms: Some(1_234),
            ..LeaseStatusSnapshot::default()
        });
        publish_outcome(dir.path(), &outcome);
        let parsed: LeaseOutcome =
            read_json(&dir.path().join(OUTCOME_FILE)).expect("published outcome");
        assert!(!parsed.granted);
        assert_eq!(parsed.status.lease_id.as_deref(), Some("lease-9"));
        assert_eq!(parsed.status.remaining_ms, Some(1_234));
    }

    #[test]
    fn extend_requests_parse_only_whole_deadlines() {
        let dir = tempfile::tempdir().expect("control dir");
        let path = dir.path().join(EXTEND_FILE);
        assert_eq!(read_extend_request(&path), None);
        fs::write(&path, b" 1750000000000\n").unwrap();
        assert_eq!(read_extend_request(&path), Some(1_750_000_000_000));
        fs::write(&path, b"soon").unwrap();
        assert_eq!(read_extend_request(&path), None);
    }
}
