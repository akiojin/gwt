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
use std::time::{Duration, Instant};

use gwt_core::index_coordinator::{
    CoordinatorError, HeavyHolderKind, HeavyLease, HeavyLeaseStatus, IndexCoordinator,
    JobAdmission, JobOutcome, JobPriority, TargetJobGuard, VERIFICATION_RESERVATION_TTL,
};
use gwt_github::{client::ApiError, SpecOpsError};

use crate::cli::board::{BoardCommand, BoardPostCommand};
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
    lease: Option<HeavyLease>,
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
    fn settle(&mut self, outcome: JobOutcome) {
        if let Some(lease) = self.lease.take() {
            let _ = lease.release();
        }
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
    /// How long until the holder hands the lease back. `None` when the holder
    /// publishes neither a TTL nor batch progress, so no honest estimate
    /// exists.
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
fn holder_notice(status: &HeavyLeaseStatus) -> HolderNotice {
    if !status.held {
        return HolderNotice {
            detail: "verification lease was contended".to_string(),
            retry_after: None,
        };
    }
    let kind = status
        .holder_kind
        .unwrap_or(HeavyHolderKind::Other)
        .as_str();
    let target = status.target.as_deref().unwrap_or("unknown target");
    let pid = status
        .owner
        .as_ref()
        .map(|owner| owner.pid.to_string())
        .unwrap_or_else(|| "?".to_string());
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
    match status.remaining_ms {
        Some(remaining_ms) => HolderNotice {
            detail: format!(
                "verification lease held by {kind} {target} (pid {pid}, {}s left{progress})",
                remaining_ms / 1000
            ),
            retry_after,
        },
        None => HolderNotice {
            detail: format!(
                "verification lease held by {kind} {target} (pid {pid}, no TTL — it releases \
                 only when its job finishes{progress})"
            ),
            retry_after,
        },
    }
}

fn describe_holder(coordinator: &IndexCoordinator) -> HolderNotice {
    match coordinator.heavy_lease_status() {
        Ok(status) => holder_notice(&status),
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
        Some(retry_after) => format!(
            "rerun `verify.run` in about {}s, when the current holder's lease lapses",
            retry_after.as_secs()
        ),
        None => "rerun `verify.run` after the current lease holder finishes".to_string(),
    };
    unexpected(format!(
        "verify: deferred — host busy for {}s (budget {}s): {detail}; {next} — the wait counts \
         as one gwt-verify lease attempt",
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
    _worktree: &Path,
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
    let lease = loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match guard.acquire_heavy_with_ttl(remaining.min(POLL), LEASE_TTL) {
            Ok(lease) => break lease,
            Err(CoordinatorError::Timeout { .. }) => {
                let holder = describe_holder(&coordinator);
                if Instant::now() >= deadline {
                    let _ = guard.complete(JobOutcome::Failed {
                        message: "host admission deferred".to_string(),
                    });
                    // Issue #4086 AC-1: the rerun must be admitted before any
                    // background index job that queues in the meantime.
                    let _ = coordinator.reserve_heavy(
                        &key,
                        JobPriority::ManualRebuild,
                        VERIFICATION_RESERVATION_TTL,
                        Some("verify.run deferred"),
                    );
                    let mut detail = holder.detail;
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
    let admission = Admission {
        guard: Some(guard),
        lease_id: lease.id().to_string(),
        lease: Some(lease),
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

    /// Issue #4140 AC-3: a holder with a TTL must publish a usable ETA, and a
    /// holder without one must say so instead of reading as "0s left" — the
    /// index job's untimed lease is exactly the case that misled agents into
    /// waiting indefinitely.
    #[test]
    fn holder_notice_reports_an_eta_only_when_the_holder_has_a_ttl() {
        let timed = holder_notice(&HeavyLeaseStatus {
            held: true,
            target: Some("repo--issues".to_string()),
            owner: Some(gwt_core::index_coordinator::OwnerIdentity {
                pid: 32420,
                start_id: "start".to_string(),
            }),
            remaining_ms: Some(320_000),
            ..HeavyLeaseStatus::default()
        });
        assert_eq!(timed.retry_after, Some(Duration::from_secs(320)));
        assert!(timed.detail.contains("repo--issues"), "{}", timed.detail);
        assert!(timed.detail.contains("320s left"), "{}", timed.detail);

        let untimed = holder_notice(&HeavyLeaseStatus {
            held: true,
            target: Some("repo--issues".to_string()),
            remaining_ms: None,
            ..HeavyLeaseStatus::default()
        });
        assert_eq!(untimed.retry_after, None);
        assert!(
            !untimed.detail.contains("0s left"),
            "an untimed lease must not claim it is about to lapse: {}",
            untimed.detail
        );
        assert!(untimed.detail.contains("no TTL"), "{}", untimed.detail);

        let free = holder_notice(&HeavyLeaseStatus::default());
        assert_eq!(free.retry_after, None);
        assert!(free.detail.contains("contended"), "{}", free.detail);
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
            let coordinator = IndexCoordinator::open_default().unwrap();
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
            format!(
                "verification lease {holder}; this process is pid {}; lock {}; ticket {}",
                std::process::id(),
                self.coordinator.heavy_lock_path().display(),
                self.coordinator.heavy_ticket_path().display()
            )
        }

        fn status(&self) -> HeavyLeaseStatus {
            self.coordinator
                .heavy_lease_status()
                .unwrap_or_else(|err| panic!("the lease status must stay readable: {err}"))
        }

        /// Wait for the release to become observable.
        ///
        /// `heavy_lease_status` probes the kernel lock rather than the ticket,
        /// and every `Command::spawn` on any thread of this test binary
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
                if !self.status().held {
                    return;
                }
                assert!(
                    Instant::now() < deadline,
                    "{context} — still held {}s after the release; {}",
                    RELEASE_OBSERVATION_BUDGET.as_secs(),
                    self.describe()
                );
                std::thread::sleep(Duration::from_millis(10));
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

        let admission =
            admit(&mut env, worktree.path(), Duration::from_secs(5)).unwrap_or_else(|err| {
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

    #[test]
    fn canonical_runs_in_the_same_worktree_do_not_share_admission() {
        let lease_root = IsolatedLeaseRoot::new();
        let worktree = tempfile::tempdir().unwrap();
        let mut env = crate::cli::TestEnv::new(worktree.path().to_path_buf());
        let first = admit(&mut env, worktree.path(), Duration::ZERO).unwrap();
        let second = admit(&mut env, worktree.path(), Duration::ZERO);
        assert!(
            second.is_err(),
            "a second canonical run must not borrow the first run's lease: {second:?}"
        );
        assert!(second.unwrap_err().to_string().contains("deferred"));
        drop(first);
        lease_root.assert_free("the first run releases its own lease");
        let next = admit(&mut env, worktree.path(), Duration::ZERO).unwrap();
        drop(next);
        lease_root.assert_free("the next run releases its own lease");
    }

    #[test]
    fn admit_defers_when_another_target_holds_the_lease() {
        let lease_root = IsolatedLeaseRoot::new();
        let worktree = tempfile::tempdir().unwrap();
        let other = TargetKey::repo_shared("other-repo", "issues");
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
        let err = admit(&mut env, worktree.path(), Duration::from_secs(1)).unwrap_err();

        let message = err.to_string();
        assert!(message.contains("deferred"), "{message}");
        assert!(message.contains("rerun `verify.run`"), "{message}");
        assert!(message.contains("queue_position: 1"), "{message}");
        assert!(
            message.contains(&other.file_stem()),
            "the refusal must name the holder: {message}"
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
