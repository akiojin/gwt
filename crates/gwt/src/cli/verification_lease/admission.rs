//! SPEC #3576: automatic admission for canonical `verify.run` execution.
//!
//! Each run acquires its own target job and host-wide heavy lease in-process.
//! Even runs in the same worktree must wait for each other. Dropping the
//! admission releases both locks; there is no detached pre-acquisition.
//! Ordinary builds and development tests do not participate in admission.
//!
//! Admission preserves the existing bounded wait, FIFO reservations, holder
//! diagnostics, and Board notice. A deferred invocation writes no run record.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gwt_core::index_coordinator::{
    CoordinatorError, HeavyHolderKind, HeavyLease, HeavyLeaseStatus, IndexCoordinator,
    JobAdmission, JobOutcome, JobPriority, TargetJobGuard, VERIFICATION_RESERVATION_TTL,
};
use gwt_github::{client::ApiError, SpecOpsError};

use crate::cli::board::{BoardCommand, BoardPostCommand};
use crate::cli::verification_lease::holder_activity::{HolderActivity, HolderProbe};
use crate::cli::verification_lease::{self, DEFAULT_TTL_MINUTES};
use crate::cli::CliEnv;

/// Default admission wait when `params.max_wait_secs` is absent.
pub(crate) const DEFAULT_MAX_WAIT_SECS: u64 = 300;
/// Hard cap for `params.max_wait_secs`. Must stay below the Issue Monitor's
/// default `stuck_timeout_secs` (1800) so one bounded wait never reads as a
/// stalled agent.
pub(crate) const MAX_WAIT_SECS: u64 = 1500;
const _: () = assert!(DEFAULT_MAX_WAIT_SECS <= MAX_WAIT_SECS);
/// How often the wait re-checks the lease and the host.
const POLL: Duration = Duration::from_secs(5);
/// Our own verification target job only ever contends with a same-worktree
/// claimant, so claiming it does not need to block.
const NON_BLOCKING: Duration = Duration::from_millis(250);
/// TTL of the in-process lease. The kernel lock releases on exit regardless;
/// the TTL only bounds how long crash residue can look live.
const LEASE_TTL: Duration = Duration::from_secs(DEFAULT_TTL_MINUTES * 60);
/// Waits shorter than one poll are not worth a Board post.
const BOARD_NOTICE_AFTER: Duration = POLL;
/// In-process lease holder; dropping releases the heavy lease and completes
/// the target job, in the reverse of the acquisition order.
pub(crate) struct Admission {
    guard: Option<TargetJobGuard>,
    lease: Option<Arc<Mutex<HeavyLease>>>,
    renewal: Option<super::renewal::Renewal>,
    commands: Arc<super::CommandProgress>,
    lease_id: String,
    waited: Duration,
}

impl std::fmt::Debug for Admission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Admission")
            .field("lease_id", &self.lease_id)
            .field("waited", &self.waited)
            .finish_non_exhaustive()
    }
}

impl Admission {
    pub(crate) fn command_progress(&self) -> &super::CommandProgress {
        &self.commands
    }

    fn settle(&mut self, outcome: JobOutcome) {
        drop(self.renewal.take());
        drop(self.lease.take());
        if let Some(guard) = self.guard.take() {
            let _ = guard.complete(outcome);
        }
    }
}

impl Drop for Admission {
    fn drop(&mut self) {
        self.settle(JobOutcome::Completed);
    }
}

impl Admission {
    /// The admitted lease identity travels with the command output.
    pub(crate) fn summary(&self) -> String {
        format!(
            "verify: host admission — lease {} acquired (waited {}s)",
            self.lease_id,
            self.waited.as_secs(),
        )
    }

    /// Publish how far this run's command matrix has got (Issue #4280 AC-2):
    /// `done` of `total` commands finished in `elapsed`. Waiters then see the
    /// commands left and a paced estimate instead of the TTL remainder. Best
    /// effort, because progress is a diagnostic and never gates the run.
    pub(crate) fn publish_progress(&self, done: usize, total: usize, elapsed: Duration) {
        let Some(lease) = &self.lease else {
            return;
        };
        let Ok(lease) = lease.lock() else {
            return;
        };
        let unit_ms = match done {
            0 => 0,
            done => elapsed.as_millis() as u64 / done as u64,
        };
        if let Err(err) = lease.publish_progress(done as u64, total as u64, unit_ms) {
            tracing::warn!(error = %err, "verify.run admission: progress not published");
        }
    }

    #[cfg(test)]
    pub(crate) fn lease_id(&self) -> Option<&str> {
        Some(&self.lease_id)
    }

    #[cfg(test)]
    pub(crate) fn waited(&self) -> Duration {
        self.waited
    }
}

fn unexpected(message: String) -> SpecOpsError {
    SpecOpsError::from(ApiError::Unexpected(message))
}

/// Resolve `params.max_wait_secs` into a bounded duration.
pub(crate) fn resolve_max_wait(requested: Option<u64>) -> Result<Duration, SpecOpsError> {
    let secs = requested.unwrap_or(DEFAULT_MAX_WAIT_SECS);
    if secs > MAX_WAIT_SECS {
        return Err(unexpected(format!(
            "max_wait_secs must be at most {MAX_WAIT_SECS} (got {secs}); a longer wait inside one \
             call would read as a stalled agent to the Issue Monitor — rerun `verify.run` instead"
        )));
    }
    Ok(Duration::from_secs(secs))
}

/// Which of `roots` a process belongs to, judged by its cwd first and its
/// executable path second.
pub(crate) fn attribute_worktree<'a>(
    cwd: Option<&Path>,
    exe: Option<&Path>,
    roots: &'a [PathBuf],
) -> Option<&'a Path> {
    [cwd, exe].into_iter().flatten().find_map(|path| {
        roots
            .iter()
            .find(|root| path.starts_with(root))
            .map(PathBuf::as_path)
    })
}

/// What the current holder is, and when it is worth coming back
/// (Issue #4140 AC-3, Issue #4086 AC-4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HolderNotice {
    detail: String,
    /// A retry hint from batch progress or TTL, never a completion promise:
    /// a working holder can renew its deadline.
    retry_after: Option<Duration>,
}

