use chrono::{DateTime, Utc};
use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

pub const START_WORK_BASE_BRANCH_CANDIDATES: [&str; 4] = [
    "origin/develop",
    START_WORK_REMOTE_HEAD_REF,
    "origin/main",
    "origin/master",
];
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
            Self::MissingBaseBranch => f.write_str(
                "No default base branch found (origin/develop, origin/HEAD, origin/main, origin/master)",
            ),
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
    resolve_start_work_base_branch_with(|candidates| lookup_short_refs(git_root, candidates))
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
    let reservations_dir = gwt_core::paths::gwt_project_dir_for_repo_path(repo_path)
        .join("workspace")
        .join("start-work-reservations");
    reserve_start_work_branch_name_with_reservations(
        now,
        |candidate| local_or_remote_branch_exists(repo_path, candidate),
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
    use std::{cell::RefCell, collections::HashSet, rc::Rc};

    use chrono::{TimeZone, Utc};

    use super::{
        refallback_start_work_base_branch_with, remote_tracking_ref,
        reserve_start_work_branch_name_with, reserve_start_work_branch_name_with_reservations,
        resolve_start_work_base_branch_with, StartWorkError, START_WORK_BASE_BRANCH_CANDIDATES,
    };

    fn ok_existing(existing: &HashSet<String>) -> HashSet<String> {
        existing.clone()
    }

    #[test]
    fn start_work_base_branch_prefers_develop_before_remote_head() {
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
    fn start_work_base_branch_uses_remote_head_when_develop_is_missing() {
        let existing = HashSet::from(["origin/HEAD".to_string(), "origin/main".to_string()]);
        let resolved = resolve_start_work_base_branch_with(|_| Ok(ok_existing(&existing)))
            .expect("resolve base branch");

        assert_eq!(resolved, "origin/HEAD");
    }

    #[test]
    fn start_work_base_branch_refalls_back_after_selected_develop_is_pruned() {
        let existing = HashSet::from(["origin/HEAD".to_string(), "origin/main".to_string()]);
        let resolved = refallback_start_work_base_branch_with(
            "work/20260507-0734",
            "origin/develop",
            |candidate| Ok::<_, std::convert::Infallible>(existing.contains(candidate)),
        )
        .expect("refallback")
        .expect("fallback base");

        assert_eq!(resolved, "origin/HEAD");
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
    fn start_work_base_branch_falls_back_to_develop_main_master_order() {
        let existing = HashSet::from(["origin/main".to_string(), "origin/master".to_string()]);
        let resolved = resolve_start_work_base_branch_with(|_| Ok(ok_existing(&existing)))
            .expect("resolve base branch");

        assert_eq!(resolved, "origin/main");
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
            remote_tracking_ref("origin/HEAD"),
            "refs/remotes/origin/HEAD"
        );
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
