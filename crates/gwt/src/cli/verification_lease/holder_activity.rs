//! Issue #4405 AC-3: tell a starved lease holder from a progressing one.
//!
//! The host-wide lease is held by one process tree. When that tree is not
//! making progress, every waiter stalls behind it, yet from the outside the
//! holder only looks "busy". Averaging each live process's CPU time over its
//! own lifetime separates the two without a second sample: a stuck tree runs
//! at a percent or two of one core, a working one at tens to hundreds.
//!
//! Low holder CPU alone cannot say *why* the tree is stuck. The host's CPU
//! load does: on a saturated host the tree is runnable but not scheduled
//! (starved); on a host with idle CPU it is blocked waiting on I/O, a lock
//! or a prompt, and may be hung. The #4426 lease holder, wedged on
//! `git-credential-manager` prompts, is the second case.

use std::time::Duration;

/// A holder younger than this is never called stuck: a matrix spends its
/// first minutes on short-lived processes whose CPU time is already gone.
pub(crate) const STARVED_AFTER: Duration = Duration::from_secs(5 * 60);
/// Below this share of one core, a held tree is not making progress.
pub(crate) const STARVED_BELOW_CPU_PERCENT: f64 = 10.0;
/// At or above this host-wide CPU usage, a stuck holder is starved rather
/// than blocked.
pub(crate) const HOST_SATURATED_CPU_PERCENT: f64 = 80.0;

/// One live process, as the host reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProcessSample {
    pub pid: u32,
    pub parent: Option<u32>,
    pub cpu_ms: u64,
    pub run_ms: u64,
}

/// How the lease holder's process tree is doing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct HolderActivity {
    pub held_ms: u64,
    /// Sum over the live tree of each process's CPU time over its lifetime,
    /// as a percentage of one core.
    pub cpu_percent: f64,
    pub processes: usize,
    /// Host-wide CPU usage in percent, `None` when it could not be sampled.
    pub host_cpu_percent: Option<f64>,
}

impl HolderActivity {
    fn stuck(&self) -> bool {
        u128::from(self.held_ms) >= STARVED_AFTER.as_millis()
            && self.cpu_percent < STARVED_BELOW_CPU_PERCENT
    }

    /// Stuck on a saturated host: runnable but not scheduled.
    pub(crate) fn starved(&self) -> bool {
        self.stuck()
            && self
                .host_cpu_percent
                .is_some_and(|host| host >= HOST_SATURATED_CPU_PERCENT)
    }

    pub(crate) fn state(&self) -> &'static str {
        if !self.stuck() {
            "progressing"
        } else if self.starved() {
            "starved"
        } else {
            "stalled"
        }
    }

    pub(crate) fn describe(&self) -> String {
        let held = format!("{}m{:02}s", self.held_ms / 60_000, self.held_ms / 1000 % 60);
        let cpu = self.cpu_percent;
        let processes = self.processes;
        if !self.stuck() {
            return format!(
                "holder progressing: held {held}, its {processes} processes use {cpu:.1}% of \
                 one core"
            );
        }
        match self.host_cpu_percent {
            Some(host) if self.starved() => format!(
                "holder starved: held {held}, its {processes} processes get {cpu:.1}% of one core \
                 while the host CPU is {host:.0}% busy — it is runnable but not being scheduled, \
                 not hung"
            ),
            Some(host) => format!(
                "holder stalled: held {held}, its {processes} processes get {cpu:.1}% of one core \
                 although the host CPU is only {host:.0}% busy — it is blocked waiting (I/O, a \
                 lock or a prompt), not starved, and may be hung"
            ),
            None => format!(
                "holder stalled: held {held}, its {processes} processes get {cpu:.1}% of one core \
                 — starved of CPU or blocked waiting; the host load could not be read"
            ),
        }
    }
}

/// The activity of `owner_pid` and its live descendants, or `None` when the
/// owner is not among `samples`.
pub(crate) fn activity_from_samples(
    owner_pid: u32,
    held_ms: u64,
    samples: &[ProcessSample],
    host_cpu_percent: Option<f64>,
) -> Option<HolderActivity> {
    if !samples.iter().any(|sample| sample.pid == owner_pid) {
        return None;
    }
    let mut tree = vec![owner_pid];
    let mut next = 0;
    while let Some(&parent) = tree.get(next) {
        for sample in samples {
            if sample.parent == Some(parent) && !tree.contains(&sample.pid) {
                tree.push(sample.pid);
            }
        }
        next += 1;
    }
    let cpu_percent = samples
        .iter()
        .filter(|sample| tree.contains(&sample.pid))
        // A process younger than a second has no meaningful lifetime share.
        .map(|sample| sample.cpu_ms as f64 / sample.run_ms.max(1000) as f64 * 100.0)
        .sum();
    Some(HolderActivity {
        held_ms,
        cpu_percent,
        processes: tree.len(),
        host_cpu_percent,
    })
}

