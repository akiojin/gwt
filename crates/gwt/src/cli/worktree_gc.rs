//! `worktree.*` JSON operations (Issue #4009).
//!
//! `worktree.gc_build_artifacts` reclaims the `target/` directories that
//! merged, idle worktrees leave behind. Build output is a cache — deleting it
//! costs a rebuild, never work — but deleting it under a running agent breaks
//! that agent's build, so the sweep keeps every worktree that still has a
//! process or a tracked launch, and by default every worktree whose HEAD is
//! not yet merged into `origin/<base>`. An unqualified call is a dry run: it
//! reports the candidates, the kept worktrees with their reasons, and the
//! bytes that would be freed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use gwt_github::SpecOpsError;
use serde::Serialize;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

use super::verification_lease::admission::attribute_worktree;
use super::CliEnv;

/// Command model for the `worktree.*` family.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreeCommand {
    /// `worktree.gc_build_artifacts` — remove `target/` from merged, idle
    /// worktrees.
    GcBuildArtifacts {
        /// Report only. Defaults to `true` so an exploratory call never
        /// deletes (AC-1).
        dry_run: bool,
        /// Base branch the merge check runs against (`origin/<base>`).
        base: Option<String>,
        /// Also reclaim worktrees whose HEAD is not merged (AC-3). Never
        /// overrides the running-process exclusion.
        include_unmerged: bool,
        /// Also reclaim the shared base-branch workspaces (`develop`,
        /// `main`, …). Off by default: those are shared surfaces, and the
        /// rebuild is a cost paid by whoever touches them next rather than by
        /// the operator running the sweep. Never overrides the
        /// running-process, tracked-launch or current-worktree exclusions.
        include_protected_workspaces: bool,
    },
}

/// Directory whose removal frees the build cache.
pub const BUILD_ARTIFACT_DIR: &str = "target";

/// Everything the sweep knows about one worktree before judging it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorktreeProbe {
    pub root: PathBuf,
    pub branch: Option<String>,
    /// The repository's main worktree; never a candidate.
    pub is_main: bool,
    /// The worktree holds a shared base branch (`develop`, `main`, …) rather
    /// than a per-launch branch.
    pub is_base_branch_workspace: bool,
    /// `<root>/target` exists.
    pub has_build_artifacts: bool,
    /// Processes whose cwd or executable lies under `root` (`name (pid N)`).
    pub active_processes: Vec<String>,
    /// gwt session ids whose launch is still live under `root`.
    pub tracked_sessions: Vec<String>,
    /// Whether HEAD is an ancestor of `origin/<base>`; `Err` when the check
    /// itself failed.
    pub merged: Result<bool, String>,
}

/// A worktree root the sweep must never touch, with the reason it reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProtectedRoot {
    pub root: PathBuf,
    pub reason: String,
}

/// A worktree whose `target/` will be (or was) removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct GcCandidate {
    pub worktree: PathBuf,
    pub branch: Option<String>,
    pub target: PathBuf,
    /// Measured after planning, only for candidates: sizing every worktree
    /// would walk hundreds of gigabytes for nothing.
    pub bytes: u64,
}

/// A worktree the sweep keeps, with the reason shown to the operator (AC-2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct GcKept {
    pub worktree: PathBuf,
    pub branch: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub(crate) struct GcPlan {
    pub candidates: Vec<GcCandidate>,
    pub kept: Vec<GcKept>,
}

/// The two opt-ins that widen the sweep. Both are off by default and neither
/// can reach a worktree something is using.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct GcOptions {
    pub include_unmerged: bool,
    pub include_protected_workspaces: bool,
}

/// Judge every probed worktree. Pure, so each exclusion branch is a unit
/// test (AC-5).
pub(crate) fn plan(
    probes: Vec<WorktreeProbe>,
    base: &str,
    options: GcOptions,
    protected: &[ProtectedRoot],
) -> GcPlan {
    let mut plan = GcPlan::default();
    for probe in probes {
        match keep_reason(&probe, base, options, protected) {
            Some(reason) => plan.kept.push(GcKept {
                worktree: probe.root,
                branch: probe.branch,
                reason,
            }),
            None => plan.candidates.push(GcCandidate {
                target: probe.root.join(BUILD_ARTIFACT_DIR),
                worktree: probe.root,
                branch: probe.branch,
                bytes: 0,
            }),
        }
    }
    plan
}

