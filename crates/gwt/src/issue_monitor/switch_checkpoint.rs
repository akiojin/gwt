//! Host-side checkpoint observations. None of these observations stops a runtime.
use std::{
    io,
    path::{Path, PathBuf},
};

/// One process census; missing cwd information never establishes quiescence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSwitchCwdObservation {
    pub matching_pids: Vec<u32>,
    pub unknown_pids: Vec<u32>,
    pub complete: bool,
}

impl AgentSwitchCwdObservation {
    pub fn is_clear(&self) -> bool {
        self.complete && self.matching_pids.is_empty() && self.unknown_pids.is_empty()
    }
}

fn collect_cwd_observation(
    worktree: &Path,
    processes: impl IntoIterator<Item = (u32, Option<PathBuf>)>,
    (enumerated_count, captured_count, supported_with_self): (usize, usize, bool),
) -> AgentSwitchCwdObservation {
    let mut observation = AgentSwitchCwdObservation {
        matching_pids: Vec::new(),
        unknown_pids: Vec::new(),
        complete: supported_with_self && enumerated_count == captured_count,
    };
    let Ok(worktree) = dunce::canonicalize(worktree) else {
        observation.complete = false;
        return observation;
    };
    for (pid, cwd) in processes {
        match cwd.and_then(|cwd| dunce::canonicalize(cwd).ok()) {
            Some(cwd) if cwd.starts_with(&worktree) => observation.matching_pids.push(pid),
            Some(_) => {}
            None => observation.unknown_pids.push(pid),
        }
    }
    observation.matching_pids.sort_unstable();
    observation.unknown_pids.sort_unstable();
    observation
}

/// Observe all live process cwd values, including the source provider itself.
/// This is a cwd observation, not filesystem containment or a process freeze.
pub fn observe_agent_switch_cwd(worktree: &Path) -> AgentSwitchCwdObservation {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System, UpdateKind};
    let mut system = System::new();
    let enumerated_count = system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_cwd(UpdateKind::Always),
    );
    // sysinfo can enumerate a PID but omit its inaccessible process record.
    // Missing records or an unsupported/failed census cannot prove quiescence.
    let supported_with_self =
        sysinfo::IS_SUPPORTED_SYSTEM && system.process(Pid::from_u32(std::process::id())).is_some();
    collect_cwd_observation(
        worktree,
        system
            .processes()
            .iter()
            .filter(|(_, process)| {
                !matches!(
                    process.status(),
                    ProcessStatus::Dead | ProcessStatus::Zombie
                )
            })
            .map(|(pid, process)| (pid.as_u32(), process.cwd().map(Path::to_path_buf))),
        (
            enumerated_count,
            system.processes().len(),
            supported_with_self,
        ),
    )
}

fn contains_operation(text: Option<&str>, operation_id: &str) -> bool {
    text.is_some_and(|text| text.split_whitespace().any(|word| word == operation_id))
}

/// Validate continuation evidence against the existing canonical source Work.
/// The caller owns authentication and must lease the captured source identity.
pub fn verify_switch_continuation(
    worktree: &Path,
    session_id: &str,
    operation_id: &str,
    work_id: &str,
    board_entry_id: &str,
) -> io::Result<()> {
    let invalid =
        || io::Error::other("checkpoint continuation does not match the source Work and operation");
    if !uuid::Uuid::parse_str(operation_id)
        .is_ok_and(|id| id.hyphenated().to_string() == operation_id)
    {
        return Err(invalid());
    }
    let target =
        crate::agent_project_state::resolve_session_work_mutation_target(worktree, session_id)
            .map_err(io::Error::other)?;
    if target.work_id != work_id {
        return Err(invalid());
    }
    let owner = target.owner.as_deref().ok_or_else(invalid)?;
    let owner_number = owner
        .strip_prefix("Issue #")
        .or_else(|| owner.strip_prefix("SPEC-"))
        .ok_or_else(invalid)?;
    let board =
        gwt_core::coordination::load_board_entry(&target.project_state_root, board_entry_id)
            .map_err(io::Error::other)?
            .ok_or_else(invalid)?;
    if board.origin_session_id.as_deref() != Some(session_id)
        || !board.related_owners.iter().any(|candidate| {
            candidate == owner || candidate.trim_start_matches('#') == owner_number
        })
        || !contains_operation(Some(&board.body), operation_id)
    {
        return Err(invalid());
    }
    let works =
        gwt_core::workspace_projection::load_workspace_work_items(&target.project_state_root)
            .map_err(io::Error::other)?
            .ok_or_else(invalid)?;
    let work = works
        .work_items
        .iter()
        .find(|work| work.id == work_id)
        .ok_or_else(invalid)?;
    if work.status_category != gwt_core::workspace_projection::WorkspaceStatusCategory::Idle
        || !work.board_refs.iter().any(|id| id == board_entry_id)
    {
        return Err(invalid());
    }
    let event = work
        .events
        .iter()
        .rev()
        .find(|event| {
            event.agent_session_id.as_deref() == Some(session_id)
                && event.owner.as_deref() == Some(owner)
                && event.agent_id.as_deref() == Some(target.agent_id.as_str())
                && contains_operation(event.summary.as_deref(), operation_id)
                && event
                    .progress_summary
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty())
                && event
                    .next_action
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty())
        })
        .ok_or_else(invalid)?;
    if !gwt_core::workspace_projection::workspace_work_event_shard_matches(
        &target.work_event_root,
        event,
    )
    .map_err(io::Error::other)?
    {
        return Err(invalid());
    }
    Ok(())
}

