use std::{
    fs,
    path::Path,
    time::{Duration, SystemTime},
};

use gwt_agent::{
    runtime_gc::collect_stale_runtime_namespaces, AgentId, AgentStatus, Session,
    SessionRuntimeState,
};

const AGE: Duration = Duration::from_secs(24 * 60 * 60);

#[test]
fn runtime_gc_keeps_namespace_during_sidecar_publication() {
    use fs2::FileExt;
    let root = tempfile::tempdir().unwrap();
    let now = SystemTime::now();
    namespace(
        root.path(),
        1,
        "publishing",
        now - AGE - Duration::from_secs(60),
    );
    let publication = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.path().join(".runtime-publication.lock"))
        .unwrap();
    FileExt::lock_shared(&publication).unwrap();
    let report = collect_stale_runtime_namespaces(root.path(), now, AGE, |_| Some(false)).unwrap();
    assert_eq!(report.reclaimed, 0);
    assert_eq!(report.reasons["publication_busy"], 1);
    let path = gwt_agent::runtime_state_path_for_pid(root.path(), 1, "publishing");
    SessionRuntimeState::new(AgentStatus::Running)
        .save(&path)
        .unwrap();
    FileExt::unlock(&publication).unwrap();
    let report =
        collect_stale_runtime_namespaces(root.path(), SystemTime::now(), AGE, |_| Some(false))
            .unwrap();
    assert_eq!(report.reclaimed, 0);
    assert_eq!(report.reasons["recent"], 1);
    assert_eq!(
        SessionRuntimeState::load(&path).unwrap().status,
        AgentStatus::Running
    );
}

fn namespace(root: &Path, pid: u32, id: &str, modified: SystemTime) {
    let dir = gwt_agent::runtime_state_dir_for_pid(root, pid);
    let path = dir.join(format!("{id}.json"));
    let mut runtime = SessionRuntimeState::new(AgentStatus::Stopped);
    runtime.updated_at = modified.into();
    runtime.last_activity_at = modified.into();
    fs::create_dir_all(&dir).unwrap();
    fs::write(&path, serde_json::to_vec(&runtime).unwrap()).unwrap();
    for path in [&path, &dir] {
        filetime::set_file_mtime(path, modified.into()).unwrap();
    }
}

#[test]
fn runtime_gc_retains_live_unknown_recent_and_restore_namespaces() {
    let root = tempfile::tempdir().unwrap();
    let now = SystemTime::now();
    let old = now - AGE - Duration::from_secs(60);
    for (pid, id) in [
        (1, "dead"),
        (2, "live"),
        (3, "unknown"),
        (4, "recent"),
        (5, "restore"),
        (6, "active"),
        (7, "terminal"),
        (8, "interrupted"),
    ] {
        namespace(root.path(), pid, id, if pid == 4 { now } else { old });
    }
    let mut restore = Session::new(root.path(), "main", AgentId::Codex);
    restore.id = "restore".into();
    restore.status = AgentStatus::Stopped;
    restore.restore_window_on_startup = true;
    restore.agent_session_id = Some("conversation".into());
    restore.save(root.path()).unwrap();
    let original = fs::read(root.path().join("restore.toml")).unwrap();
    let mut active = restore.clone();
    active.id = "active".into();
    active.status = AgentStatus::Running;
    active.restore_window_on_startup = false;
    active.save(root.path()).unwrap();
    let mut terminal = active.clone();
    terminal.id = "terminal".into();
    terminal.status = AgentStatus::Stopped;
    terminal.save(root.path()).unwrap();
    let mut interrupted = terminal.clone();
    interrupted.id = "interrupted".into();
    interrupted.status = AgentStatus::Interrupted;
    interrupted.worktree_path = root.path().join("missing-worktree");
    interrupted.save(root.path()).unwrap();

    let report = collect_stale_runtime_namespaces(root.path(), now, AGE, |pid| match pid {
        2 => Some(true),
        3 => None,
        _ => Some(false),
    })
    .unwrap();
    assert_eq!(report.reclaimed, 2);
    assert_eq!(report.retained, 6);
    assert_eq!(report.reasons["live_pid"], 1);
    assert_eq!(report.reasons["unknown_pid"], 1);
    assert_eq!(report.reasons["recent"], 1);
    assert_eq!(report.reasons["session_recovery"], 3);
    assert_eq!(
        fs::read(root.path().join("restore.toml")).unwrap(),
        original
    );
    assert_eq!(
        Session::load(&root.path().join("restore.toml"))
            .unwrap()
            .agent_session_id
            .as_deref(),
        Some("conversation")
    );
    for pid in [2, 3, 4, 5, 6, 8] {
        assert!(gwt_agent::runtime_state_dir_for_pid(root.path(), pid).exists());
    }
    assert!(!gwt_agent::runtime_state_dir_for_pid(root.path(), 7).exists());
    assert!(root.path().join("terminal.toml").exists());
}