/// Why a worktree is kept, or `None` when its `target/` may go. The
/// running-process reasons come first: they are the ones an operator must
/// not miss, whatever else is true of the worktree (AC-2).
fn keep_reason(
    probe: &WorktreeProbe,
    base: &str,
    options: GcOptions,
    protected: &[ProtectedRoot],
) -> Option<String> {
    if probe.is_main {
        return Some("main worktree".to_string());
    }
    if let Some(guard) = protected.iter().find(|guard| guard.root == probe.root) {
        return Some(guard.reason.clone());
    }
    if !probe.active_processes.is_empty() {
        return Some(format!(
            "active process: {}",
            probe.active_processes.join(", ")
        ));
    }
    if !probe.tracked_sessions.is_empty() {
        return Some(format!(
            "tracked launch: {}",
            probe.tracked_sessions.join(", ")
        ));
    }
    // A base-branch workspace is an ancestor of the base by definition, so
    // the merge rule alone would always select it — and on this host that is
    // the single largest `target/` there is. It is shared, so the rebuild
    // lands on whoever opens it next rather than on the operator sweeping.
    // Reclaimable, but only when somebody asks for it by name.
    if !options.include_protected_workspaces && probe.is_base_branch_workspace {
        return Some(
            "shared base-branch workspace (pass include_protected_workspaces:true to reclaim)"
                .to_string(),
        );
    }
    if !probe.has_build_artifacts {
        return Some(format!("no {BUILD_ARTIFACT_DIR}/ directory"));
    }
    match &probe.merged {
        Err(error) => Some(format!("merge state unknown: {error}")),
        Ok(false) if !options.include_unmerged => Some(format!(
            "not merged into origin/{base} (pass include_unmerged:true to reclaim)"
        )),
        Ok(_) => None,
    }
}

pub(super) fn run<E: CliEnv>(
    env: &mut E,
    command: WorktreeCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    match command {
        WorktreeCommand::GcBuildArtifacts {
            dry_run,
            base,
            include_unmerged,
            include_protected_workspaces,
        } => {
            let base =
                base.unwrap_or_else(|| gwt_git::pr_status::SETTLEMENT_BASE_BRANCH.to_string());
            let repo_path = env.repo_path().to_path_buf();
            let report = run_gc(
                &repo_path,
                &base,
                GcOptions {
                    include_unmerged,
                    include_protected_workspaces,
                },
                dry_run,
            )?;
            out.push_str(
                &serde_json::to_string_pretty(&report).map_err(super::serde_as_api_error)?,
            );
            out.push('\n');
            Ok(0)
        }
    }
}

#[derive(Debug, Serialize)]
struct GcRemoval {
    worktree: PathBuf,
    target: PathBuf,
    bytes: u64,
}

#[derive(Debug, Serialize)]
struct GcFailure {
    worktree: PathBuf,
    target: PathBuf,
    reason: String,
}

#[derive(Debug, Serialize)]
struct GcReport {
    dry_run: bool,
    base: String,
    include_unmerged: bool,
    include_protected_workspaces: bool,
    candidates: Vec<GcCandidate>,
    kept: Vec<GcKept>,
    reclaimable_bytes: u64,
    removed: Vec<GcRemoval>,
    failed: Vec<GcFailure>,
    reclaimed_bytes: u64,
    disk_space: crate::disk_space::DiskSpaceStatus,
}

