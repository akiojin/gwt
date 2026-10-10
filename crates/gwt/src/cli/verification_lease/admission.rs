//! SPEC #3576: automatic admission for canonical `verify.run` execution.
//!
//! Each Heavy command acquires its target job and host-wide lease in-process.
//! Even runs in the same worktree must wait for each other. Dropping the
//! admission releases both locks; there is no detached pre-acquisition.
//! Ordinary builds and development tests do not participate in admission.
//!
//! Admission preserves the existing bounded wait, FIFO reservations, holder
//! diagnostics, and Board notice. The runner retains partial results when a
//! later command defers; a first-command deferral writes no new run record.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gwt_core::index_coordinator::{
    CoordinatorError, HeavyAttempt, HeavyHolderKind, HeavyLease, HeavyLeaseStatus,
    IndexCoordinator, JobAdmission, JobOutcome, JobPriority, TargetJobGuard,
    VERIFICATION_RESERVATION_TTL,
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
const CANCELLABLE_POLL: Duration = Duration::from_millis(100);
/// Our own verification target job only ever contends with a same-worktree
/// claimant, so claiming it does not need to block.
const NON_BLOCKING: Duration = Duration::from_millis(250);
/// TTL of the in-process lease. The kernel lock releases on exit regardless;
/// the TTL only bounds how long crash residue can look live.
const LEASE_TTL: Duration = Duration::from_secs(DEFAULT_TTL_MINUTES * 60);
/// Waits shorter than one poll are not worth a Board post.
const BOARD_NOTICE_AFTER: Duration = POLL;

/// Admission is a control-flow outcome, not a failed verification command.
/// Keep its recovery facts typed all the way to the JSON envelope (#5085).
#[derive(Debug, serde::Serialize)]
pub(crate) struct Deferral {
    pub waited_secs: u64,
    pub budget_secs: u64,
    pub cleanup_secs: u64,
    pub elapsed_secs: u64,
    pub next_turn_reserved: Option<bool>,
    pub queue_position: Option<usize>,
    pub retry_after_secs: Option<u64>,
    pub reason: String,
    pub recovery: String,
    #[serde(skip)]
    pub output_suffix: String,
}

impl Deferral {
    pub(crate) fn new(
        waited: Duration,
        budget: Duration,
        cleanup: Duration,
        reason: &str,
        retry_after: Option<Duration>,
    ) -> Self {
        let recovery = match retry_after {
            Some(Duration::ZERO) => "rerun `verify.run` now to recheck admission — TTL expiry does not release a live holder".to_string(),
            Some(delay) => format!(
                "rerun `verify.run` in about {}s (timing hint only; a progressing holder renews its TTL)",
                delay.as_secs()
            ),
            None => "rerun `verify.run` to recheck admission when the reported blocker clears".to_string(),
        };
        Self {
            waited_secs: waited.as_secs(),
            budget_secs: budget.as_secs(),
            cleanup_secs: cleanup.as_secs(),
            elapsed_secs: (waited + cleanup).as_secs(),
            next_turn_reserved: None,
            queue_position: None,
            retry_after_secs: retry_after.map(|duration| duration.as_secs()),
            reason: reason.to_string(),
            recovery,
            output_suffix: String::new(),
        }
    }
}

impl std::fmt::Display for Deferral {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "verify: deferred — admission waited {}s (budget {}s; cleanup {}s; total {}s): {}; {} — a deferral is not a failure and there is no attempt cap: keep rerunning `verify.run` while the holder makes progress{}",
            self.waited_secs, self.budget_secs, self.cleanup_secs, self.elapsed_secs,
            self.reason, self.recovery, self.output_suffix,
        )
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum VerificationError {
    #[error("{0}")]
    Deferred(Box<Deferral>),
    #[error("{0}")]
    Failed(String),
}

impl From<String> for VerificationError {
    fn from(message: String) -> Self {
        Self::Failed(message)
    }
}

