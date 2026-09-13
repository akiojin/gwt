//! SPEC #3576: only canonical `verify.run` owns verification leases.
//!
//! A lease lives inside its runner rather than a detached process with no
//! workload. The retired acquire/hold/extend operations keep actionable
//! diagnostics; status/release remain available to drain pre-upgrade holders.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gwt_core::index_coordinator::{
    coordinator_root, HeavyHolderKind, HeavyLeaseStatus, HeavyQueueEntry, IndexCoordinator,
    JobPriority, TargetKey, VERIFICATION_RESERVATION_TTL,
};
use gwt_core::paths::{project_scope_hash, resolve_current_worktree_root};
use gwt_core::worktree_hash::compute_worktree_hash;
use gwt_github::{client::ApiError, SpecOpsError};
use serde::{Deserialize, Serialize};

use crate::cli::CliEnv;

/// Issue #3913: `verify.run` host admission — the lease's in-process claimant.
pub(crate) mod admission;

/// PM operational value: 45 minutes covered every observed heavy matrix.
pub const DEFAULT_TTL_MINUTES: u64 = 45;
const CONTROL_DIR: &str = "verification.control";
const OUTCOME_FILE: &str = "outcome.json";
const RELEASE_FILE: &str = "release";
const CONTROL_ACK_TIMEOUT: Duration = Duration::from_secs(15);
const CONTROL_POLL: Duration = Duration::from_millis(50);

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
    /// Retired internal operation; parsed only to explain the migration.
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
        VerificationLeaseCommand::Acquire { .. }
        | VerificationLeaseCommand::Extend { .. }
        | VerificationLeaseCommand::Hold { .. } => Err(unexpected(
            "manual verification leases are retired: use `verify.run` for canonical verification; \
             it acquires and releases its own lease. Run development builds, tests, lint, and \
             bootstrap builds directly without a lease. Use `verify.lease.status` and \
             `verify.lease.release` to inspect and drain a legacy holder."
                .to_string(),
        )),
        VerificationLeaseCommand::Release { lease_id, reason } => {
            release(env, &lease_id, reason.as_deref(), out)
        }
    }
}