fn run_gc(
    repo_path: &Path,
    base: &str,
    options: GcOptions,
    dry_run: bool,
) -> Result<GcReport, SpecOpsError> {
    let repo_path = dunce::canonicalize(repo_path).unwrap_or_else(|_| repo_path.to_path_buf());
    let worktrees = gwt_git::worktree::WorktreeManager::new(&repo_path)
        .list()
        .map_err(|error| unexpected(format!("git worktree list failed: {error}")))?;
    let roots: Vec<(PathBuf, Option<String>)> = worktrees
        .iter()
        .filter(|info| !info.prunable)
        .map(|info| {
            (
                dunce::canonicalize(&info.path).unwrap_or_else(|_| info.path.clone()),
                info.branch.clone(),
            )
        })
        .collect();
    let root_paths: Vec<PathBuf> = roots.iter().map(|(root, _)| root.clone()).collect();
    let processes = scan_processes(&root_paths);
    let tracked = scan_tracked_launches(&gwt_core::paths::gwt_sessions_dir());
    let probes: Vec<WorktreeProbe> = roots
        .iter()
        .enumerate()
        .map(|(index, (root, branch))| WorktreeProbe {
            root: root.clone(),
            is_base_branch_workspace: branch.as_deref().is_some_and(gwt_git::is_protected_branch),
            branch: branch.clone(),
            is_main: index == 0,
            has_build_artifacts: root.join(BUILD_ARTIFACT_DIR).is_dir(),
            active_processes: processes.get(root).cloned().unwrap_or_default(),
            tracked_sessions: tracked.get(root).cloned().unwrap_or_default(),
            merged: head_is_merged_into(&repo_path, root, base),
        })
        .collect();
    let protected = protected_roots(&repo_path, &root_paths);
    let mut plan = plan(probes, base, options, &protected);
    for candidate in &mut plan.candidates {
        candidate.bytes = directory_size(&candidate.target);
    }
    let reclaimable_bytes = plan.candidates.iter().map(|c| c.bytes).sum();
    let mut removed = Vec::new();
    let mut failed = Vec::new();
    if !dry_run {
        for candidate in &plan.candidates {
            // Only ever `<worktree>/target` of a listed worktree; the plan
            // cannot name anything else.
            debug_assert_eq!(
                candidate.target,
                candidate.worktree.join(BUILD_ARTIFACT_DIR)
            );
            match std::fs::remove_dir_all(&candidate.target) {
                Ok(()) => removed.push(GcRemoval {
                    worktree: candidate.worktree.clone(),
                    target: candidate.target.clone(),
                    bytes: candidate.bytes,
                }),
                Err(error) => failed.push(GcFailure {
                    worktree: candidate.worktree.clone(),
                    target: candidate.target.clone(),
                    reason: error.to_string(),
                }),
            }
        }
    }
    let reclaimed_bytes = removed.iter().map(|r| r.bytes).sum();
    let coordinator_root = gwt_core::index_coordinator::coordinator_root();
    let disk_space = crate::disk_space::probe(&[repo_path.as_path(), coordinator_root.as_path()]);
    Ok(GcReport {
        dry_run,
        base: base.to_string(),
        include_unmerged: options.include_unmerged,
        include_protected_workspaces: options.include_protected_workspaces,
        candidates: plan.candidates,
        kept: plan.kept,
        reclaimable_bytes,
        removed,
        failed,
        reclaimed_bytes,
        disk_space,
    })
}

/// The calling worktree and the worktree whose `target/` hosts the running
/// `gwtd`: deleting either would pull the floor from under this very call.
fn protected_roots(repo_path: &Path, roots: &[PathBuf]) -> Vec<ProtectedRoot> {
    let mut protected = vec![ProtectedRoot {
        root: repo_path.to_path_buf(),
        reason: "current worktree".to_string(),
    }];
    if let Some(owner) = std::env::current_exe()
        .ok()
        .and_then(|exe| gwt_skills::build_output_owner_root(&exe))
    {
        let owner = dunce::canonicalize(&owner).unwrap_or_else(|_| PathBuf::from(&owner));
        if let Some(root) = roots.iter().find(|root| **root == owner) {
            protected.push(ProtectedRoot {
                root: root.clone(),
                reason: "hosts the running gwtd binary".to_string(),
            });
        }
    }
    protected
}

