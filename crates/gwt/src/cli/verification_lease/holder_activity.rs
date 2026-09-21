//! Issue #4405 AC-3: tell a starved lease holder from a progressing one.
//!
//! The host-wide lease is held by one process tree. When that tree is not
//! making progress, every waiter stalls behind it, yet from the outside the
//! holder only looks "busy".
//!
//! Issue #4409 AC-8/9/10 settles *how* that question is asked. A single
//! snapshot cannot answer it: averaging each process's CPU over its own
//! lifetime reads low whenever the tree has been alive far longer than it has
//! been busy, and both shapes of real verification work land there. The PM
//! measured two of them in one morning — an I/O-bound ChromaDB matrix whose
//! leaves wait on disk, and a `cargo` build whose `rustc` children live for
//! seconds each, so freshly spawned ones hold no CPU at all. Both were
//! reported "stalled ... may be hung" while working, and the PM nearly killed
//! one of them.
//!
//! So the verdict comes from two readings instead: how much CPU the tree
//! *gained* between them, and whether its process set turned over. Neither is
//! a guess. Wait-only intermediates (`gwtd`, `cargo`) gain nothing by design
//! and are left out of the working set, because counting them only dilutes
//! what the leaves are doing.
//!
//! Low CPU still has two causes, and the host's load separates them: on a
//! saturated host the tree is runnable but not being scheduled (starved). But
//! "may be hung" is now reserved for the one measurement that supports it —
//! the tree gained no CPU at all and nothing started or exited. The #4426
//! holder, wedged on `git-credential-manager` prompts, is that case.

use std::collections::{BTreeSet, HashMap};
use std::time::{Duration, Instant};

/// A holder younger than this is never called stuck: a matrix spends its
/// first minutes on short-lived processes whose CPU time is already gone.
pub(crate) const STARVED_AFTER: Duration = Duration::from_secs(5 * 60);
/// Below this share of one core, a held tree is not being scheduled.
pub(crate) const STARVED_BELOW_CPU_PERCENT: f64 = 10.0;
/// At or above this host-wide CPU usage, a slow tree is starved rather than
/// blocked.
pub(crate) const HOST_SATURATED_CPU_PERCENT: f64 = 80.0;
/// How far apart the two readings are taken when the caller has no earlier
/// one. Long enough that a leaf at a single percent of one core still shows
/// a gain under Linux's 10 ms clock tick.
pub(crate) const PROGRESS_WINDOW: Duration = Duration::from_millis(1_200);

/// One live process, as the host reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProcessSample {
    pub pid: u32,
    pub parent: Option<u32>,
    pub cpu_ms: u64,
}

/// How the lease holder's process tree is doing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct HolderActivity {
    pub held_ms: u64,
    /// CPU the working set gained over the sample window, as a percentage of
    /// one core.
    pub cpu_percent: f64,
    /// Milliseconds of CPU the tree gained over the window. Zero — with no
    /// turnover — is the only evidence that supports "may be hung" (AC-10).
    pub cpu_gained_ms: u64,
    /// The tree gained or lost a process over the window. Short-lived `rustc`
    /// children turning over is progress even when the rate reads low.
    pub turnover: bool,
    /// Processes that did work: the tree's leaves plus any parent that burned
    /// CPU of its own. Wait-only intermediates are excluded (AC-8).
    pub processes: usize,
    /// How long the two readings were apart.
    pub window_ms: u64,
    /// Host-wide CPU usage in percent, `None` when it could not be sampled.
    pub host_cpu_percent: Option<f64>,
}

impl HolderActivity {
    fn old_enough(&self) -> bool {
        u128::from(self.held_ms) >= STARVED_AFTER.as_millis()
    }

    /// The tree moved: it gained CPU, or a process started or exited.
    fn advancing(&self) -> bool {
        self.cpu_gained_ms > 0 || self.turnover
    }

    /// Barely scheduled on a saturated host: runnable, just not getting a
    /// turn. It is still moving, so it is never called hung.
    pub(crate) fn starved(&self) -> bool {
        self.old_enough()
            && self.cpu_percent < STARVED_BELOW_CPU_PERCENT
            && self
                .host_cpu_percent
                .is_some_and(|host| host >= HOST_SATURATED_CPU_PERCENT)
    }

    /// Nothing in the tree gained a millisecond and its process set did not
    /// change. The only measured basis for "may be hung" (AC-10).
    fn wedged(&self) -> bool {
        self.old_enough() && !self.advancing()
    }

