//! SPEC #3576: only canonical `verify.run` owns verification leases.
//!
//! A lease lives inside its runner rather than a detached process with no
//! workload. The retired acquire/hold/extend operations keep actionable
//! diagnostics; status/release remain available to drain pre-upgrade holders.
//!
//! Issue #4285: the lease lives on the verification lane
//! (`~/.gwt/runtime/verification-coordinator`), not on the model lane that
//! searches and index builds share. A pre-upgrade binary still runs its
//! canonical verification on the model lane; that holder stays observable
//! and drainable here until every binary on the host has moved.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gwt_core::index_coordinator::{
    coordinator_root, verification_coordinator_root, HeavyHolderKind, HeavyLeaseStatus,
    HeavyQueueEntry, IndexCoordinator, TargetKey,
};
use gwt_core::paths::{project_scope_hash, resolve_current_worktree_root};
use gwt_core::worktree_hash::compute_worktree_hash;
use gwt_github::{client::ApiError, SpecOpsError};
use serde::{Deserialize, Serialize};

use crate::cli::CliEnv;

/// Issue #3913: `verify.run` host admission — the lease's in-process claimant.
pub(crate) mod admission;
/// Issue #4405: starved-versus-progressing reading of the lease holder.
pub(crate) mod holder_activity;

const CARGO_SCOPED_SUBCOMMANDS: &[&str] = &["test", "t", "nextest"];
/// Flags that widen a `cargo test` past a single target, wherever they sit.
const CARGO_SCOPE_WIDENING_FLAGS: &[&str] = &[
    "--workspace",
    "--all",
    "--all-features",
    "--all-targets",
    "--benches",
    "--bins",
    "--examples",
    "--tests",
    "--doc",
    "--bench",
    "--exclude",
];
/// Flags that name one target, so they narrow a `cargo test` on their own.
const CARGO_NAMED_TARGET_SELECTORS: &[&str] = &["--test", "--bin", "--example"];

/// How much of the shared host one *requested* verification command needs.
///
/// Classify the requested command line before canonical verification starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommandWeight {
    /// Narrow enough that several worktrees can run it side by side.
    Light,
    /// Builds or runs enough of the tree to need the host to itself.
    Heavy,
}

/// Classify one command `verify.run` was asked to execute (Issue #4196).
///
/// The host lease exists to stop several worktrees compiling the world at
/// once, and every `cargo test` used to claim it whatever its scope: a single
/// `--test <name>` queued behind `cargo test --workspace --all-features`, and
/// the fleet's verification throughput was pinned at one window at a time. So
/// the weight follows the scope the command will actually build — a widening
/// flag is heavy, and a run narrowed to named targets of at most one package
/// is light.
///
/// Anything this module cannot bound stays heavy. An unrecognized program may
/// compile the world, and guessing light for it would trade one window's wait
/// for the host-wide oversubscription the lease was built to prevent
/// (Issue #3913).
pub(crate) fn classify_command(command: &str) -> CommandWeight {
    let Ok(args) = crate::cli::verification_record::split_command_line(command) else {
        return CommandWeight::Heavy;
    };
    let Some(program) = args.first() else {
        return CommandWeight::Heavy;
    };
    if Path::new(program)
        .file_stem()
        .and_then(|stem| stem.to_str())
        != Some("cargo")
    {
        return CommandWeight::Heavy;
    }
    // Cargo's own arguments end at a bare `--`; everything after it is the
    // test binary's filter and says nothing about what cargo will build.
    let cargo_args: Vec<&str> = args[1..]
        .iter()
        .map(String::as_str)
        .take_while(|arg| *arg != "--")
        .collect();
    // Only skip global arguments known not to consume a value. Otherwise a
    // --config path (even one named "fmt") could be mistaken for a command.
    let mut args = cargo_args.iter().copied();
    let subcommand = loop {
        let Some(arg) = args.next() else {
            return CommandWeight::Heavy;
        };
        if arg.starts_with('+')
            || matches!(
                arg,
                "-v" | "--verbose" | "-q" | "--quiet" | "--offline" | "--locked" | "--frozen"
            )
        {
            continue;
        }
        if arg.starts_with('-') {
            return CommandWeight::Heavy;
        }
        break arg;
    };
    if matches!(subcommand, "fmt" | "metadata") {
        return CommandWeight::Light;
    }
    if !CARGO_SCOPED_SUBCOMMANDS.contains(&subcommand) {
        return CommandWeight::Heavy;
    }
    classify_cargo_scope(&cargo_args)
}

