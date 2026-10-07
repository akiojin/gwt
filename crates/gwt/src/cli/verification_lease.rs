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
    HeavyQueueEntry, HeavySlotStatus, IndexCoordinator, TargetKey,
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
mod renewal;
pub(crate) use renewal::CommandProgress;

const CARGO_SCOPED_SUBCOMMANDS: &[&str] = &["test", "t", "nextest"];
/// Flags that widen a `cargo test` past a single target, wherever they sit.
const CARGO_SCOPE_WIDENING_FLAGS: &[&str] = &[
    "--workspace",
    "--all",
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
    /// Requires a host verification slot; unbounded resources stay exclusive.
    Heavy,
}

/// Classify one command `verify.run` was asked to execute (Issue #4196).
///
/// The host lease exists to stop several worktrees compiling the world at
/// once, and every `cargo test` used to claim it whatever its scope: a single
/// `--test <name>` queued behind `cargo test --workspace --all-features`, and
/// the fleet's verification throughput was pinned at one window at a time. So
/// the weight follows the scope the command will actually build — a widening
/// flag is heavy, and a run narrowed to a named target or a filtered library
/// of one package is light. Enabling features does not widen that selection.
///
/// Anything this module cannot bound stays heavy. An unrecognized program may
/// compile the world, and guessing light for it would trade one window's wait
/// for the host-wide oversubscription the lease was built to prevent
/// (Issue #3913).
pub(crate) fn classify_command(command: &str) -> CommandWeight {
    let Ok(args) = crate::cli::verification_record::split_command_line(command)
        .and_then(crate::cli::verification_record::take_env_assignments)
        .map(|(_, args)| args)
    else {
        return CommandWeight::Heavy;
    };
    let Some(program) = args.first() else {
        return CommandWeight::Heavy;
    };
    if is_short_non_cargo_gate(&args) {
        return CommandWeight::Light;
    }
    let program = Path::new(program)
        .file_stem()
        .and_then(|stem| stem.to_str());
    let lint_program = if matches!(program, Some("bunx" | "npx")) {
        let package_index = match (program, args.get(1).map(String::as_str)) {
            (Some("bunx"), Some("--bun")) | (Some("npx"), Some("--yes")) => 2,
            _ => 1,
        };
        args.get(package_index).map(String::as_str)
    } else {
        program
    };
    if matches!(
        lint_program,
        Some("markdownlint" | "markdownlint-cli" | "markdownlint-cli2")
    ) {
        return CommandWeight::Light;
    }
    if program != Some("cargo") {
        return CommandWeight::Heavy;
    }
    // Cargo's own arguments end at a bare `--`; everything after it is the
    // test binary's filter. Keep that filter for library-run classification.
    let test_filter = args
        .iter()
        .position(|arg| arg == "--")
        .map_or(Some(false), |separator| {
            has_libtest_filter(&args[separator + 1..])
        });
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
    if subcommand == "doc" && args.clone().any(|arg| arg == "--no-deps") {
        return CommandWeight::Light;
    }
    if !CARGO_SCOPED_SUBCOMMANDS.contains(&subcommand) {
        return CommandWeight::Heavy;
    }
    if subcommand == "nextest" && args.next() != Some("run") {
        return CommandWeight::Heavy;
    }
    let Some(test_filter) = test_filter else {
        return CommandWeight::Heavy;
    };
    classify_cargo_scope(&args.collect::<Vec<_>>(), test_filter)
}

/// Issue #5086: the explicit short-gate allowlist also owns its execution bound.
/// Arbitrary interpreters/wrappers and coverage producers remain Heavy.
pub(crate) fn short_non_cargo_timeout(command: &str) -> Option<Duration> {
    let (_, args) = crate::cli::verification_record::split_command_line(command)
        .and_then(crate::cli::verification_record::take_env_assignments)
        .ok()?;
    is_short_non_cargo_gate(&args).then_some(Duration::from_secs(60))
}

fn is_short_non_cargo_gate(args: &[String]) -> bool {
    let program = args
        .first()
        .and_then(|program| Path::new(program).file_name())
        .and_then(|name| name.to_str())
        .map(|name| name.strip_suffix(".exe").unwrap_or(name));
    let subcommand = args.get(1).map(String::as_str);
    let checks = |allowed_flags: &[&str]| {
        let mut has_check = false;
        for arg in args.iter().skip(2).take_while(|arg| arg.as_str() != "--") {
            if arg == "--check" {
                has_check = true;
            } else if arg.starts_with('-') && !allowed_flags.contains(&arg.as_str()) {
                // An unknown option may consume --check as its value.
                return false;
            }
        }
        has_check
    };
    match program {
        // Static file analysis; none of these programs builds or runs a suite.
        Some("actionlint" | "shellcheck" | "yamllint" | "typos") => true,
        Some("git") => {
            subcommand == Some("diff")
                && checks(&["--cached", "--staged", "--no-ext-diff", "--no-textconv"])
        }
        Some("taplo") => subcommand == Some("check") || (subcommand == Some("fmt") && checks(&[])),
        // This reader consumes an existing JSON, unlike coverage-summary.mjs
        // which invokes llvm-cov. Do not classify node scripts by basename.
        Some("node") => subcommand.is_some_and(|script| {
            script.replace('\\', "/").trim_start_matches("./")
                == "scripts/check-coverage-threshold.mjs"
        }),
        _ => false,
    }
}

/// None means unbounded or unknown, not merely the absence of a filter.
/// Libtest ORs filters, so an empty filter wins over every non-empty filter.
/// Option values such as `--skip some_test` are not positive filters.
fn has_libtest_filter(args: &[String]) -> Option<bool> {
    let mut has_filter = false;
    let mut args = args.iter().map(String::as_str);
    while let Some(arg) = args.next() {
        if arg.is_empty() {
            return None;
        }
        if !arg.starts_with('-') {
            has_filter = true;
            continue;
        }
        let (flag, inline_value) = arg
            .split_once('=')
            .map_or((arg, None), |(flag, value)| (flag, Some(value)));
        if matches!(
            flag,
            "--skip" | "--test-threads" | "--format" | "--logfile" | "--color"
        ) {
            if inline_value.is_none() {
                args.next()?;
            }
        } else if inline_value.is_some()
            || !matches!(
                flag,
                "--exact"
                    | "--ignored"
                    | "--include-ignored"
                    | "--nocapture"
                    | "--show-output"
                    | "--quiet"
                    | "-q"
            )
        {
            return None;
        }
    }
    Some(has_filter)
}