    pub(crate) fn state(&self) -> &'static str {
        if self.starved() {
            "starved"
        } else if self.wedged() {
            "stalled"
        } else {
            "progressing"
        }
    }

    pub(crate) fn describe(&self) -> String {
        let held = format!("{}m{:02}s", self.held_ms / 60_000, self.held_ms / 1000 % 60);
        let cpu = self.cpu_percent;
        let processes = self.processes;
        let window = self.window_ms as f64 / 1000.0;
        if self.starved() {
            let host = self.host_cpu_percent.unwrap_or_default();
            return format!(
                "holder starved: held {held}, its {processes} working processes got {cpu:.1}% of \
                 one core over the last {window:.1}s while the host CPU is {host:.0}% busy — it \
                 is runnable but not being scheduled, not hung"
            );
        }
        if self.wedged() {
            return format!(
                "holder stalled: held {held}, its {processes} processes gained no CPU time over \
                 the last {window:.1}s and none started or exited — it is blocked waiting (I/O, a \
                 lock or a prompt) and may be hung"
            );
        }
        let gained = self.cpu_gained_ms;
        let turnover = if self.turnover {
            ", and its process set turned over"
        } else {
            ""
        };
        format!(
            "holder progressing: held {held}, its {processes} working processes gained {gained}ms \
             of CPU over the last {window:.1}s ({cpu:.1}% of one core){turnover}"
        )
    }
}

/// The activity of `owner_pid` and its live descendants across two readings
/// `window_ms` apart, or `None` when the owner is not in `second`.
///
/// `first` is the earlier reading. Processes present in only one of them are
/// turnover: a child that exited finished its work, and one that appeared
/// took over.
pub(crate) fn activity_from_samples(
    owner_pid: u32,
    held_ms: u64,
    first: &[ProcessSample],
    second: &[ProcessSample],
    window_ms: u64,
    host_cpu_percent: Option<f64>,
) -> Option<HolderActivity> {
    let tree = descendants(owner_pid, second)?;
    let before: HashMap<u32, u64> = first
        .iter()
        .map(|sample| (sample.pid, sample.cpu_ms))
        .collect();
    let earlier_tree = descendants(owner_pid, first).unwrap_or_default();

    // A parent that only ever waits gains nothing; the leaves are what the
    // reading is about (AC-8).
    let has_live_child = |pid: u32| {
        second
            .iter()
            .any(|sample| sample.parent == Some(pid) && tree.contains(&sample.pid))
    };
    let mut cpu_gained_ms = 0u64;
    let mut processes = 0usize;
    for sample in second.iter().filter(|sample| tree.contains(&sample.pid)) {
        // A process absent from the earlier reading was born inside the
        // window, so all of its CPU was gained inside it.
        let gained = sample
            .cpu_ms
            .saturating_sub(before.get(&sample.pid).copied().unwrap_or(0));
        cpu_gained_ms = cpu_gained_ms.saturating_add(gained);
        if gained > 0 || !has_live_child(sample.pid) {
            processes += 1;
        }
    }
    let turnover = tree != earlier_tree;
    let cpu_percent = cpu_gained_ms as f64 / window_ms.max(1) as f64 * 100.0;
    Some(HolderActivity {
        held_ms,
        cpu_percent,
        cpu_gained_ms,
        turnover,
        processes,
        window_ms,
        host_cpu_percent,
    })
}

/// `owner_pid` and every live descendant of it in `samples`, or `None` when
/// the owner itself is not there.
fn descendants(owner_pid: u32, samples: &[ProcessSample]) -> Option<BTreeSet<u32>> {
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
    Some(tree.into_iter().collect())
}

/// Repeated readings of one holder. Kept across a wait loop's polls so each
/// verdict spans the whole interval since the last one instead of a window
/// this call has to pay for itself.
#[derive(Default)]
pub(crate) struct HolderProbe {
    previous: Option<(Vec<ProcessSample>, Instant)>,
}

