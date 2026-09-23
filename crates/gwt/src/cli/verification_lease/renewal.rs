//! Issue #4666: renew only while the holder's workload makes progress.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use gwt_core::error_ledger::{self, ErrorKind, ErrorRecord, ErrorTarget};
use gwt_core::index_coordinator::{
    CoordinatorError, HeavyLease, HeavyLeaseStatus, IndexCoordinator,
};

use super::holder_activity::{HolderActivity, HolderProbe, HolderWorkload};

const RENEW_TTL: Duration = Duration::from_secs(super::DEFAULT_TTL_MINUTES * 60);
const OBSERVE_EVERY: Duration = Duration::from_secs(30);
const PENDING_THRESHOLD: usize = 3;
const PRESSURE_AFTER: Duration = Duration::from_secs(5 * 60);

/// The runner publishes its exact command root, including daemon-hosted
/// commands. Other light verifications on that daemon cannot renew this run.
#[derive(Default)]
pub(crate) struct CommandProgress {
    pid: AtomicU32,
}

impl CommandProgress {
    pub(crate) fn start(&self, pid: u32) -> RunningCommand<'_> {
        self.pid.store(pid, Ordering::Release);
        RunningCommand(self)
    }

    fn observe(
        &self,
        sample: impl FnOnce(u32) -> Option<HolderActivity>,
    ) -> Option<HolderActivity> {
        let pid = self.pid.load(Ordering::Acquire);
        if pid == 0 {
            return None;
        }
        let activity = sample(pid);
        // A command can finish or be replaced while process sampling runs.
        (self.pid.load(Ordering::Acquire) == pid)
            .then_some(activity)
            .flatten()
    }
}

pub(crate) struct RunningCommand<'a>(&'a CommandProgress);

impl Drop for RunningCommand<'_> {
    fn drop(&mut self) {
        self.0.pid.store(0, Ordering::Release);
    }
}

/// The admission owns both the monitor and the lease. Joining the monitor
/// before releasing the lease prevents a late renewal from rewriting a
/// successor's ticket. The channel wakes shutdown without a 30-second wait.
pub(super) struct Renewal {
    stop: mpsc::Sender<()>,
    worker: Option<JoinHandle<()>>,
}

impl Renewal {
    pub(super) fn start(
        lease: Arc<Mutex<HeavyLease>>,
        coordinator: IndexCoordinator,
        worktree: PathBuf,
        commands: Arc<CommandProgress>,
    ) -> std::io::Result<Self> {
        let (stop, receiver) = mpsc::channel();
        let worker = std::thread::Builder::new()
            .name("verification-renewal".to_string())
            .spawn(move || {
                let mut probe = HolderProbe::default();
                let mut pressure = QueuePressure::load(coordinator.root());
                // Observe immediately so a holder handoff does not add another
                // full polling interval to the gap between queue readings.
                observe_pressure(&coordinator, &worktree, &mut pressure);
                while matches!(
                    receiver.recv_timeout(OBSERVE_EVERY),
                    Err(mpsc::RecvTimeoutError::Timeout)
                ) {
                    let status = coordinator.heavy_lease_status().ok();
                    pressure.publish(coordinator.root(), &worktree, status.as_ref(), epoch_ms());
                    let activity = commands.observe(|pid| {
                        probe.observe(
                            pid,
                            &HolderWorkload::Owned,
                            status.as_ref().and_then(|s| s.acquired_at_ms),
                        )
                    });
                    let Ok(mut lease) = lease.lock() else { break };
                    let now = chrono::Utc::now().timestamp_millis().max(0) as u64;
                    if let Err(error) = renew_observed(&mut lease, activity, now) {
                        tracing::warn!(%error, "verification lease renewal failed");
                    }
                }
            })?;
        Ok(Self {
            stop,
            worker: Some(worker),
        })
    }
}

