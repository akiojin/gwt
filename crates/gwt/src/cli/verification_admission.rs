//! Issue #3913: host admission for `verify.run`.
//!
//! `verify.run` is the heaviest thing an agent starts, and on a shared host
//! it used to start blind: it neither claimed the SPEC #3576 verification
//! lease nor looked at what sibling worktrees were already compiling, so
//! seven agent windows could run seven matrices at once (load 15–57 was
//! measured) and every wall-clock-bound test flaked. This module makes
//! `verify.run` its own claimant:
//!
//! 1. A lease the agent already holds for this worktree
//!    (`verify.lease.acquire`) is honored as-is — the holder never waits.
//! 2. Otherwise the run claims the host-wide heavy lease in-process. The
//!    kernel lock makes the claim atomic, and while the claim is pending the
//!    coordinator lists it under `verify.lease.status` `pending`.
//! 3. With the lease held, the run waits for heavy processes that belong to
//!    *other worktrees of the same repository* (`cargo`, `rustc`,
//!    `clippy-driver`, test binaries under `target/`) to drain — those are the
//!    raw skill runs that never took the lease.
//! 4. The whole wait is bounded (`params.max_wait_secs`, default
//!    [`DEFAULT_MAX_WAIT_SECS`], hard cap [`MAX_WAIT_SECS`]). The cap stays
//!    below the Issue Monitor's default `stuck_timeout_secs` on purpose: a
//!    bounded wait inside one tool call can never be mistaken for a stalled
//!    agent, and the retry an agent makes after a `deferred` answer is a
//!    fresh tool call — a heartbeat — so the wait consumes no autonomous
//!    attempt (#3844 / #3849).
//!
//! The wait is visible in three places: the coordinator's `pending` count
//! while the lease is contended, one Board `status` post once the wait has
//! lasted longer than a poll, and the admission line in the run output.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gwt_core::index_coordinator::{
    CoordinatorError, HeavyLease, IndexCoordinator, JobAdmission, JobOutcome, JobPriority,
    TargetJobGuard,
};
use gwt_github::{client::ApiError, SpecOpsError};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

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
/// How many foreign processes a refusal names before summarizing.
const DESCRIBE_LIMIT: usize = 6;

/// Process names that are compilers or compile drivers whatever they were
/// asked to do.
const COMPILERS: &[&str] = &[
    "rustc",
    "clippy-driver",
    "rustdoc",
    "cargo-llvm-cov",
    "cargo-clippy",
    "cargo-nextest",
];
/// `cargo` subcommands that compile or run compiled tests.
const CARGO_HEAVY_SUBCOMMANDS: &[&str] = &[
    "test", "t", "clippy", "build", "b", "check", "c", "llvm-cov", "nextest", "doc", "d", "bench",
    "run", "r",
];

/// What made a process count as heavy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HeavyKind {
    /// `rustc`, `clippy-driver`, `rustdoc`, coverage / nextest drivers.
    Compiler,
    /// `cargo` running a subcommand that compiles or tests.
    CargoBuild,
    /// An executable under `target/**/deps` or `target/**/build`.
    TargetBinary,
}

/// A heavy process that belongs to another worktree of the same repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ForeignHeavyProcess {
    pub pid: u32,
    pub name: String,
    pub kind: HeavyKind,
    pub worktree: PathBuf,
}

/// Outcome of a successful admission.
#[derive(Debug)]
pub(crate) enum Admission {
    /// The agent already holds the lease for this worktree.
    PreHeld { lease_id: Option<String> },
    /// The run claimed the lease itself; it is released on drop. Boxed: the
    /// guard and lease carry open lock files and paths, and the enum is
    /// passed around by value.
    Acquired(Box<HeldLease>),
}

/// In-process lease holder; dropping releases the heavy lease and completes
/// the target job, in the reverse of the acquisition order.
pub(crate) struct HeldLease {
    guard: Option<TargetJobGuard>,
    lease: Option<HeavyLease>,
    lease_id: String,
    waited: Duration,
}

impl std::fmt::Debug for HeldLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HeldLease")
            .field("lease_id", &self.lease_id)
            .field("waited", &self.waited)
            .finish_non_exhaustive()
    }
}

impl HeldLease {
    fn settle(&mut self, outcome: JobOutcome) {
        if let Some(lease) = self.lease.take() {
            let _ = lease.release();
        }
        if let Some(guard) = self.guard.take() {
            let _ = guard.complete(outcome);
        }
    }
}

impl Drop for HeldLease {
    fn drop(&mut self) {
        self.settle(JobOutcome::Completed);
    }
}

