//! Issue #4405 AC-3: tell a starved lease holder from a progressing one.
//!
//! The host-wide lease protects one workload. When that workload is not
//! making progress, every waiter stalls behind it, yet from the outside the
//! holder only looks "busy".
//!
//! Issue #4561 corrects what this module used to assume — that the workload
//! *is* the holder's process tree. A `spawn_host: daemon` holder launches its
//! commands from the daemon instead, so its own tree is a socket wait however
//! hard the verification works. `HolderWorkload` names where to look, and a
//! reading that could not find the work says `unknown` rather than `stalled`.
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
//!
//! Issue #4633 adds the case none of those verdicts covers: a holder whose
//! requester is gone. Closing an agent window left its `gwtd verify.run`
//! alive under `ppid 1` with nothing left to run, and it kept the host-wide
//! lease until the TTL. That verdict rests on process *structure* — the parent
//! exited, and no command of the holder's is left in either tree — not on a
//! CPU rate, because a rate cannot tell a dead holder from a slow one.

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
/// Below this share of one core, CPU a working set gained is scheduler noise
/// — timer and socket wakeups of processes that only wait — rather than a
/// workload moving (Issue #4633 AC-5). An idle `gwtd` wakes for well under
/// 0.1% of a core; the slowest real workload measured, the I/O-bound ChromaDB
/// matrix, reads 1.7% (Issue #4409 AC-9). Half a percent sits between them.
pub(crate) const PROGRESS_FLOOR_CPU_PERCENT: f64 = 0.5;

/// Where the work this lease protects actually runs (Issue #4561).
///
/// The doc comment above assumed one process tree, and that assumption breaks
/// the moment a holder escapes its own: a `spawn_host: daemon` holder
/// (Issue #4409) launches every command from the daemon and then only waits on
/// a socket, so its own tree gains nothing however hard the verification works.
/// Read alone, that tree reports *every* daemon-hosted holder stalled five
/// minutes in — measured on 2026-09-21 against a test binary at 34.5% of a
/// core, and acted on three times.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HolderWorkload {
    /// Inside the holder's own tree, which `owner_pid`'s descendants measure.
    Owned,
    /// Launched from other process trees. `hosts` are the launchers — the
    /// project's live daemons — whose verification children carry the work.
    /// Only those children count: a daemon runs scans and hook children of
    /// its own, and that is not this lease's work (Issue #4633 AC-5).
    Delegated { hosts: Vec<u32> },
}

impl HolderWorkload {
    fn hosts(&self) -> &[u32] {
        match self {
            Self::Owned => &[],
            Self::Delegated { hosts } => hosts,
        }
    }

    fn is_delegated(&self) -> bool {
        matches!(self, Self::Delegated { .. })
    }
}

/// One live process, as the host reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProcessSample {
    pub pid: u32,
    pub parent: Option<u32>,
    pub cpu_ms: u64,
    /// The process leads its own session. The daemon `setsid`s every
    /// verification child it spawns (Issue #4409), and nothing else it runs,
    /// so this is what separates a delegated workload from its housekeeping.
    pub session_leader: bool,
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
    /// The holder launched its commands outside its own process tree
    /// (Issue #4561). A silent working set then has two explanations — the
    /// work is wedged, or this reading never found it — so the verdict may
    /// not claim the first one.
    pub delegated: bool,
    /// The holder's parent exited: it was reparented to `init`, or its parent
    /// pid no longer names a live process (Issue #4633 AC-1).
    pub parent_gone: bool,
    /// Processes in the working set besides the holder itself, at the later
    /// reading: its own descendants plus any delegated verification child.
    pub workload_processes: usize,
}

impl HolderActivity {
    fn old_enough(&self) -> bool {
        u128::from(self.held_ms) >= STARVED_AFTER.as_millis()
    }

    /// The tree moved: it gained more CPU than scheduler noise, or a
    /// process started or exited.
    fn advancing(&self) -> bool {
        self.cpu_percent >= PROGRESS_FLOOR_CPU_PERCENT || self.turnover
    }