/// Weigh a scoped `cargo test` by the selection it builds.
///
/// `--lib` is deliberately not enough on its own. This repository is a virtual
/// workspace with `default-members`, so `cargo test --lib` with no package
/// selects the lib target of *every* default member — the workspace-wide build
/// this classification exists to catch, wearing a narrowing flag.
fn classify_cargo_scope(cargo_args: &[&str]) -> CommandWeight {
    let mut named_targets = 0usize;
    let mut lib_target = false;
    let mut packages = 0usize;
    let mut args = cargo_args.iter().copied();
    while let Some(arg) = args.next() {
        // `--test=name` and `--test name` select the same target.
        let (flag, inline_value) = arg
            .split_once('=')
            .map_or((arg, None), |(name, value)| (name, Some(value)));
        if CARGO_SCOPE_WIDENING_FLAGS.contains(&flag) {
            return CommandWeight::Heavy;
        }
        if flag == "--lib" {
            lib_target = true;
        }
        let attached_package = flag.strip_prefix("-p").filter(|value| !value.is_empty());
        let package = flag == "-p" || flag == "--package" || attached_package.is_some();
        if package || CARGO_NAMED_TARGET_SELECTORS.contains(&flag) {
            let Some(value) = attached_package.or(inline_value).or_else(|| args.next()) else {
                return CommandWeight::Heavy;
            };
            // Cargo expands these itself, including quoted package patterns.
            if value.is_empty() || value.starts_with('-') || value.contains(['*', '?', '[', ']']) {
                return CommandWeight::Heavy;
            }
            if package {
                packages += 1;
            } else {
                named_targets += 1;
            }
        }
    }
    if packages > 1 || named_targets + usize::from(lib_target) > 1 {
        return CommandWeight::Heavy;
    }
    if named_targets == 1 || (lib_target && packages == 1) {
        CommandWeight::Light
    } else {
        CommandWeight::Heavy
    }
}

/// The first command of a matrix that needs the host to itself, if any.
///
/// A matrix is only as light as its heaviest command, and naming the command
/// that forces the wait is what lets an agent see in advance whether the run
/// will queue — previously that was only discoverable by idling.
pub(crate) fn first_heavy_command(commands: &[String]) -> Option<&String> {
    commands
        .iter()
        .find(|command| classify_command(command) == CommandWeight::Heavy)
}

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
            let mut status = status()?;
            let worktree = resolve_current_worktree_root(env.repo_path());
            observe_holder_activity(&mut status, &worktree);
            render(out, "held", "free", &status);
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
            let worktree = resolve_current_worktree_root(env.repo_path());
            release(
                &lease_id,
                reason.as_deref(),
                &mut SystemReclaimer::new(&worktree),
                out,
            )
        }
    }
}

