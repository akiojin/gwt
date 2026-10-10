//! Bounded retention for safely settled machine-local Session history.

use std::{
    collections::{BTreeSet, HashMap},
    fs, io,
    path::Path,
};

use chrono::{DateTime, Duration, Utc};
use gwt::cli::execution_state::{
    self, ExecutionControlStatus, ExecutionOwnerKey, ExecutionOwnerKind,
};
use gwt_agent::{AgentStatus, Session, SessionExecutionIdentity};

const RETENTION_DAYS: i64 = 30;

#[derive(Debug, Default)]
pub(crate) struct RetentionStats {
    pub sessions_pruned: usize,
    pub temporary_files_pruned: usize,
}

pub(crate) fn prune_session_ledger(
    sessions_dir: &Path,
    now: DateTime<Utc>,
) -> io::Result<RetentionStats> {
    let entries = match fs::read_dir(sessions_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(RetentionStats::default())
        }
        Err(error) => return Err(error),
    };
    let saved_windows = saved_window_session_ids()?;
    let sessions: HashMap<_, _> = gwt_agent::session_ledger::load_sessions(sessions_dir)?
        .into_iter()
        .map(|session| (session.id.clone(), session))
        .collect();
    let mut stats = RetentionStats::default();
    for entry in entries {
        let Ok(entry) = entry else { continue };
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_file() {
            continue;
        }
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if is_session_temporary_file(name) {
            if prune_temporary_file(&path, now).unwrap_or(false) {
                stats.temporary_files_pruned += 1;
            }
            continue;
        }
        if name.starts_with('.') || path.extension().is_none_or(|value| value != "toml") {
            continue;
        }
        let Some(session) = path
            .file_stem()
            .and_then(|value| value.to_str())
            .and_then(|id| sessions.get(id))
        else {
            continue;
        };
        if path.file_stem().and_then(|value| value.to_str()) != Some(session.id.as_str())
            || !expired_stopped_session(session, now)
            || saved_windows.contains(&session.id)
        {
            continue;
        }
        let result = match SessionExecutionIdentity::from_session(session) {
            Ok(Some(identity)) => {
                let Some(owner) = session_owner(session) else {
                    continue;
                };
                // The existing cleanup primitive takes owner -> Session leases;
                // the callback rechecks age, runtime, and authority under both.
                execution_state::remove_exact_session_with_owner_lease(
                    &session.worktree_path,
                    owner,
                    sessions_dir,
                    &identity,
                    || {
                        let current = Session::load(&path)?;
                        if !safe_to_prune(&current, sessions_dir, now, &saved_windows)? {
                            return Err(io::Error::new(
                                io::ErrorKind::PermissionDenied,
                                "Session remains needed",
                            ));
                        }
                        Ok(())
                    },
                )
            }
            Ok(None) => gwt_agent::with_session_lease(sessions_dir, &session.id, |current| {
                if current.execution_binding.is_some()
                    || !safe_to_prune(current, sessions_dir, now, &saved_windows)?
                {
                    return Ok(false);
                }
                fs::remove_file(&path)?;
                Ok(true)
            }),
            Err(_) => continue,
        };
        if matches!(result, Ok(true)) {
            stats.sessions_pruned += 1;
        }
    }
    Ok(stats)
}

fn expired_stopped_session(session: &Session, now: DateTime<Utc>) -> bool {
    let latest = [
        Some(session.created_at),
        Some(session.updated_at),
        Some(session.last_activity_at),
        session.last_hook_event_at,
        session.last_completed_stop_at,
        session.last_exited_at,
    ]
    .into_iter()
    .flatten()
    .max()
    .unwrap_or(now);
    session.status == AgentStatus::Stopped
        && !session.restore_window_on_startup
        && latest < now - Duration::days(RETENTION_DAYS)
}

fn safe_to_prune(
    session: &Session,
    sessions_dir: &Path,
    now: DateTime<Utc>,
    saved_windows: &BTreeSet<String>,
) -> io::Result<bool> {
    if !expired_stopped_session(session, now)
        || saved_windows.contains(&session.id)
        || path_present(&gwt_agent::active_launch_handshake_path(
            sessions_dir,
            &session.id,
        ))?
        || path_present(&gwt_agent::manual_handoff_path(sessions_dir, &session.id))?
        || has_runtime_sidecar(sessions_dir, &session.id)?
    {
        return Ok(false);
    }
    let project_root = session
        .project_state_root
        .as_deref()
        .unwrap_or(&session.worktree_path);
    if let Some(hash) = &session.repo_hash {
        let Ok(hash) = gwt_core::repo_hash::RepoHash::parse(hash) else {
            return Ok(false);
        };
        // A moved/deleted worktree cannot resolve the original recovery scope.
        // Keep it when that scope contains recovery evidence rather than guess.
        if gwt_core::paths::project_scope_hash(project_root) != hash
            && path_present(&gwt_core::paths::gwt_project_dir(&hash).join("recovery"))?
        {
            return Ok(false);
        }
    }
    match gwt_core::recovery::RecoveryStore::open_for_repo_if_exists(project_root, &session.id) {
        Ok(Some(store))
            if store
                .has_no_records()
                .map_err(|_| io::Error::other("Recovery storage unreadable"))? => {}
        Ok(None) => {}
        Ok(Some(_)) | Err(_) => return Ok(false),
    }
    execution_is_settled_for(session)
}