fn release<E: CliEnv>(
    env: &mut E,
    lease_id: &str,
    reason: Option<&str>,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let Some(control) = control_dir_for(lease_id) else {
        if held_index_lease(lease_id)? {
            return request_index_yield(env, lease_id, reason, out);
        }
        return Err(missing_lease(lease_id));
    };
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
/// matching on the lease id alone would route release requests to
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

/// Status rendering and the pre-upgrade detached holder wire format.
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
    /// Issue #4169: who is waiting, in the order they will be served.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    queue: Vec<HeavyQueueEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    holder_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    remaining_batches: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    estimated_remaining_ms: Option<u64>,
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
            queue: status.queue,
            holder_kind: status.holder_kind.map(|kind| kind.as_str().to_string()),
            remaining_batches: status.remaining_batches,
            estimated_remaining_ms: status.estimated_remaining_ms,
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

fn status() -> Result<LeaseStatusSnapshot, SpecOpsError> {
    Ok(open_coordinator()?
        .heavy_lease_status()
        .map_err(|err| unexpected(format!("failed to read the verification lease: {err}")))?
        .into())
}

pub(super) fn open_coordinator() -> Result<IndexCoordinator, SpecOpsError> {
    IndexCoordinator::open_default()
        .map_err(|err| unexpected(format!("verification lease coordinator unavailable: {err}")))
}

pub(super) fn verification_key<E: CliEnv>(env: &mut E) -> Result<TargetKey, SpecOpsError> {
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

/// The live lease named by `lease_id` when it belongs to an index job
/// (Issue #4086): such a lease has no verification control directory, so
/// release requests are arbitrated through the coordinator instead.
fn held_index_lease(lease_id: &str) -> Result<bool, SpecOpsError> {
    let status = status()?;
    Ok(status.held
        && status.lease_id.as_deref() == Some(lease_id)
        && status.holder_kind.as_deref() == Some(HeavyHolderKind::Index.as_str()))
}

/// PM arbitration of an index lease (Issue #4086): leave a verification-
/// priority reservation for the caller's worktree. The runner observes it at
/// its next batch boundary and yields; the host then defers to the
/// reservation instead of re-taking the lease.
fn request_index_yield<E: CliEnv>(
    env: &mut E,
    lease_id: &str,
    reason: Option<&str>,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let key = verification_key(env)?;
    open_coordinator()?
        .reserve_heavy(
            &key,
            JobPriority::ManualRebuild,
            VERIFICATION_RESERVATION_TTL,
            Some(reason.unwrap_or("verify.lease.release arbitration")),
        )
        .map_err(|err| unexpected(format!("failed to reserve the heavy lease: {err}")))?;
    out.push_str("verification lease: yield requested\n");
    out.push_str(&format!("lease_id: {lease_id}\n"));
    if let Some(reason) = reason {
        out.push_str(&format!("reason: {reason}\n"));
    }
    push_status_fields(out, &status()?);
    out.push_str(&format!(
        "note: an index job holds this lease; it releases at its next batch boundary \
         (at most {}s after this request when progress is published) and background index \
         jobs defer to the reservation left for this worktree\n",
        VERIFICATION_RESERVATION_TTL.as_secs()
    ));
    Ok(0)
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
    if let Some(kind) = &status.holder_kind {
        out.push_str(&format!("holder_kind: {kind}\n"));
    }
    if let Some(batches) = status.remaining_batches {
        out.push_str(&format!("remaining_batches: {batches}\n"));
    }
    if let Some(estimate) = status.estimated_remaining_ms {
        out.push_str(&format!("estimated_remaining_ms: {estimate}\n"));
    }
    out.push_str(&format!("pending: {}\n", status.pending));
    // Issue #4169 AC-2: `pending` is a count, and a count cannot tell an agent
    // whether it is next or fifth. The queue names every claimant and how long
    // it has been waiting, in the order the lease will be handed over.
    for (position, entry) in status.queue.iter().enumerate() {
        out.push_str(&format!(
            "queue[{position}]: target={} priority={} queued_at_ms={} waiting_ms={}\n",
            entry.target.as_deref().unwrap_or("unknown"),
            entry.priority.as_str(),
            entry.queued_at_ms,
            entry.waiting_ms,
        ));
    }
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

fn missing_lease(lease_id: &str) -> SpecOpsError {
    let held = status()
        .ok()
        .filter(|status| status.held && status.lease_id.as_deref() == Some(lease_id));
    match held {
        Some(_) => unexpected(format!(
            "verification lease {lease_id} is held without a legacy control channel. \
             Canonical leases are owned by `verify.run` and release when the runner finishes; \
             they cannot be released or extended through the manual API. \
             Check `verify.lease.status` for the current holder."
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
                queue: vec![
                    HeavyQueueEntry {
                        target: Some("repo--verification--early".to_string()),
                        priority: JobPriority::ManualRebuild,
                        queued_at_ms: 500,
                        waiting_ms: 90_000,
                    },
                    HeavyQueueEntry {
                        target: Some("repo--verification--late".to_string()),
                        priority: JobPriority::ManualRebuild,
                        queued_at_ms: 900,
                        waiting_ms: 89_600,
                    },
                ],
                holder_kind: Some("verification".to_string()),
                remaining_batches: None,
                estimated_remaining_ms: Some(60_000),
            },
        );
        // Issue #4169 AC-2: the waiters are named in service order, each with
        // the moment it joined and how long it has been waiting.
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
             holder_kind: verification\n\
             estimated_remaining_ms: 60000\n\
             pending: 2\n\
             queue[0]: target=repo--verification--early priority=manual-rebuild \
             queued_at_ms=500 waiting_ms=90000\n\
             queue[1]: target=repo--verification--late priority=manual-rebuild \
             queued_at_ms=900 waiting_ms=89600\n"
        );
    }

    #[test]
    fn legacy_granted_outcome_remains_readable() {
        let parsed: LeaseOutcome =
            serde_json::from_str(r#"{"granted":true,"held":true,"lease_id":"legacy-lease"}"#)
                .expect("pre-upgrade holder outcome");
        assert!(parsed.granted);
        assert_eq!(parsed.status.lease_id.as_deref(), Some("legacy-lease"));
    }
}