    /// The requester is gone and the holder has nothing left to finish
    /// (Issue #4633 AC-1): its parent exited, no command of its own is alive
    /// in its tree or under the daemon, and it gained no more than noise.
    ///
    /// A live holder cannot meet this. Its parent is the agent, or the daemon
    /// for a daemon-spawned holder; a holder that lost its parent mid-run
    /// still has the command it is waiting on. Host load cannot produce it
    /// either — saturation slows a process, it does not reparent it — so a
    /// starved holder is never read as an orphan.
    pub(crate) fn orphaned(&self) -> bool {
        self.parent_gone && self.workload_processes == 0 && !self.advancing()
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
    /// change. The only measured basis for "may be hung" (AC-10) — and only
    /// for a holder whose work is inside the tree that was read
    /// (Issue #4561 AC-2).
    fn wedged(&self) -> bool {
        self.old_enough() && !self.advancing() && !self.delegated
    }

    /// The same silence, from a holder whose work runs somewhere this reading
    /// may not have reached. Not knowing and being stopped are different
    /// answers and must not share a value (Issue #4561 AC-2).
    fn undecidable(&self) -> bool {
        self.old_enough() && !self.advancing() && self.delegated
    }

    pub(crate) fn state(&self) -> &'static str {
        if self.orphaned() {
            "orphaned"
        } else if self.undecidable() {
            "unknown"
        } else if self.starved() {
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
        if self.orphaned() {
            return format!(
                "holder orphaned: held {held}, its parent process exited and no command of its \
                 own is left running — in its tree or under the daemon — over the last \
                 {window:.1}s. Nobody is waiting for this run and it will not release the lease \
                 before the TTL; reclaim it with `verify.lease.release` and a reason"
            );
        }
        if self.undecidable() {
            return format!(
                "holder state unknown: held {held}, it launched its commands outside its own \
                 process tree (spawn-host daemon) and none of the {processes} processes this \
                 reading could reach gained CPU over the last {window:.1}s — that is not evidence \
                 of a stopped holder, only that the work was not found. Confirm with `ps -eo \
                 pid,ppid,time,command` before acting on it"
            );
        }
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
                "holder stalled: held {held}, its {processes} processes gained no CPU time beyond \
                 scheduler noise (< {PROGRESS_FLOOR_CPU_PERCENT}% of one core) over the last \
                 {window:.1}s and none started or exited — it is blocked waiting (I/O, a lock or \
                 a prompt) and may be hung"
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

/// The activity of the work `workload` says this lease is protecting, across
/// two readings `window_ms` apart, or `None` when the owner is not in
/// `second`.
///
/// `first` is the earlier reading. Processes present in only one of them are
/// turnover: a child that exited finished its work, and one that appeared
/// took over.
pub(crate) fn activity_from_samples(
    owner_pid: u32,
    workload: &HolderWorkload,
    held_ms: u64,
    first: &[ProcessSample],
    second: &[ProcessSample],
    window_ms: u64,
    host_cpu_percent: Option<f64>,
) -> Option<HolderActivity> {
    // The owner's own absence still means the holder is gone (#4496), so it
    // stays the one root that must be present.
    let mut tree = descendants(owner_pid, second)?;
    let mut earlier_tree = descendants(owner_pid, first).unwrap_or_default();
    for host in workload.hosts() {
        tree.extend(delegated_descendants(*host, second));
        earlier_tree.extend(delegated_descendants(*host, first));
    }
    let before: HashMap<u32, u64> = first
        .iter()
        .map(|sample| (sample.pid, sample.cpu_ms))
        .collect();

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
    let parent_gone = second
        .iter()
        .find(|sample| sample.pid == owner_pid)
        .and_then(|owner| owner.parent)
        .is_none_or(|parent| parent <= 1 || !second.iter().any(|sample| sample.pid == parent));
    Some(HolderActivity {
        held_ms,
        cpu_percent,
        cpu_gained_ms,
        turnover,
        processes,
        window_ms,
        host_cpu_percent,
        delegated: workload.is_delegated(),
        parent_gone,
        workload_processes: tree.len().saturating_sub(1),
    })
}

/// The verification children a launcher spawned, and their descendants
/// (Issue #4561).
///
/// The daemon is where the work was started from, not the work: it runs
/// Issue Monitor scans and hook children of its own, and counting its CPU as
/// this lease's progress would answer "progressing" for a holder whose
/// commands died. Issue #4633 measured exactly that — a dead holder reported
/// progressing on its daemon's hook children — so only the children that lead
/// their own session, which the daemon makes of verification commands alone,
/// are followed.
fn delegated_descendants(host_pid: u32, samples: &[ProcessSample]) -> BTreeSet<u32> {
    samples
        .iter()
        .filter(|sample| sample.parent == Some(host_pid) && sample.session_leader)
        .filter_map(|child| descendants(child.pid, samples))
        .flatten()
        .collect()
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
        workload: &HolderWorkload,
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
            workload,
            held_ms,
            &first,
            &second,
            window_ms,
            host_cpu_percent,
        )
    }
}

/// One-shot reading, for a caller with no wait loop to amortize it over.
pub(crate) fn observe(
    owner_pid: u32,
    workload: &HolderWorkload,
    acquired_at_ms: Option<u64>,
) -> Option<HolderActivity> {
    HolderProbe::default().observe(owner_pid, workload, acquired_at_ms)
}

/// What a ticket's `holder_spawn_host` says about where to look for the work
/// (Issue #4561). Only `daemon` escapes the holder's tree; `inherit` and an
/// unresolved host both keep their commands inside it.
pub(crate) fn workload_for(
    spawn_host: Option<&str>,
    hosts: impl FnOnce() -> Vec<u32>,
) -> HolderWorkload {
    match spawn_host {
        Some("daemon") => HolderWorkload::Delegated { hosts: hosts() },
        _ => HolderWorkload::Owned,
    }
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
            session_leader: cfg!(unix) && process.session_id() == Some(process.pid()),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW_MS: u64 = 1_000;

    /// The agent that launched `gwtd`. Every holder in these fixtures is
    /// still attached to it unless a test says otherwise.
    const AGENT_PID: u32 = 700;

    fn sample(pid: u32, parent: Option<u32>, cpu_ms: u64) -> ProcessSample {
        ProcessSample {
            pid,
            parent,
            cpu_ms,
            session_leader: false,
        }
    }

    /// A daemon-spawned verification child: `setsid` makes it lead its own
    /// session, which is what tells it apart from the daemon's own work.
    fn leader(pid: u32, parent: Option<u32>, cpu_ms: u64) -> ProcessSample {
        ProcessSample {
            session_leader: true,
            ..sample(pid, parent, cpu_ms)
        }
    }

    fn agent() -> ProcessSample {
        sample(AGENT_PID, Some(1), 50_000)
    }

    fn activity(
        first: &[ProcessSample],
        second: &[ProcessSample],
        held_ms: u64,
        host_cpu_percent: Option<f64>,
    ) -> HolderActivity {
        activity_from_samples(
            21468,
            &HolderWorkload::Owned,
            held_ms,
            first,
            second,
            WINDOW_MS,
            host_cpu_percent,
        )
        .expect("the owner is sampled")
    }

    /// `gwtd` waits on `cargo`, which waits on the binary that does the work.
    /// Only the last one is asked whether the tree is moving.
    fn tree(leaf_cpu_ms: u64) -> [ProcessSample; 5] {
        [
            agent(),
            sample(21468, Some(AGENT_PID), 11_120),
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
            agent(),
            sample(95217, Some(AGENT_PID), 80),
            sample(96677, Some(95217), 560),
            sample(98114, Some(96677), 3_920),
            sample(98275, Some(96677), 830),
            sample(98304, Some(96677), 270),
        ];
        // The three `rustc` processes finished and a fresh one took over; it
        // has barely any CPU of its own yet.
        let second = [
            agent(),
            sample(95217, Some(AGENT_PID), 80),
            sample(96677, Some(95217), 560),
            sample(98503, Some(96677), 30),
        ];
        let activity = activity_from_samples(
            95217,
            &HolderWorkload::Owned,
            576_000,
            &first,
            &second,
            WINDOW_MS,
            Some(40.0),
        )
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
            agent(),
            sample(21468, Some(AGENT_PID), 1_000),
            sample(86652, Some(21468), 2_000),
            sample(16492, Some(86652), 90_000),
        ];
        let second = [
            agent(),
            sample(21468, Some(AGENT_PID), 1_000),
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
        let first = [agent(), sample(21468, Some(AGENT_PID), 10)];
        let activity = activity(&first, &first, 60_000, Some(95.0));

        assert!(activity.cpu_percent < STARVED_BELOW_CPU_PERCENT);
        assert_eq!(activity.state(), "progressing");
    }

    #[test]
    fn a_missing_owner_has_no_activity() {
        let samples = [sample(10, Some(1), 10)];
        assert_eq!(
            activity_from_samples(
                42,
                &HolderWorkload::Owned,
                60_000,
                &samples,
                &samples,
                WINDOW_MS,
                None
            ),
            None
        );
    }

    const DAEMON_PID: u32 = 56109;

    /// The 2026-09-21 reading, as the PM measured it: the holder (`12121`)
    /// waits on a socket with no children at all, while the verification it
    /// launched runs under the daemon — `cargo test` (`94966`) driving a test
    /// binary (`6491`) at a third of a core.
    fn daemon_hosted(leaf_cpu_ms: u64) -> [ProcessSample; 5] {
        [
            agent(),
            sample(12121, Some(AGENT_PID), 4_100),
            sample(DAEMON_PID, Some(1), 900_000),
            leader(94966, Some(DAEMON_PID), 2_430),
            sample(6491, Some(94966), leaf_cpu_ms),
        ]
    }

    fn delegated() -> HolderWorkload {
        HolderWorkload::Delegated {
            hosts: vec![DAEMON_PID],
        }
    }

    fn daemon_activity(
        first: &[ProcessSample],
        second: &[ProcessSample],
        held_ms: u64,
    ) -> HolderActivity {
        activity_from_samples(
            12121,
            &delegated(),
            held_ms,
            first,
            second,
            WINDOW_MS,
            Some(30.0),
        )
        .expect("the owner is sampled")
    }

    /// Issue #4561 AC-1/AC-3: the holder's own tree is empty of work because
    /// the work is not in it. Reading only that tree reported every
    /// daemon-hosted holder stalled after five minutes, and the PM asked a
    /// live `verify.run` to be interrupted on the strength of it.
    #[test]
    fn a_daemon_hosted_holder_is_progressing_while_its_workload_burns_cpu() {
        let activity = daemon_activity(&daemon_hosted(15_400), &daemon_hosted(15_750), 499_762);

        assert_eq!(activity.cpu_gained_ms, 350, "{activity:?}");
        assert_eq!(activity.state(), "progressing", "{activity:?}");
        let described = activity.describe();
        assert!(!described.contains("may be hung"), "{described}");
        assert!(!described.contains("stalled"), "{described}");
    }

    /// Issue #4561 AC-2: the same holder with nothing moving anywhere. The
    /// reading cannot separate a wedged holder from work it never found, so
    /// it must not present the two the same way.
    #[test]
    fn a_silent_daemon_hosted_holder_is_unknown_rather_than_stalled() {
        let activity = daemon_activity(&daemon_hosted(15_750), &daemon_hosted(15_750), 499_762);

        assert_eq!(activity.cpu_gained_ms, 0, "{activity:?}");
        assert_eq!(activity.state(), "unknown", "{activity:?}");
        let described = activity.describe();
        assert!(!described.contains("may be hung"), "{described}");
        assert!(described.contains("not evidence"), "{described}");
        assert!(described.contains("ps -eo"), "{described}");
    }

    /// The daemon is the launcher, not the work. Its own CPU — Issue Monitor
    /// scans, hook children — must not stand in for a workload that is gone,
    /// or the verdict flips to "progressing" for a holder with nothing left
    /// running.
    #[test]
    fn the_daemons_own_cpu_is_not_the_holders_progress() {
        let idle = [
            agent(),
            sample(12121, Some(AGENT_PID), 4_100),
            sample(DAEMON_PID, Some(1), 900_000),
        ];
        let busy = [
            agent(),
            sample(12121, Some(AGENT_PID), 4_100),
            sample(DAEMON_PID, Some(1), 903_000),
        ];
        let activity = daemon_activity(&idle, &busy, 499_762);

        assert_eq!(activity.cpu_gained_ms, 0, "{activity:?}");
        assert_eq!(activity.state(), "unknown", "{activity:?}");
    }

    /// A holder that never left its own tree keeps the #4409 verdict: the
    /// measurement covers its work, so silence still means wedged.
    #[test]
    fn an_in_tree_holder_is_still_reported_stalled() {
        let activity = activity(&tree(3_600), &tree(3_600), 7_260_000, Some(30.0));

        assert!(!activity.delegated, "{activity:?}");
        assert_eq!(activity.state(), "stalled");
    }

    /// Issue #4633 AC-5: the reading behind the "progressing" report for a
    /// dead holder. Hook children the daemon runs for itself come and go and
    /// burn a few milliseconds each; they are not a verification this lease
    /// launched, so they must not count as its progress.
    #[test]
    fn the_daemons_housekeeping_children_are_not_the_holders_progress() {
        let first = [
            agent(),
            sample(12121, Some(AGENT_PID), 4_100),
            sample(DAEMON_PID, Some(1), 900_000),
            sample(95001, Some(DAEMON_PID), 10),
        ];
        let second = [
            agent(),
            sample(12121, Some(AGENT_PID), 4_100),
            sample(DAEMON_PID, Some(1), 900_400),
            sample(95001, Some(DAEMON_PID), 42),
            sample(95002, Some(DAEMON_PID), 50),
        ];
        let activity = daemon_activity(&first, &second, 1_480_094);

        assert_eq!(activity.cpu_gained_ms, 0, "{activity:?}");
        assert!(!activity.turnover, "{activity:?}");
        assert_eq!(activity.state(), "unknown", "{activity:?}");
    }

    /// Issue #4633 AC-5: a few milliseconds over a window is scheduler noise
    /// — timers and socket wakeups of a process that is only waiting — not a
    /// workload moving. Counting it reported a dead holder as progressing.
    #[test]
    fn scheduler_noise_is_not_progress() {
        let activity = activity(&tree(3_600), &tree(3_603), 7_260_000, Some(30.0));

        assert!(
            activity.cpu_percent < PROGRESS_FLOOR_CPU_PERCENT,
            "{activity:?}"
        );
        assert_eq!(activity.state(), "stalled", "{activity:?}");
    }

    /// Issue #4633 AC-1/AC-6 (b): the 2026-09-22 reading. The window that
    /// ran `verify.run` was closed; its `gwtd` survived with `ppid 1`, no
    /// children, and no verification child left under the daemon. It holds
    /// the lease for a requester that no longer exists.
    #[test]
    fn a_holder_whose_parent_exited_with_no_work_left_is_orphaned() {
        let first = [
            sample(70012, Some(1), 4_100),
            sample(DAEMON_PID, Some(1), 900_000),
            sample(95001, Some(DAEMON_PID), 10),
        ];
        let second = [
            sample(70012, Some(1), 4_100),
            sample(DAEMON_PID, Some(1), 900_220),
            sample(95001, Some(DAEMON_PID), 42),
        ];
        let activity = activity_from_samples(
            70012,
            &delegated(),
            1_480_094,
            &first,
            &second,
            WINDOW_MS,
            Some(95.0),
        )
        .expect("the owner is sampled");

        assert!(activity.parent_gone, "{activity:?}");
        assert_eq!(activity.state(), "orphaned", "{activity:?}");
        let described = activity.describe();
        assert!(described.contains("orphaned"), "{described}");
        assert!(described.contains("verify.lease.release"), "{described}");
        assert!(!described.contains("progressing"), "{described}");
    }

    /// A parent that is no longer in the process table is gone too — the
    /// Windows shape, where a child keeps its dead parent's pid.
    #[test]
    fn a_holder_whose_parent_left_the_process_table_is_orphaned() {
        let alone = [sample(21468, Some(AGENT_PID), 11_120)];
        let activity = activity(&alone, &alone, 60_000, Some(30.0));

        assert!(activity.parent_gone, "{activity:?}");
        assert_eq!(activity.state(), "orphaned", "{activity:?}");
    }

    /// Losing the parent alone is not enough: a holder still running its
    /// commands finishes them and releases the lease on its own.
    #[test]
    fn an_orphan_still_running_its_commands_is_not_orphaned() {
        let first = [
            sample(21468, Some(1), 11_120),
            sample(86652, Some(21468), 2_430),
            sample(16492, Some(86652), 3_600),
        ];
        let second = [
            sample(21468, Some(1), 11_120),
            sample(86652, Some(21468), 2_430),
            sample(16492, Some(86652), 3_600),
        ];
        let activity = activity(&first, &second, 7_260_000, Some(30.0));

        assert!(activity.parent_gone, "{activity:?}");
        assert_ne!(activity.state(), "orphaned", "{activity:?}");
    }

    /// The same for a daemon-hosted holder: while its verification child is
    /// alive under the daemon — even idle, waiting on the network — the
    /// holder may still get an answer, so it is not reclaimable.
    #[test]
    fn a_daemon_hosted_orphan_with_a_live_verification_child_is_not_orphaned() {
        let samples = [
            sample(12121, Some(1), 4_100),
            sample(DAEMON_PID, Some(1), 900_000),
            leader(94966, Some(DAEMON_PID), 2_430),
        ];
        let activity = daemon_activity(&samples, &samples, 499_762);

        assert!(activity.parent_gone, "{activity:?}");
        assert_eq!(activity.state(), "unknown", "{activity:?}");
    }

    /// Issue #4633 AC-1/AC-6 (a): a live holder is never orphaned, however
    /// quiet it is — the silent daemon-hosted reading stays `unknown`.
    #[test]
    fn a_quiet_holder_whose_parent_lives_is_not_orphaned() {
        let activity = daemon_activity(&daemon_hosted(15_750), &daemon_hosted(15_750), 499_762);

        assert!(!activity.parent_gone, "{activity:?}");
        assert_ne!(activity.state(), "orphaned", "{activity:?}");
    }

    /// Issue #4633 AC-5: a starved holder is runnable and attached; the host
    /// being busy cannot reparent it, so it stays `starved`. A real orphan on
    /// the same saturated host is still called an orphan.
    #[test]
    fn saturation_never_turns_a_starved_holder_into_an_orphan() {
        let starved = activity(&tree(3_600), &tree(3_614), 7_260_000, Some(95.0));
        assert_eq!(starved.state(), "starved", "{starved:?}");

        let alone = [sample(21468, Some(1), 11_120)];
        let orphan = activity(&alone, &alone, 7_260_000, Some(95.0));
        assert_eq!(orphan.state(), "orphaned", "{orphan:?}");
    }

    #[test]
    fn only_a_daemon_spawn_host_looks_outside_the_holders_tree() {
        assert_eq!(
            workload_for(Some("daemon"), || vec![DAEMON_PID]),
            delegated()
        );
        assert_eq!(
            workload_for(Some("inherit"), || vec![DAEMON_PID]),
            HolderWorkload::Owned
        );
        assert_eq!(workload_for(None, Vec::new), HolderWorkload::Owned);
    }
}