fn git(worktree: &Path, args: &[&str]) -> io::Result<String> {
    let output = gwt_core::process::run_git_logged(args, Some(worktree))?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "checkpoint Git observation failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    String::from_utf8(output.stdout)
        .map(|text| text.trim().to_owned())
        .map_err(io::Error::other)
}

/// Check the actual remote ref, rather than trusting a cached tracking ref.
pub fn verify_switch_git_checkpoint(worktree: &Path, expected_head: &str) -> io::Result<()> {
    if !matches!(expected_head.len(), 40 | 64)
        || !expected_head.bytes().all(|byte| byte.is_ascii_hexdigit())
        || git(worktree, &["rev-parse", "HEAD"])? != expected_head
    {
        return Err(io::Error::other(
            "checkpoint HEAD changed; save and acknowledge again",
        ));
    }
    if !git(
        worktree,
        &["status", "--porcelain=v1", "--untracked-files=all"],
    )?
    .is_empty()
    {
        return Err(io::Error::other(
            "checkpoint requires a clean worktree, including untracked files",
        ));
    }
    let branch = git(worktree, &["symbolic-ref", "--quiet", "--short", "HEAD"])?;
    let remote = git(
        worktree,
        &["config", "--get", &format!("branch.{branch}.remote")],
    )?;
    let merge = git(
        worktree,
        &["config", "--get", &format!("branch.{branch}.merge")],
    )?;
    if remote.is_empty() || remote == "." || !merge.starts_with("refs/heads/") {
        return Err(io::Error::other(
            "checkpoint requires a pushed upstream branch",
        ));
    }
    let observed = git(worktree, &["ls-remote", "--exit-code", &remote, &merge])?;
    if !observed.lines().any(|line| {
        line.split_once('\t')
            .is_some_and(|(sha, reference)| sha == expected_head && reference == merge)
    }) {
        return Err(io::Error::other(
            "checkpoint HEAD is absent from the live upstream; push before acknowledging",
        ));
    }
    // Read again after the network operation; an observation is never a write lease.
    if git(worktree, &["rev-parse", "HEAD"])? != expected_head
        || !git(
            worktree,
            &["status", "--porcelain=v1", "--untracked-files=all"],
        )?
        .is_empty()
    {
        return Err(io::Error::other(
            "checkpoint HEAD or clean state changed during upstream verification",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switch_cwd_collector_retains_unreadable_and_uncanonicalizable_processes() {
        let root = tempfile::tempdir().unwrap();
        let nested = root.path().join("nested");
        std::fs::create_dir(&nested).unwrap();
        let observation = collect_cwd_observation(
            root.path(),
            [
                (11, Some(root.path().to_path_buf())),
                (12, Some(nested)),
                (13, None),
                (14, Some(root.path().join("missing"))),
            ],
            (4, 4, true),
        );
        assert_eq!(observation.matching_pids, [11, 12]);
        assert_eq!(observation.unknown_pids, [13, 14]);
        assert!(!observation.is_clear());
        assert!(collect_cwd_observation(root.path(), [], (0, 0, true)).is_clear());
        assert!(!collect_cwd_observation(root.path(), [], (0, 0, false)).is_clear());
        assert!(
            !collect_cwd_observation(root.path(), [], (1, 0, true)).is_clear(),
            "a PID omitted from the captured process map makes the census incomplete"
        );
    }
}