impl Drop for Renewal {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn renew_observed(
    lease: &mut HeavyLease,
    activity: Option<HolderActivity>,
    now_ms: u64,
) -> Result<(), CoordinatorError> {
    if activity.is_some_and(|activity| activity.advancing()) {
        lease.extend_until(now_ms.saturating_add(RENEW_TTL.as_millis() as u64))?;
    }
    Ok(())
}

fn epoch_ms() -> u64 {
    chrono::Utc::now().timestamp_millis().max(0) as u64
}

fn observe_pressure(coordinator: &IndexCoordinator, worktree: &Path, pressure: &mut QueuePressure) {
    let status = coordinator.heavy_lease_status().ok();
    pressure.publish(coordinator.root(), worktree, status.as_ref(), epoch_ms());
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct QueuePressure {
    since: Option<u64>,
    last_observed: Option<u64>,
    reported: bool,
}

impl QueuePressure {
    fn load(root: &Path) -> Self {
        std::fs::read(root.join("verification-pressure.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn save(&self, root: &Path) -> std::io::Result<()> {
        let bytes = serde_json::to_vec(self).map_err(std::io::Error::other)?;
        gwt_core::atomic_file::write_atomic(&root.join("verification-pressure.json"), &bytes)
    }

    fn publish(
        &mut self,
        root: &Path,
        worktree: &Path,
        status: Option<&HeavyLeaseStatus>,
        now: u64,
    ) {
        if self.observe(status.map(|status| status.pending), now) {
            if let Some(status) = status {
                record_pressure(
                    worktree,
                    status.lease_id.as_deref().unwrap_or("unknown"),
                    status.pending,
                    Duration::from_millis(now.saturating_sub(self.since.unwrap_or(now))),
                );
            }
        }
        // Only the heavy lease holder writes this host-wide state. Its monitor
        // is joined before release, so a successor cannot race this publication.
        if let Err(error) = self.save(root) {
            tracing::warn!(%error, "verification pressure state publication failed");
        }
    }

    fn observe(&mut self, pending: Option<usize>, now: u64) -> bool {
        if self
            .last_observed
            .is_some_and(|last| now < last || now - last > (OBSERVE_EVERY.as_millis() * 2) as u64)
        {
            *self = Self::default();
        }
        if !pending.is_some_and(|pending| pending >= PENDING_THRESHOLD) {
            *self = Self::default();
            return false;
        }
        self.last_observed = Some(now);
        let since = self.since.get_or_insert(now);
        if !self.reported && now.saturating_sub(*since) >= PRESSURE_AFTER.as_millis() as u64 {
            self.reported = true;
            return true;
        }
        false
    }
}

fn record_pressure(worktree: &Path, lease_id: &str, pending: usize, sustained: Duration) {
    let record = ErrorRecord::new(
        ErrorKind::VerificationCongestion,
        "検証待機が3件以上の状態を5分以上継続観測しました。自動介入は行いません。",
        ErrorTarget {
            project_root: Some(worktree.display().to_string()),
            ..Default::default()
        },
    )
    .with_context(
        [
            ("lease_id".to_string(), lease_id.to_string()),
            ("pending".to_string(), pending.to_string()),
            (
                "sustained_ms".to_string(),
                sustained.as_millis().to_string(),
            ),
            (
                "sample_interval_ms".to_string(),
                OBSERVE_EVERY.as_millis().to_string(),
            ),
        ]
        .into(),
    );
    if let Err(error) = error_ledger::record(record) {
        tracing::warn!(%error, "verification congestion record failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_core::index_coordinator::{IndexCoordinator, JobAdmission, JobPriority, TargetKey};

    fn activity() -> HolderActivity {
        HolderActivity {
            held_ms: 2 * RENEW_TTL.as_millis() as u64,
            cpu_percent: 1.0,
            cpu_gained_ms: 300,
            turnover: false,
            processes: 1,
            window_ms: 30_000,
            host_cpu_percent: Some(20.0),
            delegated: true,
            parent_gone: false,
            workload_processes: 1,
        }
    }

    #[test]
    fn command_scope_clears_the_pid_and_discards_a_replaced_sample() {
        let commands = CommandProgress::default();
        assert!(commands.observe(|_| panic!("no active command")).is_none());
        let mut running = Some(commands.start(30));
        assert!(commands
            .observe(|pid| {
                assert_eq!(pid, 30);
                Some(activity())
            })
            .is_some());
        let mut successor = None;
        let stale = commands.observe(|_| {
            drop(running.take());
            successor = Some(commands.start(40));
            Some(activity())
        });
        assert!(
            stale.is_none(),
            "the previous command cannot renew its successor"
        );
        drop(successor);
        assert!(commands
            .observe(|_| panic!("command has finished"))
            .is_none());
    }

    #[test]
    fn progress_renews_a_job_beyond_its_initial_ttl_then_silence_expires() {
        let root = tempfile::tempdir().unwrap();
        let coordinator = IndexCoordinator::open(root.path()).unwrap();
        let key = TargetKey::verification("repo", "long-running");
        let JobAdmission::Owner(guard) = coordinator
            .request_job(&key, JobPriority::ManualRebuild, Duration::from_secs(1))
            .unwrap()
        else {
            panic!("private coordinator must admit fixture");
        };
        let mut lease = guard
            .acquire_heavy_with_ttl(Duration::from_secs(1), RENEW_TTL)
            .unwrap();
        // An explicit past deadline models a job that has outlived its TTL;
        // no wall-clock sleep controls this regression.
        let now = chrono::Utc::now().timestamp_millis() as u64;
        lease.extend_until(now - 1).unwrap();
        assert!(coordinator.heavy_lease_status().unwrap().expired);
        renew_observed(&mut lease, Some(activity()), now).unwrap();
        let status = coordinator.heavy_lease_status().unwrap();
        assert!(!status.expired, "a progressing long job must renew");
        assert_eq!(
            status.expires_at_ms,
            Some(now + RENEW_TTL.as_millis() as u64)
        );

        let silent = HolderActivity {
            cpu_percent: 0.0,
            cpu_gained_ms: 0,
            ..activity()
        };
        lease.extend_until(now - 1).unwrap();
        renew_observed(&mut lease, Some(silent), now).unwrap();
        renew_observed(&mut lease, None, now).unwrap();
        assert!(coordinator.heavy_lease_status().unwrap().expired);
        assert_eq!(lease.expires_at_ms(), Some(now - 1));
        // Short-lived children turning over also count, even at zero CPU.
        renew_observed(
            &mut lease,
            Some(HolderActivity {
                turnover: true,
                ..silent
            }),
            now,
        )
        .unwrap();
        assert!(!coordinator.heavy_lease_status().unwrap().expired);
    }

    #[test]
    fn another_delegated_verification_does_not_renew_a_stopped_holder() {
        use super::super::holder_activity::{activity_from_samples, HolderWorkload, ProcessSample};

        let root = tempfile::tempdir().unwrap();
        let coordinator = IndexCoordinator::open(root.path()).unwrap();
        let key = TargetKey::verification("repo", "stopped-holder");
        let JobAdmission::Owner(guard) = coordinator
            .request_job(&key, JobPriority::ManualRebuild, Duration::from_secs(1))
            .unwrap()
        else {
            panic!("private coordinator must admit fixture");
        };
        let mut lease = guard
            .acquire_heavy_with_ttl(Duration::from_secs(1), RENEW_TTL)
            .unwrap();
        let now = epoch_ms();
        lease.extend_until(now - 1).unwrap();
        // Holder 10 and its delegated command 30 are stopped. Command 40
        // belongs to another light verification on the same daemon 20.
        let first = [
            ProcessSample {
                pid: 10,
                parent: Some(1),
                cpu_ms: 0,
                session_leader: false,
            },
            ProcessSample {
                pid: 20,
                parent: Some(1),
                cpu_ms: 0,
                session_leader: false,
            },
            ProcessSample {
                pid: 30,
                parent: Some(20),
                cpu_ms: 0,
                session_leader: true,
            },
            ProcessSample {
                pid: 40,
                parent: Some(20),
                cpu_ms: 0,
                session_leader: true,
            },
        ];
        let mut second = first;
        second[3].cpu_ms = 300;
        let commands = CommandProgress::default();
        let _command = commands.start(30);
        let activity = commands.observe(|pid| {
            activity_from_samples(
                pid,
                &HolderWorkload::Owned,
                RENEW_TTL.as_millis() as u64,
                &first,
                &second,
                30_000,
                None,
            )
        });
        renew_observed(&mut lease, activity, now).unwrap();
        assert_eq!(
            lease.expires_at_ms(),
            Some(now - 1),
            "another verification's CPU must not renew this stopped holder"
        );
    }

    #[test]
    fn unobserved_time_does_not_establish_continuous_pressure() {
        let mut pressure = QueuePressure::default();
        assert!(!pressure.observe(Some(3), 0));
        assert!(!pressure.observe(Some(3), 300_000));
        assert_eq!(pressure.since, Some(300_000));
    }

    #[test]
    fn queue_pressure_survives_holder_handoffs_and_reports_once() {
        let root = tempfile::tempdir().unwrap();
        let mut pressure = QueuePressure::load(root.path());
        for now in (0..=120_000).step_by(30_000) {
            assert!(!pressure.observe(Some(3), now));
        }
        pressure.save(root.path()).unwrap();
        let mut successor = QueuePressure::load(root.path());
        for now in (150_000..300_000).step_by(30_000) {
            assert!(!successor.observe(Some(5), now));
        }
        assert!(successor.observe(Some(3), 300_000));
        successor.save(root.path()).unwrap();
        let mut third = QueuePressure::load(root.path());
        assert!(!third.observe(Some(4), 330_000));
        assert_eq!(third.since, Some(0));
        assert_eq!(third.last_observed, Some(330_000));
        assert!(third.reported);
    }

    #[test]
    fn queue_pressure_resets_on_low_pending_unknown_or_clock_reversal() {
        let mut pressure = QueuePressure::default();
        assert!(!pressure.observe(Some(3), 100_000));
        assert!(!pressure.observe(Some(2), 130_000));
        assert_eq!(pressure.since, None);
        assert!(!pressure.observe(Some(3), 160_000));
        assert!(!pressure.observe(None, 190_000));
        assert_eq!(pressure.since, None);
        assert!(!pressure.observe(Some(3), 220_000));
        assert!(!pressure.observe(Some(3), 210_000));
        assert_eq!(pressure.since, Some(210_000));
        for now in (240_000..510_000).step_by(30_000) {
            assert!(!pressure.observe(Some(3), now));
        }
        assert!(pressure.observe(Some(3), 510_000));
    }

    #[test]
    fn congestion_is_readable_through_errors_list() {
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        record_pressure(
            home.path(),
            "long-running-lease",
            5,
            Duration::from_secs(310),
        );
        let mut env = crate::cli::TestEnv::new(home.path().to_path_buf());
        let mut output = String::new();
        crate::cli::diagnostics::errors::run(
            &mut env,
            crate::cli::diagnostics::errors::ErrorsCommand::List { since: None },
            &mut output,
        )
        .unwrap();
        let payload: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(payload["count"], 1);
        let record = &payload["errors"][0];
        assert_eq!(record["kind"], "verification_congestion");
        assert_eq!(record["context"]["lease_id"], "long-running-lease");
        assert_eq!(record["context"]["pending"], "5");
        assert_eq!(record["context"]["sustained_ms"], "310000");
    }
}