impl HolderProbe {
    /// Describe the tree under `owner_pid`. `acquired_at_ms` is the lease
    /// ticket's Unix-epoch acquisition time.
    ///
    /// The first call has nothing to compare against, so it pays for its own
    /// `PROGRESS_WINDOW`; later calls compare against the previous one.
    pub(crate) fn observe(
        &mut self,
        owner_pid: u32,
        acquired_at_ms: Option<u64>,
    ) -> Option<HolderActivity> {
        let (first, taken_at) = match self.previous.take() {
            Some(previous) => previous,
            None => {
                let first = read_processes();
                let at = Instant::now();
                std::thread::sleep(PROGRESS_WINDOW);
                (first, at)
            }
        };
        let (second, second_at, host_cpu_percent) = read_processes_and_host_cpu();
        let window_ms =
            u64::try_from(second_at.duration_since(taken_at).as_millis()).unwrap_or(u64::MAX);
        self.previous = Some((second.clone(), second_at));

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_millis();
        let held_ms = acquired_at_ms
            .map(|at| u64::try_from(now_ms.saturating_sub(u128::from(at))).unwrap_or(u64::MAX))
            .unwrap_or(0);
        activity_from_samples(
            owner_pid,
            held_ms,
            &first,
            &second,
            window_ms,
            host_cpu_percent,
        )
    }
}

/// One-shot reading, for a caller with no wait loop to amortize it over.
pub(crate) fn observe(owner_pid: u32, acquired_at_ms: Option<u64>) -> Option<HolderActivity> {
    HolderProbe::default().observe(owner_pid, acquired_at_ms)
}

fn read_processes() -> Vec<ProcessSample> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_cpu(),
    );
    collect(&system)
}

/// Host CPU needs two readings of its own, so it is sampled alongside the
/// process table rather than in a pass of its own.
fn read_processes_and_host_cpu() -> (Vec<ProcessSample>, Instant, Option<f64>) {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

    let mut system = System::new();
    system.refresh_cpu_usage();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_cpu(),
    );
    let read_at = Instant::now();
    let samples = collect(&system);
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    system.refresh_cpu_usage();
    let host = f64::from(system.global_cpu_usage());
    (
        samples,
        read_at,
        (host.is_finite() && host > 0.0).then_some(host),
    )
}

