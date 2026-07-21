use chrono::{DateTime, Utc};
use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

pub const START_WORK_BASE_BRANCH_CANDIDATES: [&str; 1] = ["origin/develop"];
pub const START_WORK_REMOTE_HEAD_REF: &str = "origin/HEAD";
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartWorkError {
    MissingBaseBranch,
    ReservationIo(String),
    Lookup(String),
}

impl std::fmt::Display for StartWorkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingBaseBranch => f.write_str("No default base branch found (origin/develop)"),
            Self::ReservationIo(error) => {
                write!(f, "Failed to reserve Start Work branch name: {error}")
            }
            Self::Lookup(error) => {
                write!(f, "Failed to look up existing Start Work refs: {error}")
            }
        }
    }
}

impl std::error::Error for StartWorkError {}

/// Resolve the canonical Start Work base branch (FR-PERF-001).
///
/// The closure is invoked **once** with every candidate in
/// [`START_WORK_BASE_BRANCH_CANDIDATES`] and must return the subset of
/// candidates that resolve to existing refs (or a `StartWorkError::Lookup`
/// on failure). The first candidate in the ordered list that appears in the
/// returned set wins. The wrapper [`resolve_start_work_base_branch`] supplies
/// a closure backed by [`gwt_git::list_existing_refs`], collapsing the four
/// historical `git show-ref` invocations into a single
/// `git for-each-ref`. This is the dominant cold-open cost on Windows where
/// `CreateProcess` and Defender real-time scanning add several hundred
/// milliseconds per spawn (SPEC-2014 Phase B).
pub fn resolve_start_work_base_branch_with(
    remote_branches_existing: impl FnOnce(&[&str]) -> Result<HashSet<String>, StartWorkError>,
) -> Result<String, StartWorkError> {
    let existing = remote_branches_existing(&START_WORK_BASE_BRANCH_CANDIDATES)?;
    START_WORK_BASE_BRANCH_CANDIDATES
        .iter()
        .copied()
        .find(|candidate| existing.contains(*candidate))
        .map(str::to_string)
        .ok_or(StartWorkError::MissingBaseBranch)
}

pub fn refallback_start_work_base_branch_with<E>(
    branch_name: &str,
    selected_base_branch: &str,
    mut remote_branch_exists: impl FnMut(&str) -> Result<bool, E>,
) -> Result<Option<String>, E> {
    if !is_start_work_branch_name(branch_name)
        || !START_WORK_BASE_BRANCH_CANDIDATES.contains(&selected_base_branch)
    {
        return Ok(None);
    }
    if remote_branch_exists(selected_base_branch)? {
        return Ok(Some(selected_base_branch.to_string()));
    }
    for candidate in START_WORK_BASE_BRANCH_CANDIDATES {
        if candidate == selected_base_branch {
            continue;
        }
        if remote_branch_exists(candidate)? {
            return Ok(Some(candidate.to_string()));
        }
    }
    Ok(None)
}

fn is_start_work_branch_name(branch_name: &str) -> bool {
    branch_name
        .strip_prefix("work/")
        .is_some_and(|name| !name.is_empty())
}

pub fn resolve_start_work_base_branch(repo_path: &Path) -> Result<String, StartWorkError> {
    let git_root = gwt_git::worktree::main_worktree_root(repo_path)
        .unwrap_or_else(|_| repo_path.to_path_buf());
    resolve_start_work_base_branch_in(&git_root)
}

/// Same as [`resolve_start_work_base_branch`] but skips the
/// `git rev-parse --git-common-dir` spawn by accepting a pre-resolved
/// `git_root`. Callers should pass the same value they already obtained
/// from [`gwt_git::worktree::main_worktree_root`] or a tab-level cache
/// (FR-PERF-003).
pub fn resolve_start_work_base_branch_in(git_root: &Path) -> Result<String, StartWorkError> {
    gwt_git::WorktreeManager::new(git_root)
        .prepare_start_work_remote_develop()
        .map_err(|error| StartWorkError::Lookup(error.to_string()))?;
    resolve_start_work_base_branch_with(|candidates| lookup_short_refs(git_root, candidates))
}