impl Admission {
    /// One line for the `verify.run` output, so the wait travels with the
    /// evidence.
    pub(crate) fn summary(&self) -> String {
        match self {
            Admission::PreHeld { lease_id } => format!(
                "verify: host admission — lease {} already held by this worktree; started without waiting",
                lease_id.as_deref().unwrap_or("?")
            ),
            Admission::Acquired(held) => format!(
                "verify: host admission — lease {} acquired (waited {}s)",
                held.lease_id,
                held.waited.as_secs()
            ),
        }
    }

    #[cfg(test)]
    pub(crate) fn lease_id(&self) -> Option<&str> {
        match self {
            Admission::PreHeld { lease_id } => lease_id.as_deref(),
            Admission::Acquired(held) => Some(held.lease_id.as_str()),
        }
    }

    #[cfg(test)]
    pub(crate) fn waited(&self) -> Duration {
        match self {
            Admission::PreHeld { .. } => Duration::ZERO,
            Admission::Acquired(held) => held.waited,
        }
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

fn file_stem_of(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::to_string)
}

/// Pure classification of one host process.
pub(crate) fn classify_heavy(name: &str, cmd: &[String], exe: Option<&Path>) -> Option<HeavyKind> {
    // The reported name may be truncated by the kernel; the executable path
    // and argv[0] are more reliable when present.
    let base = exe
        .and_then(file_stem_of)
        .or_else(|| cmd.first().and_then(|first| file_stem_of(Path::new(first))))
        .unwrap_or_else(|| name.to_string());
    if COMPILERS.contains(&base.as_str()) {
        return Some(HeavyKind::Compiler);
    }
    if base == "cargo" {
        let subcommand = cmd
            .iter()
            .skip(1)
            .map(String::as_str)
            .find(|arg| !arg.starts_with('-') && !arg.starts_with('+'));
        return subcommand
            .filter(|sub| CARGO_HEAVY_SUBCOMMANDS.contains(sub))
            .map(|_| HeavyKind::CargoBuild);
    }
    exe.filter(|exe| is_target_artifact(exe))
        .map(|_| HeavyKind::TargetBinary)
}

/// `target/**/deps/*` and `target/**/build/*` are test binaries and build
/// scripts; `target/debug/<tool>` is a built tool such as `gwtd` itself.
fn is_target_artifact(exe: &Path) -> bool {
    let parts: Vec<&str> = exe
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .collect();
    let dirs = &parts[..parts.len().saturating_sub(1)];
    match dirs.iter().position(|part| *part == "target") {
        Some(index) => dirs[index + 1..]
            .iter()
            .any(|part| *part == "deps" || *part == "build"),
        None => false,
    }
}

/// Parse `git worktree list --porcelain`, dropping bare entries and `own`.
pub(crate) fn sibling_worktrees_from_porcelain(stdout: &str, own: &Path) -> Vec<PathBuf> {
    let mut siblings = Vec::new();
    for block in stdout.split("\n\n") {
        let mut path = None;
        let mut bare = false;
        for line in block.lines() {
            if let Some(rest) = line.strip_prefix("worktree ") {
                path = Some(PathBuf::from(rest.trim()));
            } else if line.trim() == "bare" {
                bare = true;
            }
        }
        if let Some(path) = path {
            if !bare && path != own {
                siblings.push(path);
            }
        }
    }
    siblings
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

/// Enumerate the sibling worktrees of `own` (same repository, other paths),
/// canonicalized so kernel-reported process paths compare directly.
pub(crate) fn sibling_worktrees(own: &Path) -> Vec<PathBuf> {
    if !gwt_core::paths::git_repository_discovery_possible(own) {
        return Vec::new();
    }
    let own = dunce::canonicalize(own).unwrap_or_else(|_| own.to_path_buf());
    let mut command = gwt_core::process::hidden_command("git");
    command
        .args(["worktree", "list", "--porcelain"])
        .current_dir(&own);
    gwt_core::process::scrub_git_env(&mut command);
    let Ok(output) = command.output() else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    sibling_worktrees_from_porcelain(&String::from_utf8_lossy(&output.stdout), &own)
        .into_iter()
        .filter_map(|path| dunce::canonicalize(&path).ok())
        .filter(|path| path != &own)
        .collect()
}

/// Heavy processes on this host that belong to one of `siblings`. Anything
/// rooted in `own` is this worktree's business and never counts.
pub(crate) fn scan_foreign_heavy(own: &Path, siblings: &[PathBuf]) -> Vec<ForeignHeavyProcess> {
    if siblings.is_empty() {
        return Vec::new();
    }
    let own = dunce::canonicalize(own).unwrap_or_else(|_| own.to_path_buf());
    let own_pid = std::process::id();
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_cmd(UpdateKind::Always)
            .with_exe(UpdateKind::Always)
            .with_cwd(UpdateKind::Always),
    );
    let mut found: Vec<ForeignHeavyProcess> = system
        .processes()
        .iter()
        .filter_map(|(pid, process)| {
            let pid = pid.as_u32();
            if pid == own_pid {
                return None;
            }
            let cmd: Vec<String> = process
                .cmd()
                .iter()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect();
            let exe = process.exe();
            let cwd = process.cwd();
            // The kernel truncates reported names (16 bytes on macOS); the
            // executable's file name is the one humans can match.
            let name = exe
                .and_then(Path::file_name)
                .map(|file| file.to_string_lossy().into_owned())
                .unwrap_or_else(|| process.name().to_string_lossy().into_owned());
            let kind = classify_heavy(&name, &cmd, exe)?;
            if [cwd, exe]
                .into_iter()
                .flatten()
                .any(|path| path.starts_with(&own))
            {
                return None;
            }
            let worktree = attribute_worktree(cwd, exe, siblings)?;
            Some(ForeignHeavyProcess {
                pid,
                name,
                kind,
                worktree: worktree.to_path_buf(),
            })
        })
        .collect();
    found.sort_by_key(|process| process.pid);
    found
}

fn describe_foreign(list: &[ForeignHeavyProcess]) -> String {
    let mut parts: Vec<String> = list
        .iter()
        .take(DESCRIBE_LIMIT)
        .map(|process| {
            format!(
                "{} (pid {}) in {}",
                process.name,
                process.pid,
                process.worktree.display()
            )
        })
        .collect();
    if list.len() > DESCRIBE_LIMIT {
        parts.push(format!("and {} more", list.len() - DESCRIBE_LIMIT));
    }
    parts.join(", ")
}

fn describe_holder(coordinator: &IndexCoordinator) -> String {
    match coordinator.heavy_lease_status() {
        Ok(status) if status.held => format!(
            "verification lease held by {} (pid {}, {}s left)",
            status.target.as_deref().unwrap_or("unknown target"),
            status
                .owner
                .as_ref()
                .map(|owner| owner.pid.to_string())
                .unwrap_or_else(|| "?".to_string()),
            status.remaining_ms.unwrap_or(0) / 1000
        ),
        Ok(_) => "verification lease was contended".to_string(),
        Err(err) => format!("verification lease status unavailable: {err}"),
    }
}

fn deferred(started: Instant, max_wait: Duration, detail: &str) -> SpecOpsError {
    unexpected(format!(
        "verify: deferred — host busy for {}s (budget {}s): {detail}; rerun `verify.run` once \
         the host quiets down or `verify.lease.acquire` is granted — the wait counts as one \
         gwt-verify lease attempt",
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
/// admission through [`Admission::summary`].
pub(super) fn admit<E: CliEnv>(
    env: &mut E,
    worktree: &Path,
    max_wait: Duration,
) -> Result<Admission, SpecOpsError> {
    let key = verification_lease::verification_key(env)?;
    let coordinator = verification_lease::open_coordinator()?;
    let started = Instant::now();
    let deadline = started + max_wait;
    let own = dunce::canonicalize(worktree).unwrap_or_else(|_| worktree.to_path_buf());
    let mut notice = BoardNotice::default();

    // Phase 1: the host-wide lease. A lease this worktree already holds is
    // the agent's, taken through `verify.lease.acquire`; honor it and leave
    // it alone.
    let status = coordinator
        .heavy_lease_status()
        .map_err(|err| unexpected(format!("failed to read the verification lease: {err}")))?;
    if status.held && status.target.as_deref() == Some(key.file_stem().as_str()) {
        return Ok(Admission::PreHeld {
            lease_id: status.lease_id,
        });
    }
    let guard = loop {
        match coordinator
            .request_job(&key, JobPriority::ManualRebuild, NON_BLOCKING)
            .map_err(|err| unexpected(format!("verification job admission failed: {err}")))?
        {
            JobAdmission::Owner(guard) => break guard,
            JobAdmission::Joined(waiter) => {
                // A concurrent claimant in this same worktree (another
                // verify.run, or a lease acquire still handshaking).
                drop(waiter);
                if Instant::now() >= deadline {
                    return Err(deferred(
                        started,
                        max_wait,
                        "another verification claimant in this worktree owns the target job",
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
                    return Err(deferred(started, max_wait, &holder));
                }
                notice.maybe_post(env, started, max_wait, &holder);
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
    let mut held = HeldLease {
        guard: Some(guard),
        lease_id: lease.id().to_string(),
        lease: Some(lease),
        waited: Duration::ZERO,
    };

    // Phase 2: with the lease held, wait for raw heavy runs of sibling
    // worktrees to drain. Holding the lease first reserves this run's turn;
    // lease-respecting claimants queue behind it instead of racing.
    let siblings = sibling_worktrees(&own);
    loop {
        let foreign = scan_foreign_heavy(&own, &siblings);
        if foreign.is_empty() {
            break;
        }
        let detail = format!(
            "heavy processes of other worktrees still running: {}",
            describe_foreign(&foreign)
        );
        if Instant::now() >= deadline {
            held.settle(JobOutcome::Failed {
                message: "host admission deferred".to_string(),
            });
            return Err(deferred(started, max_wait, &detail));
        }
        notice.maybe_post(env, started, max_wait, &detail);
        sleep_until(deadline);
    }
    held.waited = started.elapsed();
    Ok(Admission::Acquired(Box::new(held)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_core::index_coordinator::{IndexCoordinator, JobAdmission, JobPriority, TargetKey};
    use gwt_core::test_support::ScopedGwtHome;

    fn strings(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|part| part.to_string()).collect()
    }

    #[test]
    fn classify_heavy_recognizes_compilers_cargo_subcommands_and_target_binaries() {
        assert_eq!(
            classify_heavy("rustc", &strings(&["rustc", "--crate-name", "gwt"]), None),
            Some(HeavyKind::Compiler)
        );
        assert_eq!(
            classify_heavy("clippy-driver", &strings(&["clippy-driver"]), None),
            Some(HeavyKind::Compiler)
        );
        assert_eq!(
            classify_heavy(
                "cargo",
                &strings(&["/toolchain/bin/cargo", "test", "-p", "gwt"]),
                Some(Path::new("/toolchain/bin/cargo"))
            ),
            Some(HeavyKind::CargoBuild)
        );
        assert_eq!(
            classify_heavy(
                "cargo",
                &strings(&["cargo", "+stable", "clippy", "--all-targets"]),
                None
            ),
            Some(HeavyKind::CargoBuild)
        );
        assert_eq!(
            classify_heavy("cargo", &strings(&["cargo", "build", "-p", "gwt"]), None),
            Some(HeavyKind::CargoBuild)
        );
        assert_eq!(
            classify_heavy(
                "gwt_core-923d7088e882f577",
                &strings(&["/wt/target/debug/deps/gwt_core-923d7088e882f577"]),
                Some(Path::new("/wt/target/debug/deps/gwt_core-923d7088e882f577"))
            ),
            Some(HeavyKind::TargetBinary)
        );
        assert_eq!(
            classify_heavy(
                "build-script-build",
                &strings(&["/wt/target/debug/build/ring-abc/build-script-build"]),
                Some(Path::new(
                    "/wt/target/debug/build/ring-abc/build-script-build"
                ))
            ),
            Some(HeavyKind::TargetBinary)
        );
    }

    #[test]
    fn classify_heavy_ignores_light_processes() {
        assert_eq!(
            classify_heavy("git", &strings(&["git", "status"]), None),
            None
        );
        assert_eq!(
            classify_heavy("cargo", &strings(&["cargo", "metadata"]), None),
            None
        );
        assert_eq!(
            classify_heavy("cargo", &strings(&["cargo", "fmt"]), None),
            None
        );
        assert_eq!(
            classify_heavy(
                "gwtd",
                &strings(&["/wt/target/debug/gwtd"]),
                Some(Path::new("/wt/target/debug/gwtd"))
            ),
            None,
            "a built binary outside deps/build is a tool, not a compile"
        );
    }

    #[test]
    fn sibling_worktrees_from_porcelain_excludes_bare_and_own() {
        let porcelain = "worktree /repo/gwt.git\nbare\n\n\
                         worktree /repo/work/issue-1\nHEAD abc\nbranch refs/heads/work/issue-1\n\n\
                         worktree /repo/work/issue-2\nHEAD def\nbranch refs/heads/work/issue-2\n\n";
        let siblings = sibling_worktrees_from_porcelain(porcelain, Path::new("/repo/work/issue-1"));
        assert_eq!(siblings, vec![PathBuf::from("/repo/work/issue-2")]);
    }

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
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let worktree = tempfile::tempdir().unwrap();
        let mut env = crate::cli::TestEnv::new(worktree.path().to_path_buf());

        let admission = admit(&mut env, worktree.path(), Duration::from_secs(5)).unwrap();

        let coordinator = IndexCoordinator::open_default().unwrap();
        let status = coordinator.heavy_lease_status().unwrap();
        assert!(status.held, "admission must hold the host-wide lease");
        assert_eq!(
            status.target.as_deref(),
            Some(verification_key_for(worktree.path()).file_stem().as_str())
        );
        assert_eq!(admission.lease_id(), status.lease_id.as_deref());
        assert!(
            admission.summary().contains("host admission"),
            "{}",
            admission.summary()
        );
        assert!(admission.waited() < Duration::from_secs(5));

        drop(admission);
        assert!(
            !coordinator.heavy_lease_status().unwrap().held,
            "dropping the admission must release the lease"
        );
    }

    #[test]
    fn admit_honors_a_lease_already_held_by_this_worktree() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let worktree = tempfile::tempdir().unwrap();
        let coordinator = IndexCoordinator::open_default().unwrap();
        let key = verification_key_for(worktree.path());
        let JobAdmission::Owner(guard) = coordinator
            .request_job(&key, JobPriority::ManualRebuild, Duration::from_millis(250))
            .unwrap()
        else {
            panic!("fresh coordinator must admit the owner");
        };
        let lease = guard
            .acquire_heavy_with_ttl(Duration::from_millis(250), Duration::from_secs(60))
            .unwrap();

        let mut env = crate::cli::TestEnv::new(worktree.path().to_path_buf());
        let admission = admit(&mut env, worktree.path(), Duration::ZERO).unwrap();

        assert!(matches!(admission, Admission::PreHeld { .. }));
        assert_eq!(admission.lease_id(), Some(lease.id()));
        assert!(
            admission.summary().contains("already held"),
            "{}",
            admission.summary()
        );
        drop(admission);
        assert!(
            coordinator.heavy_lease_status().unwrap().held,
            "a pre-held lease belongs to the agent and must survive the run"
        );
        drop(lease);
        drop(guard);
    }

    #[test]
    fn admit_defers_when_another_target_holds_the_lease() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let worktree = tempfile::tempdir().unwrap();
        let coordinator = IndexCoordinator::open_default().unwrap();
        let other = TargetKey::repo_shared("other-repo", "issues");
        let JobAdmission::Owner(guard) = coordinator
            .request_job(
                &other,
                JobPriority::ManualRebuild,
                Duration::from_millis(250),
            )
            .unwrap()
        else {
            panic!("fresh coordinator must admit the owner");
        };
        let _lease = guard
            .acquire_heavy_with_ttl(Duration::from_millis(250), Duration::from_secs(60))
            .unwrap();

        let mut env = crate::cli::TestEnv::new(worktree.path().to_path_buf());
        let err = admit(&mut env, worktree.path(), Duration::from_secs(1)).unwrap_err();

        let message = err.to_string();
        assert!(message.contains("deferred"), "{message}");
        assert!(message.contains("rerun `verify.run`"), "{message}");
        assert!(
            message.contains(&other.file_stem()),
            "the refusal must name the holder: {message}"
        );
    }

    /// Self-exec target: parks the way a running test binary does. Platform
    /// binaries such as `/bin/sleep` cannot stand in for it — a copy placed
    /// under `target/` is killed by the kernel right after exec on macOS.
    #[test]
    #[ignore = "spawned by scan_foreign_heavy_finds_a_target_binary_running_in_a_sibling"]
    fn fake_heavy_process_parks() {
        std::thread::sleep(Duration::from_secs(60));
    }

    #[test]
    fn scan_foreign_heavy_finds_a_target_binary_running_in_a_sibling() {
        let sibling = tempfile::tempdir().unwrap();
        let own = tempfile::tempdir().unwrap();
        // This test binary lives under `target/debug/deps/`, exactly where a
        // sibling worktree's test binaries live.
        let exe = std::env::current_exe().unwrap();
        let mut child = gwt_core::process::hidden_command(&exe)
            .args([
                "--ignored",
                "--exact",
                "cli::verification_admission::tests::fake_heavy_process_parks",
            ])
            .current_dir(sibling.path())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        std::thread::sleep(Duration::from_millis(300));

        let siblings = vec![dunce::canonicalize(sibling.path()).unwrap()];
        let found = scan_foreign_heavy(own.path(), &siblings);
        let _ = child.kill();
        let _ = child.wait();

        let hit = found
            .iter()
            .find(|process| process.pid == child.id())
            .unwrap_or_else(|| panic!("the parked test binary must be reported: {found:?}"));
        assert_eq!(hit.kind, HeavyKind::TargetBinary);
        assert_eq!(hit.worktree, siblings[0]);
        assert!(
            scan_foreign_heavy(own.path(), &[]).is_empty(),
            "no siblings means nothing can be foreign"
        );
    }
}