/// Render one lease status into a refusal detail and an ETA.
///
/// The detail names the holder's *kind* (Issue #4086 AC-4), not just its
/// target: `repo--issues` only reads as "a background index job is in front
/// of me" to someone who already knows the target naming scheme, and that
/// was the difference between waiting the full 45 minutes and rerunning.
///
/// The ETA prefers the holder's own estimate — remaining batches × batch
/// duration for an index job — over the raw TTL remainder, because a
/// 10-minute cap says nothing about a job that has two batches left.
/// `remaining_ms` is `None` for a lease taken without a TTL, and reporting
/// that as `0s left` told agents the host was about to free up when the
/// holder was in fact unbounded — the background issue index job was exactly
/// that holder.
fn holder_notice(status: &HeavyLeaseStatus, activity: Option<&HolderActivity>) -> HolderNotice {
    let mut notice = holder_identity_notice(status);
    // Issue #4405 AC-4: `(pid 21468, 0s left)` alone reads as a hang. Say
    // whether the holder is progressing or starved of CPU.
    if let Some(activity) = activity.filter(|_| status.held) {
        notice
            .detail
            .push_str(&format!("; {}", activity.describe()));
    }
    notice
}

/// What the coordinator could establish about the holder behind the ticket
/// (Issue #4470 AC-3). `(pid 54739, 1458s left)` on its own left the waiter
/// unable to tell a working holder from residue, and the only thing it could
/// act on — the TTL — was the one number that did not apply to residue.
fn holder_liveness(status: &HeavyLeaseStatus) -> String {
    let pid = status
        .owner
        .as_ref()
        .map(|owner| owner.pid.to_string())
        .unwrap_or_else(|| "?".to_string());
    let liveness = match status.holder_alive {
        Some(true) => "alive",
        Some(false) => "gone",
        None => "liveness unknown",
    };
    let job = match status.holder_job_status {
        Some(job) => format!("job {}", job.as_str()),
        None => "job status unpublished".to_string(),
    };
    format!("pid {pid} {liveness}, {job}")
}

