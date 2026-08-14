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
/// How long a control directory is treated as "still being set up" rather than
/// residue left by a killed holder. Mirrors the coordinator's own
/// `REGISTRATION_RESIDUE_GRACE`, and must stay comfortably above
/// [`HANDSHAKE_TIMEOUT`] so a slow-starting holder is never swept.
const CONTROL_RESIDUE_GRACE: Duration = Duration::from_secs(120);

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
    sweep_abandoned_control_dirs(&root, status()?.lease_id.as_deref());
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
    // A holder that exited normally already removed this; a holder that was
    // killed cannot, so clean up on the caller's side too.
    let _ = fs::remove_dir_all(&control);
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
    ensure_control_dir_is_ours(control)?;
    // Past this point the caller is waiting on `outcome.json`, so a failure
    // has to be published rather than returned: the spawning invocation reads
    // our stdout from `Stdio::null()` and would otherwise learn nothing until
    // its handshake timeout.
    let prepared = validate_ttl_minutes(ttl_minutes)
        .and_then(|ttl| verification_key(env).map(|key| (ttl, key)))
        .and_then(|(ttl, key)| open_coordinator().map(|coordinator| (ttl, key, coordinator)));
    let (ttl, key, coordinator) = match prepared {
        Ok(prepared) => prepared,
        Err(err) => {
            publish_outcome(control, &LeaseOutcome::failed(err.to_string()));
            return Err(err);
        }
    };

    let admission = match coordinator.request_job(&key, JobPriority::ManualRebuild, NON_BLOCKING) {
        Ok(admission) => admission,
        Err(err) => {
            let message = format!("verification job admission failed: {err}");
            publish_outcome(control, &LeaseOutcome::failed(message.clone()));
            return Err(unexpected(message));
        }
    };
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
            // Deliberately leave the directory in place: the holder may still
            // be starting and would lose its release channel — and therefore
            // hold the lease until its TTL — if we removed it here. The
            // grace-gated sweep collects it if the holder never arrives.
            return Err(unexpected(format!(
                "the verification lease holder did not answer within {}s — check \
                 `verify.lease.status`; if a lease is now held, release it with its lease_id",
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

/// Drop control directories whose holder is gone. A killed holder cannot
/// clean up after itself, and its directory would otherwise sit next to the
/// live one forever.
///
/// `current_lease_id` is a snapshot taken before this scan, so it can go stale
/// the moment another claimant wins the lease. Deleting a live holder's
/// directory would be unrecoverable — the holder watches it for the release
/// signal, and `control_dir_for` needs it to route `release` / `extend` — so a
/// directory is only swept once it has sat untouched for
/// [`CONTROL_RESIDUE_GRACE`]. A directory that appeared or was published
/// during the snapshot gap is younger than that by construction, which is the
/// same protection [`gwt_core::index_coordinator`] gives its own registration
/// files.
fn sweep_abandoned_control_dirs(root: &Path, current_lease_id: Option<&str>) {
    let Ok(entries) = fs::read_dir(root.join(CONTROL_DIR)) else {
        return;
    };
    for dir in entries.flatten().map(|entry| entry.path()) {
        let outcome_path = dir.join(OUTCOME_FILE);
        match read_json::<LeaseOutcome>(&outcome_path) {
            // A granted directory naming the current holder is the live
            // control channel; anything else granted belonged to a holder
            // that is no longer on the lease.
            Some(outcome)
                if outcome.granted && outcome.status.lease_id.as_deref() == current_lease_id =>
            {
                continue
            }
            // A directory with no published outcome may belong to a holder
            // that is still starting up; the grace window covers that.
            Some(_) | None => {}
        }
        if older_than(&outcome_path, CONTROL_RESIDUE_GRACE)
            .unwrap_or_else(|| older_than(&dir, CONTROL_RESIDUE_GRACE).unwrap_or(false))
        {
            let _ = fs::remove_dir_all(&dir);
        }
    }
}

/// `Some(true)` when `path` was last modified more than `grace` ago,
/// `None` when the timestamp cannot be read (never treat that as abandoned).
fn older_than(path: &Path, grace: Duration) -> Option<bool> {
    let age = fs::metadata(path).ok()?.modified().ok()?.elapsed().ok()?;
    Some(age > grace)
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

    fn failed(error: String) -> Self {
        Self {
            granted: false,
            status: LeaseStatusSnapshot::default(),
            error: Some(error),
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

/// `verify.lease.hold` is reachable through the ordinary envelope dispatcher,
/// so refuse a control directory outside the coordinator runtime. A holder
/// pointed elsewhere would take the real host-wide lease while
/// [`control_dir_for`] could never find it, leaving it unreleasable until its
/// TTL.
fn ensure_control_dir_is_ours(control: &Path) -> Result<(), SpecOpsError> {
    let expected_parent = coordinator_root().join(CONTROL_DIR);
    if control.parent() == Some(expected_parent.as_path()) {
        return Ok(());
    }
    Err(unexpected(format!(
        "verify.lease.hold is internal: params.control must be a directory directly under {} \
         (got {}). Use verify.lease.acquire instead.",
        expected_parent.display(),
        control.display()
    )))
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

/// Distinguish "that lease is already gone" from "that lease is held but its
/// control channel is missing" — the second is not resolvable by retrying, so
/// saying the holder died would send the caller down the wrong path.
fn missing_lease(lease_id: &str) -> SpecOpsError {
    let held = status()
        .ok()
        .filter(|status| status.held && status.lease_id.as_deref() == Some(lease_id));
    match held {
        Some(status) => unexpected(format!(
            "verification lease {lease_id} is still held but has no control channel, so it \
             cannot be released or extended. It lapses on its own at expires_at_ms={}. \
             This means the control directory under the coordinator runtime was removed \
             while the holder was alive.",
            status
                .expires_at_ms
                .map(|at| at.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
        )),
        None => unexpected(format!(
            "no live verification lease {lease_id} — check `verify.lease.status`; \
             a holder that died has already released the lease"
        )),
    }
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

    fn control_dir(root: &Path, name: &str) -> PathBuf {
        let path = root.join(CONTROL_DIR).join(name);
        fs::create_dir_all(&path).expect("control dir");
        path
    }

    fn published(root: &Path, name: &str, granted: bool, lease_id: &str) -> PathBuf {
        let path = control_dir(root, name);
        publish_outcome(
            &path,
            &LeaseOutcome {
                granted,
                status: LeaseStatusSnapshot {
                    held: true,
                    lease_id: Some(lease_id.to_string()),
                    ..LeaseStatusSnapshot::default()
                },
                error: None,
            },
        );
        path
    }

    /// Backdate a directory and its outcome past the grace window so the sweep
    /// treats it as residue without the test having to wait.
    fn age_out(dir: &Path) {
        let stale = std::time::SystemTime::now() - CONTROL_RESIDUE_GRACE * 2;
        for path in [dir.join(OUTCOME_FILE), dir.to_path_buf()] {
            if path.exists() {
                let file = fs::File::options()
                    .write(true)
                    .open(&path)
                    .or_else(|_| fs::File::open(&path))
                    .expect("open for backdating");
                file.set_modified(stale).expect("backdate");
            }
        }
    }

    #[test]
    fn sweep_removes_only_aged_out_residue() {
        let root = tempfile::tempdir().expect("coordinator root");
        let live = published(root.path(), "live", true, "lease-live");
        let abandoned = published(root.path(), "abandoned", true, "lease-dead");
        let starting_up = control_dir(root.path(), "starting-up");
        age_out(&live);
        age_out(&abandoned);
        age_out(&starting_up);

        sweep_abandoned_control_dirs(root.path(), Some("lease-live"));

        assert!(live.is_dir(), "the live control channel must survive");
        assert!(
            !abandoned.is_dir(),
            "a dead holder's aged-out directory is swept"
        );
        assert!(
            !starting_up.is_dir(),
            "a directory that never published an outcome is residue once aged out"
        );
    }

    /// The regression that matters: `current_lease_id` is a snapshot taken
    /// before the scan, so a claimant that wins the lease during that gap is
    /// invisible to it. Deleting that holder's directory would strand the
    /// host-wide lease until its TTL, because the holder watches that
    /// directory for the release signal.
    #[test]
    fn sweep_never_removes_a_freshly_published_control_dir() {
        let root = tempfile::tempdir().expect("coordinator root");
        let raced_in = published(root.path(), "raced-in", true, "lease-new");
        let starting_up = control_dir(root.path(), "starting-up");

        // Snapshot said the lease was free; another claimant took it since.
        sweep_abandoned_control_dirs(root.path(), None);

        assert!(
            raced_in.is_dir(),
            "a holder that published during the snapshot gap must not be swept"
        );
        assert!(
            starting_up.is_dir(),
            "a holder that has not published yet must not be swept"
        );
    }

    #[test]
    fn hold_refuses_a_control_dir_outside_the_coordinator_runtime() {
        let outside = tempfile::tempdir().expect("outside dir");
        let error = ensure_control_dir_is_ours(&outside.path().join("lease-1"))
            .expect_err("an out-of-tree control dir must be refused");
        assert!(
            error.to_string().contains("verify.lease.hold is internal"),
            "the refusal must name the cause: {error}"
        );
        assert!(
            ensure_control_dir_is_ours(&coordinator_root().join(CONTROL_DIR).join("lease-1"))
                .is_ok()
        );
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