/// `git merge-base --is-ancestor <worktree HEAD> origin/<base>`, run in the
/// main repository so every worktree is judged against the same refs. No
/// fetch: a stale `origin/<base>` can only under-report merges, which keeps
/// a worktree rather than deleting one.
fn head_is_merged_into(repo_path: &Path, worktree: &Path, base: &str) -> Result<bool, String> {
    let head = git_stdout(worktree, &["rev-parse", "--verify", "HEAD"])?;
    let mut command = gwt_core::process::hidden_command("git");
    command
        .args([
            "merge-base",
            "--is-ancestor",
            head.trim(),
            &format!("origin/{base}"),
        ])
        .current_dir(repo_path);
    gwt_core::process::scrub_git_env(&mut command);
    let output = command
        .output()
        .map_err(|error| format!("git merge-base failed to start: {error}"))?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(format!(
            "git merge-base --is-ancestor failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )),
    }
}

fn git_stdout(cwd: &Path, args: &[&str]) -> Result<String, String> {
    let mut command = gwt_core::process::hidden_command("git");
    command.args(args).current_dir(cwd);
    gwt_core::process::scrub_git_env(&mut command);
    let output = command
        .output()
        .map_err(|error| format!("git {} failed to start: {error}", args.join(" ")))?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Every process on the host whose cwd or executable lies under one of
/// `roots`, keyed by root (`name (pid N)`). Any process counts, not only
/// heavy ones: an agent shell idling in the worktree still owns its build.
fn scan_processes(roots: &[PathBuf]) -> BTreeMap<PathBuf, Vec<String>> {
    let mut found: BTreeMap<PathBuf, Vec<String>> = BTreeMap::new();
    if roots.is_empty() {
        return found;
    }
    let own_pid = std::process::id();
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_exe(UpdateKind::Always)
            .with_cwd(UpdateKind::Always),
    );
    for (pid, process) in system.processes() {
        let pid = pid.as_u32();
        if pid == own_pid {
            continue;
        }
        let exe = process.exe();
        let cwd = process.cwd();
        let Some(root) = attribute_worktree(cwd, exe, roots) else {
            continue;
        };
        let name = exe
            .and_then(Path::file_name)
            .map(|file| file.to_string_lossy().into_owned())
            .unwrap_or_else(|| process.name().to_string_lossy().into_owned());
        found
            .entry(root.to_path_buf())
            .or_default()
            .push(format!("{name} (pid {pid})"));
    }
    for names in found.values_mut() {
        names.sort();
    }
    found
}

/// Session ids with a live launch, keyed by the worktree their Session
/// record names. A launch is live while the gwt host process that wrote its
/// runtime sidecar is alive, the sidecar is not terminal, and the PTY child
/// it names (when it names one) is alive. Session status alone is not
/// evidence: hundreds of `Running` records outlive their panes.
fn scan_tracked_launches(sessions_dir: &Path) -> BTreeMap<PathBuf, Vec<String>> {
    let mut found: BTreeMap<PathBuf, Vec<String>> = BTreeMap::new();
    let Ok(namespaces) = std::fs::read_dir(sessions_dir.join("runtime")) else {
        return found;
    };
    for namespace in namespaces.flatten() {
        let Some(host_pid) = namespace
            .file_name()
            .to_str()
            .and_then(|value| value.parse::<u32>().ok())
        else {
            continue;
        };
        if !crate::process::is_process_alive(host_pid) {
            continue;
        }
        let Ok(sidecars) = std::fs::read_dir(namespace.path()) else {
            continue;
        };
        for sidecar in sidecars.flatten() {
            let path = sidecar.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let Some(session_id) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            let Ok(state) = std::fs::read_to_string(&path)
                .map_err(|_| ())
                .and_then(|text| {
                    serde_json::from_str::<gwt_agent::session::SessionRuntimeState>(&text)
                        .map_err(|_| ())
                })
            else {
                continue;
            };
            if matches!(
                state.status,
                gwt_agent::AgentStatus::Stopped | gwt_agent::AgentStatus::Interrupted
            ) {
                continue;
            }
            if state
                .child_pid
                .is_some_and(|child| !crate::process::is_process_alive(child))
            {
                continue;
            }
            let Ok(session) = gwt_agent::Session::load_and_migrate(
                &sessions_dir.join(format!("{session_id}.toml")),
            ) else {
                continue;
            };
            let root = dunce::canonicalize(&session.worktree_path)
                .unwrap_or_else(|_| session.worktree_path.clone());
            found.entry(root).or_default().push(session_id.to_string());
        }
    }
    for ids in found.values_mut() {
        ids.sort();
        ids.dedup();
    }
    found
}

/// Sum of file sizes under `dir`, following no symlinks. Unreadable entries
/// count as zero: the size is an estimate for the operator, not a ledger.
fn directory_size(dir: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if metadata.is_dir() {
                stack.push(entry.path());
            } else if metadata.is_file() {
                total += metadata.len();
            }
        }
    }
    total
}