/// Weigh a scoped `cargo test` by the selection it builds.
///
/// `--lib` is deliberately not enough on its own. This repository is a virtual
/// workspace with `default-members`, so `cargo test --lib` with no package
/// selects the lib target of *every* default member — the workspace-wide build
/// this classification exists to catch, wearing a narrowing flag.
fn classify_cargo_scope(cargo_args: &[&str], mut test_filter: bool) -> CommandWeight {
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
            continue;
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
            continue;
        }
        if matches!(
            flag,
            "--features"
                | "-F"
                | "--jobs"
                | "-j"
                | "--target"
                | "--manifest-path"
                | "--target-dir"
                | "--profile"
                | "--config"
                | "--color"
                | "--message-format"
        ) {
            if inline_value.or_else(|| args.next()).is_none() {
                return CommandWeight::Heavy;
            }
        } else if !arg.is_empty() && !arg.starts_with('-') {
            test_filter = true;
        } else if !matches!(
            flag,
            "--all-features"
                | "--no-default-features"
                | "--no-run"
                | "--no-fail-fast"
                | "--release"
                | "-r"
                | "--locked"
                | "--offline"
                | "--frozen"
                | "--quiet"
                | "-q"
                | "--verbose"
                | "-v"
                | "--keep-going"
                | "--future-incompat-report"
        ) {
            return CommandWeight::Heavy;
        }
    }
    if packages > 1 || named_targets + usize::from(lib_target) > 1 {
        return CommandWeight::Heavy;
    }
    if named_targets == 1 || (lib_target && packages == 1 && test_filter) {
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
            render(
                out,
                "held",
                "free",
                &status,
                project_scope_hash(&worktree).as_str(),
            );
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
                project_scope_hash(&worktree).as_str(),
                &lease_id,
                reason.as_deref(),
                &mut SystemReclaimer::new(&worktree),
                out,
            )
        }
    }
}