fn collect(system: &sysinfo::System) -> Vec<ProcessSample> {
    system
        .processes()
        .values()
        .map(|process| ProcessSample {
            pid: process.pid().as_u32(),
            parent: process.parent().map(sysinfo::Pid::as_u32),
            cpu_ms: process.accumulated_cpu_time(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW_MS: u64 = 1_000;

    fn sample(pid: u32, parent: Option<u32>, cpu_ms: u64) -> ProcessSample {
        ProcessSample {
            pid,
            parent,
            cpu_ms,
        }
    }

    fn activity(
        first: &[ProcessSample],
        second: &[ProcessSample],
        held_ms: u64,
        host_cpu_percent: Option<f64>,
    ) -> HolderActivity {
        activity_from_samples(21468, held_ms, first, second, WINDOW_MS, host_cpu_percent)
            .expect("the owner is sampled")
    }

    /// `gwtd` waits on `cargo`, which waits on the binary that does the work.
    /// Only the last one is asked whether the tree is moving.
    fn tree(leaf_cpu_ms: u64) -> [ProcessSample; 4] {
        [
            sample(21468, Some(1), 11_120),
            sample(86652, Some(21468), 2_430),
            sample(16492, Some(86652), leaf_cpu_ms),
            // An unrelated busy process must not lend the holder its CPU.
            sample(999, Some(1), 3_600_000),
        ]
    }

    /// Issue #4409 AC-8: `gwtd` and `cargo` only ever wait on their children,
    /// so counting them dilutes whatever the leaf is doing. The PM read
    /// "its 10 processes get 1.7% of one core" off a tree whose `rustc`
    /// children were using every core they could get.
    #[test]
    fn wait_only_intermediates_are_not_counted_as_working_processes() {
        let activity = activity(&tree(3_600), &tree(4_500), 900_000, Some(30.0));

        assert_eq!(activity.processes, 1, "{activity:?}");
        assert_eq!(activity.cpu_gained_ms, 900, "{activity:?}");
        assert!((activity.cpu_percent - 90.0).abs() < 0.001, "{activity:?}");
    }

    /// Issue #4409 AC-9, first mechanism: the ChromaDB verification is I/O
    /// bound. It gains little CPU because it is waiting on disk, not because
    /// it is wedged, and the PM nearly killed it over that reading.
    #[test]
    fn an_io_bound_tree_that_keeps_gaining_cpu_is_progressing() {
        let activity = activity(&tree(3_600), &tree(3_617), 900_000, Some(17.0));

        assert!(
            activity.cpu_percent < STARVED_BELOW_CPU_PERCENT,
            "{activity:?}"
        );
        assert_eq!(activity.state(), "progressing", "{activity:?}");
        let described = activity.describe();
        assert!(!described.contains("may be hung"), "{described}");
        assert!(!described.contains("stalled"), "{described}");
    }

    /// Issue #4409 AC-9, second mechanism: `rustc` processes live for seconds.
    /// Freshly spawned ones hold almost no CPU, so an instantaneous rate reads
    /// low exactly while the tree is compiling as fast as it can.
    #[test]
    fn a_tree_turning_over_short_lived_children_is_progressing() {
        let first = [
            sample(95217, Some(1), 80),
            sample(96677, Some(95217), 560),
            sample(98114, Some(96677), 3_920),
            sample(98275, Some(96677), 830),
            sample(98304, Some(96677), 270),
        ];
        // The three `rustc` processes finished and a fresh one took over; it
        // has barely any CPU of its own yet.
        let second = [
            sample(95217, Some(1), 80),
            sample(96677, Some(95217), 560),
            sample(98503, Some(96677), 30),
        ];
        let activity =
            activity_from_samples(95217, 576_000, &first, &second, WINDOW_MS, Some(40.0))
                .expect("the owner is sampled");

        assert!(activity.turnover, "{activity:?}");
        assert_eq!(activity.state(), "progressing", "{activity:?}");
        assert!(!activity.describe().contains("may be hung"));
    }

    /// Issue #4409 AC-10: the #4426 holder, wedged on `git-credential-manager`
    /// prompts. Nothing in the tree gained a millisecond and nothing started
    /// or exited — the only measurement that supports "may be hung".
    #[test]
    fn only_a_tree_that_gained_no_cpu_may_be_hung() {
        let activity = activity(&tree(3_600), &tree(3_600), 7_260_000, Some(30.0));

        assert_eq!(activity.cpu_gained_ms, 0, "{activity:?}");
        assert!(!activity.turnover, "{activity:?}");
        assert_eq!(activity.state(), "stalled");
        let described = activity.describe();
        assert!(described.contains("may be hung"), "{described}");
        assert!(described.contains("no CPU time"), "{described}");
        assert!(!described.contains("not hung"), "{described}");
    }

    /// Without the host load, a wedged holder is still wedged: the verdict
    /// rests on the tree's own CPU, not on how busy the host is.
    #[test]
    fn an_unknown_host_load_still_reports_a_wedged_holder() {
        let activity = activity(&tree(3_600), &tree(3_600), 7_260_000, None);

        assert_eq!(activity.state(), "stalled");
        assert!(activity.describe().contains("may be hung"));
    }

    /// A saturated host explains a low rate without accusing the holder of
    /// hanging, so waiters keep waiting instead of escalating.
    #[test]
    fn a_low_rate_on_a_saturated_host_is_starved_not_hung() {
        let activity = activity(&tree(3_600), &tree(3_614), 7_260_000, Some(95.0));

        assert!(activity.starved(), "{activity:?}");
        assert_eq!(activity.state(), "starved");
        let described = activity.describe();
        assert!(described.contains("not hung"), "{described}");
        assert!(!described.contains("may be hung"), "{described}");
    }

    #[test]
    fn a_tree_using_its_cores_is_progressing() {
        let first = [
            sample(21468, Some(1), 1_000),
            sample(86652, Some(21468), 2_000),
            sample(16492, Some(86652), 90_000),
        ];
        let second = [
            sample(21468, Some(1), 1_000),
            sample(86652, Some(21468), 2_000),
            sample(16492, Some(86652), 92_800),
        ];
        let activity = activity(&first, &second, 1_200_000, Some(95.0));

        assert!(activity.cpu_percent > 100.0, "{activity:?}");
        assert!(!activity.starved(), "{activity:?}");
        assert_eq!(activity.state(), "progressing");
    }

    /// A young holder has not had time to show a trend.
    #[test]
    fn a_young_holder_is_never_stuck() {
        let first = [sample(21468, Some(1), 10)];
        let activity = activity(&first, &first, 60_000, Some(95.0));

        assert!(activity.cpu_percent < STARVED_BELOW_CPU_PERCENT);
        assert_eq!(activity.state(), "progressing");
    }

    #[test]
    fn a_missing_owner_has_no_activity() {
        let samples = [sample(10, Some(1), 10)];
        assert_eq!(
            activity_from_samples(42, 60_000, &samples, &samples, WINDOW_MS, None),
            None
        );
    }
}