fn holder_identity_notice(status: &HeavyLeaseStatus) -> HolderNotice {
    let kind = status
        .holder_kind
        .unwrap_or(HeavyHolderKind::Other)
        .as_str();
    let target = status.target.as_deref().unwrap_or("unknown target");
    // Issue #4470 AC-1/AC-2: a ticket whose owner is gone, or which already
    // published a terminal job status, describes nobody. Waiting out its TTL
    // is waiting for a process that will never hand anything back.
    if status.holder_stale {
        return HolderNotice {
            detail: format!(
                "verification lease residue from {kind} {target} ({}) — no live holder, so \
                 nothing frees at the ticket's TTL",
                holder_liveness(status)
            ),
            retry_after: Some(Duration::ZERO),
        };
    }
    if !status.held {
        return HolderNotice {
            detail: "verification lease was contended".to_string(),
            retry_after: None,
        };
    }
    let holder = holder_liveness(status);
    let progress = match (status.remaining_batches, status.estimated_remaining_ms) {
        (Some(batches), Some(estimate)) => {
            format!(", {batches} batches ≈ {}s", estimate / 1000)
        }
        _ => String::new(),
    };
    let retry_after = status
        .estimated_remaining_ms
        .or(status.remaining_ms)
        .map(Duration::from_millis);
    // Issue #4409 AC-4: a deferred caller is deciding whether to keep waiting,
    // and a starved holder changes that answer.
    let priority = match (status.holder_nice, status.holder_spawn_host.as_deref()) {
        (None, None) => String::new(),
        (nice, host) => format!(
            ", nice {}, spawn-host {}",
            nice.map(|nice| nice.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
            host.unwrap_or("unknown")
        ),
    };
    match status.remaining_ms {
        Some(remaining_ms) => HolderNotice {
            detail: format!(
                "verification lease held by {kind} {target} ({holder}{priority}, {}s left{progress})",
                remaining_ms / 1000
            ),
            retry_after,
        },
        None => HolderNotice {
            detail: format!(
                "verification lease held by {kind} {target} ({holder}{priority}, no TTL — it releases \
                 only when its job finishes{progress})"
            ),
            retry_after,
        },
    }
}

/// `probe` is kept across the wait loop's polls on purpose: it makes the
/// window between readings the poll interval instead of one this call has to
/// sleep through, and a longer window is what keeps a slow-but-moving holder
/// off the "may be hung" verdict (Issue #4409 AC-9/AC-10).
fn describe_holder(
    coordinator: &IndexCoordinator,
    probe: &mut HolderProbe,
    worktree: &Path,
) -> HolderNotice {
    match coordinator.heavy_lease_status() {
        Ok(status) => {
            // Issue #4561: a `daemon` holder runs its commands outside its
            // own tree, so that tree alone reports it stalled however hard
            // the verification is working.
            let workload = crate::cli::verification_lease::holder_activity::workload_for(
                status.holder_spawn_host.as_deref(),
                || crate::cli::daemon::verification_host::live_daemon_pids(worktree),
            );
            let activity = status
                .owner
                .as_ref()
                .filter(|_| status.held)
                .and_then(|owner| probe.observe(owner.pid, &workload, status.acquired_at_ms));
            holder_notice(&status, activity.as_ref())
        }
        Err(err) => HolderNotice {
            detail: format!("verification lease status unavailable: {err}"),
            retry_after: None,
        },
    }
}

/// A refusal must always leave the caller with a next step: an ETA when the
/// holder published one, and otherwise a canonical retry trigger.
fn deferred(
    started: Instant,
    max_wait: Duration,
    detail: &str,
    retry_after: Option<Duration>,
) -> SpecOpsError {
    let next = match retry_after {
        // A stale holder and a live holder past its TTL both yield zero.
        // Only admission can establish whether the lock is now available.
        Some(Duration::ZERO) => {
            "rerun `verify.run` now to recheck admission — TTL expiry does not release a live holder".to_string()
        }
        Some(retry_after) => format!(
            "rerun `verify.run` in about {}s (timing hint only; a progressing holder renews its TTL)",
            retry_after.as_secs()
        ),
        None => "rerun `verify.run` after the current lease holder finishes".to_string(),
    };
    unexpected(format!(
        "verify: deferred — host busy for {}s (budget {}s): {detail}; {next} — a deferral is \
         not a failure and there is no attempt cap: your turn stays reserved, so keep rerunning \
         `verify.run` while the holder makes progress",
        started.elapsed().as_secs(),
        max_wait.as_secs()
    ))
}

fn sleep_until(deadline: Instant) {
    let remaining = deadline.saturating_duration_since(Instant::now());
    std::thread::sleep(remaining.min(POLL));
}

/// One Board `status` post per admission, and only once the wait has
/// outlived a poll: a wait nobody can see is the failure mode #3844 records.
#[derive(Default)]
struct BoardNotice {
    posted: bool,
}

impl BoardNotice {
    fn maybe_post<E: CliEnv>(
        &mut self,
        env: &mut E,
        started: Instant,
        max_wait: Duration,
        reason: &str,
    ) {
        if self.posted || started.elapsed() < BOARD_NOTICE_AFTER {
            return;
        }
        self.posted = true;
        let body = format!(
            "現在の状態: verify.run は host 排他待ちです（{}s 経過、上限 {}s）。\n\n\
             理由: {reason}\n\n\
             次: 上限まで待って開始します。超過した場合は deferred で返し、agent が再実行します。",
            started.elapsed().as_secs(),
            max_wait.as_secs()
        );
        let command = BoardCommand::Post(Box::new(BoardPostCommand {
            kind: "status".to_string(),
            body: Some(body),
            broadcast: true,
            ..Default::default()
        }));
        let mut scratch = String::new();
        if let Err(err) = crate::cli::board::run(env, command, &mut scratch) {
            tracing::warn!(error = %err, "verify.run admission: Board notice failed");
        }
    }
}

/// Claim host admission for a `verify.run` in `worktree`, waiting at most
/// `max_wait`. A wait that outlives the budget answers with a `deferred`
/// error naming what the host was busy with; the caller reports a granted
/// admission through `Admission::summary`.
pub(crate) fn admit<E: CliEnv>(
    env: &mut E,
    worktree: &Path,
    max_wait: Duration,
) -> Result<Admission, SpecOpsError> {
    let key = verification_lease::verification_key(env)?;
    let coordinator = verification_lease::open_coordinator()?;
    let started = Instant::now();
    let deadline = started + max_wait;
    let mut notice = BoardNotice::default();

    // Every invocation owns its locks; matching the worktree is not proof
    // that another run's lease belongs to this invocation.
    let guard = loop {
        match coordinator
            .request_job(&key, JobPriority::ManualRebuild, NON_BLOCKING)
            .map_err(|err| unexpected(format!("verification job admission failed: {err}")))?
        {
            JobAdmission::Owner(guard) => break guard,
            JobAdmission::Joined(waiter) => {
                // A concurrent canonical run in this same worktree owns
                // the target job until its command matrix finishes.
                drop(waiter);
                if Instant::now() >= deadline {
                    return Err(deferred(
                        started,
                        max_wait,
                        "another verification claimant in this worktree owns the target job",
                        None,
                    ));
                }
                notice.maybe_post(
                    env,
                    started,
                    max_wait,
                    "同じ worktree の別 claimant が verification target job を保持",
                );
                sleep_until(deadline);
            }
        }
    };
    let mut probe = HolderProbe::default();
    let lease = loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match guard.acquire_heavy_with_ttl(remaining.min(POLL), LEASE_TTL) {
            Ok(lease) => break lease,
            Err(CoordinatorError::Timeout { .. }) => {
                let holder = describe_holder(&coordinator, &mut probe, worktree);
                if Instant::now() >= deadline {
                    let _ = guard.complete(JobOutcome::Failed {
                        message: "host admission deferred".to_string(),
                    });
                    // Issue #4086 AC-1: the rerun must be admitted before any
                    // background index job that queues in the meantime.
                    let reserved = coordinator.reserve_heavy(
                        &key,
                        JobPriority::ManualRebuild,
                        VERIFICATION_RESERVATION_TTL,
                        Some("verify.run deferred"),
                    );
                    let mut detail = holder.detail;
                    // Issue #4337 AC-3: name the reservation outcome outright.
                    // `queue_position` below only ever appears on success, so
                    // on its own it leaves the rerun unable to tell a failed
                    // reservation from a failed status read — and the two call
                    // for opposite expectations: a reserved turn is kept for
                    // the rerun, an unreserved one rejoins at the back.
                    match &reserved {
                        Ok(_) => detail.push_str("; next_turn_reserved: yes"),
                        Err(err) => {
                            detail.push_str(&format!("; next_turn_reserved: no ({err})"));
                        }
                    }
                    if let Ok(status) = coordinator.heavy_lease_status() {
                        if let Some(position) = status.queue.iter().position(|entry| {
                            entry.target.as_deref() == Some(key.file_stem().as_str())
                        }) {
                            detail.push_str(&format!("; queue_position: {}", position + 1));
                        }
                    }
                    return Err(deferred(started, max_wait, &detail, holder.retry_after));
                }
                notice.maybe_post(env, started, max_wait, &holder.detail);
            }
            Err(err) => {
                let _ = guard.complete(JobOutcome::Failed {
                    message: err.to_string(),
                });
                return Err(unexpected(format!(
                    "verification lease acquisition failed: {err}"
                )));
            }
        }
    };
    let mut lease = lease;
    // Issue #4409 AC-4: a waiter needs to know whether this holder escaped the
    // agent process tree, because a holder that did not will take far longer
    // than its history suggests.
    let worktree = gwt_core::paths::resolve_current_worktree_root(env.repo_path());
    let (spawn_host, _) = crate::cli::daemon::verification_host::describe_for_lease(&worktree);
    lease.record_spawn_host(spawn_host);
    let lease_id = lease.id().to_string();
    let lease = Arc::new(Mutex::new(lease));
    let commands = Arc::new(super::CommandProgress::default());
    let renewal = super::renewal::Renewal::start(
        Arc::clone(&lease),
        coordinator,
        worktree,
        Arc::clone(&commands),
    )
    .map_err(|error| {
        unexpected(format!(
            "verification renewal monitor failed to start: {error}"
        ))
    })?;
    let admission = Admission {
        guard: Some(guard),
        lease_id,
        lease: Some(lease),
        renewal: Some(renewal),
        commands,
        waited: started.elapsed(),
    };

    Ok(admission)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_core::index_coordinator::{
        HeavyLeaseStatus, IndexCoordinator, JobAdmission, JobPriority, TargetKey,
    };
    use gwt_core::test_support::ScopedGwtHome;

    #[test]
    fn attribute_worktree_prefers_cwd_then_exe_and_ignores_unrelated() {
        let roots = vec![PathBuf::from("/repo/work/a"), PathBuf::from("/repo/work/b")];
        assert_eq!(
            attribute_worktree(
                Some(Path::new("/repo/work/b/crates/gwt")),
                Some(Path::new("/toolchain/bin/rustc")),
                &roots
            ),
            Some(Path::new("/repo/work/b"))
        );
        assert_eq!(
            attribute_worktree(
                None,
                Some(Path::new("/repo/work/a/target/debug/deps/x-1")),
                &roots
            ),
            Some(Path::new("/repo/work/a"))
        );
        assert_eq!(
            attribute_worktree(
                Some(Path::new("/elsewhere")),
                Some(Path::new("/toolchain/bin/rustc")),
                &roots
            ),
            None
        );
    }

    /// AC-3: the bounded wait must never outlive the Issue Monitor's stuck
    /// window, or a single `verify.run` call would consume an autonomous
    /// attempt while doing exactly what it was told to.
    #[test]
    fn wait_budget_defaults_and_hard_cap_stay_below_stuck_timeout() {
        let stuck = crate::issue_monitor::AutonomousTuning::default().stuck_timeout_secs;
        assert!(
            MAX_WAIT_SECS < stuck,
            "{MAX_WAIT_SECS} must stay below {stuck}"
        );
        assert_eq!(
            resolve_max_wait(None).unwrap(),
            Duration::from_secs(DEFAULT_MAX_WAIT_SECS)
        );
        assert_eq!(resolve_max_wait(Some(0)).unwrap(), Duration::ZERO);
        assert_eq!(
            resolve_max_wait(Some(MAX_WAIT_SECS)).unwrap(),
            Duration::from_secs(MAX_WAIT_SECS)
        );
        let err = resolve_max_wait(Some(MAX_WAIT_SECS + 1)).unwrap_err();
        assert!(err.to_string().contains("max_wait_secs"), "{err}");
    }

    /// Issue #4409 AC-4: a deferred caller is deciding whether waiting is
    /// worth it, and a holder that is itself starved will take far longer than
    /// its history suggests. The refusal has to say so.
    #[test]
    fn a_deferred_refusal_reports_the_holders_priority_and_spawn_host() {
        let notice = holder_notice(
            &HeavyLeaseStatus {
                held: true,
                target: Some("repo--verification".to_string()),
                owner: Some(gwt_core::index_coordinator::OwnerIdentity {
                    pid: 4242,
                    start_id: "start".to_string(),
                }),
                remaining_ms: Some(60_000),
                holder_nice: Some(10),
                holder_spawn_host: Some("inherit".to_string()),
                ..HeavyLeaseStatus::default()
            },
            None,
        );
        assert!(notice.detail.contains("nice 10"), "{}", notice.detail);
        assert!(
            notice.detail.contains("spawn-host inherit"),
            "{}",
            notice.detail
        );
    }

    /// A pre-#4409 ticket carries neither field. The refusal must stay
    /// readable rather than printing "nice unknown, spawn-host unknown" at
    /// every caller that ever waits on an older holder.
    #[test]
    fn a_holder_that_published_no_priority_is_described_without_empty_fields() {
        let notice = holder_notice(
            &HeavyLeaseStatus {
                held: true,
                target: Some("repo--verification".to_string()),
                remaining_ms: Some(60_000),
                ..HeavyLeaseStatus::default()
            },
            None,
        );
        assert!(!notice.detail.contains("nice"), "{}", notice.detail);
        assert!(!notice.detail.contains("spawn-host"), "{}", notice.detail);
    }

    /// Issue #4140 AC-3: a holder with a TTL must publish a usable ETA, and a
    /// holder without one must say so instead of reading as "0s left" — the
    /// index job's untimed lease is exactly the case that misled agents into
    /// waiting indefinitely.
    #[test]
    fn holder_notice_reports_an_eta_only_when_the_holder_has_a_ttl() {
        let timed = holder_notice(
            &HeavyLeaseStatus {
                held: true,
                target: Some("repo--issues".to_string()),
                owner: Some(gwt_core::index_coordinator::OwnerIdentity {
                    pid: 32420,
                    start_id: "start".to_string(),
                }),
                remaining_ms: Some(320_000),
                ..HeavyLeaseStatus::default()
            },
            None,
        );
        assert_eq!(timed.retry_after, Some(Duration::from_secs(320)));
        assert!(timed.detail.contains("repo--issues"), "{}", timed.detail);
        assert!(timed.detail.contains("320s left"), "{}", timed.detail);

        let untimed = holder_notice(
            &HeavyLeaseStatus {
                held: true,
                target: Some("repo--issues".to_string()),
                remaining_ms: None,
                ..HeavyLeaseStatus::default()
            },
            None,
        );
        assert_eq!(untimed.retry_after, None);
        assert!(
            !untimed.detail.contains("0s left"),
            "an untimed lease must not claim it is about to lapse: {}",
            untimed.detail
        );
        assert!(untimed.detail.contains("no TTL"), "{}", untimed.detail);

        let free = holder_notice(&HeavyLeaseStatus::default(), None);
        assert_eq!(free.retry_after, None);
        assert!(free.detail.contains("contended"), "{}", free.detail);
    }

    /// Issue #4405 AC-4: a waiter must be able to tell a starved holder from
    /// a hung one. `host busy for 60s ... (pid 21468, 0s left)` read as a
    /// hang, and four windows considered `execution.blocked` over it.
    #[test]
    fn holder_notice_says_a_starved_holder_is_running_not_hung() {
        let status = HeavyLeaseStatus {
            held: true,
            target: Some("repo--verification--wt".to_string()),
            owner: Some(gwt_core::index_coordinator::OwnerIdentity {
                pid: 21468,
                start_id: "start".to_string(),
            }),
            remaining_ms: Some(0),
            ..HeavyLeaseStatus::default()
        };
        let starved = HolderActivity {
            held_ms: 7_260_000,
            cpu_percent: 1.4,
            cpu_gained_ms: 17,
            turnover: false,
            processes: 1,
            window_ms: 1_200,
            host_cpu_percent: Some(95.0),
            delegated: false,
            parent_gone: false,
            workload_processes: 1,
        };
        let notice = holder_notice(&status, Some(&starved));
        assert!(notice.detail.contains("pid 21468"), "{}", notice.detail);
        assert!(notice.detail.contains("starved"), "{}", notice.detail);
        assert!(notice.detail.contains("not hung"), "{}", notice.detail);

        let progressing = HolderActivity {
            held_ms: 600_000,
            cpu_percent: 380.0,
            cpu_gained_ms: 4_560,
            turnover: true,
            processes: 5,
            window_ms: 1_200,
            host_cpu_percent: Some(95.0),
            delegated: false,
            parent_gone: false,
            workload_processes: 1,
        };
        let notice = holder_notice(&status, Some(&progressing));
        assert!(notice.detail.contains("progressing"), "{}", notice.detail);
    }

    /// Issue #4470 AC-3: `(pid 54739, 1458s left)` gave a waiter nothing to
    /// decide with. Every refusal now states whether the holder's process is
    /// still there and what job status it last published.
    #[test]
    fn holder_notice_states_holder_liveness_and_job_status() {
        let live = holder_notice(
            &HeavyLeaseStatus {
                held: true,
                target: Some("repo--verification--wt".to_string()),
                owner: Some(gwt_core::index_coordinator::OwnerIdentity {
                    pid: 36696,
                    start_id: "start".to_string(),
                }),
                remaining_ms: Some(320_000),
                holder_alive: Some(true),
                holder_job_status: Some(gwt_core::index_coordinator::JobStatus::Running),
                ..HeavyLeaseStatus::default()
            },
            None,
        );
        assert!(live.detail.contains("pid 36696"), "{}", live.detail);
        assert!(live.detail.contains("alive"), "{}", live.detail);
        assert!(live.detail.contains("running"), "{}", live.detail);
        assert!(live.detail.contains("320s left"), "{}", live.detail);
    }

    /// Issue #4470 AC-1 / AC-2: residue must read as residue. A holder that
    /// is gone offers no reason to wait, so the refusal says so and points at
    /// an immediate rerun instead of the ticket's TTL remainder.
    #[test]
    fn holder_notice_calls_a_gone_holder_residue_and_retries_at_once() {
        let stale = holder_notice(
            &HeavyLeaseStatus {
                held: false,
                target: Some("repo--verification--wt".to_string()),
                owner: Some(gwt_core::index_coordinator::OwnerIdentity {
                    pid: 54739,
                    start_id: "start".to_string(),
                }),
                holder_alive: Some(false),
                holder_job_status: Some(gwt_core::index_coordinator::JobStatus::Completed),
                holder_stale: true,
                ..HeavyLeaseStatus::default()
            },
            None,
        );
        assert!(stale.detail.contains("residue"), "{}", stale.detail);
        assert!(stale.detail.contains("pid 54739"), "{}", stale.detail);
        assert!(stale.detail.contains("gone"), "{}", stale.detail);
        assert!(stale.detail.contains("completed"), "{}", stale.detail);
        assert!(
            !stale.detail.contains("s left"),
            "residue must not offer a TTL to wait out: {}",
            stale.detail
        );
        assert_eq!(stale.retry_after, Some(Duration::ZERO));

        let refusal = deferred(
            Instant::now(),
            Duration::from_secs(300),
            &stale.detail,
            stale.retry_after,
        )
        .to_string();
        assert!(
            refusal.contains("rerun `verify.run` now"),
            "a lease with no live holder must be retried immediately: {refusal}"
        );
    }

    /// Issue #4140 AC-3: every refusal carries a concrete next step, so an
    /// agent never has to guess whether waiting again is pointless.
    #[test]
    fn deferred_always_names_a_retry_trigger() {
        let with_eta = deferred(
            Instant::now(),
            Duration::from_secs(300),
            "verification lease held by repo--issues",
            Some(Duration::from_secs(320)),
        )
        .to_string();
        assert!(with_eta.contains("deferred"), "{with_eta}");
        assert!(with_eta.contains("about 320s"), "{with_eta}");
        assert!(with_eta.contains("verify.run"), "{with_eta}");
        assert!(with_eta.contains("timing hint only"), "{with_eta}");

        let expired = deferred(
            Instant::now(),
            Duration::from_secs(300),
            "verification lease held by a live holder",
            Some(Duration::ZERO),
        )
        .to_string();
        assert!(expired.contains("TTL expiry does not release"), "{expired}");
        assert!(!expired.contains("no live holder"), "{expired}");

        let without_eta = deferred(
            Instant::now(),
            Duration::from_secs(300),
            "another canonical verification is still running",
            None,
        )
        .to_string();
        assert!(without_eta.contains("verify.run"), "{without_eta}");
        assert!(
            !without_eta.contains("verify.lease.acquire"),
            "canonical admission must not recommend detached manual acquisition: {without_eta}"
        );
        // Issue #4280 AC-3: a deferral is a reserved turn, not a spent
        // attempt — counting it toward a cap is what made waiters give up.
        for message in [&with_eta, &without_eta] {
            assert!(!message.contains("lease attempt"), "{message}");
            assert!(message.contains("no attempt cap"), "{message}");
            assert!(message.contains("turn stays reserved"), "{message}");
        }
    }

    /// How long a released lease may still read as held before the release is
    /// treated as broken. Sized for a fork/exec window on a saturated CI
    /// runner, not for a lease that was never released (Issue #3937).
    const RELEASE_OBSERVATION_BUDGET: Duration = Duration::from_secs(5);

    /// Issue #3937: one test's private verification-lease root, plus the
    /// diagnosis a failure owes its reader.
    ///
    /// [`ScopedGwtHome`] is thread-local, so every `gwt_home()`-derived path a
    /// test resolves — the coordinator root included — is private to the
    /// running test thread. What the CI failure showed missing was proof and
    /// evidence: the drop assertion read "dropping the admission must release
    /// the lease" and named neither the holder nor the file that was still
    /// locked, so one message fitted a leaked ambient lease, a foreign holder,
    /// and a genuine release bug alike. This fixture proves the isolation
    /// before the test body runs and renders the holder and both lease paths
    /// into every assertion it owns.
    struct IsolatedLeaseRoot {
        coordinator: IndexCoordinator,
        home: tempfile::TempDir,
        // Dropped last so the override outlives every path this test resolves.
        _home_guard: ScopedGwtHome,
    }

    impl IsolatedLeaseRoot {
        fn new() -> Self {
            let home = tempfile::tempdir().unwrap();
            let _home_guard = ScopedGwtHome::set(home.path());
            let coordinator = IndexCoordinator::open_default_verification().unwrap();
            let root = Self {
                coordinator,
                home,
                _home_guard,
            };
            assert!(
                root.coordinator.root().starts_with(root.home.path()),
                "the coordinator root must be this test's temp lease root, not {}",
                root.coordinator.root().display()
            );
            root.assert_free("a private lease root must start free");
            root
        }

        /// Holder identity and both lease paths (AC-3), so a CI-only
        /// recurrence can be read without a local reproduction.
        fn describe(&self) -> String {
            let holder = match self.coordinator.heavy_lease_status() {
                Ok(status) if status.held => {
                    let owner = status
                        .owner
                        .as_ref()
                        .map(|owner| owner.pid.to_string())
                        .unwrap_or_else(|| "?".to_string());
                    format!(
                        "held by target {} (lease {}, owner pid {owner}, {} waiting)",
                        status.target.as_deref().unwrap_or("<no ticket>"),
                        status.lease_id.as_deref().unwrap_or("<no lease id>"),
                        status.pending
                    )
                }
                Ok(status) => format!("free ({} waiting)", status.pending),
                Err(err) => format!("unreadable: {err}"),
            };
            let targets = self
                .lock_paths()
                .into_iter()
                .skip(1)
                .map(|path| {
                    let state = std::fs::read_to_string(path.with_extension("state.json"))
                        .unwrap_or_else(|err| format!("unreadable holder state: {err}"));
                    format!("target lock {}; holder state {state}", path.display())
                })
                .collect::<Vec<_>>()
                .join("; ");
            format!(
                "verification lease {holder}; this process is pid {}; lock {}; ticket {}; {targets}",
                std::process::id(),
                self.coordinator.heavy_lock_path().display(),
                self.coordinator.heavy_ticket_path().display()
            )
        }

        fn status(&self) -> HeavyLeaseStatus {
            self.coordinator.heavy_lease_status().unwrap_or_else(|err| {
                panic!(
                    "the lease status must stay readable: {err}; {}",
                    self.describe()
                )
            })
        }

        fn admit(
            &self,
            env: &mut crate::cli::TestEnv,
            worktree: &Path,
            budget: Duration,
        ) -> Result<Admission, SpecOpsError> {
            super::admit(env, worktree, budget)
                .map_err(|err| unexpected(format!("{err}; {}", self.describe())))
        }

        /// Include every target in this private coordinator, so future
        /// acquisition tests inherit the same release observation.
        fn lock_paths(&self) -> Vec<PathBuf> {
            let mut paths = vec![self.coordinator.heavy_lock_path()];
            for entry in std::fs::read_dir(self.coordinator.root().join("targets")).unwrap() {
                let path = entry.unwrap().path();
                if path.extension().is_some_and(|ext| ext == "lock") {
                    paths.push(path);
                }
            }
            paths
        }

        fn is_free(&self) -> bool {
            self.lock_paths().iter().all(|path| {
                let probe = std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .open(path)
                    .unwrap();
                match fs2::FileExt::try_lock_exclusive(&probe) {
                    Ok(()) => {
                        // Explicit unlock also releases any fork-inherited
                        // copy of this probe's own file description.
                        fs2::FileExt::unlock(&probe).unwrap();
                        true
                    }
                    Err(err)
                        if err.raw_os_error() == fs2::lock_contended_error().raw_os_error() =>
                    {
                        false
                    }
                    Err(err) => panic!("lock probe failed: {err}; {}", self.describe()),
                }
            })
        }

        /// Wait for the release to become observable.
        ///
        /// Probe both the heavy and target kernel locks: status metadata can
        /// already say free while an inherited descriptor remains locked.
        /// Every `Command::spawn` on any thread of this test binary
        /// duplicates the holder's descriptor into the forked child —
        /// `O_CLOEXEC` closes it at `exec`, not at `fork`. So for the length of
        /// one fork/exec window an unrelated child keeps the `flock` alive and
        /// the lease still reads as held, seconds after its owner dropped it.
        /// Measured on a 3000-release loop: 0 phantom holds with no concurrent
        /// spawns, 5 with two spawner threads — the rate that reddened an
        /// unrelated PR's CI (Issue #3937) inside a ~2900-test binary that
        /// spawns subprocesses continuously. Waiting keeps the assertion
        /// honest: a lease that is genuinely never released never becomes
        /// free, so a real regression still fails here.
        fn assert_free(&self, context: &str) {
            let deadline = Instant::now() + RELEASE_OBSERVATION_BUDGET;
            loop {
                if self.is_free() {
                    return;
                }
                assert!(
                    Instant::now() < deadline,
                    "{context} — still held {}s after the release; {}",
                    RELEASE_OBSERVATION_BUDGET.as_secs(),
                    self.describe()
                );
                std::thread::sleep(Duration::from_millis(100));
            }
        }

        fn assert_held(&self, context: &str) -> HeavyLeaseStatus {
            let status = self.status();
            assert!(status.held, "{context} — {}", self.describe());
            status
        }
    }

    /// AC-3: every lease assertion in this module reports the holder, this
    /// process, and both lease files.
    #[test]
    fn lease_diagnostics_name_the_holder_and_both_lease_paths() {
        let lease_root = IsolatedLeaseRoot::new();
        assert!(
            lease_root.describe().contains("free"),
            "{}",
            lease_root.describe()
        );

        let worktree = tempfile::tempdir().unwrap();
        let key = verification_key_for(worktree.path());
        let JobAdmission::Owner(guard) = lease_root
            .coordinator
            .request_job(&key, JobPriority::ManualRebuild, Duration::from_millis(250))
            .unwrap()
        else {
            panic!(
                "a private lease root must admit the owner — {}",
                lease_root.describe()
            );
        };
        let lease = guard
            .acquire_heavy_with_ttl(Duration::from_millis(250), Duration::from_secs(60))
            .unwrap();

        let described = lease_root.describe();
        for expected in [
            key.file_stem(),
            lease.id().to_string(),
            std::process::id().to_string(),
            lease_root
                .coordinator
                .heavy_lock_path()
                .display()
                .to_string(),
            lease_root
                .coordinator
                .heavy_ticket_path()
                .display()
                .to_string(),
            lease_root
                .coordinator
                .target_lock_path(&key)
                .display()
                .to_string(),
            "holder state".to_string(),
        ] {
            assert!(
                described.contains(&expected),
                "the diagnosis must name {expected}: {described}"
            );
        }

        drop(lease);
        drop(guard);
        lease_root.assert_free("releasing the lease must leave the private root free");
    }

    fn verification_key_for(worktree: &Path) -> TargetKey {
        let root = gwt_core::paths::resolve_current_worktree_root(worktree);
        TargetKey::verification(
            gwt_core::paths::project_scope_hash(&root).as_str(),
            gwt_core::worktree_hash::compute_worktree_hash(&root)
                .unwrap()
                .as_str(),
        )
    }

    #[test]
    fn admit_acquires_the_lease_in_process_and_releases_on_drop() {
        let lease_root = IsolatedLeaseRoot::new();
        let worktree = tempfile::tempdir().unwrap();
        let mut env = crate::cli::TestEnv::new(worktree.path().to_path_buf());

        let admission = lease_root
            .admit(&mut env, worktree.path(), Duration::from_secs(5))
            .unwrap_or_else(|err| {
                panic!(
                    "a private lease root must grant admission: {err} — {}",
                    lease_root.describe()
                )
            });

        let status = lease_root.assert_held("admission must hold the host-wide lease");
        assert_eq!(
            status.target.as_deref(),
            Some(verification_key_for(worktree.path()).file_stem().as_str()),
            "{}",
            lease_root.describe()
        );
        assert_eq!(
            admission.lease_id(),
            status.lease_id.as_deref(),
            "{}",
            lease_root.describe()
        );
        assert!(
            admission.summary().contains("host admission"),
            "{}",
            admission.summary()
        );
        assert!(
            admission.waited() < Duration::from_secs(5),
            "{}",
            admission.summary()
        );

        drop(admission);
        lease_root.assert_free("dropping the admission must release the lease");
    }

    /// Issue #4280 AC-2: the admitted run publishes its command progress, so
    /// a waiter reads the commands left and a paced ETA from the status.
    #[test]
    fn admission_publishes_the_runs_command_progress() {
        let lease_root = IsolatedLeaseRoot::new();
        let worktree = tempfile::tempdir().unwrap();
        let mut env = crate::cli::TestEnv::new(worktree.path().to_path_buf());
        let admission = lease_root
            .admit(&mut env, worktree.path(), Duration::from_secs(5))
            .unwrap();

        admission.publish_progress(0, 4, Duration::ZERO);
        let status = lease_root.assert_held("the run holds the lease");
        assert_eq!(status.remaining_batches, Some(4));
        assert_eq!(status.estimated_remaining_ms, status.remaining_ms);

        // Two commands took 60 s in total: two more at 30 s each.
        admission.publish_progress(2, 4, Duration::from_secs(60));
        let status = lease_root.assert_held("the run still holds the lease");
        assert_eq!(status.remaining_batches, Some(2));
        assert_eq!(status.estimated_remaining_ms, Some(60_000));

        drop(admission);
        lease_root.assert_free("dropping the admission must release the lease");
    }

    #[test]
    fn canonical_runs_in_the_same_worktree_do_not_share_admission() {
        let lease_root = IsolatedLeaseRoot::new();
        let worktree = tempfile::tempdir().unwrap();
        let mut env = crate::cli::TestEnv::new(worktree.path().to_path_buf());
        let first = lease_root
            .admit(&mut env, worktree.path(), Duration::ZERO)
            .unwrap();
        let second = lease_root.admit(&mut env, worktree.path(), Duration::ZERO);
        assert!(
            second.is_err(),
            "a second canonical run must not borrow the first run's lease: {second:?}; {}",
            lease_root.describe()
        );
        let refusal = second.unwrap_err().to_string();
        assert!(refusal.contains("deferred"), "{refusal}");
        drop(first);
        lease_root.assert_free("the first run releases its own lease");
        let next = lease_root
            .admit(&mut env, worktree.path(), Duration::ZERO)
            .unwrap();
        drop(next);
        lease_root.assert_free("the next run releases its own lease");
    }

    /// Issue #4593: a concurrent fork between the two drops in Admission::settle
    /// can keep only the target lock alive. A free heavy lock is not enough.
    #[cfg(unix)]
    #[test]
    fn release_observation_waits_for_a_fork_inherited_target_lock() {
        use std::os::fd::AsRawFd;
        use std::os::unix::net::UnixStream;

        let lease_root = IsolatedLeaseRoot::new();
        let worktree = tempfile::tempdir().unwrap();
        let mut env = crate::cli::TestEnv::new(worktree.path().to_path_buf());
        let key = verification_key_for(worktree.path());
        // Construct the state after the heavy lease has been released, so
        // an unrelated fork cannot inherit a heavy lock from this test.
        let JobAdmission::Owner(first) = lease_root
            .coordinator
            .request_job(&key, JobPriority::ManualRebuild, Duration::from_secs(1))
            .unwrap()
        else {
            panic!("private target must be free; {}", lease_root.describe());
        };

        let (reader, release) = UnixStream::pair().unwrap();
        let read_fd = reader.as_raw_fd();
        let release_fd = release.as_raw_fd();
        // SAFETY: the fork child only uses async-signal-safe libc calls and
        // _exit; it never enters Rust allocation, unwinding, or destructors.
        let child = unsafe { libc::fork() };
        assert!(child >= 0, "{}", std::io::Error::last_os_error());
        if child == 0 {
            unsafe {
                libc::close(release_fd);
                let mut byte = 0u8;
                // EOF is the parent's release signal; an interrupted read
                // must not release the inherited lock early.
                while libc::read(read_fd, (&mut byte as *mut u8).cast(), 1) != 0 {}
                libc::_exit(0);
            }
        }
        drop(reader);
        // Always release and reap the child, including when the regression
        // assertion fails. No wall-clock deadline controls the interleaving.
        let observed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            first.complete(JobOutcome::Completed).unwrap();
            assert!(!lease_root.status().held, "heavy lock must already be free");
            assert!(
                matches!(
                    lease_root
                        .coordinator
                        .request_job(&key, JobPriority::ManualRebuild, Duration::from_secs(1))
                        .unwrap(),
                    JobAdmission::Joined(_)
                ),
                "the child must retain the target lock; {}",
                lease_root.describe()
            );
            assert!(
                !lease_root.is_free(),
                "the inherited target lock must prevent a free observation: {}",
                lease_root.describe()
            );
        }));
        // A different concurrent fork may inherit the parent's socket too.
        // Shutdown affects the socket itself, unlike dropping one descriptor.
        let released = release.shutdown(std::net::Shutdown::Write);
        drop(release);
        loop {
            let waited = unsafe { libc::waitpid(child, std::ptr::null_mut(), 0) };
            if waited == child {
                break;
            }
            assert_eq!(
                std::io::Error::last_os_error().kind(),
                std::io::ErrorKind::Interrupted
            );
        }
        released.unwrap();
        if let Err(panic) = observed {
            std::panic::resume_unwind(panic);
        }
        lease_root.assert_free("the child's exit releases the inherited target lock");
        let next = lease_root
            .admit(&mut env, worktree.path(), Duration::ZERO)
            .unwrap();
        drop(next);
        lease_root.assert_free("the next admission releases both locks");
    }

    /// Issue #4285 AC-1 / AC-3 / AC-4: a canonical verification holding its
    /// lease must not stop a query encode, and the model lane must still
    /// admit only one model-loaded runner tree (FR-417 / AS-30) while both
    /// lanes are busy.
    #[test]
    fn search_and_index_keep_their_own_exclusion_while_verification_holds_its_lease() {
        let lease_root = IsolatedLeaseRoot::new();
        let worktree = tempfile::tempdir().unwrap();
        let mut env = crate::cli::TestEnv::new(worktree.path().to_path_buf());
        let admission = lease_root
            .admit(&mut env, worktree.path(), Duration::from_secs(5))
            .unwrap();
        lease_root.assert_held("admission must hold the verification lease");

        let model_lane = IndexCoordinator::open_default().unwrap();
        assert_ne!(
            model_lane.heavy_lock_path(),
            lease_root.coordinator.heavy_lock_path(),
            "verification and the model lane must not share heavy.lock"
        );
        // AC-1: the query encode is admitted while verification runs.
        let search = model_lane
            .acquire_interactive_search_heavy(
                &TargetKey::search("repo", None),
                Duration::from_millis(500),
            )
            .unwrap_or_else(|err| {
                panic!(
                    "search must not wait for canonical verification: {err} — {}",
                    lease_root.describe()
                )
            });
        // AC-3: a build cannot load a second model tree next to the search.
        let JobAdmission::Owner(build) = model_lane
            .request_job(
                &TargetKey::repo_shared("repo", "issues"),
                JobPriority::Background,
                Duration::from_millis(250),
            )
            .unwrap()
        else {
            panic!("the build target must be free");
        };
        match build.acquire_heavy(Duration::from_millis(120)) {
            Err(CoordinatorError::Timeout { .. }) => {}
            Err(other) => panic!("expected a timeout on the model lane: {other:?}"),
            Ok(_) => panic!("the model lane must stay exclusive next to a search"),
        }
        build.complete(JobOutcome::Completed).unwrap();
        drop(search);
        drop(admission);
        lease_root.assert_free("dropping the admission must release the lease");
    }

    /// Issue #4285 AC-2: a query encode holding the model lane must not stop
    /// canonical verification from being admitted.
    #[test]
    fn verification_is_admitted_while_a_search_holds_the_model_lease() {
        let lease_root = IsolatedLeaseRoot::new();
        let worktree = tempfile::tempdir().unwrap();
        let model_lane = IndexCoordinator::open_default().unwrap();
        let _search = model_lane
            .acquire_interactive_search_heavy(
                &TargetKey::search("repo", None),
                Duration::from_millis(500),
            )
            .unwrap();

        let mut env = crate::cli::TestEnv::new(worktree.path().to_path_buf());
        let admission = lease_root
            .admit(&mut env, worktree.path(), Duration::from_secs(1))
            .unwrap_or_else(|err| {
                panic!(
                    "verification must not wait for a search: {err} — {}",
                    lease_root.describe()
                )
            });
        lease_root.assert_held("admission must hold the verification lease");
        drop(admission);
        lease_root.assert_free("dropping the admission must release the lease");
    }

    #[test]
    fn admit_defers_when_another_target_holds_the_lease() {
        let lease_root = IsolatedLeaseRoot::new();
        let worktree = tempfile::tempdir().unwrap();
        // Issue #4285: only another canonical verification contends on this
        // lane; index builds and searches live on the model lane.
        let other = TargetKey::verification("other-repo", "other-worktree");
        let JobAdmission::Owner(guard) = lease_root
            .coordinator
            .request_job(
                &other,
                JobPriority::ManualRebuild,
                Duration::from_millis(250),
            )
            .unwrap()
        else {
            panic!(
                "a private lease root must admit the owner — {}",
                lease_root.describe()
            );
        };
        let _lease = guard
            .acquire_heavy_with_ttl(Duration::from_millis(250), Duration::from_secs(60))
            .unwrap();

        let mut env = crate::cli::TestEnv::new(worktree.path().to_path_buf());
        let err = lease_root
            .admit(&mut env, worktree.path(), Duration::from_secs(1))
            .unwrap_err();

        let message = err.to_string();
        assert!(message.contains("deferred"), "{message}");
        assert!(message.contains("rerun `verify.run`"), "{message}");
        assert!(message.contains("queue_position: 1"), "{message}");
        assert!(
            message.contains(&other.file_stem()),
            "the refusal must name the holder: {message}"
        );
        // Issue #4337 AC-3: the refusal states the reservation outcome
        // outright. `queue_position` alone only ever appears on success, so
        // its absence reads the same whether the reservation failed or the
        // status read did — and the rerun's outlook differs entirely.
        assert!(
            message.contains("next_turn_reserved: yes"),
            "the refusal must say the next turn is reserved: {message}"
        );
        // Issue #4086: a deferred run leaves its turn reserved so the rerun
        // is admitted before any background index job.
        let key = verification_lease::verification_key(&mut env).unwrap();
        assert!(
            lease_root.coordinator.heavy_reservation_path(&key).exists(),
            "a deferred admission must reserve the next turn — {}",
            lease_root.describe()
        );
        assert_eq!(
            lease_root.coordinator.heavy_lease_status().unwrap().pending,
            1
        );
    }
}
