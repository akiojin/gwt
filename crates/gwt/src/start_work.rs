use chrono::{DateTime, Utc};
use std::path::Path;

pub const START_WORK_BASE_BRANCH_CANDIDATES: [&str; 3] =
    ["origin/develop", "origin/main", "origin/master"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartWorkError {
    MissingBaseBranch,
}

impl std::fmt::Display for StartWorkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingBaseBranch => f.write_str(
                "No default base branch found (origin/develop, origin/main, origin/master)",
            ),
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
        reserve_start_work_branch_name_with, resolve_start_work_base_branch_with, StartWorkError,
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
}