/// Resolve the existing local branch that Launch Agent should use as its base.
///
/// Normal repositories preserve their current branch. Container workspaces
/// resolve through their canonical main repository and prefer a checked-out
/// `develop`, then `main`, before falling back to the main repository's
/// symbolic `HEAD`. Every candidate must resolve to a commit.
pub fn resolve_launch_agent_base_branch(project_root: &Path) -> Result<String, String> {
    const NO_BRANCHES_ERROR: &str =
        "No branches exist in this repository; create an initial commit first";
    const NO_SELECTED_BRANCH_ERROR: &str = "Current branch is unavailable: repository has local \
        branches, but no current or checked-out develop/main base branch could be resolved";

    let existing_branch = |repo_path: &Path, branch: &str| -> Result<Option<String>, String> {
        let branch = normalize_launch_branch_name(branch);
        if branch.is_empty() || !local_branch_resolves_to_commit(repo_path, &branch)? {
            Ok(None)
        } else {
            Ok(Some(branch))
        }
    };
    let mut deferred_error = None;

    if is_git_worktree(project_root) {
        match symbolic_head_branch(project_root) {
            Ok(Some(branch)) => match existing_branch(project_root, &branch) {
                Ok(Some(branch)) => return Ok(branch),
                Ok(None) => {}
                Err(error) => {
                    deferred_error = Some(error);
                }
            },
            Ok(None) => {}
            Err(error) => deferred_error = Some(error),
        }
    }

    let remember_error = |slot: &mut Option<String>, error: String| {
        if slot.is_none() {
            *slot = Some(error);
        }
    };

    let main_repo_path = gwt_git::worktree::main_worktree_root(project_root)
        .map_err(|error| format!("Current branch is unavailable: {error}"))?;
    let worktrees = gwt_git::WorktreeManager::new(&main_repo_path)
        .list()
        .map_err(|error| format!("Current branch is unavailable: {error}"))?;
    for branch in ["develop", "main"] {
        if !has_usable_worktree_for_branch(&worktrees, branch) {
            continue;
        }
        match existing_branch(&main_repo_path, branch) {
            Ok(Some(branch)) => return Ok(branch),
            Ok(None) => {}
            Err(error) => remember_error(&mut deferred_error, error),
        }
    }

    match symbolic_head_branch(&main_repo_path) {
        Ok(Some(branch)) => match existing_branch(&main_repo_path, &branch) {
            Ok(Some(branch)) => return Ok(branch),
            Ok(None) => {}
            Err(error) => remember_error(&mut deferred_error, error),
        },
        Ok(None) => {}
        Err(error) => remember_error(&mut deferred_error, error),
    }

    match local_branches_resolving_to_commits(&main_repo_path, "refs/heads/") {
        Ok(branches) if branches.is_empty() => {
            if let Some(error) = deferred_error {
                Err(error)
            } else {
                Err(NO_BRANCHES_ERROR.to_string())
            }
        }
        Ok(_) => Err(deferred_error.unwrap_or_else(|| NO_SELECTED_BRANCH_ERROR.to_string())),
        Err(error) => Err(error),
    }
}

fn normalize_launch_branch_name(branch_name: &str) -> String {
    if let Some(name) = branch_name.strip_prefix("refs/remotes/") {
        return name.strip_prefix("origin/").unwrap_or(name).to_string();
    }
    if let Some(name) = branch_name.strip_prefix("origin/") {
        return name.to_string();
    }
    branch_name.to_string()
}

fn local_branch_resolves_to_commit(repo_path: &Path, branch_name: &str) -> Result<bool, String> {
    let ref_name = format!("refs/heads/{branch_name}");
    Ok(local_branches_resolving_to_commits(repo_path, &ref_name)?
        .iter()
        .any(|branch| branch == branch_name))
}