fn has_runtime_sidecar(sessions_dir: &Path, session_id: &str) -> io::Result<bool> {
    let namespaces = match fs::read_dir(sessions_dir.join("runtime")) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    for namespace in namespaces {
        let namespace = namespace?;
        if !namespace.file_type()?.is_dir() {
            return Ok(true);
        }
        if path_present(&namespace.path().join(format!("{session_id}.json")))? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn path_present(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn session_owner(session: &Session) -> Option<ExecutionOwnerKey> {
    let binding = session.execution_binding.as_ref()?;
    let kind = match binding.owner_kind.as_str() {
        "issue" => ExecutionOwnerKind::Issue,
        "spec" => ExecutionOwnerKind::Spec,
        _ => return None,
    };
    Some(ExecutionOwnerKey {
        kind,
        number: binding.owner_number,
    })
}

fn execution_is_settled_for(session: &Session) -> io::Result<bool> {
    if let Some(owner) = session_owner(session) {
        let Some(ledger) =
            execution_state::load_owner_generation_ledger(&session.worktree_path, owner)?
        else {
            return Ok(false);
        };
        let binding = session
            .execution_binding
            .as_ref()
            .expect("owner has binding");
        if !ledger
            .generations
            .iter()
            .any(|generation| generation.identity.generation_id == binding.identity.generation_id)
            || ledger.continuation_attempts.iter().any(|attempt| {
                attempt.status == execution_state::ContinuationAttemptStatus::Prepared
            })
            || ledger.takeover_attempts.iter().any(|attempt| {
                attempt.status == execution_state::GenerationTakeoverAttemptStatus::Prepared
            })
        {
            return Ok(false);
        }
        let Some(current) = ledger.current_generation() else {
            return Ok(false);
        };
        return Ok(
            current.identity.generation_id != binding.identity.generation_id
                || ledger.current_effective_status() == Some(ExecutionControlStatus::Completed),
        );
    }
    match execution_state::load(&session.worktree_path)? {
        Some(record) if record.primary_session_id == session.id => Ok(record.status
            == ExecutionControlStatus::Completed
            && execution_state::integrity_ok(&record)),
        Some(_) => Ok(session.linked_issue_number.is_none()),
        None => Ok(session.linked_issue_number.is_none()),
    }
}

fn saved_window_session_ids() -> io::Result<BTreeSet<String>> {
    let projects = match fs::read_dir(gwt_core::paths::gwt_projects_dir()) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(BTreeSet::new()),
        Err(error) => return Err(error),
    };
    let mut ids = BTreeSet::new();
    for project in projects {
        let project = project?;
        if !project.file_type()?.is_dir() {
            continue;
        }
        let workspace =
            gwt::persistence::load_workspace_state(&project.path().join("workspace.json"))?;
        ids.extend(
            workspace
                .windows
                .into_iter()
                .filter_map(|window| window.session_id),
        );
    }
    Ok(ids)
}

fn is_session_temporary_file(name: &str) -> bool {
    (name.starts_with(".tmp") && name.ends_with(".toml"))
        || (name.starts_with('.') && name.contains(".toml.tmp-"))
}

fn prune_temporary_file(path: &Path, now: DateTime<Utc>) -> io::Result<bool> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || DateTime::<Utc>::from(metadata.modified()?) >= now - Duration::days(RETENTION_DAYS)
    {
        return Ok(false);
    }
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if let Some((_, suffix)) = name.split_once(".toml.tmp-") {
        let Some(pid) = suffix
            .split('-')
            .next()
            .and_then(|value| value.parse::<u32>().ok())
        else {
            return Ok(false);
        };
        if gwt::process::is_process_alive(pid) {
            return Ok(false);
        }
    }
    fs::remove_file(path)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use chrono::{DateTime, Duration, Utc};
    use gwt_agent::{AgentId, AgentStatus, Session};
    use gwt_core::test_support::ScopedGwtHome;

    use super::prune_session_ledger;

    fn old_session(sessions: &Path, worktree: &Path, id: &str, now: DateTime<Utc>) -> Session {
        let mut session = Session::new(worktree, "develop", AgentId::Codex);
        session.id = id.to_string();
        session.status = AgentStatus::Stopped;
        session.restore_window_on_startup = false;
        session.created_at = now - Duration::days(31);
        session.updated_at = session.created_at;
        session.last_activity_at = session.created_at;
        session.save(sessions).unwrap();
        session
    }

    #[test]
    fn expires_only_old_stopped_history_and_keeps_recovery_and_runtime_references() {
        let dir = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(dir.path().join("home"));
        let sessions = dir.path().join("sessions");
        let worktree = dir.path().join("worktree");
        fs::create_dir_all(&worktree).unwrap();
        let now = Utc::now();
        old_session(&sessions, &worktree, "expired", now);
        for (id, status, restore) in [
            ("running", AgentStatus::Running, false),
            ("interrupted", AgentStatus::Interrupted, false),
            ("restore", AgentStatus::Stopped, true),
        ] {
            let mut session = old_session(&sessions, &worktree, id, now);
            session.status = status;
            session.restore_window_on_startup = restore;
            session.save(&sessions).unwrap();
        }
        let mut recent = old_session(&sessions, &worktree, "recent", now);
        recent.updated_at = now;
        recent.save(&sessions).unwrap();
        for (id, path) in [
            (
                "runtime",
                gwt_agent::runtime_state_path_for_pid(&sessions, 12345, "runtime"),
            ),
            (
                "launch",
                gwt_agent::active_launch_handshake_path(&sessions, "launch"),
            ),
            (
                "handoff",
                gwt_agent::manual_handoff_path(&sessions, "handoff"),
            ),
        ] {
            old_session(&sessions, &worktree, id, now);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "unreadable evidence must be retained").unwrap();
        }
        fs::write(sessions.join("broken.toml"), "broken = [").unwrap();

        prune_session_ledger(&sessions, now).unwrap();

        assert!(!sessions.join("expired.toml").exists());
        for id in [
            "running",
            "interrupted",
            "restore",
            "recent",
            "runtime",
            "launch",
            "handoff",
            "broken",
        ] {
            assert!(sessions.join(format!("{id}.toml")).exists(), "{id}");
        }
        assert!(sessions.join(".expired.lock").exists());
        assert!(gwt_agent::runtime_state_path_for_pid(&sessions, 12345, "runtime").exists());
    }

    #[test]
    fn keeps_saved_windows_and_unreadable_recovery_storage() {
        let dir = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(dir.path().join("home"));
        let sessions = dir.path().join("sessions");
        let worktree = dir.path().join("worktree");
        fs::create_dir_all(&worktree).unwrap();
        let now = Utc::now();
        old_session(&sessions, &worktree, "saved-window", now);
        let mut workspace = gwt::persistence::default_workspace_state();
        workspace.windows[0].session_id = Some("saved-window".to_string());
        gwt::persistence::save_workspace_state(
            &gwt::persistence::workspace_state_path(&worktree),
            &workspace,
        )
        .unwrap();
        prune_session_ledger(&sessions, now).unwrap();
        assert!(sessions.join("saved-window.toml").exists());

        old_session(&sessions, &worktree, "recovery", now);
        let recovery =
            gwt_core::paths::gwt_project_dir_for_repo_path(&worktree).join("recovery/intents");
        fs::create_dir_all(recovery.parent().unwrap()).unwrap();
        fs::write(&recovery, "unreadable recovery directory").unwrap();
        prune_session_ledger(&sessions, now).unwrap();
        assert!(sessions.join("recovery.toml").exists());

        fs::remove_file(&recovery).unwrap();
        gwt_core::recovery::RecoveryStore::for_repo(&worktree, "recovery").unwrap();
        let authority_root = fs::read_dir(&recovery)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let incomplete_record = authority_root.join("a".repeat(64));
        fs::create_dir(&incomplete_record).unwrap();
        old_session(&sessions, &worktree, "empty-recovery", now);
        gwt_core::recovery::RecoveryStore::for_repo(&worktree, "empty-recovery").unwrap();

        prune_session_ledger(&sessions, now).unwrap();
        assert!(sessions.join("recovery.toml").exists());
        assert_eq!(fs::read_dir(&incomplete_record).unwrap().count(), 0);
        assert!(!sessions.join("empty-recovery.toml").exists());

        let original_hash =
            gwt_core::repo_hash::compute_repo_hash("https://github.com/example/moved-repo.git");
        let mut moved = old_session(&sessions, &worktree, "moved-recovery", now);
        moved.repo_hash = Some(original_hash.as_str().to_string());
        moved.save(&sessions).unwrap();
        let original_recovery = gwt_core::paths::gwt_project_dir(&original_hash).join("recovery");
        fs::create_dir_all(original_recovery.parent().unwrap()).unwrap();
        fs::write(original_recovery, "unreadable original recovery directory").unwrap();
        prune_session_ledger(&sessions, now).unwrap();
        assert!(sessions.join("moved-recovery.toml").exists());
    }

    #[test]
    fn expires_abandoned_temporary_files_without_touching_fresh_writes_or_foreign_files() {
        let dir = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(dir.path().join("home"));
        let now = Utc::now();
        let old_time: std::time::SystemTime = (now - Duration::days(31)).into();
        let old = fs::FileTimes::new().set_modified(old_time);
        // Keep the unused PID positive after Unix's pid_t cast: u32::MAX
        // becomes -1 and probes all processes instead of a dead writer.
        for name in [".tmp123.toml", ".expired.toml.tmp-2147483647-abandoned"] {
            let file = fs::File::create(dir.path().join(name)).unwrap();
            file.set_times(old).unwrap();
        }
        for name in [
            ".tmp456.toml",
            ".fresh.toml.tmp-2147483647-current",
            "settings.toml",
        ] {
            fs::write(dir.path().join(name), "display_mode = \"grid\"").unwrap();
        }

        prune_session_ledger(dir.path(), now).unwrap();

        assert!(!dir.path().join(".tmp123.toml").exists());
        assert!(!dir
            .path()
            .join(".expired.toml.tmp-2147483647-abandoned")
            .exists());
        for name in [
            ".tmp456.toml",
            ".fresh.toml.tmp-2147483647-current",
            "settings.toml",
        ] {
            assert!(dir.path().join(name).exists(), "{name}");
        }
    }

    #[test]
    fn expires_completed_bound_sessions_but_keeps_active_and_blocked_execution_holders() {
        use gwt::cli::execution_state::{
            self, ExecutionControlRecord, ExecutionControlStatus, ExecutionOwnerKey,
            ExecutionOwnerKind, LegacyActiveDisposition,
        };

        let dir = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(dir.path().join("home"));
        let sessions = dir.path().join("sessions");
        let now = Utc::now();
        for (number, id, status) in [
            (1, "completed", ExecutionControlStatus::Completed),
            (2, "active", ExecutionControlStatus::Active),
            (3, "blocked", ExecutionControlStatus::Blocked),
        ] {
            let worktree = dir.path().join(id);
            fs::create_dir_all(&worktree).unwrap();
            for args in [
                vec!["init", "--quiet"],
                vec![
                    "remote",
                    "add",
                    "origin",
                    "https://github.com/example/session-retention.git",
                ],
            ] {
                let output = gwt_core::process::resolved_command(
                    gwt_core::process::ProcessPlanRequest::new("git"),
                )
                .unwrap()
                .args(args)
                .current_dir(&worktree)
                .output()
                .unwrap();
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            let owner = ExecutionOwnerKey {
                kind: ExecutionOwnerKind::Issue,
                number,
            };
            let mut record = ExecutionControlRecord {
                owner_kind: owner.kind,
                owner_number: number,
                primary_session_id: id.to_string(),
                entrypoint: "launch".to_string(),
                bundled_required_owners: Vec::new(),
                status,
                blocked_reason: None,
                missing_verification: None,
                launched_at: now - Duration::days(32),
                settled_at: (status != ExecutionControlStatus::Active)
                    .then_some(now - Duration::days(31)),
                completion_evidence: None,
                transfers: Vec::new(),
                recoveries: Vec::new(),
                content_hash: String::new(),
                permission_decision: None,
            };
            record.content_hash = execution_state::compute_content_hash(&record);
            execution_state::save(&worktree, &record).unwrap();
            execution_state::ensure_generation_ledger(
                &worktree,
                owner,
                LegacyActiveDisposition::Live,
            )
            .unwrap();
            let mut session = old_session(&sessions, &worktree, id, now);
            let repo_hash = gwt_core::repo_hash::detect_repo_hash(&worktree)
                .unwrap()
                .as_str()
                .to_string();
            session.repo_hash = Some(repo_hash.clone());
            session.linked_issue_number = Some(number);
            session.execution_binding = Some(gwt_agent::SessionExecutionBinding {
                schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
                session_id: id.to_string(),
                repo_hash,
                owner_kind: "issue".to_string(),
                owner_number: number,
                identity: execution_state::current_owner_execution_binding(&worktree, owner)
                    .unwrap()
                    .unwrap(),
                capability_generation: 1,
            });
            session.save(&sessions).unwrap();
        }

        prune_session_ledger(&sessions, now).unwrap();

        assert!(!sessions.join("completed.toml").exists());
        assert!(sessions.join("active.toml").exists());
        assert!(sessions.join("blocked.toml").exists());
    }
}