impl From<SpecOpsError> for VerificationError {
    fn from(error: SpecOpsError) -> Self {
        Self::Failed(match error {
            SpecOpsError::Api(ApiError::Unexpected(cause)) => cause,
            other => other.to_string(),
        })
    }
}
/// In-process lease holder; dropping releases the heavy lease and completes
/// the target job, in the reverse of the acquisition order.
pub(crate) struct Admission {
    guard: Option<TargetJobGuard>,
    artifacts: Option<verification_lease::BuildArtifactGuard>,
    lease: Option<Arc<Mutex<HeavyLease>>>,
    renewal: Option<super::renewal::Renewal>,
    commands: Arc<super::CommandProgress>,
    lease_id: String,
    waited: Duration,
    queue_wait_ms: u64,
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
        drop(self.artifacts.take());
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
            "verify: host admission — lease {} acquired (waited {}s; queue_wait_ms: {})",
            self.lease_id,
            self.waited.as_secs(),
            self.queue_wait_ms,
        )
    }

    pub(crate) fn queue_wait_ms(&self) -> u64 {
        self.queue_wait_ms
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

    /// Recorded on the run as its provenance (Issue #4528).
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

fn check_cancellation(attempt: Option<&HeavyAttempt<'_>>) -> Result<(), SpecOpsError> {
    let Some(attempt) = attempt else {
        return Ok(());
    };
    if (attempt.check_cancelled)().map_err(|error| unexpected(error.to_string()))? {
        return Err(cancelled(attempt.id));
    }
    Ok(())
}

fn cancelled(attempt_id: &str) -> SpecOpsError {
    unexpected(format!("verification attempt canceled (attempt {attempt_id}); rerun verify.run with the identical full matrix"))
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
        let head = status
            .queue
            .first()
            .map(|head| {
                format!(
                    "; queue head {} (resident: {}, {}, attempt: {})",
                    head.target.as_deref().unwrap_or("unknown target"),
                    if head.resident { "yes" } else { "no" },
                    head.job_status
                        .map(|status| format!("job {}", status.as_str()))
                        .unwrap_or_else(|| "job status unpublished".to_string()),
                    head.attempt_id.as_deref().unwrap_or("legacy"),
                )
            })
            .unwrap_or_else(|| "; queue empty".to_string());
        return HolderNotice {
            detail: format!("verification lease has no current holder{head}"),
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
            let mut notice = holder_notice(&status, activity.as_ref());
            if let Ok(pool) = coordinator.heavy_pool_status() {
                notice.detail.push_str(&format!(
                    "; pool capacity={} running={} available={}",
                    pool.capacity, pool.used, pool.available
                ));
                for slot in pool.slots.iter().filter(|slot| slot.status.held) {
                    notice.detail.push_str(&format!(
                        "; slot {}: {}",
                        slot.slot
                            .map_or_else(|| "exclusive".to_string(), |slot| slot.to_string()),
                        holder_identity_notice(&slot.status).detail
                    ));
                }
                notice.retry_after = pool
                    .slots
                    .iter()
                    .filter(|slot| slot.status.held)
                    .filter_map(|slot| {
                        slot.status
                            .estimated_remaining_ms
                            .or(slot.status.remaining_ms)
                    })
                    .min()
                    .map(Duration::from_millis)
                    .or(notice.retry_after);
            }
            notice
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
) -> VerificationError {
    VerificationError::Deferred(Box::new(Deferral::new(
        started.elapsed(),
        max_wait,
        Duration::ZERO,
        detail,
        retry_after,
    )))
}

fn sleep_until(deadline: Instant, poll: Duration) {
    let remaining = deadline.saturating_duration_since(Instant::now());
    std::thread::sleep(remaining.min(poll));
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
#[cfg(test)]
pub(crate) fn admit<E: CliEnv>(
    env: &mut E,
    worktree: &Path,
    command: Option<&str>,
    max_wait: Duration,
    on_host_deferred: impl FnOnce(Option<&verification_lease::BuildArtifactGuard>) -> String,
) -> Result<Admission, VerificationError> {
    admit_inner(env, worktree, command, max_wait, on_host_deferred, None)
}

/// Admit only while this caller-owned attempt remains active. Target and
/// artifact waits poll promptly; host enrollment and refresh are fenced by
/// the coordinator's queue metadata lock.
pub(crate) fn admit_for_attempt<E: CliEnv>(
    env: &mut E,
    worktree: &Path,
    command: Option<&str>,
    max_wait: Duration,
    on_host_deferred: impl FnOnce(Option<&verification_lease::BuildArtifactGuard>) -> String,
    attempt: &HeavyAttempt<'_>,
) -> Result<Admission, VerificationError> {
    admit_inner(
        env,
        worktree,
        command,
        max_wait,
        on_host_deferred,
        Some(attempt),
    )
}

fn admit_inner<E: CliEnv>(
    env: &mut E,
    worktree: &Path,
    command: Option<&str>,
    max_wait: Duration,
    on_host_deferred: impl FnOnce(Option<&verification_lease::BuildArtifactGuard>) -> String,
    attempt: Option<&HeavyAttempt<'_>>,
) -> Result<Admission, VerificationError> {
    check_cancellation(attempt)?;
    let key = verification_lease::verification_key(env)?;
    let coordinator = verification_lease::open_coordinator()?;
    let started = Instant::now();
    let deadline = started + max_wait;
    let mut notice = BoardNotice::default();
    let poll = if attempt.is_some() {
        CANCELLABLE_POLL
    } else {
        POLL
    };
    let target = command
        .map(|command| verification_lease::effective_cargo_target(worktree, command, false))
        .transpose()
        .map_err(unexpected)?
        .flatten();
    let budgets = match command {
        Some(command) => {
            let temporary = verification_lease::command_temporary_base(worktree, command)
                .map_err(unexpected)?;
            let paths = vec![
                target.clone().unwrap_or_else(|| worktree.join("target")),
                temporary,
            ];
            verification_lease::command_disk_budgets(&paths).map_err(unexpected)?
        }
        None => Vec::new(),
    };
    check_cancellation(attempt)?;

    // Every invocation owns its locks; matching the worktree is not proof
    // that another run's lease belongs to this invocation.
    let guard = loop {
        check_cancellation(attempt)?;
        match coordinator
            .request_job(&key, JobPriority::ManualRebuild, NON_BLOCKING.min(poll))
            .map_err(|err| unexpected(format!("verification job admission failed: {err}")))?
        {
            JobAdmission::Owner(guard) => break guard,
            JobAdmission::Joined(waiter) => {
                // A concurrent canonical run in this same worktree owns
                // the target job until its command matrix finishes.
                drop(waiter);
                check_cancellation(attempt)?;
                if Instant::now() >= deadline {
                    return Err(deferred(
                        started,
                        max_wait,
                        "another verification claimant in this worktree owns the target job; \
                         gwtd artifact restoration: skipped (another verification owns the target job)",
                        None,
                    ));
                }
                notice.maybe_post(
                    env,
                    started,
                    max_wait,
                    "同じ worktree の別 claimant が verification target job を保持",
                );
                sleep_until(deadline, poll);
            }
        }
    };
    check_cancellation(attempt)?;
    let artifacts = if let Some(target) = &target {
        loop {
            check_cancellation(attempt)?;
            match verification_lease::try_lock_build_artifacts(target)
                .map_err(|error| unexpected(format!("build artifact admission failed: {error}")))?
            {
                Some(guard) => break Some(guard),
                None => {
                    check_cancellation(attempt)?;
                    if Instant::now() >= deadline {
                        return Err(deferred(
                            started,
                            max_wait,
                            &format!(
                                "another canonical verification or GC owns Cargo target {}",
                                target.display()
                            ),
                            None,
                        ));
                    }
                    notice.maybe_post(
                        env,
                        started,
                        max_wait,
                        &format!(
                            "Cargo target {} は別の検証またはGCが使用中",
                            target.display()
                        ),
                    );
                    sleep_until(deadline, poll);
                }
            }
        }
    } else {
        None
    };
    check_cancellation(attempt)?;
    let reserve = || match attempt {
        Some(attempt) => coordinator.reserve_heavy_for_attempt(
            &key,
            JobPriority::ManualRebuild,
            VERIFICATION_RESERVATION_TTL,
            Some("verify.run deferred"),
            attempt,
        ),
        None => coordinator.reserve_heavy(
            &key,
            JobPriority::ManualRebuild,
            VERIFICATION_RESERVATION_TTL,
            Some("verify.run deferred"),
        ),
    };
    let mut probe = HolderProbe::default();
    let lease = loop {
        check_cancellation(attempt)?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        let acquired = match (target.is_some(), attempt) {
            (true, Some(attempt)) => guard.acquire_heavy_with_disk_budgets_for_attempt(
                remaining.min(POLL),
                LEASE_TTL,
                &budgets,
                attempt,
            ),
            (false, Some(attempt)) => guard.acquire_exclusive_heavy_with_disk_budget_for_attempt(
                remaining.min(POLL),
                LEASE_TTL,
                &budgets,
                attempt,
            ),
            (true, None) => {
                guard.acquire_heavy_with_disk_budgets(remaining.min(POLL), LEASE_TTL, &budgets)
            }
            (false, None) => guard.acquire_exclusive_heavy_with_disk_budget(
                remaining.min(POLL),
                LEASE_TTL,
                &budgets,
            ),
        };
        match acquired {
            Ok(lease) => break lease,
            Err(CoordinatorError::Timeout { .. }) => {
                check_cancellation(attempt)?;
                let waited = started.elapsed();
                let mut holder = describe_holder(&coordinator, &mut probe, worktree);
                for budget in &budgets {
                    holder.detail.push_str(&format!(
                        "; disk {}: reserve={} floor={} free={}",
                        budget.volume,
                        budget.bytes,
                        budget.floor_bytes,
                        fs2::available_space(&budget.path).unwrap_or(0)
                    ));
                }
                if Instant::now() >= deadline {
                    // Issue #4086 AC-1: the rerun must be admitted before any
                    // background job that queues while recovery is running.
                    // The budget ends here; required recovery is separate.
                    if let Err(CoordinatorError::Cancelled { attempt_id }) = reserve() {
                        return Err(cancelled(&attempt_id).into());
                    }
                    check_cancellation(attempt)?;
                    // Issue #4982: recover before releasing this worktree's
                    // target guard, so another admitted run cannot rearm the
                    // operational artifact while recovery is in progress.
                    // Issue #5106: lend the artifact boundary to recovery;
                    // reacquiring it through a new FD would deadlock this run.
                    let recovery = on_host_deferred(artifacts.as_ref());
                    check_cancellation(attempt)?;
                    // A long recovery may outlive the reservation's existing
                    // TTL. Refresh it before reporting the rerun's final state.
                    let reserved = reserve();
                    if let Err(CoordinatorError::Cancelled { attempt_id }) = &reserved {
                        return Err(cancelled(attempt_id).into());
                    }
                    check_cancellation(attempt)?;
                    let _ = guard.complete(JobOutcome::Failed {
                        message: "host admission deferred".to_string(),
                    });
                    let mut detail = holder.detail;
                    if !recovery.is_empty() {
                        detail.push_str(&format!("; {recovery}"));
                    }
                    // Issue #4337 AC-3: name the reservation outcome outright.
                    // `queue_position` below only ever appears on success, so
                    // on its own it leaves the rerun unable to tell a failed
                    // reservation from a failed status read. Issue #4969 AC-2:
                    // a refresh error cannot establish that an earlier valid
                    // reservation is absent, so report that state as unknown.
                    match &reserved {
                        Ok(_) => detail.push_str("; next_turn_reserved: yes"),
                        Err(err) => {
                            detail.push_str(&format!(
                                "; next_turn_reserved: unknown (reservation refresh failed: {err})"
                            ));
                        }
                    }
                    let queue_position = coordinator.heavy_lease_status().ok().and_then(|status| {
                        status
                            .queue
                            .iter()
                            .position(|entry| {
                                entry.target.as_deref() == Some(key.file_stem().as_str())
                            })
                            .map(|position| position + 1)
                    });
                    if let Some(position) = queue_position {
                        detail.push_str(&format!("; queue_position: {position}"));
                    }
                    let mut deferral = Deferral::new(
                        waited,
                        max_wait,
                        started.elapsed().saturating_sub(waited),
                        &detail,
                        holder.retry_after,
                    );
                    deferral.next_turn_reserved = reserved.as_ref().ok().map(|_| true);
                    deferral.queue_position = queue_position;
                    return Err(VerificationError::Deferred(Box::new(deferral)));
                }
                notice.maybe_post(env, started, max_wait, &holder.detail);
            }
            Err(CoordinatorError::Cancelled { attempt_id }) => {
                return Err(cancelled(&attempt_id).into())
            }
            Err(err) => {
                let _ = guard.complete(JobOutcome::Failed {
                    message: err.to_string(),
                });
                return Err(
                    unexpected(format!("verification lease acquisition failed: {err}")).into(),
                );
            }
        }
    };
    check_cancellation(attempt)?;
    let mut lease = lease;
    // Issue #4409 AC-4: a waiter needs to know whether this holder escaped the
    // agent process tree, because a holder that did not will take far longer
    // than its history suggests.
    let worktree = gwt_core::paths::resolve_current_worktree_root(env.repo_path());
    let (spawn_host, _) = crate::cli::daemon::verification_host::describe_for_lease(&worktree);
    lease.record_spawn_host(spawn_host);
    let lease_id = lease.id().to_string();
    let queue_wait_ms = lease.queue_wait_ms();
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
        artifacts,
        lease_id,
        lease: Some(lease),
        renewal: Some(renewal),
        commands,
        waited: started.elapsed(),
        queue_wait_ms,
    };
    check_cancellation(attempt)?;

    Ok(admission)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deferral_separates_wait_budget_from_required_cleanup() {
        let deferral = Deferral::new(
            Duration::from_secs(1500),
            Duration::from_secs(1500),
            Duration::from_secs(64),
            "holder busy; artifact restored",
            Some(Duration::from_secs(10)),
        );
        let data = serde_json::to_value(&deferral).unwrap();
        assert_eq!(data["waited_secs"], 1500);
        assert_eq!(data["budget_secs"], 1500);
        assert_eq!(data["cleanup_secs"], 64);
        assert_eq!(data["elapsed_secs"], 1564);
        assert_eq!(data["next_turn_reserved"], serde_json::Value::Null);
        assert_eq!(data["retry_after_secs"], 10);
        assert!(deferral.to_string().contains("admission waited 1500s"));
        assert!(deferral.to_string().contains("cleanup 64s"));
    }

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
    fn a_free_slot_reports_the_absent_queue_head_instead_of_a_holder() {
        let notice = holder_notice(
            &HeavyLeaseStatus {
                queue: vec![gwt_core::index_coordinator::HeavyQueueEntry {
                    target: Some("project--verification--worktree".into()),
                    priority: JobPriority::ManualRebuild,
                    queued_at_ms: 1,
                    waiting_ms: 200,
                    resident: false,
                    attempt_id: Some("deferred-attempt".into()),
                    job_status: Some(gwt_core::index_coordinator::JobStatus::Failed),
                }],
                ..HeavyLeaseStatus::default()
            },
            None,
        );
        assert!(
            notice.detail.contains("no current holder"),
            "{}",
            notice.detail
        );
        assert!(
            notice
                .detail
                .contains("queue head project--verification--worktree"),
            "{}",
            notice.detail
        );
        assert!(notice.detail.contains("resident: no"), "{}", notice.detail);
        assert!(
            notice.detail.contains("attempt: deferred-attempt"),
            "{}",
            notice.detail
        );
        assert!(notice.detail.contains("job failed"), "{}", notice.detail);
        assert!(!notice.detail.contains("TTL"), "{}", notice.detail);
    }

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
        assert!(free.detail.contains("no current holder"), "{}", free.detail);
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
        // Issue #4280 AC-3 / #4969 AC-2: keep retrying without an attempt
        // cap, but a failed reservation refresh cannot promise a reserved turn.
        for message in [&with_eta, &without_eta] {
            assert!(!message.contains("lease attempt"), "{message}");
            assert!(message.contains("no attempt cap"), "{message}");
            assert!(message.contains("keep rerunning"), "{message}");
            assert!(!message.contains("your turn stays reserved"), "{message}");
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
            super::admit(env, worktree, None, budget, |_| String::new())
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
    fn independent_worktrees_use_two_slots_but_a_shared_cargo_target_waits() {
        let _lock = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let original_home = dirs::home_dir().unwrap();
        let cargo_home = std::env::var_os("CARGO_HOME")
            .unwrap_or_else(|| original_home.join(".cargo").into_os_string());
        let rustup_home = std::env::var_os("RUSTUP_HOME")
            .unwrap_or_else(|| original_home.join(".rustup").into_os_string());
        let _cargo_home = gwt_core::test_support::ScopedEnvVar::set("CARGO_HOME", cargo_home);
        let _rustup_home = gwt_core::test_support::ScopedEnvVar::set("RUSTUP_HOME", rustup_home);
        let _home_env = gwt_core::test_support::ScopedEnvVar::set("HOME", home.path());
        let _profile_env = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", home.path());
        let _home = ScopedGwtHome::set(home.path());
        std::fs::create_dir_all(home.path().join(".gwt")).unwrap();
        std::fs::write(
            gwt_config::Settings::global_config_path_for_home(home.path()),
            "[verification]\nslots=2\ndisk_budget_bytes=0\n[build_artifact_gc]\nbelow_bytes=0\nbelow_percent=0\n",
        )
        .unwrap();
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        for directory in [first.path(), second.path()] {
            std::fs::create_dir_all(directory.join("src")).unwrap();
            std::fs::write(
                directory.join("Cargo.toml"),
                "[package]\nname='admission-fixture'\nversion='0.0.0'\nedition='2021'\n",
            )
            .unwrap();
            std::fs::write(directory.join("src/lib.rs"), "").unwrap();
        }
        let shared = home.path().join("shared-target");
        let command = format!(
            "cargo test --workspace --target-dir \"{}\"",
            shared.display()
        );
        let mut first_env = crate::cli::TestEnv::new(first.path().to_path_buf());
        let mut second_env = crate::cli::TestEnv::new(second.path().to_path_buf());
        let first_admission = super::admit(
            &mut first_env,
            first.path(),
            Some(&command),
            Duration::ZERO,
            |_| String::new(),
        )
        .unwrap();
        let error = super::admit(
            &mut second_env,
            second.path(),
            Some(&command),
            Duration::ZERO,
            |_| String::new(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("Cargo target"), "{error}");
        let independent_command = format!(
            "cargo test --workspace --target-dir \"{}\"",
            second.path().join("target").display()
        );
        let second_admission = super::admit(
            &mut second_env,
            second.path(),
            Some(&independent_command),
            Duration::ZERO,
            |_| String::new(),
        )
        .unwrap();
        let pool = verification_lease::open_coordinator()
            .unwrap()
            .heavy_pool_status()
            .unwrap();
        assert_eq!(pool.capacity, 2);
        assert_eq!(pool.used, 2);
        assert!(verification_lease::try_lock_build_artifacts(&shared)
            .unwrap()
            .is_none());
        drop(first_admission);
        assert!(verification_lease::try_lock_build_artifacts(&shared)
            .unwrap()
            .is_some());
        drop(second_admission);
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
        let second = super::admit(&mut env, worktree.path(), None, Duration::ZERO, |_| {
            panic!("a contender must not restore another verifier's artifact")
        });
        assert!(
            second.is_err(),
            "a second canonical run must not borrow the first run's lease: {second:?}; {}",
            lease_root.describe()
        );
        let refusal = second.unwrap_err().to_string();
        assert!(refusal.contains("deferred"), "{refusal}");
        assert!(
            refusal.contains(
                "gwtd artifact restoration: skipped (another verification owns the target job)"
            ),
            "{refusal}"
        );
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
    fn cancellation_during_deferred_recovery_does_not_rearm_the_reservation() {
        let lease_root = IsolatedLeaseRoot::new();
        let worktree = tempfile::tempdir().unwrap();
        let mut env = crate::cli::TestEnv::new(worktree.path().to_path_buf());
        let key = verification_lease::verification_key(&mut env).unwrap();
        let other = TargetKey::verification("different-project", "worktree");
        let JobAdmission::Owner(holder) = lease_root
            .coordinator
            .request_job(&other, JobPriority::ManualRebuild, Duration::ZERO)
            .unwrap()
        else {
            panic!("private holder target must be free")
        };
        let lease = holder
            .acquire_heavy_with_ttl(Duration::ZERO, LEASE_TTL)
            .unwrap();
        let cancelled = std::cell::Cell::new(false);
        let check = || Ok(cancelled.get());
        let attempt = gwt_core::index_coordinator::HeavyAttempt {
            id: "cancelled-recovery",
            check_cancelled: &check,
        };
        let result = super::admit_for_attempt(
            &mut env,
            worktree.path(),
            None,
            Duration::ZERO,
            |_| {
                cancelled.set(true);
                assert!(lease_root
                    .coordinator
                    .clear_heavy_reservation_for_attempt(&key, attempt.id)
                    .unwrap());
                String::new()
            },
            &attempt,
        );
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("verification attempt canceled"));
        assert!(!lease_root.coordinator.heavy_reservation_path(&key).exists());
        let status = lease_root.status();
        assert!(
            status.held,
            "canceling the waiting attempt must preserve its unrelated holder"
        );
        assert_eq!(status.target.as_deref(), Some(other.file_stem().as_str()));
        assert!(status.queue.is_empty());
        drop(lease);
        holder.complete(JobOutcome::Completed).unwrap();
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
        let key = verification_lease::verification_key(&mut env).unwrap();
        let recovery_calls = std::cell::Cell::new(0);
        let recover = |_: Option<&verification_lease::BuildArtifactGuard>| {
            recovery_calls.set(recovery_calls.get() + 1);
            let reservation = lease_root.coordinator.heavy_reservation_path(&key);
            assert!(reservation.exists());
            assert!(
                matches!(
                    lease_root
                        .coordinator
                        .request_job(&key, JobPriority::ManualRebuild, Duration::ZERO,)
                        .unwrap(),
                    JobAdmission::Joined(_)
                ),
                "recovery must retain the target lock"
            );
            // Model a long build without waiting for the reservation's TTL.
            let mut entry: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&reservation).unwrap()).unwrap();
            entry["reserved_until_ms"] = 0.into();
            std::fs::write(&reservation, serde_json::to_vec(&entry).unwrap()).unwrap();
            "gwtd artifact restoration: restored".to_string()
        };
        let err =
            super::admit(&mut env, worktree.path(), None, Duration::ZERO, recover).unwrap_err();

        let message = err.to_string();
        assert_eq!(recovery_calls.get(), 1);
        let reservation: serde_json::Value = serde_json::from_slice(
            &std::fs::read(lease_root.coordinator.heavy_reservation_path(&key)).unwrap(),
        )
        .unwrap();
        assert!(reservation["reserved_until_ms"].as_u64().unwrap() > 0);
        assert!(
            message.contains("gwtd artifact restoration: restored"),
            "{message}"
        );
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
        assert!(
            lease_root.coordinator.heavy_reservation_path(&key).exists(),
            "a deferred admission must reserve the next turn — {}",
            lease_root.describe()
        );
        assert_eq!(
            lease_root.coordinator.heavy_lease_status().unwrap().pending,
            1
        );
        // Issue #4746 AC-5: exercise a new admission, not a retry on the
        // same guard. A deferred caller keeps its place ahead of new arrivals.
        let before = lease_root.coordinator.heavy_lease_status().unwrap().queue;
        let later = TargetKey::verification("later-repo", "later-worktree");
        lease_root
            .coordinator
            .reserve_heavy(
                &later,
                JobPriority::ManualRebuild,
                Duration::from_secs(60),
                Some("later arrival"),
            )
            .unwrap();
        let again = super::admit(&mut env, worktree.path(), None, Duration::ZERO, recover)
            .unwrap_err()
            .to_string();
        assert_eq!(recovery_calls.get(), 2);
        assert!(again.contains("next_turn_reserved: yes"), "{again}");
        let after = lease_root.coordinator.heavy_lease_status().unwrap().queue;
        assert_eq!(after.len(), 2);
        assert_eq!(after[0].target.as_deref(), Some(key.file_stem().as_str()));
        assert_eq!(after[0].queued_at_ms, before[0].queued_at_ms);
        assert_eq!(after[1].target.as_deref(), Some(later.file_stem().as_str()));
        drop(_lease);
        let admitted = lease_root
            .admit(&mut env, worktree.path(), Duration::from_secs(1))
            .unwrap();
        let acquired_at = lease_root.status().acquired_at_ms.unwrap();
        assert_eq!(
            admitted.queue_wait_ms(),
            acquired_at.saturating_sub(before[0].queued_at_ms),
            "queue telemetry includes the original reservation, not only the final invocation"
        );
    }
}