#[test]
fn runtime_gc_retains_uncertain_sidecars_and_pending_recovery() {
    let root = tempfile::tempdir().unwrap();
    let now = SystemTime::now();
    let old = now - AGE - Duration::from_secs(60);
    for (pid, id) in [
        (1, "corrupt"),
        (2, "child"),
        (3, "receipt"),
        (4, "unreadable"),
        (5, "fresh-file"),
    ] {
        namespace(root.path(), pid, id, old);
    }
    let corrupt = gwt_agent::runtime_state_path_for_pid(root.path(), 1, "corrupt");
    fs::write(&corrupt, "{").unwrap();
    filetime::set_file_mtime(&corrupt, old.into()).unwrap();
    let child = gwt_agent::runtime_state_path_for_pid(root.path(), 2, "child");
    let mut runtime = SessionRuntimeState::load(&child).unwrap();
    runtime.child_pid = Some(42);
    runtime.child_started_at = Some(1);
    runtime.save(&child).unwrap();
    filetime::set_file_mtime(&child, old.into()).unwrap();
    filetime::set_file_mtime(child.parent().unwrap(), old.into()).unwrap();
    fs::create_dir(root.path().join("execution-launch-recovery")).unwrap();
    fs::write(
        root.path().join("execution-launch-recovery/receipt.json"),
        "{}",
    )
    .unwrap();
    fs::write(root.path().join("unreadable.toml"), "invalid").unwrap();
    filetime::set_file_mtime(
        gwt_agent::runtime_state_path_for_pid(root.path(), 5, "fresh-file"),
        now.into(),
    )
    .unwrap();
    let report = collect_stale_runtime_namespaces(root.path(), now, AGE, |pid| {
        if pid == 42 {
            None
        } else {
            Some(false)
        }
    })
    .unwrap();
    assert_eq!(report.reclaimed, 0);
    assert_eq!(report.retained, 5);
    assert_eq!(report.reasons["unreadable"], 1);
    assert_eq!(report.reasons["child_runtime"], 1);
    assert_eq!(report.reasons["session_recovery"], 1);
    assert_eq!(report.reasons["session_unavailable"], 1);
    assert_eq!(report.reasons["recent"], 1);
}

#[cfg(unix)]
#[test]
fn runtime_gc_never_follows_namespace_or_sidecar_links() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let now = SystemTime::now();
    let old = now - AGE - Duration::from_secs(60);
    namespace(root.path(), 1, "linked", old);
    fs::write(outside.path().join("proof.json"), "keep").unwrap();
    symlink(outside.path(), root.path().join("runtime/2")).unwrap();
    let file = gwt_agent::runtime_state_path_for_pid(root.path(), 1, "linked");
    fs::remove_file(&file).unwrap();
    symlink(outside.path().join("proof.json"), file).unwrap();
    filetime::set_file_mtime(root.path().join("runtime/1"), old.into()).unwrap();
    let report = collect_stale_runtime_namespaces(root.path(), now, AGE, |_| Some(false)).unwrap();
    assert_eq!(report.reclaimed, 0);
    assert_eq!(report.retained, 2);
    assert_eq!(
        fs::read(outside.path().join("proof.json")).unwrap(),
        b"keep"
    );
}

#[cfg(windows)]
#[test]
fn runtime_gc_never_follows_namespace_junctions() {
    use gwt_core::process::{resolved_command, ProcessPlanRequest};

    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("runtime")).unwrap();
    fs::write(outside.path().join("proof.json"), "keep").unwrap();
    let junction = root.path().join("runtime").join("1");
    let status = resolved_command(
        ProcessPlanRequest::new("cmd.exe")
            .args(["/c", "mklink", "/J"])
            .arg(&junction)
            .arg(outside.path()),
    )
    .unwrap()
    .output()
    .unwrap();
    assert!(status.status.success(), "junction creation: {:?}", status);
    let report =
        collect_stale_runtime_namespaces(root.path(), SystemTime::now(), AGE, |_| Some(false))
            .unwrap();
    assert_eq!(report.reclaimed, 0);
    assert_eq!(report.reasons["unknown_path"], 1);
    assert_eq!(
        fs::read(outside.path().join("proof.json")).unwrap(),
        b"keep"
    );
}

#[test]
fn runtime_gc_6000_stale_namespaces_leave_only_live_census_targets() {
    let root = tempfile::tempdir().unwrap();
    let now = SystemTime::now();
    let old = now - AGE - Duration::from_secs(60);
    for pid in 1..=6000 {
        namespace(root.path(), pid, "orphan", old);
    }
    for pid in 6001..=6008 {
        namespace(root.path(), pid, "live", old);
    }
    let report =
        collect_stale_runtime_namespaces(root.path(), now, AGE, |pid| Some(pid > 6000)).unwrap();
    assert_eq!(report.reclaimed, 6000);
    assert_eq!(report.retained, 8);
    let remaining: Vec<_> = fs::read_dir(root.path().join("runtime"))
        .unwrap()
        .map(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .parse::<u32>()
                .unwrap()
        })
        .collect();
    assert_eq!(remaining.len(), 8);
    assert!(remaining.iter().all(|pid| *pid > 6000));
}