fn symbolic_head_branch(repo_path: &Path) -> Result<Option<String>, String> {
    let output = gwt_core::process::hidden_command("git")
        .args(["symbolic-ref", "--quiet", "--short", "HEAD"])
        .current_dir(repo_path)
        .output()
        .map_err(|error| format!("git symbolic-ref HEAD: {error}"))?;
    match output.status.code() {
        Some(0) => {
            let raw_branch = String::from_utf8_lossy(&output.stdout);
            let branch = normalize_launch_branch_name(raw_branch.trim());
            if branch.is_empty() {
                Err(format!(
                    "git symbolic-ref HEAD in {} returned an empty branch",
                    repo_path.display()
                ))
            } else {
                Ok(Some(branch))
            }
        }
        Some(1) => Ok(None),
        _ => Err(format!(
            "git symbolic-ref HEAD in {} failed with status {}: {}",
            repo_path.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )),
    }
}

fn local_branches_resolving_to_commits(
    repo_path: &Path,
    pattern: &str,
) -> Result<Vec<String>, String> {
    let format = "--format=%(refname)\t%(objecttype)\t%(*objecttype)";
    let output = gwt_core::process::hidden_command("git")
        .args(["for-each-ref", format, pattern])
        .current_dir(repo_path)
        .output()
        .map_err(|error| format!("git for-each-ref {pattern}: {error}"))?;
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !output.status.success() || !stderr.is_empty() {
        return Err(format!(
            "git for-each-ref {pattern} in {} failed with status {}: {}",
            repo_path.display(),
            output.status,
            stderr
        ));
    }

    let mut branches = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if line.trim().is_empty() {
            continue;
        }
        let mut fields = line.splitn(3, '\t');
        let (Some(ref_name), Some(object_type), Some(peeled_object_type)) =
            (fields.next(), fields.next(), fields.next())
        else {
            return Err(format!(
                "git for-each-ref {pattern} in {} returned malformed output",
                repo_path.display()
            ));
        };
        let Some(branch) = ref_name.strip_prefix("refs/heads/") else {
            return Err(format!(
                "git for-each-ref {pattern} in {} returned non-local ref {ref_name}",
                repo_path.display()
            ));
        };
        if object_type == "commit" || peeled_object_type == "commit" {
            branches.push(branch.to_string());
        }
    }
    Ok(branches)
}