fn release(
    current_project: &str,
    lease_id: &str,
    reason: Option<&str>,
    reclaimer: &mut dyn OrphanReclaimer,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let snapshot = status_for_lease(lease_id)?.ok_or_else(|| missing_lease(lease_id))?;
    let relation = holder_project_relation(snapshot.target.as_deref(), current_project);
    if relation != HolderProjectRelation::SameProject {
        return Err(unexpected(format!(
            "verification lease {lease_id}: {}. You must not reclaim or stop this holder. \
             Wait for it to finish; only the owning project may request canonical release. \
             Do not bypass this refusal with kill or pkill.",
            relation.reason()
        )));
    }
    let Some(control) = control_dir_for(lease_id) else {
        return reclaim_holder(current_project, lease_id, reason, reclaimer, out);
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
    push_status_fields(out, &status()?, current_project);
    Ok(0)
}

/// How long a holder that looked reclaimable is watched again
/// before it is ended. The first reading spans [`PROGRESS_WINDOW`]; this one
/// has to show the same — nothing of its own running, no CPU
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
        #[cfg(windows)]
        {
            // Hidden console children have no cooperative signal channel.
            // Match daemon_supervisor's Windows termination, without /T:
            // only the twice-observed holder is authorized for reclamation.
            let _ = force;
            let output = gwt_core::process::hidden_command("taskkill")
                .args(["/PID", &pid.to_string(), "/F"])
                .output()
                .map_err(|error| format!("failed to terminate holder pid {pid}: {error}"))?;
            if output.status.success() || !crate::process::is_host_process_alive(pid) {
                Ok(())
            } else {
                Err(format!(
                    "failed to terminate holder pid {pid}: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ))
            }
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (pid, force);
            Err("reclaiming a verification holder is unsupported on this platform".to_string())
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

/// Free a canonical lease whose holder is orphaned or stalled without work.
///
/// A canonical lease is a kernel lock inside its `verify.run`, so the only way
/// to free it from outside is to end that process. An orphan, or an old
/// in-tree driver with no remaining workload (#4746), must be confirmed by
/// two readings [`ORPHAN_CONFIRM_WINDOW`] apart. Every other
/// holder keeps the protection it had — the manual API cannot touch it.
fn reclaim_holder(
    current_project: &str,
    lease_id: &str,
    reason: Option<&str>,
    reclaimer: &mut dyn OrphanReclaimer,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let coordinator = open_coordinator()?;
    let held =
        |coordinator: &IndexCoordinator| holder_for_lease(coordinator, lease_id).ok().flatten();
    let Some(status) = held(&coordinator) else {
        return Err(missing_lease(lease_id));
    };
    let Some(owner) = status.owner.clone() else {
        return Err(canonical_refusal(lease_id, None));
    };
    protect_renewed_holder(&status, lease_id)?;
    let first = reclaimer.observe(owner.pid, &status, Duration::ZERO);
    let Some(first) = first.filter(holder_activity::HolderActivity::reclaimable) else {
        return Err(canonical_refusal(
            lease_id,
            first.map(|activity| activity.describe()).as_deref(),
        ));
    };
    let Some(reason) = reason.map(str::trim).filter(|reason| !reason.is_empty()) else {
        return Err(unexpected(format!(
            "verification lease {lease_id} is held by a reclaimable holder ({}). Reclaiming it \
             ends pid {} and is recorded in the lease ledger, so it needs a reason: pass \
             `params.reason`.",
            first.describe(),
            owner.pid
        )));
    };
    let confirmed = reclaimer.observe(owner.pid, &status, ORPHAN_CONFIRM_WINDOW);
    let Some(confirmed) = confirmed.filter(holder_activity::HolderActivity::reclaimable) else {
        return Err(canonical_refusal(
            lease_id,
            Some(&format!(
                "the holder looked reclaimable, but a second reading {}s later did not confirm it: {}",
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
    let current = held(&coordinator).ok_or_else(|| missing_lease(lease_id))?;
    if current.owner != Some(owner.clone()) {
        return Err(missing_lease(lease_id));
    }
    protect_renewed_holder(&current, lease_id)?;
    reclaimer.terminate(owner.pid, false).map_err(unexpected)?;
    if await_settled(lease_id).is_err() {
        reclaimer.terminate(owner.pid, true).map_err(unexpected)?;
        await_settled(lease_id)?;
    }
    let target = status.target.as_deref().unwrap_or("unknown");
    let record = format!(
        "{} holder pid {} reclaimed by pid {}: {reason}",
        confirmed.state(),
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
    push_status_fields(out, &self::status()?, current_project);
    Ok(0)
}

/// Renewal observes the exact command tree; a reclaimer's process sample can
/// miss delegated work. Keep a renewed lease protected until that TTL expires.
/// Expiry alone still does not authorize reclaiming it.
fn protect_renewed_holder(status: &HeavyLeaseStatus, lease_id: &str) -> Result<(), SpecOpsError> {
    // The diagnostic event ledger is best-effort. Only the atomically
    // published ticket can establish that a valid TTL has not been renewed.
    if status.expires_at_ms.is_some() && !status.expired && status.ttl_renewed != Some(false) {
        return Err(canonical_refusal(
            lease_id,
            Some("the holder renewed its active TTL or its renewal state is unknown; wait for its verification to finish"),
        ));
    }
    Ok(())
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
    open_coordinator()
        .ok()?
        .heavy_pool_status()
        .ok()?
        .slots
        .iter()
        .find_map(|slot| lease_note_for_target(&target, &slot.status))
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
        if status_for_lease(lease_id)?.is_none() {
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

/// Locate the control directory of a live legacy lease. Only a *granted*
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
                                   left running; `reclaimable` means an old in-tree driver has \
                                   no remaining workload. `ps -eo pid,ppid,time,command` is diagnostic \
                                   only; neither it nor repeated sampling grants permission to \
                                   stop a holder. Follow holder_intervention; never bypass a \
                                   canonical release refusal with kill or pkill.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HolderProjectRelation {
    SameProject,
    OtherProject,
    Unknown,
}

impl HolderProjectRelation {
    fn as_str(self) -> &'static str {
        match self {
            Self::SameProject => "same_project",
            Self::OtherProject => "other_project",
            Self::Unknown => "unknown",
        }
    }

    fn reason(self) -> &'static str {
        match self {
            Self::SameProject => "held by this project",
            Self::OtherProject => "held by another project",
            Self::Unknown => "the holder's project could not be established",
        }
    }
}

fn holder_project_relation(target: Option<&str>, current_project: &str) -> HolderProjectRelation {
    let Some((project, worktree)) = target.and_then(|target| target.split_once("--verification--"))
    else {
        return HolderProjectRelation::Unknown;
    };
    if project.is_empty() || worktree.is_empty() || current_project.is_empty() {
        HolderProjectRelation::Unknown
    } else if project == current_project {
        HolderProjectRelation::SameProject
    } else {
        HolderProjectRelation::OtherProject
    }
}

/// Status rendering and the pre-upgrade detached holder wire format.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct LeaseStatusSnapshot {
    held: bool,
    /// Pool diagnostics are additive output, not part of legacy control outcomes.
    #[serde(skip)]
    capacity: Option<usize>,
    #[serde(skip)]
    running: usize,
    #[serde(skip)]
    available: usize,
    #[serde(skip)]
    slots: Vec<HeavySlotStatus>,
    /// Derived from the live lease's control channel, never trusted from a
    /// saved pre-upgrade outcome.
    #[serde(skip)]
    legacy_release_available: bool,
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
            legacy_release_available: false,
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
            ..Self::default()
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
    let pool = open_coordinator()?
        .heavy_pool_status()
        .map_err(|err| unexpected(format!("failed to read the verification lease: {err}")))?;
    let status = pool
        .slots
        .iter()
        .find(|slot| slot.status.held)
        .or_else(|| pool.slots.iter().find(|slot| slot.status.holder_stale))
        .map(|slot| slot.status.clone())
        .unwrap_or_default();
    let legacy = if pool.used == 0 && pool.queue.is_empty() {
        IndexCoordinator::open_default()
            .and_then(|coordinator| coordinator.heavy_lease_status())
            .ok()
            .filter(|legacy| {
                legacy.held && legacy.holder_kind == Some(HeavyHolderKind::Verification)
            })
    } else {
        None
    };
    let mut snapshot = LeaseStatusSnapshot::from(legacy.clone().unwrap_or(status));
    snapshot.capacity = Some(pool.capacity);
    snapshot.running = pool.used.max(usize::from(legacy.is_some()));
    snapshot.available = if legacy.is_some() { 0 } else { pool.available };
    snapshot.slots = pool.slots;
    if let Some(status) = legacy {
        snapshot.slots.push(HeavySlotStatus { slot: None, status });
    }
    snapshot.pending = pool.queue.len();
    snapshot.queue = pool.queue;
    snapshot.legacy_release_available = snapshot.held
        && snapshot
            .lease_id
            .as_deref()
            .and_then(control_dir_for)
            .is_some();
    Ok(snapshot)
}

/// A representative holder is unsuitable for control and renewal: another
/// slot can remain active after that representative changes or finishes.
pub(super) fn holder_for_lease(
    coordinator: &IndexCoordinator,
    lease_id: &str,
) -> Result<Option<HeavyLeaseStatus>, gwt_core::index_coordinator::CoordinatorError> {
    let pool = coordinator.heavy_pool_status()?;
    let mut status = pool
        .slots
        .into_iter()
        .map(|slot| slot.status)
        .find(|status| status.held && status.lease_id.as_deref() == Some(lease_id));
    if let Some(status) = &mut status {
        status.pending = pool.queue.len();
        status.queue = pool.queue;
    }
    Ok(status)
}

fn status_for_lease(lease_id: &str) -> Result<Option<LeaseStatusSnapshot>, SpecOpsError> {
    let status = holder_for_lease(&open_coordinator()?, lease_id)
        .map_err(|err| unexpected(format!("failed to read the verification lease: {err}")))?;
    let status = status.or_else(|| {
        IndexCoordinator::open_default()
            .and_then(|coordinator| holder_for_lease(&coordinator, lease_id))
            .ok()
            .flatten()
            .filter(|status| status.holder_kind == Some(HeavyHolderKind::Verification))
    });
    Ok(status.map(LeaseStatusSnapshot::from))
}

pub(super) fn open_coordinator() -> Result<IndexCoordinator, SpecOpsError> {
    let settings = gwt_config::Settings::load()
        .map_err(|err| unexpected(format!("verification configuration invalid: {err}")))?;
    let capacity = settings.verification.slots.map_or_else(
        || {
            let mut host = sysinfo::System::new();
            host.refresh_memory();
            automatic_slot_capacity(
                std::thread::available_parallelism().map_or(1, usize::from),
                host.total_memory(),
            )
        },
        |slots| usize::from(slots.get()),
    );
    IndexCoordinator::open_verification(verification_coordinator_root(), capacity)
        .map_err(|err| unexpected(format!("verification lease coordinator unavailable: {err}")))
}

fn automatic_slot_capacity(logical_cores: usize, memory_bytes: u64) -> usize {
    (logical_cores / 8)
        .min((memory_bytes / (16 * 1024 * 1024 * 1024)) as usize)
        .clamp(1, 4) // Largest all-PASS canonical capacity measured for #5082.
}

pub(super) fn cargo_subcommand(args: &[String]) -> Option<&str> {
    let mut globals = args.iter().skip(1).take_while(|arg| arg.as_str() != "--");
    loop {
        let argument = globals.next()?;
        if argument.starts_with('+')
            || matches!(
                argument.as_str(),
                "-v" | "--verbose" | "-q" | "--quiet" | "--offline" | "--locked" | "--frozen"
            )
        {
            continue;
        }
        if matches!(argument.as_str(), "--config" | "-Z") {
            globals.next();
            continue;
        }
        if argument.starts_with("--config=") {
            continue;
        }
        return Some(argument.as_str());
    }
}

pub(super) fn effective_cargo_target(
    worktree: &Path,
    command: &str,
    isolated_baseline: bool,
) -> Result<Option<PathBuf>, String> {
    let (assignments, args) = crate::cli::verification_record::take_env_assignments(
        crate::cli::verification_record::split_command_line(command)?,
    )?;
    if Path::new(&args[0]).file_stem().and_then(|s| s.to_str()) != Some("cargo") {
        return Ok(None);
    }
    let cargo_args: Vec<_> = args[1..]
        .iter()
        .take_while(|arg| arg.as_str() != "--")
        .collect();
    // Cargo plugins can choose their own build roots. Keep those commands
    // exclusive unless their artifact directory can be established.
    let Some(subcommand) = cargo_subcommand(&args) else {
        return Ok(None);
    };
    if !matches!(
        subcommand,
        "build"
            | "check"
            | "clippy"
            | "test"
            | "t"
            | "nextest"
            | "rustc"
            | "doc"
            | "bench"
            | "clean"
    ) {
        return Ok(None);
    }
    let mut metadata = gwt_core::process::hidden_command(&args[0]);
    if let Some(toolchain) = args.get(1).filter(|arg| arg.starts_with('+')) {
        metadata.arg(toolchain);
    }
    metadata
        .args([
            "metadata",
            "--offline",
            "--no-deps",
            "--format-version",
            "1",
        ])
        .current_dir(worktree);
    gwt_core::process::scrub_git_env(&mut metadata);
    for (key, value) in assignments {
        metadata.env(key, value);
    }
    if isolated_baseline {
        metadata.env_remove("CARGO_TARGET_DIR");
    }
    let mut index = 0;
    while index < cargo_args.len() {
        let argument = cargo_args[index];
        let (flag, inline) = argument
            .split_once('=')
            .map_or((argument.as_str(), None), |(flag, value)| {
                (flag, Some(value))
            });
        if matches!(flag, "--target-dir" | "--manifest-path" | "--config") {
            let value = match inline {
                Some(value) => value,
                None => {
                    index += 1;
                    cargo_args
                        .get(index)
                        .ok_or_else(|| format!("{flag} requires a value"))?
                        .as_str()
                }
            };
            if flag == "--target-dir" {
                metadata.env("CARGO_TARGET_DIR", value);
            } else {
                metadata.args([flag, value]);
            }
        }
        index += 1;
    }
    let output = metadata
        .output()
        .map_err(|error| format!("Cargo target resolution failed: {error}"))?;
    if !output.status.success() {
        // A wrapper or unresolved metadata is safe on the legacy exclusive
        // route; it must never silently receive a parallel slot.
        return Ok(None);
    }
    Ok(serde_json::from_slice::<serde_json::Value>(&output.stdout)
        .ok()
        .and_then(|metadata| metadata["target_directory"].as_str().map(PathBuf::from)))
}

pub(super) fn command_temporary_base(worktree: &Path, command: &str) -> Result<PathBuf, String> {
    let (assignments, _) = crate::cli::verification_record::take_env_assignments(
        crate::cli::verification_record::split_command_line(command)?,
    )?;
    let keys = if cfg!(windows) {
        ["TEMP", "TMP", "TMPDIR"]
    } else {
        ["TMPDIR", "TMP", "TEMP"]
    };
    let base = keys
        .iter()
        .find_map(|key| {
            assignments
                .iter()
                .rev()
                .find(|(name, value)| name == key && !value.is_empty())
                .map(|(_, value)| PathBuf::from(value))
        })
        .unwrap_or_else(std::env::temp_dir);
    Ok(if base.is_absolute() {
        base
    } else {
        worktree.join(base)
    })
}

// #5082: sampled target growth 4,650,508,061 + temporary growth 269,853,053
// bytes across the changed-surface matrix, with 20% headroom (rounded up).
const DEFAULT_VERIFICATION_DISK_BUDGET_BYTES: u64 = 5_904_433_337;

pub(super) fn command_disk_budgets(
    paths: &[PathBuf],
) -> Result<Vec<gwt_core::index_coordinator::VerificationDiskBudget>, String> {
    let settings = gwt_config::Settings::load().map_err(|error| error.to_string())?;
    let bytes = settings
        .verification
        .disk_budget_bytes
        .unwrap_or(DEFAULT_VERIFICATION_DISK_BUDGET_BYTES);
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let mut budgets: Vec<gwt_core::index_coordinator::VerificationDiskBudget> = Vec::new();
    for path in paths {
        let mut probe = path.clone();
        while !probe.exists() {
            if !probe.pop() {
                return Err(format!("cannot locate disk for {}", path.display()));
            }
        }
        let probe = dunce::canonicalize(probe).map_err(|error| error.to_string())?;
        let mount = disks
            .iter()
            .filter(|disk| probe.starts_with(disk.mount_point()))
            .max_by_key(|disk| disk.mount_point().components().count())
            .ok_or_else(|| {
                format!(
                    "cannot identify verification volume for {}",
                    probe.display()
                )
            })?;
        #[cfg(windows)]
        let volume = mount.mount_point().to_string_lossy().to_lowercase();
        #[cfg(unix)]
        let volume = {
            use std::os::unix::fs::MetadataExt;
            format!(
                "device:{}",
                fs::metadata(&probe)
                    .map_err(|error| error.to_string())?
                    .dev()
            )
        };
        #[cfg(not(any(windows, unix)))]
        let volume = mount.mount_point().to_string_lossy().into_owned();
        // Unix bind mounts may have different mount paths on the same device.
        let _ = mount;
        if budgets.iter().any(|budget| budget.volume == volume) {
            continue;
        }
        let total = fs2::total_space(&probe).map_err(|error| error.to_string())?;
        let floor = settings
            .build_artifact_gc
            .below_bytes
            .max(total.saturating_mul(settings.build_artifact_gc.below_percent) / 100);
        budgets.push(gwt_core::index_coordinator::VerificationDiskBudget {
            volume,
            path: probe,
            bytes,
            floor_bytes: floor,
        });
    }
    Ok(budgets)
}

/// The shared target-directory boundary for canonical verification and GC.
/// The lock lives outside the target, so deletion never removes its identity.
pub(crate) fn try_lock_build_artifacts(target: &Path) -> std::io::Result<Option<fs::File>> {
    let file = build_artifact_lock(target)?;
    match fs2::FileExt::try_lock_exclusive(&file) {
        Ok(()) => Ok(Some(file)),
        Err(error)
            if error.kind() == std::io::ErrorKind::WouldBlock
                || error.raw_os_error() == fs2::lock_contended_error().raw_os_error() =>
        {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

pub(super) fn lock_build_artifacts(target: &Path) -> std::io::Result<fs::File> {
    let file = build_artifact_lock(target)?;
    fs2::FileExt::lock_exclusive(&file)?;
    Ok(file)
}

fn build_artifact_lock(target: &Path) -> std::io::Result<fs::File> {
    use sha2::{Digest, Sha256};
    let mut existing = if target.is_absolute() {
        target.to_path_buf()
    } else {
        std::env::current_dir()?.join(target)
    };
    let mut missing = Vec::new();
    while !existing.exists() {
        missing.push(
            existing
                .file_name()
                .ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "invalid Cargo target path",
                    )
                })?
                .to_os_string(),
        );
        existing.pop();
    }
    let mut normalized = dunce::canonicalize(existing)?;
    for component in missing.into_iter().rev() {
        normalized.push(component);
    }
    let identity = normalized.to_string_lossy();
    let identity = if cfg!(windows) {
        identity.to_lowercase()
    } else {
        identity.into_owned()
    };
    let digest = format!("{:x}", Sha256::digest(identity.as_bytes()));
    let locks = verification_coordinator_root().join("build-artifacts");
    fs::create_dir_all(&locks)?;
    fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(locks.join(format!("{digest}.lock")))
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

fn render(
    out: &mut String,
    held_label: &str,
    free_label: &str,
    status: &LeaseStatusSnapshot,
    current_project: &str,
) {
    let label = if status.held { held_label } else { free_label };
    out.push_str(&format!("verification lease: {label}\n"));
    push_status_fields(out, status, current_project);
}

fn push_status_fields(out: &mut String, status: &LeaseStatusSnapshot, current_project: &str) {
    if let Some(capacity) = status.capacity {
        out.push_str(&format!(
            "capacity: {capacity}\nrunning: {}\navailable: {}\n",
            status.running, status.available
        ));
        for slot in &status.slots {
            let label = slot
                .slot
                .map_or_else(|| "legacy".to_string(), |slot| slot.to_string());
            let holder = &slot.status;
            out.push_str(&format!(
                "slots[{label}]: held={} lease_id={} target={} owner_pid={} remaining_ms={} estimated_remaining_ms={} estimated_remaining_ms_uncertain=true holder_alive={} holder_stale={} holder_project_relation={}\n",
                holder.held,
                holder.lease_id.as_deref().unwrap_or("none"),
                holder.target.as_deref().unwrap_or("none"),
                holder.owner.as_ref().map_or_else(|| "unknown".to_string(), |owner| owner.pid.to_string()),
                holder.remaining_ms.map_or_else(|| "unknown".to_string(), |remaining| remaining.to_string()),
                holder.estimated_remaining_ms.map_or_else(|| "unknown".to_string(), |remaining| remaining.to_string()),
                holder.holder_alive.map_or_else(|| "unknown".to_string(), |alive| alive.to_string()),
                holder.holder_stale,
                holder_project_relation(holder.target.as_deref(), current_project).as_str(),
            ));
        }
    }
    if let Some(lease_id) = &status.lease_id {
        out.push_str(&format!("lease_id: {lease_id}\n"));
    }
    if let Some(target) = &status.target {
        out.push_str(&format!("target: {target}\n"));
    }
    if status.held {
        let relation = holder_project_relation(status.target.as_deref(), current_project);
        let candidate = relation == HolderProjectRelation::SameProject
            && (status.legacy_release_available
                || matches!(
                    status.holder_state.as_deref(),
                    Some("orphaned" | "reclaimable")
                ));
        out.push_str(&format!("holder_project_relation: {}\n", relation.as_str()));
        out.push_str(&format!("holder_reclaim_candidate: {candidate}\n"));
        if candidate {
            out.push_str("holder_intervention: canonical_release_only\n");
            if status.legacy_release_available {
                out.push_str("holder_intervention_reason: only verify.lease.release may drain this project's legacy control channel; never use kill or pkill\n");
            } else {
                out.push_str("holder_intervention_reason: only verify.lease.release with a reason may re-check and reclaim this project's holder; never use kill or pkill\n");
            }
        } else {
            out.push_str("holder_intervention: forbidden\n");
            out.push_str(&format!(
                "holder_intervention_reason: {}; you must not reclaim or stop this holder. Wait; do not bypass canonical release refusals with kill or pkill\n",
                relation.reason()
            ));
        }
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
        out.push_str("estimated_remaining_ms_uncertain: true\n");
        out.push_str("estimated_remaining_ms_basis: batch estimate or lease TTL, not a live progress counter; unchanged estimates are not evidence of a stalled holder\n");
    }
    if let Some(held) = status.holder_held_ms {
        out.push_str(&format!("holder_held_ms: {held}\n"));
    }
    if let Some(cpu) = status.holder_cpu_percent {
        out.push_str(&format!("holder_cpu_percent: {cpu:.1}\n"));
    }
    if let Some(state) = &status.holder_state {
        out.push_str(&format!("holder_state: {state}\n"));
        if state == "unknown" {
            out.push_str("holder_state_constraint: observation is inconclusive; repeated sampling does not authorize reclaiming or stopping the holder\n");
        }
        if let Some(detail) = &status.holder_state_detail {
            out.push_str(&format!("holder_state_detail: {detail}\n"));
        }
        // Issue #4561 AC-4: the value is one sampled reading, and the reader
        // is usually deciding whether to interrupt someone. Say what it does
        // and does not license, next to the value itself.
        out.push_str(&format!("holder_state_advice: {HOLDER_STATE_ADVICE}\n"));
    }
    out.push_str(&format!("pending: {}\n", status.pending));
    if status.held || status.pending > 0 {
        out.push_str("waiter_action: wait\n");
        out.push_str("waiter_reason: waiting for canonical admission is expected; queue position does not authorize reclaiming or stopping the holder\n");
    }
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
         they cannot be released or extended through the manual API unless the holder is \
         orphaned or an old in-tree driver is stalled with no workload left to run.{holder} \
         Check `verify.lease.status` for the current holder."
    ))
}

fn missing_lease(lease_id: &str) -> SpecOpsError {
    let held = status_for_lease(lease_id).ok().flatten();
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
    #[test]
    fn status_lists_both_slot_holders_and_remaining_capacity() {
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let coordinator = gwt_core::index_coordinator::IndexCoordinator::open_verification(
            gwt_core::index_coordinator::verification_coordinator_root(),
            2,
        )
        .unwrap();
        let mut holders = Vec::new();
        for worktree in ["first", "second"] {
            let key = gwt_core::index_coordinator::TargetKey::verification("fixture", worktree);
            let gwt_core::index_coordinator::JobAdmission::Owner(guard) = coordinator
                .request_job(
                    &key,
                    gwt_core::index_coordinator::JobPriority::ManualRebuild,
                    std::time::Duration::from_secs(1),
                )
                .unwrap()
            else {
                panic!("unique worktree")
            };
            let lease = guard
                .acquire_heavy_with_ttl(
                    std::time::Duration::from_secs(1),
                    std::time::Duration::from_secs(60),
                )
                .unwrap();
            holders.push((guard, lease));
        }
        let mut out = String::new();
        let snapshot = super::status().unwrap();
        super::push_status_fields(&mut out, &snapshot, "fixture");
        assert!(out.contains("running: 2\n"), "{out}");
        assert!(
            out.contains(&format!("capacity: {}\n", snapshot.capacity.unwrap())),
            "{out}"
        );
        assert!(
            out.contains(&format!(
                "available: {}\n",
                snapshot.capacity.unwrap().saturating_sub(2)
            )),
            "{out}"
        );
        assert!(out.contains("slots[0]:"), "{out}");
        assert!(out.contains("slots[1]:"), "{out}");
        struct NoWork;
        impl super::OrphanReclaimer for NoWork {
            fn observe(
                &mut self,
                _: u32,
                _: &gwt_core::index_coordinator::HeavyLeaseStatus,
                _: std::time::Duration,
            ) -> Option<super::holder_activity::HolderActivity> {
                None
            }
            fn terminate(&mut self, _: u32, _: bool) -> Result<(), String> {
                panic!("a live slot holder must not be terminated")
            }
        }
        for (_, lease) in &holders {
            assert!(out.contains(lease.id()), "{out}");
            assert_eq!(
                super::holder_for_lease(&coordinator, lease.id())
                    .unwrap()
                    .unwrap()
                    .lease_id
                    .as_deref(),
                Some(lease.id())
            );
            let error = super::release(
                "fixture",
                lease.id(),
                Some("inspect live slot"),
                &mut NoWork,
                &mut String::new(),
            )
            .unwrap_err()
            .to_string();
            assert!(
                error.contains("Canonical leases are owned by `verify.run`"),
                "{error}"
            );
        }
    }

    #[test]
    fn automatic_capacity_is_bounded_by_cpu_and_memory() {
        let gib = 1024 * 1024 * 1024;
        assert_eq!(super::automatic_slot_capacity(64, 256 * gib), 4);
        assert_eq!(super::automatic_slot_capacity(32, 128 * gib), 4);
        assert_eq!(super::automatic_slot_capacity(32, 32 * gib), 2);
        assert_eq!(super::automatic_slot_capacity(2, 8 * gib), 1);
    }

    #[test]
    fn cargo_target_resolution_respects_configuration_and_command_overrides() {
        let _lock = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(dir.path()).unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::create_dir_all(dir.path().join(".cargo")).unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname='target-fixture'\nversion='0.0.0'\nedition='2021'\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "").unwrap();
        std::fs::write(
            dir.path().join(".cargo/config.toml"),
            "[build]\ntarget-dir='configured-target'\n",
        )
        .unwrap();
        let _target = gwt_core::test_support::ScopedEnvVar::unset("CARGO_TARGET_DIR");
        assert_eq!(
            super::effective_cargo_target(&root, "cargo test --workspace", false).unwrap(),
            Some(root.join("configured-target"))
        );
        assert_eq!(super::effective_cargo_target(&root, "CARGO_TARGET_DIR=assigned-target cargo test --workspace --target-dir selected-target", false).unwrap(), Some(root.join("selected-target")));
        assert_eq!(
            super::effective_cargo_target(dir.path(), "python runner.py", false).unwrap(),
            None
        );
    }

    fn command_strings(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|part| part.to_string()).collect()
    }

    #[test]
    fn known_short_non_cargo_gates_are_light_and_unknown_commands_stay_heavy() {
        for command in [
            "git diff --check",
            "git diff --check --cached",
            "git.exe diff --cached --check -- README.md",
            "node scripts/check-coverage-threshold.mjs target/coverage-summary.json 90",
            "node ./scripts/check-coverage-threshold.mjs summary.json 80 --scope-exclude 'crates/gwt/'",
            "actionlint .github/workflows/test.yml",
            "shellcheck scripts/check.sh",
            "yamllint .github/workflows",
            "taplo check Cargo.toml",
            "taplo fmt --check Cargo.toml",
            "typos README.md",
        ] {
            assert_eq!(classify_command(command), CommandWeight::Light, "{command}");
            assert_eq!(short_non_cargo_timeout(command), Some(Duration::from_secs(60)), "{command}");
        }
        for command in [
            "git diff",
            "git status",
            "git diff --line-prefix --check --ext-diff -- README.md",
            "taplo fmt --config --check Cargo.toml",
            "actionlint.sh .github/workflows/test.yml",
            "shellcheck.py scripts/check.sh",
            "git.sh diff --check",
            "node.sh scripts/check-coverage-threshold.mjs summary.json 90",
            "node scripts/coverage-summary.mjs -- --workspace --all-features",
            "node arbitrary-script.mjs",
            "node other/check-coverage-threshold.mjs summary.json 90",
            "python3 runner.py",
            "bash scripts/run-frontend-tests.sh",
            "npx playwright test --headed",
            "taplo lsp stdio",
            "cargo build -p gwt",
            "cargo test --workspace --all-features",
        ] {
            assert_eq!(classify_command(command), CommandWeight::Heavy, "{command}");
            assert_eq!(short_non_cargo_timeout(command), None, "{command}");
        }
        assert_eq!(short_non_cargo_timeout("markdownlint README.md"), None);
    }

    /// Issue #4196 AC-1 / AC-3: what a requested command weighs follows the
    /// scope it will actually build, not merely the fact that it says
    /// `cargo test`. AC-3 pins the two ends down: a workspace-wide run is
    /// heavy and a single named test target is light.
    #[test]
    fn classify_command_reads_the_scope_of_a_cargo_run() {
        // Issue #4823: features do not widen target selection, and documentation
        // checks share the host. A library run must also select a test filter.
        for command in [
            "cargo test -p gwt --all-features --test verification_lease",
            "cargo test -p gwt --all-features --lib verification_lease::tests",
            "cargo test -p gwt --all-features --lib -- verification_lease::tests --exact",
            r#"cargo test -p gwt --lib verification_lease -- --skip """#,
            "markdownlint README.md",
            "markdownlint-cli2 README.md",
            "bunx markdownlint-cli2 README.md",
            "npx markdownlint-cli2 README.md",
            "bunx --bun markdownlint-cli . --config .markdownlint.json --ignore target --ignore CHANGELOG.md --ignore tasks",
            "npx --yes markdownlint-cli README.md",
            r#"RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --document-private-items"#,
        ] {
            assert_eq!(classify_command(command), CommandWeight::Light, "{command}");
        }
        for command in [
            "cargo test -p gwt --all-features",
            "cargo test -p gwt --all-features --lib",
            "cargo test -p gwt --lib --features extra",
            "cargo test -p gwt --lib -- --skip ignored_case",
            "cargo test -p gwt --lib -- --test-threads 1",
            r#"cargo test -p gwt --all-features --lib verification_lease -- """#,
            r#"cargo test -p gwt --lib -- verification_lease """#,
            r#"cargo test -p gwt --lib -- "" verification_lease"#,
            r#"cargo test -p gwt --lib "" -- verification_lease"#,
            r#"cargo test -p gwt --lib -- """#,
            "cargo test -p gwt --doc",
            "cargo doc --workspace",
            "bunx arbitrary-script README.md",
            "bunx --bun arbitrary-script README.md",
            "bunx --arbitrary markdownlint-cli README.md",
        ] {
            assert_eq!(classify_command(command), CommandWeight::Heavy, "{command}");
        }
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
            classify_command("cargo test -pgwt --lib verification_lease"),
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
            CommandWeight::Heavy
        );

        // Unknown libtest options cannot establish a bounded selection,
        // even when Cargo supplied a non-empty filter before `--`.
        assert_eq!(
            classify_command("cargo test -p gwt --lib verification_lease -- --all-features"),
            CommandWeight::Heavy
        );
        assert_eq!(
            classify_command("cargo +nightly test -p gwt --lib verification_lease"),
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

        let mixed = command_strings(&[
            "cargo test -p gwt --lib admission",
            "cargo test --workspace",
        ]);
        assert_eq!(
            first_heavy_command(&mixed).map(String::as_str),
            Some("cargo test --workspace")
        );
    }

    use super::*;

    #[cfg(windows)]
    #[test]
    fn windows_reclaimer_terminates_the_confirmed_holder() {
        use std::process::Stdio;

        let worktree = tempfile::tempdir().unwrap();
        let mut child = gwt_core::process::hidden_command("cmd")
            .args(["/C", "pause"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        assert!(child.try_wait().unwrap().is_none(), "holder must be alive");
        let result = SystemReclaimer::new(worktree.path()).terminate(child.id(), false);
        let deadline = Instant::now() + Duration::from_secs(10);
        let exited = loop {
            match child.try_wait() {
                Ok(Some(_)) => break true,
                Ok(None) if result.is_ok() && Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(100));
                }
                _ => break false,
            }
        };
        // Clean up even when the native termination under test failed.
        let _ = child.kill();
        let _ = child.wait();
        assert!(result.is_ok(), "{result:?}");
        assert!(
            exited,
            "the confirmed holder must exit before the lease TTL"
        );
    }
    use gwt_core::index_coordinator::JobPriority;

    #[test]
    fn free_status_renders_without_holder_fields() {
        let mut out = String::new();
        render(
            &mut out,
            "held",
            "free",
            &LeaseStatusSnapshot::default(),
            "repo",
        );
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
                legacy_release_available: false,
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
                ..LeaseStatusSnapshot::default()
            },
            "repo",
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
             holder_project_relation: same_project\n\
             holder_reclaim_candidate: false\n\
             holder_intervention: forbidden\n\
             holder_intervention_reason: held by this project; you must not reclaim or stop this holder. Wait; do not bypass canonical release refusals with kill or pkill\n\
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
             estimated_remaining_ms_uncertain: true\n\
             estimated_remaining_ms_basis: batch estimate or lease TTL, not a live progress counter; unchanged estimates are not evidence of a stalled holder\n\
             pending: 2\n\
             waiter_action: wait\n\
             waiter_reason: waiting for canonical admission is expected; queue position does not authorize reclaiming or stopping the holder\n\
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
            "repo",
        );

        assert!(out.contains("holder_state: unknown\n"), "{out}");
        assert!(
            out.contains("holder_state_detail: holder state unknown: held 8m19s\n"),
            "{out}"
        );
        assert!(out.contains("holder_state_advice: "), "{out}");
        assert!(out.contains("not a verdict"), "{out}");
        assert!(out.contains("ps -eo pid,ppid,time,command"), "{out}");
        assert!(out.contains("holder_intervention: forbidden\n"), "{out}");
        assert!(out.contains("holder_reclaim_candidate: false\n"), "{out}");
        assert!(
            out.contains("repeated sampling does not authorize"),
            "{out}"
        );
    }

    #[test]
    fn estimates_and_queue_positions_do_not_authorize_intervention() {
        let mut out = String::new();
        render(
            &mut out,
            "held",
            "free",
            &LeaseStatusSnapshot {
                held: true,
                estimated_remaining_ms: Some(60_000),
                pending: 1,
                ..LeaseStatusSnapshot::default()
            },
            "repo",
        );
        assert!(
            out.contains("estimated_remaining_ms: 60000\nestimated_remaining_ms_uncertain: true\n"),
            "{out}"
        );
        assert!(
            out.contains("unchanged estimates are not evidence of a stalled holder"),
            "{out}"
        );
        assert!(out.contains("waiter_action: wait\n"), "{out}");
        assert!(out.contains("queue position does not authorize"), "{out}");
    }

    #[test]
    fn only_a_known_same_project_orphan_is_a_reclaim_candidate() {
        let mut snapshot = LeaseStatusSnapshot {
            held: true,
            target: Some("repo--verification--wt".to_string()),
            holder_state: Some("orphaned".to_string()),
            ..LeaseStatusSnapshot::default()
        };
        let mut own = String::new();
        render(&mut own, "held", "free", &snapshot, "repo");
        assert!(own.contains("holder_reclaim_candidate: true\n"), "{own}");
        assert!(
            own.contains("holder_intervention: canonical_release_only\n"),
            "{own}"
        );

        let mut other = String::new();
        render(&mut other, "held", "free", &snapshot, "another-repo");
        assert!(
            other.contains("holder_project_relation: other_project\n"),
            "{other}"
        );
        assert!(
            other.contains("holder_reclaim_candidate: false\n"),
            "{other}"
        );
        assert!(!other.contains("canonical_release_only"), "{other}");

        snapshot.target = None;
        let mut unknown = String::new();
        render(&mut unknown, "held", "free", &snapshot, "repo");
        assert!(
            unknown.contains("holder_project_relation: unknown\n"),
            "{unknown}"
        );
        assert!(
            unknown.contains("holder_reclaim_candidate: false\n"),
            "{unknown}"
        );
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

        fn stalled_driver() -> HolderActivity {
            HolderActivity {
                delegated: false,
                ..reading(false, 0)
            }
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

        #[test]
        fn another_projects_holder_is_protected_in_status_and_release() {
            let held = hold_lease();
            let worktree = tempfile::tempdir().unwrap();
            let mut env = crate::cli::TestEnv::new(worktree.path().to_path_buf());
            let mut out = String::new();
            run(&mut env, VerificationLeaseCommand::Status, &mut out).unwrap();
            assert!(
                out.contains("holder_project_relation: other_project\n"),
                "{out}"
            );
            assert!(out.contains("holder_reclaim_candidate: false\n"), "{out}");
            assert!(out.contains("holder_intervention: forbidden\n"), "{out}");
            assert!(!out.contains("re-checks that and reclaims"), "{out}");

            let err = run(
                &mut env,
                VerificationLeaseCommand::Release {
                    lease_id: held.lease_id.clone(),
                    reason: Some("another project is waiting".to_string()),
                },
                &mut String::new(),
            )
            .unwrap_err()
            .to_string();
            assert!(err.contains("held by another project"), "{err}");
            assert!(err.contains("must not reclaim or stop"), "{err}");
            assert!(still_held(&held.lease_id));
        }

        #[test]
        fn a_current_holder_cannot_hide_another_projects_legacy_holder_from_the_guard() {
            let current = hold_lease();
            let coordinator = IndexCoordinator::open_default().unwrap();
            let key = TargetKey::verification("another-project", "legacy");
            let JobAdmission::Owner(guard) = coordinator
                .request_job(&key, JobPriority::ManualRebuild, Duration::from_secs(1))
                .unwrap()
            else {
                panic!("private legacy lane must be free");
            };
            let lease = guard.acquire_heavy(Duration::from_secs(1)).unwrap();
            let control = coordinator_root().join(CONTROL_DIR).join("foreign-legacy");
            fs::create_dir_all(&control).unwrap();
            fs::write(
                control.join(OUTCOME_FILE),
                serde_json::json!({"granted": true, "held": true, "lease_id": lease.id()})
                    .to_string(),
            )
            .unwrap();
            let worktree = tempfile::tempdir().unwrap();
            let mut reclaimer = NeverTerminates(SystemReclaimer::new(worktree.path()));
            let err = release(
                "99a8660247f5bc49",
                lease.id(),
                Some("drain old holder"),
                &mut reclaimer,
                &mut String::new(),
            )
            .unwrap_err()
            .to_string();
            assert!(err.contains("held by another project"), "{err}");
            assert!(!control.join(RELEASE_FILE).exists());
            assert!(coordinator.heavy_lease_status().unwrap().held);
            assert!(still_held(&current.lease_id));
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
                "99a8660247f5bc49",
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
                "99a8660247f5bc49",
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

        #[test]
        fn a_renewing_holder_is_protected_even_when_process_samples_look_orphaned() {
            struct RenewingReclaimer<'a>(ScriptedReclaimer<'a>);
            impl OrphanReclaimer for RenewingReclaimer<'_> {
                fn observe(
                    &mut self,
                    pid: u32,
                    status: &HeavyLeaseStatus,
                    pause: Duration,
                ) -> Option<HolderActivity> {
                    self.0
                        .held
                        .lease
                        .as_mut()
                        .unwrap()
                        .extend(Duration::from_secs(2_700))
                        .unwrap();
                    // The diagnostic ledger is best-effort. Its absence must
                    // not erase a renewal already published in the ticket.
                    fs::remove_file(open_coordinator().unwrap().lease_event_log_path()).unwrap();
                    self.0.observe(pid, status, pause)
                }

                fn terminate(&mut self, pid: u32, force: bool) -> Result<(), String> {
                    self.0.terminate(pid, force)
                }
            }

            // Cover a renewal already published and one that arrives while
            // the reclaimer samples processes, without any real waiting.
            for already_renewed in [true, false] {
                let mut held = hold_lease();
                if already_renewed {
                    held.lease
                        .as_mut()
                        .unwrap()
                        .extend(Duration::from_secs(2_700))
                        .unwrap();
                    fs::remove_file(open_coordinator().unwrap().lease_event_log_path()).unwrap();
                }
                let lease_id = held.lease_id.clone();
                let mut reclaimer = RenewingReclaimer(ScriptedReclaimer {
                    readings: vec![Some(orphan()), Some(orphan())],
                    held: &mut held,
                    terminated: Vec::new(),
                });
                let error = release(
                    "99a8660247f5bc49",
                    &lease_id,
                    Some("suspected orphan"),
                    &mut reclaimer,
                    &mut String::new(),
                )
                .unwrap_err()
                .to_string();
                assert!(error.contains("renewed"), "{error}");
                assert!(reclaimer.0.terminated.is_empty());
                assert!(still_held(&lease_id));
            }
        }

        #[test]
        fn a_confirmed_empty_stalled_driver_releases_the_lease() {
            let mut held = hold_lease();
            let lease_id = held.lease_id.clone();
            let mut reclaimer = ScriptedReclaimer {
                readings: vec![Some(stalled_driver()), Some(stalled_driver())],
                held: &mut held,
                terminated: Vec::new(),
            };
            let mut out = String::new();

            release(
                "99a8660247f5bc49",
                &lease_id,
                Some("all commands exited"),
                &mut reclaimer,
                &mut out,
            )
            .unwrap();

            assert_eq!(reclaimer.terminated, vec![(std::process::id(), false)]);
            assert!(!still_held(&lease_id));
            let events = open_coordinator().unwrap().lease_events().unwrap();
            let event = events.last().unwrap();
            assert_eq!(event.kind, LeaseEventKind::Reclaimed);
            assert!(event
                .reason
                .as_deref()
                .unwrap()
                .contains("all commands exited"));
        }

        #[test]
        fn a_stalled_driver_that_resumes_before_confirmation_keeps_its_lease() {
            let mut held = hold_lease();
            let lease_id = held.lease_id.clone();
            let mut reclaimer = ScriptedReclaimer {
                readings: vec![
                    Some(stalled_driver()),
                    Some(HolderActivity {
                        workload_processes: 1,
                        ..stalled_driver()
                    }),
                ],
                held: &mut held,
                terminated: Vec::new(),
            };

            let err = release(
                "99a8660247f5bc49",
                &lease_id,
                Some("looked idle"),
                &mut reclaimer,
                &mut String::new(),
            )
            .unwrap_err()
            .to_string();

            assert!(err.contains("did not confirm"), "{err}");
            assert!(reclaimer.terminated.is_empty());
            assert!(still_held(&lease_id));
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

            let err = release(
                "99a8660247f5bc49",
                &lease_id,
                Some("  "),
                &mut reclaimer,
                &mut String::new(),
            )
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
                "99a8660247f5bc49",
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

            let err = release(
                "99a8660247f5bc49",
                &lease_id,
                Some("gone"),
                &mut reclaimer,
                &mut String::new(),
            )
            .unwrap_err()
            .to_string();

            assert!(err.contains("no live verification lease"), "{err}");
        }
    }
}