fn release(
    lease_id: &str,
    reason: Option<&str>,
    reclaimer: &mut dyn OrphanReclaimer,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let Some(control) = control_dir_for(lease_id) else {
        return reclaim_orphan(lease_id, reason, reclaimer, out);
    };
    // Issue #4360: the holder waits for this file to exist and then reads the
    // reason out of it, so a plain write lets it read the empty moment between
    // create and fill. Publishing by rename makes "exists" mean "complete" —
    // which also keeps an intentionally empty reason readable as itself.
    gwt_core::atomic_file::write_atomic(
        &control.join(RELEASE_FILE),
        reason.unwrap_or("").as_bytes(),
    )
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

/// Issue #4633: how long a holder that looked orphaned is watched again
/// before it is ended. The first reading spans [`PROGRESS_WINDOW`]; this one
/// has to show the same — parent gone, nothing of its own running, no CPU
/// beyond noise — over a window long enough to cover the gap between two
/// commands of a live matrix.
///
/// [`PROGRESS_WINDOW`]: holder_activity::PROGRESS_WINDOW
const ORPHAN_CONFIRM_WINDOW: Duration = Duration::from_secs(10);

/// What reclaiming an orphaned canonical lease needs from the host, so the
/// decision can be exercised without ending real processes.
trait OrphanReclaimer {
    /// Read the holder `pid` after waiting `pause`; a later call compares
    /// against the earlier one. `None` when the holder is gone.
    fn observe(
        &mut self,
        pid: u32,
        status: &HeavyLeaseStatus,
        pause: Duration,
    ) -> Option<holder_activity::HolderActivity>;

    /// End the holder: politely first, then — `force` — unconditionally.
    fn terminate(&mut self, pid: u32, force: bool) -> Result<(), String>;
}

struct SystemReclaimer {
    worktree: PathBuf,
    probe: holder_activity::HolderProbe,
}

impl SystemReclaimer {
    fn new(worktree: &Path) -> Self {
        Self {
            worktree: worktree.to_path_buf(),
            probe: holder_activity::HolderProbe::default(),
        }
    }
}

impl OrphanReclaimer for SystemReclaimer {
    fn observe(
        &mut self,
        pid: u32,
        status: &HeavyLeaseStatus,
        pause: Duration,
    ) -> Option<holder_activity::HolderActivity> {
        std::thread::sleep(pause);
        let workload = holder_workload(
            status.holder_spawn_host.as_deref(),
            status.target.as_deref(),
            &self.worktree,
        );
        self.probe.observe(pid, &workload, status.acquired_at_ms)
    }

    fn terminate(&mut self, pid: u32, force: bool) -> Result<(), String> {
        #[cfg(unix)]
        {
            let signal = if force { libc::SIGKILL } else { libc::SIGTERM };
            // SAFETY: `kill` has no memory-safety preconditions. `ESRCH` means
            // the holder is already gone, which is what the caller wants.
            let rc = unsafe { libc::kill(pid as libc::pid_t, signal) };
            let err = std::io::Error::last_os_error();
            if rc == 0 || err.raw_os_error() == Some(libc::ESRCH) {
                Ok(())
            } else {
                Err(format!("failed to signal holder pid {pid}: {err}"))
            }
        }
        #[cfg(not(unix))]
        {
            let _ = (pid, force);
            Err("reclaiming an orphaned verification holder is supported on Unix only".to_string())
        }
    }
}

/// Where the lease's work runs (Issue #4561), looking for a `daemon` holder's
/// work under the daemons of the holder's own project (Issue #4633): the lease
/// is host-wide, and the caller's worktree may belong to another project.
fn holder_workload(
    spawn_host: Option<&str>,
    target: Option<&str>,
    worktree: &Path,
) -> holder_activity::HolderWorkload {
    holder_activity::workload_for(spawn_host, || {
        target
            .and_then(crate::cli::daemon::verification_host::live_daemon_pids_for_lease_target)
            .unwrap_or_else(|| crate::cli::daemon::verification_host::live_daemon_pids(worktree))
    })
}

/// Issue #4633 AC-2/AC-3: free a canonical lease whose holder is orphaned.
///
/// A canonical lease is a kernel lock inside its `verify.run`, so the only way
/// to free it from outside is to end that process. That is allowed for one
/// holder only: one whose requester is gone and which has nothing left to
/// run, confirmed by two readings [`ORPHAN_CONFIRM_WINDOW`] apart. Every other
/// holder keeps the protection it had — the manual API cannot touch it.
fn reclaim_orphan(
    lease_id: &str,
    reason: Option<&str>,
    reclaimer: &mut dyn OrphanReclaimer,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let coordinator = open_coordinator()?;
    let held = |coordinator: &IndexCoordinator| {
        coordinator
            .heavy_lease_status()
            .ok()
            .filter(|status| status.held && status.lease_id.as_deref() == Some(lease_id))
    };
    let Some(status) = held(&coordinator) else {
        return Err(missing_lease(lease_id));
    };
    let Some(owner) = status.owner.clone() else {
        return Err(canonical_refusal(lease_id, None));
    };
    let first = reclaimer.observe(owner.pid, &status, Duration::ZERO);
    let Some(first) = first.filter(holder_activity::HolderActivity::orphaned) else {
        return Err(canonical_refusal(
            lease_id,
            first.map(|activity| activity.describe()).as_deref(),
        ));
    };
    let Some(reason) = reason.map(str::trim).filter(|reason| !reason.is_empty()) else {
        return Err(unexpected(format!(
            "verification lease {lease_id} is held by an orphaned holder ({}). Reclaiming it \
             ends pid {} and is recorded in the lease ledger, so it needs a reason: pass \
             `params.reason`.",
            first.describe(),
            owner.pid
        )));
    };
    let confirmed = reclaimer.observe(owner.pid, &status, ORPHAN_CONFIRM_WINDOW);
    let Some(confirmed) = confirmed.filter(holder_activity::HolderActivity::orphaned) else {
        return Err(canonical_refusal(
            lease_id,
            Some(&format!(
                "the holder looked orphaned, but a second reading {}s later did not confirm it: {}",
                ORPHAN_CONFIRM_WINDOW.as_secs(),
                confirmed
                    .map(|activity| activity.describe())
                    .unwrap_or_else(|| "the holder is gone".to_string())
            )),
        ));
    };
    // The lease must still be the same one, under the same owner, right
    // before its holder is ended: a pid is only a name, and ten seconds is
    // long enough for the lease to change hands.
    if held(&coordinator).and_then(|status| status.owner) != Some(owner.clone()) {
        return Err(missing_lease(lease_id));
    }
    reclaimer.terminate(owner.pid, false).map_err(unexpected)?;
    if await_settled(lease_id).is_err() {
        reclaimer.terminate(owner.pid, true).map_err(unexpected)?;
        await_settled(lease_id)?;
    }
    let target = status.target.as_deref().unwrap_or("unknown");
    let record = format!(
        "orphaned holder pid {} reclaimed by pid {}: {reason}",
        owner.pid,
        std::process::id()
    );
    coordinator.record_lease_reclaimed(lease_id, target, owner.clone(), &record);
    out.push_str("verification lease: reclaimed\n");
    out.push_str(&format!("lease_id: {lease_id}\n"));
    out.push_str(&format!("reclaimed_pid: {}\n", owner.pid));
    out.push_str(&format!("reason: {reason}\n"));
    out.push_str(&format!("holder_state_detail: {}\n", confirmed.describe()));
    out.push_str(&format!(
        "recorded: {} (kind reclaimed)\n",
        coordinator.lease_event_log_path().display()
    ));
    push_status_fields(out, &self::status()?);
    Ok(0)
}

/// Issue #4633 AC-4: what a caller closing a pane in `worktree` should know
/// about the canonical lease, if that worktree holds it.
///
/// Closing a pane does not end a `verify.run` it started: the runner keeps
/// the lease until its matrix finishes, which is the right outcome while it
/// is still working. So the close is not refused and the runner is not ended
/// first — the caller is told, and told how to reclaim the lease if the
/// runner is left with nothing to run.
pub(crate) fn closing_pane_lease_note(worktree: &Path) -> Option<String> {
    let project = project_scope_hash(worktree);
    let worktree_hash = compute_worktree_hash(worktree).ok()?;
    let target = TargetKey::verification(project.as_str(), worktree_hash.as_str()).file_stem();
    let status = open_coordinator().ok()?.heavy_lease_status().ok()?;
    lease_note_for_target(&target, &status)
}

fn lease_note_for_target(target: &str, status: &HeavyLeaseStatus) -> Option<String> {
    if !status.held || status.target.as_deref() != Some(target) {
        return None;
    }
    let lease_id = status.lease_id.as_deref().unwrap_or("unknown");
    let pid = status
        .owner
        .as_ref()
        .map(|owner| owner.pid.to_string())
        .unwrap_or_else(|| "?".to_string());
    Some(format!(
        "note: this pane's worktree holds canonical verification lease {lease_id} (holder pid \
         {pid}). Closing the pane does not end that `verify.run`; it keeps the lease until its \
         matrix finishes. If it is left with nothing to run, `verify.lease.status` reports \
         `holder_state: orphaned`, and `verify.lease.release` with `params.lease_id` and \
         `params.reason` reclaims it without waiting for the TTL.\n"
    ))
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
/// a directory with nobody listening. Pre-upgrade detached holders wrote
/// their channel under the model lane, so both lanes are scanned.
fn control_dir_for(lease_id: &str) -> Option<PathBuf> {
    [verification_coordinator_root(), coordinator_root()]
        .into_iter()
        .filter_map(|root| fs::read_dir(root.join(CONTROL_DIR)).ok())
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .find(|dir| {
            read_json::<LeaseOutcome>(&dir.join(OUTCOME_FILE)).is_some_and(|outcome| {
                outcome.granted && outcome.status.lease_id.as_deref() == Some(lease_id)
            })
        })
}

/// Issue #4561 AC-4: what a reader may do with `holder_state`.
///
/// It is a sampled reading of one process set, not a decision about the
/// holder. `stalled` was acted on three times in one morning against holders
/// that were working, so the licence has to travel with the value.
const HOLDER_STATE_ADVICE: &str = "holder_state is one sampled reading, not a verdict — never \
                                   interrupt or kill a holder on it alone. `stalled` means this \
                                   reading saw no CPU and no process turnover; `unknown` means \
                                   the work runs outside the holder's tree and was not found; \
                                   `orphaned` means its parent exited with nothing of its own \
                                   left running — `verify.lease.release` with a reason \
                                   re-checks that and reclaims the lease. Confirm with \
                                   `ps -eo pid,ppid,time,command` before acting.";

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
    /// Issue #4409 AC-4: the holder's own priority, and whether it launches
    /// verification inside or outside the agent process tree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    holder_nice: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    holder_spawn_host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    remaining_batches: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    estimated_remaining_ms: Option<u64>,
    /// Issue #4405 AC-3: how long the holder has held the lease, the CPU
    /// its process tree gets, and whether that reads as starved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    holder_held_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    holder_cpu_percent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    holder_state: Option<String>,
    /// Issue #4561 AC-4: what the state was measured from. A bare
    /// `holder_state` carries no basis, so a reader could only take it at
    /// face value — which is how three live holders were reported stopped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    holder_state_detail: Option<String>,
    /// Issue #4470 AC-3: whether the ticket's owner process still exists and
    /// what job status it last published, so a waiter can tell a working
    /// holder from residue without reading the coordinator's files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    holder_alive: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    holder_job_status: Option<String>,
    #[serde(default)]
    holder_stale: bool,
}

/// Fill in the holder's activity; only the status report pays for the
/// process-table read.
fn observe_holder_activity(status: &mut LeaseStatusSnapshot, worktree: &Path) {
    let Some(pid) = status.owner_pid.filter(|_| status.held) else {
        return;
    };
    // Issue #4561: a `daemon` holder's work is not under `owner_pid`, so the
    // working set has to reach into the daemons that launched it.
    let workload = holder_workload(
        status.holder_spawn_host.as_deref(),
        status.target.as_deref(),
        worktree,
    );
    if let Some(activity) = holder_activity::observe(pid, &workload, status.acquired_at_ms) {
        status.holder_held_ms = Some(activity.held_ms);
        status.holder_cpu_percent = Some(activity.cpu_percent);
        status.holder_state = Some(activity.state().to_string());
        status.holder_state_detail = Some(activity.describe());
    }
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
            holder_nice: status.holder_nice,
            holder_spawn_host: status.holder_spawn_host,
            remaining_batches: status.remaining_batches,
            estimated_remaining_ms: status.estimated_remaining_ms,
            holder_held_ms: None,
            holder_cpu_percent: None,
            holder_state: None,
            holder_state_detail: None,
            holder_alive: status.holder_alive,
            holder_job_status: status.holder_job_status.map(|job| job.as_str().to_string()),
            holder_stale: status.holder_stale,
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

/// The verification lane's lease. While the lane is free, a canonical
/// verification that a pre-upgrade binary still runs on the model lane is
/// reported in its place (Issue #4285 transition); index jobs and searches
/// on the model lane are never verification holders.
fn status() -> Result<LeaseStatusSnapshot, SpecOpsError> {
    let status = open_coordinator()?
        .heavy_lease_status()
        .map_err(|err| unexpected(format!("failed to read the verification lease: {err}")))?;
    if status.held || status.pending > 0 {
        return Ok(status.into());
    }
    let legacy = IndexCoordinator::open_default()
        .and_then(|coordinator| coordinator.heavy_lease_status())
        .ok()
        .filter(|legacy| legacy.held && legacy.holder_kind == Some(HeavyHolderKind::Verification));
    Ok(legacy.unwrap_or(status).into())
}

pub(super) fn open_coordinator() -> Result<IndexCoordinator, SpecOpsError> {
    IndexCoordinator::open_default_verification()
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
    if let Some(nice) = status.holder_nice {
        out.push_str(&format!("holder_nice: {nice}\n"));
    }
    if let Some(spawn_host) = &status.holder_spawn_host {
        out.push_str(&format!("holder_spawn_host: {spawn_host}\n"));
    }
    if let Some(alive) = status.holder_alive {
        out.push_str(&format!("holder_alive: {alive}\n"));
    }
    if let Some(job) = &status.holder_job_status {
        out.push_str(&format!("holder_job_status: {job}\n"));
    }
    if status.holder_stale {
        out.push_str("holder_stale: true\n");
    }
    if let Some(batches) = status.remaining_batches {
        out.push_str(&format!("remaining_batches: {batches}\n"));
    }
    if let Some(estimate) = status.estimated_remaining_ms {
        out.push_str(&format!("estimated_remaining_ms: {estimate}\n"));
    }
    if let Some(held) = status.holder_held_ms {
        out.push_str(&format!("holder_held_ms: {held}\n"));
    }
    if let Some(cpu) = status.holder_cpu_percent {
        out.push_str(&format!("holder_cpu_percent: {cpu:.1}\n"));
    }
    if let Some(state) = &status.holder_state {
        out.push_str(&format!("holder_state: {state}\n"));
        if let Some(detail) = &status.holder_state_detail {
            out.push_str(&format!("holder_state_detail: {detail}\n"));
        }
        // Issue #4561 AC-4: the value is one sampled reading, and the reader
        // is usually deciding whether to interrupt someone. Say what it does
        // and does not license, next to the value itself.
        out.push_str(&format!("holder_state_advice: {HOLDER_STATE_ADVICE}\n"));
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

/// Issue #4633 AC-3: a live canonical holder keeps its lease. The wording
/// predates #4633 and is kept, so the refusal reads the same as it always has.
fn canonical_refusal(lease_id: &str, holder: Option<&str>) -> SpecOpsError {
    let holder = holder
        .map(|detail| format!(" Holder: {detail}."))
        .unwrap_or_default();
    unexpected(format!(
        "verification lease {lease_id} is held without a legacy control channel. \
         Canonical leases are owned by `verify.run` and release when the runner finishes; \
         they cannot be released or extended through the manual API — only a holder whose \
         parent exited with nothing left to run can be reclaimed.{holder} \
         Check `verify.lease.status` for the current holder."
    ))
}

fn missing_lease(lease_id: &str) -> SpecOpsError {
    let held = status()
        .ok()
        .filter(|status| status.held && status.lease_id.as_deref() == Some(lease_id));
    match held {
        Some(_) => canonical_refusal(lease_id, None),
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
    fn command_strings(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|part| part.to_string()).collect()
    }

    /// Issue #4196 AC-1 / AC-3: what a requested command weighs follows the
    /// scope it will actually build, not merely the fact that it says
    /// `cargo test`. AC-3 pins the two ends down: a workspace-wide run is
    /// heavy and a single named test target is light.
    #[test]
    fn classify_command_reads_the_scope_of_a_cargo_run() {
        // AC-3: the two cases the Issue fixes by name.
        assert_eq!(
            classify_command("cargo test --workspace --all-features"),
            CommandWeight::Heavy
        );
        assert_eq!(
            classify_command("cargo test -p gwt --test verification_lease"),
            CommandWeight::Light
        );

        // A widening flag wins wherever it sits on the line.
        assert_eq!(
            classify_command("cargo test -p gwt --lib --all-features"),
            CommandWeight::Heavy
        );
        assert_eq!(
            classify_command("cargo test --all-targets --test admission"),
            CommandWeight::Heavy
        );
        assert_eq!(
            classify_command("cargo test --workspace --exclude gwt --lib"),
            CommandWeight::Heavy
        );
        // Global option values and Cargo's glob/attached selector syntax must
        // not let a broad run masquerade as one package and one target.
        for command in [
            "cargo --config net.offline=true test --workspace --all-features",
            "cargo test -p 'gwt-*' --lib",
            "cargo test -p gwt --test '*'",
            "cargo test -pgwt -pgwt-core --test admission",
            "cargo test -p gwt --test admission --test verification_lease",
            "cargo test -p gwt --lib --test admission",
            "cargo test -p gwt --test admission --bench benchmark",
        ] {
            assert_eq!(classify_command(command), CommandWeight::Heavy, "{command}");
        }
        assert_eq!(
            classify_command("cargo test -pgwt --lib"),
            CommandWeight::Light
        );

        // Nothing narrows these: they build every target of the selected
        // packages, which is the run the lease exists for.
        assert_eq!(classify_command("cargo test"), CommandWeight::Heavy);
        assert_eq!(
            classify_command("cargo test -p gwt-core"),
            CommandWeight::Heavy
        );
        assert_eq!(
            classify_command("cargo test -p gwt -p gwt-core --lib"),
            CommandWeight::Heavy,
            "several packages is not one narrow target"
        );
        // `--lib` names a target *per package*, and this is a virtual
        // workspace with `default-members`: with no package selected it builds
        // every member's lib, so it must not read as narrow.
        assert_eq!(classify_command("cargo test --lib"), CommandWeight::Heavy);
        assert_eq!(
            classify_command("cargo test -p gwt --lib"),
            CommandWeight::Light
        );

        // Cargo's own arguments end at `--`; the rest is the test binary's
        // filter and says nothing about what cargo builds.
        assert_eq!(
            classify_command("cargo test -p gwt --lib -- --all-features"),
            CommandWeight::Light
        );
        assert_eq!(
            classify_command("cargo +nightly test -p gwt --lib"),
            CommandWeight::Light
        );

        // Other cargo subcommands keep the weight they already had.
        assert_eq!(
            classify_command("cargo clippy --all-targets --all-features"),
            CommandWeight::Heavy
        );
        assert_eq!(classify_command("cargo fmt --check"), CommandWeight::Light);
        assert_eq!(classify_command("cargo metadata"), CommandWeight::Light);

        // Anything this module cannot bound stays heavy: guessing light for an
        // unrecognized program would trade one window's wait for the host-wide
        // oversubscription the lease was built to prevent.
        assert_eq!(
            classify_command("npx playwright test --headed"),
            CommandWeight::Heavy
        );
        assert_eq!(classify_command("cargo"), CommandWeight::Heavy);
        assert_eq!(
            classify_command("cargo test 'unbalanced"),
            CommandWeight::Heavy
        );
    }

    /// Issue #4196 AC-2 / AC-4: a matrix is only as light as its heaviest
    /// command, and the caller can name the one that forces the wait instead
    /// of leaving the agent to discover it by idling.
    #[test]
    fn first_heavy_command_names_what_forces_the_host_lease() {
        let light = command_strings(&["cargo fmt --check", "cargo test -p gwt --test admission"]);
        assert_eq!(first_heavy_command(&light), None);
        assert_eq!(first_heavy_command(&[]), None);

        let mixed = command_strings(&["cargo test -p gwt --lib", "cargo test --workspace"]);
        assert_eq!(
            first_heavy_command(&mixed).map(String::as_str),
            Some("cargo test --workspace")
        );
    }

    use super::*;
    use gwt_core::index_coordinator::JobPriority;

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
                holder_nice: Some(10),
                holder_spawn_host: Some("daemon".to_string()),
                remaining_batches: None,
                estimated_remaining_ms: Some(60_000),
                holder_held_ms: None,
                holder_cpu_percent: None,
                holder_state: None,
                holder_state_detail: None,
                holder_alive: Some(true),
                holder_job_status: Some("running".to_string()),
                holder_stale: false,
            },
        );
        // Issue #4169 AC-2: the waiters are named in service order, each with
        // the moment it joined and how long it has been waiting. Issue #4409
        // AC-4 adds the holder's own priority and where it launches from, so a
        // waiter can tell a slow holder from a starved one.
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
             holder_nice: 10\n\
             holder_spawn_host: daemon\n\
             holder_alive: true\n\
             holder_job_status: running\n\
             estimated_remaining_ms: 60000\n\
             pending: 2\n\
             queue[0]: target=repo--verification--early priority=manual-rebuild \
             queued_at_ms=500 waiting_ms=90000\n\
             queue[1]: target=repo--verification--late priority=manual-rebuild \
             queued_at_ms=900 waiting_ms=89600\n"
        );
    }

    /// Issue #4561 AC-4: `holder_state` on its own was read as a decision —
    /// three live holders were reported stopped and one was asked to abort.
    /// The value now travels with what it was measured from and with what it
    /// does not license.
    #[test]
    fn holder_state_is_rendered_with_its_basis_and_its_limits() {
        let mut out = String::new();
        render(
            &mut out,
            "held",
            "free",
            &LeaseStatusSnapshot {
                held: true,
                owner_pid: Some(12121),
                holder_spawn_host: Some("daemon".to_string()),
                holder_held_ms: Some(499_762),
                holder_cpu_percent: Some(0.0),
                holder_state: Some("unknown".to_string()),
                holder_state_detail: Some("holder state unknown: held 8m19s".to_string()),
                ..LeaseStatusSnapshot::default()
            },
        );

        assert!(out.contains("holder_state: unknown\n"), "{out}");
        assert!(
            out.contains("holder_state_detail: holder state unknown: held 8m19s\n"),
            "{out}"
        );
        assert!(out.contains("holder_state_advice: "), "{out}");
        assert!(out.contains("not a verdict"), "{out}");
        assert!(out.contains("ps -eo pid,ppid,time,command"), "{out}");
    }

    /// Issue #4352 AC-2: the retired manual acquire never reserves a lease
    /// and tells the caller that bootstrap builds run without one.
    #[test]
    fn manual_acquire_is_retired_and_exempts_bootstrap_builds() {
        let worktree = tempfile::tempdir().unwrap();
        let mut env = crate::cli::TestEnv::new(worktree.path().to_path_buf());
        let mut out = String::new();
        let err = run(
            &mut env,
            VerificationLeaseCommand::Acquire {
                ttl_minutes: DEFAULT_TTL_MINUTES,
                reason: Some("cargo build -p gwt --bin gwtd".to_string()),
            },
            &mut out,
        )
        .expect_err("manual acquire must be refused");
        let message = err.to_string();
        assert!(
            message.contains("bootstrap builds directly without a lease"),
            "retired acquire must name bootstrap builds as lease-free: {message}"
        );
        assert!(
            message.contains("use `verify.run` for canonical verification"),
            "retired acquire must route to verify.run: {message}"
        );
        assert!(
            out.is_empty(),
            "a refused acquire must not render a lease: {out}"
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

    /// Issue #4633 AC-4: closing a pane whose worktree holds the lease tells
    /// the caller, and names the way out; any other pane says nothing.
    #[test]
    fn closing_a_pane_names_the_lease_its_worktree_holds() {
        let status = HeavyLeaseStatus {
            held: true,
            lease_id: Some("ff298133".to_string()),
            target: Some("repo--verification--wt".to_string()),
            owner: Some(gwt_core::index_coordinator::OwnerIdentity {
                pid: 70012,
                start_id: "start".to_string(),
            }),
            ..HeavyLeaseStatus::default()
        };

        let note = lease_note_for_target("repo--verification--wt", &status)
            .expect("the closing pane's worktree holds the lease");
        assert!(note.contains("ff298133"), "{note}");
        assert!(note.contains("holder pid 70012"), "{note}");
        assert!(note.contains("verify.lease.release"), "{note}");
        assert!(note.contains("orphaned"), "{note}");

        assert_eq!(
            lease_note_for_target("repo--verification--other", &status),
            None
        );
        let free = HeavyLeaseStatus {
            held: false,
            ..status
        };
        assert_eq!(lease_note_for_target("repo--verification--wt", &free), None);
    }

    mod orphan_reclaim {
        use super::super::*;
        use gwt_core::index_coordinator::{
            HeavyLease, JobAdmission, JobPriority, LeaseEventKind, TargetKey,
        };
        use gwt_core::test_support::ScopedGwtHome;
        use holder_activity::HolderActivity;

        /// A canonical lease held by this test process, as `verify.run`
        /// holds one, in a private GWT home.
        struct HeldLease {
            lease: Option<HeavyLease>,
            lease_id: String,
            _guard: gwt_core::index_coordinator::TargetJobGuard,
            _home_guard: ScopedGwtHome,
            _home: tempfile::TempDir,
        }

        fn hold_lease() -> HeldLease {
            let home = tempfile::tempdir().unwrap();
            let home_guard = ScopedGwtHome::set(home.path());
            let coordinator = open_coordinator().unwrap();
            let key = TargetKey::verification("99a8660247f5bc49", "8cf366e54f228831");
            let JobAdmission::Owner(guard) = coordinator
                .request_job(&key, JobPriority::ManualRebuild, Duration::from_secs(1))
                .unwrap()
            else {
                panic!("a private lease root must admit the owner");
            };
            let lease = guard
                .acquire_heavy_with_ttl(Duration::from_secs(1), Duration::from_secs(2_700))
                .unwrap();
            let lease_id = lease.id().to_string();
            HeldLease {
                lease: Some(lease),
                lease_id,
                _guard: guard,
                _home_guard: home_guard,
                _home: home,
            }
        }

        fn reading(parent_gone: bool, workload_processes: usize) -> HolderActivity {
            HolderActivity {
                held_ms: 1_480_094,
                cpu_percent: 0.0,
                cpu_gained_ms: 0,
                turnover: false,
                processes: 1,
                window_ms: 1_200,
                host_cpu_percent: Some(95.0),
                delegated: true,
                parent_gone,
                workload_processes,
            }
        }

        fn orphan() -> HolderActivity {
            reading(true, 0)
        }

        /// Scripted readings; "ending" the holder drops the lease, the way
        /// the kernel releases it when the holder process exits.
        struct ScriptedReclaimer<'a> {
            readings: Vec<Option<HolderActivity>>,
            held: &'a mut HeldLease,
            terminated: Vec<(u32, bool)>,
        }

        impl OrphanReclaimer for ScriptedReclaimer<'_> {
            fn observe(
                &mut self,
                _pid: u32,
                _status: &HeavyLeaseStatus,
                _pause: Duration,
            ) -> Option<HolderActivity> {
                self.readings.remove(0)
            }

            fn terminate(&mut self, pid: u32, force: bool) -> Result<(), String> {
                self.terminated.push((pid, force));
                self.held.lease.take();
                Ok(())
            }
        }

        /// Reads the real process table but refuses to end anything: the
        /// holder here is this test process.
        struct NeverTerminates(SystemReclaimer);

        impl OrphanReclaimer for NeverTerminates {
            fn observe(
                &mut self,
                pid: u32,
                status: &HeavyLeaseStatus,
                pause: Duration,
            ) -> Option<HolderActivity> {
                self.0.observe(pid, status, pause)
            }

            fn terminate(&mut self, pid: u32, _force: bool) -> Result<(), String> {
                panic!("a live holder (pid {pid}) must never be ended");
            }
        }

        fn still_held(lease_id: &str) -> bool {
            open_coordinator()
                .unwrap()
                .heavy_lease_status()
                .unwrap()
                .lease_id
                .as_deref()
                == Some(lease_id)
        }

        /// Issue #4633 AC-3 / AC-6 (a): a live `verify.run` holder — this
        /// process, whose parent is alive — keeps its lease, and the refusal
        /// is the one it has always been.
        #[test]
        fn a_live_canonical_holder_keeps_its_lease() {
            let held = hold_lease();
            let worktree = tempfile::tempdir().unwrap();
            let mut reclaimer = NeverTerminates(SystemReclaimer::new(worktree.path()));
            let mut out = String::new();

            let err = release(
                &held.lease_id,
                Some("try to take a live lease"),
                &mut reclaimer,
                &mut out,
            )
            .unwrap_err()
            .to_string();

            assert!(
                err.contains("Canonical leases are owned by `verify.run`"),
                "{err}"
            );
            assert!(still_held(&held.lease_id), "{err}");
            assert!(out.is_empty(), "{out}");
        }

        /// Issue #4633 AC-2 / AC-6 (b): an orphan confirmed twice is ended,
        /// the lease frees without waiting for its TTL, and the ledger says
        /// who reclaimed it and why.
        #[test]
        fn a_confirmed_orphan_is_reclaimed_and_recorded() {
            let mut held = hold_lease();
            let lease_id = held.lease_id.clone();
            let mut reclaimer = ScriptedReclaimer {
                readings: vec![Some(orphan()), Some(orphan())],
                held: &mut held,
                terminated: Vec::new(),
            };
            let mut out = String::new();

            release(
                &lease_id,
                Some("window closed after the provider limit"),
                &mut reclaimer,
                &mut out,
            )
            .unwrap();

            assert_eq!(
                reclaimer.terminated,
                vec![(std::process::id(), false)],
                "{out}"
            );
            assert!(out.contains("verification lease: reclaimed\n"), "{out}");
            assert!(out.contains("reason: window closed"), "{out}");
            assert!(!still_held(&lease_id), "{out}");
            let events = open_coordinator().unwrap().lease_events().unwrap();
            let event = events.last().expect("the reclaim is recorded");
            assert_eq!(event.kind, LeaseEventKind::Reclaimed, "{events:?}");
            assert_eq!(event.lease_id, lease_id);
            let reason = event.reason.as_deref().unwrap_or_default();
            assert!(reason.contains("reclaimed by pid"), "{reason}");
            assert!(reason.contains("window closed"), "{reason}");
        }

        /// Ending a process is recorded, so it cannot be done anonymously.
        #[test]
        fn reclaiming_an_orphan_needs_a_reason() {
            let mut held = hold_lease();
            let lease_id = held.lease_id.clone();
            let mut reclaimer = ScriptedReclaimer {
                readings: vec![Some(orphan())],
                held: &mut held,
                terminated: Vec::new(),
            };

            let err = release(&lease_id, Some("  "), &mut reclaimer, &mut String::new())
                .unwrap_err()
                .to_string();

            assert!(err.contains("needs a reason"), "{err}");
            assert!(reclaimer.terminated.is_empty(), "{err}");
            assert!(still_held(&lease_id), "{err}");
        }

        /// One reading is not a verdict: a holder that shows work again
        /// before the confirmation keeps its lease.
        #[test]
        fn an_orphan_the_confirmation_does_not_repeat_keeps_its_lease() {
            let mut held = hold_lease();
            let lease_id = held.lease_id.clone();
            let mut reclaimer = ScriptedReclaimer {
                readings: vec![Some(orphan()), Some(reading(true, 1))],
                held: &mut held,
                terminated: Vec::new(),
            };

            let err = release(
                &lease_id,
                Some("looked idle"),
                &mut reclaimer,
                &mut String::new(),
            )
            .unwrap_err()
            .to_string();

            assert!(err.contains("did not confirm"), "{err}");
            assert!(reclaimer.terminated.is_empty(), "{err}");
            assert!(still_held(&lease_id), "{err}");
        }

        /// Issue #4633 AC-6 (c): a holder whose process is gone already
        /// released the lease in the kernel; there is nothing to end.
        #[test]
        fn a_vanished_holder_has_nothing_to_reclaim() {
            let mut held = hold_lease();
            let lease_id = held.lease_id.clone();
            held.lease.take();
            let worktree = tempfile::tempdir().unwrap();
            let mut reclaimer = NeverTerminates(SystemReclaimer::new(worktree.path()));

            let err = release(&lease_id, Some("gone"), &mut reclaimer, &mut String::new())
                .unwrap_err()
                .to_string();

            assert!(err.contains("no live verification lease"), "{err}");
        }
    }
}