fn is_git_worktree(repo_path: &Path) -> bool {
    let output = gwt_core::process::hidden_command("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(repo_path)
        .output();
    output.is_ok_and(|output| {
        output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "true"
    })
}

fn has_usable_worktree_for_branch(worktrees: &[gwt_git::WorktreeInfo], branch_name: &str) -> bool {
    worktrees.iter().any(|worktree| {
        worktree.branch.as_deref() == Some(branch_name)
            && !worktree.prunable
            && worktree.path.exists()
    })
}

/// Wrap [`gwt_git::list_existing_refs`] to translate short candidate names
/// (for example `origin/develop`) into the fully qualified refs needed by
/// `for-each-ref`, then translate the existing-set back to short names.
fn lookup_short_refs(
    repo_path: &Path,
    candidates: &[&str],
) -> Result<HashSet<String>, StartWorkError> {
    let qualified: Vec<String> = candidates
        .iter()
        .map(|candidate| remote_tracking_ref(candidate))
        .collect();
    let qualified_refs: Vec<&str> = qualified.iter().map(String::as_str).collect();
    let existing_full = gwt_git::list_existing_refs(repo_path, &qualified_refs)
        .map_err(|error| StartWorkError::Lookup(error.to_string()))?;
    Ok(candidates
        .iter()
        .filter(|candidate| existing_full.contains(&remote_tracking_ref(candidate)))
        .map(|candidate| (*candidate).to_string())
        .collect())
}

pub fn reserve_start_work_branch_name_with(
    now: DateTime<Utc>,
    mut branch_exists: impl FnMut(&str) -> Result<bool, StartWorkError>,
) -> Result<String, StartWorkError> {
    let base = format!("work/{}", now.format("%Y%m%d-%H%M"));
    if !branch_exists(&base)? {
        return Ok(base);
    }
    for suffix in 2usize.. {
        let candidate = format!("{base}-{suffix}");
        if !branch_exists(&candidate)? {
            return Ok(candidate);
        }
    }
    unreachable!("unbounded suffix search should always return")
}

pub fn reserve_start_work_branch_name(
    repo_path: &Path,
    now: DateTime<Utc>,
) -> Result<String, StartWorkError> {
    reserve_start_work_branch_name_with(now, |candidate| {
        local_or_remote_branch_exists(repo_path, candidate)
    })
}

pub fn reserve_start_work_branch_name_for_project(
    repo_path: &Path,
    now: DateTime<Utc>,
) -> Result<String, StartWorkError> {
    let git_root = gwt_git::worktree::main_worktree_root(repo_path)
        .unwrap_or_else(|_| repo_path.to_path_buf());
    let reservations_dir = gwt_core::paths::gwt_project_dir_for_repo_path(repo_path)
        .join("workspace")
        .join("start-work-reservations");
    reserve_start_work_branch_name_with_reservations(
        now,
        |candidate| local_or_remote_branch_exists(&git_root, candidate),
        &reservations_dir,
    )
}

pub fn reserve_start_work_branch_name_with_reservations(
    now: DateTime<Utc>,
    mut branch_exists: impl FnMut(&str) -> Result<bool, StartWorkError>,
    reservations_dir: &Path,
) -> Result<String, StartWorkError> {
    fs::create_dir_all(reservations_dir)
        .map_err(|error| StartWorkError::ReservationIo(error.to_string()))?;
    let base = format!("work/{}", now.format("%Y%m%d-%H%M"));
    for suffix in 1usize.. {
        let candidate = if suffix == 1 {
            base.clone()
        } else {
            format!("{base}-{suffix}")
        };
        if branch_exists(&candidate)? {
            continue;
        }
        match create_reservation(reservations_dir, &candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(StartWorkError::ReservationIo(error.to_string())),
        }
    }
    unreachable!("unbounded suffix search should always return")
}

/// Check whether either `refs/heads/<candidate>` or
/// `refs/remotes/origin/<candidate>` exists in `repo_path`, using a single
/// `git for-each-ref` invocation (FR-PERF-001). On Windows this halves the
/// number of `git.exe` spawns per Start Work candidate from two to one.
fn local_or_remote_branch_exists(
    repo_path: &Path,
    candidate: &str,
) -> Result<bool, StartWorkError> {
    let local = format!("refs/heads/{candidate}");
    let remote = format!("refs/remotes/origin/{candidate}");
    let existing = gwt_git::list_existing_refs(repo_path, &[local.as_str(), remote.as_str()])
        .map_err(|error| StartWorkError::Lookup(error.to_string()))?;
    Ok(!existing.is_empty())
}

fn create_reservation(reservations_dir: &Path, branch_name: &str) -> std::io::Result<()> {
    let path = reservation_path(reservations_dir, branch_name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(branch_name.as_bytes())?;
    file.sync_all()
}

fn reservation_path(reservations_dir: &Path, branch_name: &str) -> PathBuf {
    branch_name
        .split('/')
        .fold(reservations_dir.to_path_buf(), |path, segment| {
            path.join(segment)
        })
}

fn remote_tracking_ref(remote_ref: &str) -> String {
    if remote_ref.starts_with("refs/remotes/") {
        remote_ref.to_string()
    } else {
        format!("refs/remotes/{remote_ref}")
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::HashSet, fs, path::Path, rc::Rc};

    use chrono::{TimeZone, Utc};

    use super::{
        refallback_start_work_base_branch_with, remote_tracking_ref,
        reserve_start_work_branch_name_with, reserve_start_work_branch_name_with_reservations,
        resolve_start_work_base_branch_with, StartWorkError, START_WORK_BASE_BRANCH_CANDIDATES,
    };

    fn ok_existing(existing: &HashSet<String>) -> HashSet<String> {
        existing.clone()
    }

    fn run_git(cwd: &Path, args: &[&str]) {
        let output = gwt_core::process::hidden_command("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {args:?} in {} failed: {}",
            cwd.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn init_committed_repo(repo: &Path, branch: &str) {
        fs::create_dir_all(repo).expect("create repository");
        run_git(repo, &["init", "-q", "-b", branch]);
        run_git(repo, &["config", "user.name", "Test User"]);
        run_git(repo, &["config", "user.email", "test@example.com"]);
        fs::write(repo.join("README.md"), "fixture\n").expect("write fixture");
        run_git(repo, &["add", "README.md"]);
        run_git(repo, &["commit", "-qm", "fixture"]);
    }

    #[test]
    fn launch_agent_base_branch_public_api_preserves_normal_current_branch() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        init_committed_repo(&repo, "feature/current");

        assert_eq!(
            crate::start_work::resolve_launch_agent_base_branch(&repo),
            Ok("feature/current".to_string())
        );
    }

    #[test]
    fn start_work_base_branch_uses_develop() {
        let existing = HashSet::from([
            "origin/HEAD".to_string(),
            "origin/develop".to_string(),
            "origin/main".to_string(),
        ]);
        let resolved = resolve_start_work_base_branch_with(|_| Ok(ok_existing(&existing)))
            .expect("resolve base branch");

        assert_eq!(resolved, "origin/develop");
    }

    #[test]
    fn start_work_base_branch_reports_missing_when_develop_is_missing() {
        let existing = HashSet::from(["origin/HEAD".to_string(), "origin/main".to_string()]);
        let error = resolve_start_work_base_branch_with(|_| Ok(ok_existing(&existing)))
            .expect_err("missing base");

        assert_eq!(error, StartWorkError::MissingBaseBranch);
    }

    #[test]
    fn start_work_base_branch_does_not_refallback_after_selected_develop_is_pruned() {
        let existing = HashSet::from(["origin/HEAD".to_string(), "origin/main".to_string()]);
        let resolved = refallback_start_work_base_branch_with(
            "work/20260507-0734",
            "origin/develop",
            |candidate| Ok::<_, std::convert::Infallible>(existing.contains(candidate)),
        )
        .expect("refallback");

        assert!(resolved.is_none());
    }

    #[test]
    fn start_work_base_branch_refallback_preserves_non_start_work_base_errors() {
        let existing = HashSet::from(["origin/HEAD".to_string(), "origin/main".to_string()]);
        let resolved =
            refallback_start_work_base_branch_with("feature/demo", "origin/develop", |candidate| {
                Ok::<_, std::convert::Infallible>(existing.contains(candidate))
            })
            .expect("refallback");

        assert!(resolved.is_none());
    }

    #[test]
    fn start_work_base_branch_ignores_main_and_master() {
        let existing = HashSet::from(["origin/main".to_string(), "origin/master".to_string()]);
        let error = resolve_start_work_base_branch_with(|_| Ok(ok_existing(&existing)))
            .expect_err("missing base");

        assert_eq!(error, StartWorkError::MissingBaseBranch);
    }

    #[test]
    fn start_work_base_branch_reports_recoverable_missing_base() {
        let error =
            resolve_start_work_base_branch_with(|_| Ok(HashSet::new())).expect_err("missing base");

        assert_eq!(error, StartWorkError::MissingBaseBranch);
    }

    #[test]
    fn start_work_base_branch_invokes_lookup_only_once_for_all_candidates() {
        let calls = Rc::new(RefCell::new(0usize));
        let captured: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let calls_clone = Rc::clone(&calls);
        let captured_clone = Rc::clone(&captured);
        let existing = HashSet::from(["origin/develop".to_string()]);

        let resolved = resolve_start_work_base_branch_with(|candidates| {
            *calls_clone.borrow_mut() += 1;
            *captured_clone.borrow_mut() = candidates.iter().map(|c| (*c).to_string()).collect();
            Ok(existing.clone())
        })
        .expect("resolve base branch");

        assert_eq!(*calls.borrow(), 1, "bulk lookup must run exactly once");
        assert_eq!(
            captured.borrow().as_slice(),
            START_WORK_BASE_BRANCH_CANDIDATES
                .iter()
                .map(|c| (*c).to_string())
                .collect::<Vec<_>>()
                .as_slice(),
            "the closure must receive every Start Work candidate in order"
        );
        assert_eq!(resolved, "origin/develop");
    }

    #[test]
    fn start_work_base_branch_propagates_lookup_errors() {
        let error = resolve_start_work_base_branch_with(|_| {
            Err(StartWorkError::Lookup("git crashed".into()))
        })
        .expect_err("must surface lookup failures");

        assert_eq!(error, StartWorkError::Lookup("git crashed".into()));
    }

    #[test]
    fn start_work_remote_tracking_ref_does_not_double_origin_prefix() {
        assert_eq!(
            remote_tracking_ref("origin/develop"),
            "refs/remotes/origin/develop"
        );
    }

    #[test]
    fn start_work_branch_name_uses_timestamp_and_suffix_for_collisions() {
        let now = Utc.with_ymd_and_hms(2026, 5, 4, 12, 34, 0).unwrap();
        let existing = HashSet::from([
            "work/20260504-1234".to_string(),
            "work/20260504-1234-2".to_string(),
        ]);

        let branch =
            reserve_start_work_branch_name_with(now, |candidate| Ok(existing.contains(candidate)))
                .expect("reserve");

        assert_eq!(branch, "work/20260504-1234-3");
    }

    #[test]
    fn start_work_branch_name_tracks_pending_reservations_without_git_refs() {
        let temp = tempfile::tempdir().expect("tempdir");
        let reservations_dir = temp.path().join("reservations");
        let now = Utc.with_ymd_and_hms(2026, 5, 4, 12, 34, 0).unwrap();

        let first =
            reserve_start_work_branch_name_with_reservations(now, |_| Ok(false), &reservations_dir)
                .expect("first reservation");
        let second =
            reserve_start_work_branch_name_with_reservations(now, |_| Ok(false), &reservations_dir)
                .expect("second reservation");

        assert_eq!(first, "work/20260504-1234");
        assert_eq!(second, "work/20260504-1234-2");
        assert!(
            reservations_dir.join("work").join("20260504-1234").exists(),
            "reservation should be stored without creating a Git ref"
        );
    }

    #[test]
    fn reserve_branch_invokes_lookup_only_once_per_candidate() {
        let temp = tempfile::tempdir().expect("tempdir");
        let reservations_dir = temp.path().join("reservations");
        let now = Utc.with_ymd_and_hms(2026, 5, 4, 12, 34, 0).unwrap();
        let calls = Rc::new(RefCell::new(0usize));
        let calls_clone = Rc::clone(&calls);

        let branch = reserve_start_work_branch_name_with_reservations(
            now,
            |_candidate| {
                *calls_clone.borrow_mut() += 1;
                Ok(false)
            },
            &reservations_dir,
        )
        .expect("reserve");

        assert_eq!(branch, "work/20260504-1234");
        assert_eq!(
            *calls.borrow(),
            1,
            "lookup must run exactly once for the first available candidate"
        );
    }

    #[test]
    fn reserve_branch_propagates_lookup_errors() {
        let temp = tempfile::tempdir().expect("tempdir");
        let reservations_dir = temp.path().join("reservations");
        let now = Utc.with_ymd_and_hms(2026, 5, 4, 12, 34, 0).unwrap();

        let error = reserve_start_work_branch_name_with_reservations(
            now,
            |_| Err(StartWorkError::Lookup("git crashed".into())),
            &reservations_dir,
        )
        .expect_err("must surface lookup failures");

        assert_eq!(error, StartWorkError::Lookup("git crashed".into()));
    }
}
