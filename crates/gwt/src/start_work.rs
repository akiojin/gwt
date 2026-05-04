use chrono::{DateTime, Utc};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

pub const START_WORK_BASE_BRANCH_CANDIDATES: [&str; 3] =
    ["origin/develop", "origin/main", "origin/master"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartWorkError {
    MissingBaseBranch,
    ReservationIo(String),
}

impl std::fmt::Display for StartWorkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingBaseBranch => f.write_str(
                "No default base branch found (origin/develop, origin/main, origin/master)",
            ),
            Self::ReservationIo(error) => {
                write!(f, "Failed to reserve Start Work branch name: {error}")
            }
        }
    }
}

impl std::error::Error for StartWorkError {}

pub fn resolve_start_work_base_branch_with(
    mut remote_branch_exists: impl FnMut(&str) -> bool,
) -> Result<String, StartWorkError> {
    START_WORK_BASE_BRANCH_CANDIDATES
        .iter()
        .copied()
        .find(|candidate| remote_branch_exists(candidate))
        .map(str::to_string)
        .ok_or(StartWorkError::MissingBaseBranch)
}

pub fn resolve_start_work_base_branch(repo_path: &Path) -> Result<String, StartWorkError> {
    resolve_start_work_base_branch_with(|candidate| {
        git_ref_exists(repo_path, &format!("refs/remotes/{candidate}"))
    })
}

pub fn reserve_start_work_branch_name_with(
    now: DateTime<Utc>,
    mut branch_exists: impl FnMut(&str) -> bool,
) -> String {
    let base = format!("work/{}", now.format("%Y%m%d-%H%M"));
    if !branch_exists(&base) {
        return base;
    }
    for suffix in 2usize.. {
        let candidate = format!("{base}-{suffix}");
        if !branch_exists(&candidate) {
            return candidate;
        }
    }
    unreachable!("unbounded suffix search should always return")
}

pub fn reserve_start_work_branch_name(repo_path: &Path, now: DateTime<Utc>) -> String {
    reserve_start_work_branch_name_with(now, |candidate| {
        git_ref_exists(repo_path, &format!("refs/heads/{candidate}"))
            || git_ref_exists(repo_path, &format!("refs/remotes/origin/{candidate}"))
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
        |candidate| {
            git_ref_exists(repo_path, &format!("refs/heads/{candidate}"))
                || git_ref_exists(repo_path, &format!("refs/remotes/origin/{candidate}"))
        },
        &reservations_dir,
    )
}

pub fn reserve_start_work_branch_name_with_reservations(
    now: DateTime<Utc>,
    mut branch_exists: impl FnMut(&str) -> bool,
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
        if branch_exists(&candidate) {
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

fn git_ref_exists(repo_path: &Path, ref_name: &str) -> bool {
    gwt_core::process::hidden_command("git")
        .args(["show-ref", "--verify", "--quiet", ref_name])
        .current_dir(repo_path)
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use chrono::{TimeZone, Utc};

    use super::{
        reserve_start_work_branch_name_with, reserve_start_work_branch_name_with_reservations,
        resolve_start_work_base_branch_with, StartWorkError,
    };

    #[test]
    fn start_work_base_branch_uses_develop_main_master_order() {
        let existing = HashSet::from(["origin/main".to_string(), "origin/master".to_string()]);
        let resolved =
            resolve_start_work_base_branch_with(|candidate| existing.contains(candidate))
                .expect("resolve base branch");

        assert_eq!(resolved, "origin/main");
    }

    #[test]
    fn start_work_base_branch_reports_recoverable_missing_base() {
        let error = resolve_start_work_base_branch_with(|_| false).expect_err("missing base");

        assert_eq!(error, StartWorkError::MissingBaseBranch);
    }

    #[test]
    fn start_work_branch_name_uses_timestamp_and_suffix_for_collisions() {
        let now = Utc.with_ymd_and_hms(2026, 5, 4, 12, 34, 0).unwrap();
        let existing = HashSet::from([
            "work/20260504-1234".to_string(),
            "work/20260504-1234-2".to_string(),
        ]);

        let branch =
            reserve_start_work_branch_name_with(now, |candidate| existing.contains(candidate));

        assert_eq!(branch, "work/20260504-1234-3");
    }

    #[test]
    fn start_work_branch_name_tracks_pending_reservations_without_git_refs() {
        let temp = tempfile::tempdir().expect("tempdir");
        let reservations_dir = temp.path().join("reservations");
        let now = Utc.with_ymd_and_hms(2026, 5, 4, 12, 34, 0).unwrap();

        let first =
            reserve_start_work_branch_name_with_reservations(now, |_| false, &reservations_dir)
                .expect("first reservation");
        let second =
            reserve_start_work_branch_name_with_reservations(now, |_| false, &reservations_dir)
                .expect("second reservation");

        assert_eq!(first, "work/20260504-1234");
        assert_eq!(second, "work/20260504-1234-2");
        assert!(
            reservations_dir.join("work").join("20260504-1234").exists(),
            "reservation should be stored without creating a Git ref"
        );
    }
}