fn unexpected(message: String) -> SpecOpsError {
    SpecOpsError::Api(gwt_github::client::ApiError::Unexpected(message))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both opt-ins on, for the tests that assert an exclusion no option can
    /// override.
    fn both_opt_ins() -> GcOptions {
        GcOptions {
            include_unmerged: true,
            include_protected_workspaces: true,
        }
    }

    fn probe(root: &str) -> WorktreeProbe {
        WorktreeProbe {
            root: PathBuf::from(root),
            branch: Some(format!("work/{}", root.rsplit('/').next().unwrap_or(root))),
            is_main: false,
            is_base_branch_workspace: false,
            has_build_artifacts: true,
            active_processes: Vec::new(),
            tracked_sessions: Vec::new(),
            merged: Ok(true),
        }
    }

    fn kept_reason<'a>(plan: &'a GcPlan, root: &str) -> &'a str {
        plan.kept
            .iter()
            .find(|kept| kept.worktree == Path::new(root))
            .map(|kept| kept.reason.as_str())
            .unwrap_or_else(|| panic!("{root} not kept: {plan:?}"))
    }

    /// AC-1: a merged worktree with no process and a `target/` is reclaimed.
    #[test]
    fn merged_idle_worktree_with_build_artifacts_is_a_candidate() {
        let plan = plan(
            vec![probe("/work/issue-1")],
            "develop",
            GcOptions::default(),
            &[],
        );
        assert_eq!(plan.candidates.len(), 1, "{plan:?}");
        assert_eq!(plan.candidates[0].worktree, Path::new("/work/issue-1"));
        assert_eq!(
            plan.candidates[0].target,
            Path::new("/work/issue-1").join(BUILD_ARTIFACT_DIR)
        );
        assert_eq!(plan.candidates[0].branch.as_deref(), Some("work/issue-1"));
        assert!(plan.kept.is_empty());
    }

    /// AC-2: a worktree with a process under it is kept and the process is
    /// named, even when it is merged and `include_unmerged` is set.
    #[test]
    fn worktree_with_an_active_process_is_kept_with_the_process_named() {
        let mut busy = probe("/work/issue-2");
        busy.active_processes = vec!["claude.exe (pid 4242)".to_string()];
        let plan = plan(vec![busy], "develop", both_opt_ins(), &[]);
        assert!(plan.candidates.is_empty(), "{plan:?}");
        let reason = kept_reason(&plan, "/work/issue-2");
        assert!(reason.starts_with("active process"), "{reason}");
        assert!(reason.contains("claude.exe (pid 4242)"), "{reason}");
    }

    /// AC-2: a live tracked launch keeps the worktree even with no visible
    /// process (a docker runtime target runs elsewhere).
    #[test]
    fn worktree_with_a_tracked_launch_is_kept_with_the_session_named() {
        let mut launched = probe("/work/issue-3");
        launched.tracked_sessions = vec!["session-abc".to_string()];
        let plan = plan(vec![launched], "develop", GcOptions::default(), &[]);
        assert!(plan.candidates.is_empty(), "{plan:?}");
        let reason = kept_reason(&plan, "/work/issue-3");
        assert!(reason.starts_with("tracked launch"), "{reason}");
        assert!(reason.contains("session-abc"), "{reason}");
    }

    /// AC-3: an unmerged worktree is kept by default and the reason names
    /// the base and the flag that would include it.
    #[test]
    fn unmerged_worktree_is_kept_by_default() {
        let mut unmerged = probe("/work/issue-4");
        unmerged.merged = Ok(false);
        let plan = plan(vec![unmerged], "develop", GcOptions::default(), &[]);
        assert!(plan.candidates.is_empty(), "{plan:?}");
        let reason = kept_reason(&plan, "/work/issue-4");
        assert!(reason.contains("origin/develop"), "{reason}");
        assert!(reason.contains("include_unmerged"), "{reason}");
    }

    /// AC-3: `include_unmerged:true` makes the unmerged idle worktree a
    /// candidate.
    #[test]
    fn include_unmerged_reclaims_an_unmerged_idle_worktree() {
        let mut unmerged = probe("/work/issue-5");
        unmerged.merged = Ok(false);
        let plan = plan(vec![unmerged], "develop", both_opt_ins(), &[]);
        assert_eq!(plan.candidates.len(), 1, "{plan:?}");
    }

    /// AC-3: when the merge check itself failed the worktree is kept even
    /// with `include_unmerged`; an unreadable state is not "unmerged".
    #[test]
    fn unknown_merge_state_is_kept_with_the_error() {
        let mut unknown = probe("/work/issue-6");
        unknown.merged = Err("git rev-parse HEAD failed".to_string());
        let plan = plan(vec![unknown], "develop", both_opt_ins(), &[]);
        assert!(plan.candidates.is_empty(), "{plan:?}");
        let reason = kept_reason(&plan, "/work/issue-6");
        assert!(reason.contains("git rev-parse HEAD failed"), "{reason}");
    }

    /// The main worktree and protected roots (the calling worktree, the one
    /// hosting the running gwtd) are never candidates.
    #[test]
    fn main_and_protected_worktrees_are_kept() {
        let mut main = probe("/repo");
        main.is_main = true;
        let current = probe("/work/issue-7");
        let protected = vec![ProtectedRoot {
            root: PathBuf::from("/work/issue-7"),
            reason: "current worktree".to_string(),
        }];
        let plan = plan(vec![main, current], "develop", both_opt_ins(), &protected);
        assert!(plan.candidates.is_empty(), "{plan:?}");
        assert_eq!(kept_reason(&plan, "/repo"), "main worktree");
        assert_eq!(kept_reason(&plan, "/work/issue-7"), "current worktree");
    }

    /// A worktree without `target/` has nothing to reclaim and says so.
    #[test]
    fn worktree_without_build_artifacts_is_kept() {
        let mut clean = probe("/work/issue-8");
        clean.has_build_artifacts = false;
        let plan = plan(vec![clean], "develop", GcOptions::default(), &[]);
        assert!(plan.candidates.is_empty(), "{plan:?}");
        assert!(kept_reason(&plan, "/work/issue-8").contains("no target/"));
    }

    /// The shared `develop` workspace is merged into the base by definition,
    /// so nothing but this rule keeps the largest `target/` on the host out
    /// of an unqualified sweep.
    #[test]
    fn shared_base_branch_workspace_is_kept_by_default() {
        let mut shared = probe("/work/develop");
        shared.branch = Some("develop".to_string());
        shared.is_base_branch_workspace = true;

        let plan = plan(vec![shared], "develop", GcOptions::default(), &[]);

        assert!(plan.candidates.is_empty(), "{plan:?}");
        let reason = kept_reason(&plan, "/work/develop");
        assert!(reason.contains("shared base-branch workspace"), "{reason}");
        assert!(reason.contains("include_protected_workspaces"), "{reason}");
    }

    /// ...and an operator who understands the rebuild cost can still ask for
    /// it by name.
    #[test]
    fn include_protected_workspaces_reclaims_the_shared_workspace() {
        let mut shared = probe("/work/develop");
        shared.branch = Some("develop".to_string());
        shared.is_base_branch_workspace = true;

        let plan = plan(
            vec![shared],
            "develop",
            GcOptions {
                include_protected_workspaces: true,
                ..GcOptions::default()
            },
            &[],
        );

        assert_eq!(plan.candidates.len(), 1, "{plan:?}");
    }

    /// The opt-in widens exactly one rule: a shared workspace something is
    /// running in stays kept, with the process reason.
    #[test]
    fn include_protected_workspaces_never_overrides_a_running_process() {
        let mut shared = probe("/work/develop");
        shared.branch = Some("develop".to_string());
        shared.is_base_branch_workspace = true;
        shared.active_processes = vec!["cargo (pid 1)".to_string()];

        let plan = plan(vec![shared], "develop", both_opt_ins(), &[]);

        assert!(plan.candidates.is_empty(), "{plan:?}");
        assert!(kept_reason(&plan, "/work/develop").starts_with("active process"));
    }

    /// The process exclusion wins over every other reason: the operator must
    /// see why a merged worktree was not touched.
    #[test]
    fn active_process_reason_wins_over_unmerged() {
        let mut both = probe("/work/issue-9");
        both.active_processes = vec!["cargo.exe (pid 1)".to_string()];
        both.merged = Ok(false);
        let plan = plan(vec![both], "develop", GcOptions::default(), &[]);
        assert!(kept_reason(&plan, "/work/issue-9").starts_with("active process"));
    }

    /// AC-1 end to end on a real repository: a merged sibling worktree's
    /// `target/` is listed with its size by the dry run and removed by the
    /// apply run, while the calling (main) worktree keeps its own.
    #[test]
    fn gc_reports_then_removes_a_merged_sibling_worktree_target() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let _home = gwt_core::test_support::ScopedGwtHome::set(tmp.path().join("home"));
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        let git = |cwd: &Path, args: &[&str]| {
            let output = gwt_core::process::hidden_command("git")
                .args(args)
                .current_dir(cwd)
                .output()
                .expect("git");
            assert!(
                output.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        git(&repo, &["init", "-q", "-b", "develop"]);
        git(&repo, &["config", "user.email", "t@example.com"]);
        git(&repo, &["config", "user.name", "t"]);
        std::fs::write(repo.join("README.md"), "x").expect("write");
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "init"]);
        // `origin/develop` is what the sweep judges against; point it at the
        // same commit so the sibling worktree counts as merged.
        git(
            &repo,
            &["update-ref", "refs/remotes/origin/develop", "HEAD"],
        );
        let sibling = tmp.path().join("work").join("issue-1");
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "work/issue-1",
                sibling.to_str().expect("utf8"),
            ],
        );
        let sibling_target = sibling.join(BUILD_ARTIFACT_DIR).join("debug");
        std::fs::create_dir_all(&sibling_target).expect("target dir");
        std::fs::write(sibling_target.join("blob.bin"), vec![7u8; 4096]).expect("blob");
        std::fs::create_dir_all(repo.join(BUILD_ARTIFACT_DIR)).expect("main target");

        let mut env = crate::cli::TestEnv::new(repo.clone());
        let mut out = String::new();
        run(
            &mut env,
            WorktreeCommand::GcBuildArtifacts {
                dry_run: true,
                base: None,
                include_unmerged: false,
                include_protected_workspaces: false,
            },
            &mut out,
        )
        .expect("dry run");
        let report: serde_json::Value = serde_json::from_str(&out).expect("json: {out}");
        assert_eq!(report["dry_run"], true);
        assert_eq!(
            report["candidates"].as_array().map(Vec::len),
            Some(1),
            "{out}"
        );
        assert_eq!(report["candidates"][0]["bytes"], 4096, "{out}");
        assert_eq!(report["reclaimable_bytes"], 4096, "{out}");
        assert!(report["removed"].as_array().is_some_and(Vec::is_empty));
        assert!(
            report["kept"]
                .as_array()
                .is_some_and(|kept| kept.iter().any(|k| k["reason"] == "main worktree")),
            "{out}"
        );
        assert!(report["disk_space"]["volumes"].is_array(), "{out}");
        assert!(sibling_target.join("blob.bin").is_file());

        let mut out = String::new();
        run(
            &mut env,
            WorktreeCommand::GcBuildArtifacts {
                dry_run: false,
                base: Some("develop".to_string()),
                include_unmerged: false,
                include_protected_workspaces: false,
            },
            &mut out,
        )
        .expect("apply");
        let report: serde_json::Value = serde_json::from_str(&out).expect("json: {out}");
        assert_eq!(report["removed"].as_array().map(Vec::len), Some(1), "{out}");
        assert_eq!(report["reclaimed_bytes"], 4096, "{out}");
        assert!(!sibling.join(BUILD_ARTIFACT_DIR).exists());
        assert!(repo.join(BUILD_ARTIFACT_DIR).is_dir());
    }
}