/// Read the host's process table and CPU load and describe the tree under
/// `owner_pid`. `acquired_at_ms` is the lease ticket's Unix-epoch
/// acquisition time. Host CPU needs two samples, so this takes
/// `sysinfo::MINIMUM_CPU_UPDATE_INTERVAL` (about 200 ms).
pub(crate) fn observe(owner_pid: u32, acquired_at_ms: Option<u64>) -> Option<HolderActivity> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

    let mut system = System::new();
    system.refresh_cpu_usage();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_cpu(),
    );
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    system.refresh_cpu_usage();
    let host = f64::from(system.global_cpu_usage());
    let host_cpu_percent = (host.is_finite() && host > 0.0).then_some(host);
    let samples: Vec<ProcessSample> = system
        .processes()
        .values()
        .map(|process| ProcessSample {
            pid: process.pid().as_u32(),
            parent: process.parent().map(sysinfo::Pid::as_u32),
            cpu_ms: process.accumulated_cpu_time(),
            run_ms: process.run_time().saturating_mul(1000),
        })
        .collect();
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis();
    let held_ms = acquired_at_ms
        .map(|at| u64::try_from(now_ms.saturating_sub(u128::from(at))).unwrap_or(u64::MAX))
        .unwrap_or(0);
    activity_from_samples(owner_pid, held_ms, &samples, host_cpu_percent)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(pid: u32, parent: Option<u32>, cpu_ms: u64, run_ms: u64) -> ProcessSample {
        ProcessSample {
            pid,
            parent,
            cpu_ms,
            run_ms,
        }
    }

    /// The #4405 incident: gwtd held the lease for 2h01m while its test
    /// binary got 1.2% of a core.
    fn incident_samples() -> [ProcessSample; 4] {
        [
            sample(21468, Some(1), 11_120, 7_260_000),
            sample(86652, Some(21468), 2_430, 6_960_000),
            sample(16492, Some(86652), 3_600, 300_000),
            // An unrelated busy process must not lend the holder its CPU.
            sample(999, Some(1), 3_600_000, 3_600_000),
        ]
    }

    /// On a saturated host, a stuck holder is runnable but not scheduled.
    #[test]
    fn a_long_held_tree_on_a_saturated_host_is_starved() {
        let activity = activity_from_samples(21468, 7_260_000, &incident_samples(), Some(95.0))
            .expect("the owner is sampled");

        assert_eq!(activity.processes, 3);
        assert!(activity.cpu_percent < 2.0, "{activity:?}");
        assert!(activity.starved(), "{activity:?}");
        assert_eq!(activity.state(), "starved");
        let described = activity.describe();
        assert!(described.contains("starved"), "{described}");
        assert!(described.contains("not hung"), "{described}");
    }

    /// The #4426 lease holder: wedged on credential prompts while the host
    /// had idle CPU. Calling that "starved, not hung" sent waiters the wrong
    /// way; it is blocked and may be hung.
    #[test]
    fn a_long_held_tree_on_an_idle_host_is_stalled_not_starved() {
        let activity = activity_from_samples(21468, 7_260_000, &incident_samples(), Some(30.0))
            .expect("the owner is sampled");

        assert!(!activity.starved(), "{activity:?}");
        assert_eq!(activity.state(), "stalled");
        let described = activity.describe();
        assert!(described.contains("stalled"), "{described}");
        assert!(described.contains("blocked waiting"), "{described}");
        assert!(described.contains("may be hung"), "{described}");
        assert!(!described.contains("not hung"), "{described}");
    }

    /// Without the host load, a stuck holder must not be called starved.
    #[test]
    fn an_unknown_host_load_never_claims_starvation() {
        let activity = activity_from_samples(21468, 7_260_000, &incident_samples(), None)
            .expect("the owner is sampled");

        assert_eq!(activity.state(), "stalled");
        let described = activity.describe();
        assert!(
            described.contains("starved of CPU or blocked waiting"),
            "{described}"
        );
    }

    #[test]
    fn a_tree_using_its_cores_is_progressing() {
        let samples = [
            sample(10, Some(1), 1_000, 1_200_000),
            sample(11, Some(10), 2_000, 1_100_000),
            sample(12, Some(11), 90_000, 30_000),
        ];
        let activity = activity_from_samples(10, 1_200_000, &samples, Some(95.0)).expect("sampled");

        assert!(activity.cpu_percent > 100.0, "{activity:?}");
        assert!(!activity.starved(), "{activity:?}");
        assert_eq!(activity.state(), "progressing");
    }

    /// A young holder has not had time to show a trend.
    #[test]
    fn a_young_holder_is_never_stuck() {
        let samples = [sample(10, Some(1), 10, 60_000)];
        let activity = activity_from_samples(10, 60_000, &samples, Some(95.0)).expect("sampled");

        assert!(activity.cpu_percent < STARVED_BELOW_CPU_PERCENT);
        assert_eq!(activity.state(), "progressing");
    }

    #[test]
    fn a_missing_owner_has_no_activity() {
        let samples = [sample(10, Some(1), 10, 60_000)];
        assert_eq!(activity_from_samples(42, 60_000, &samples, None), None);
    }
}
