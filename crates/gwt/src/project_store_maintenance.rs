//! Explicit, non-destructive maintenance of abandoned machine-local stores.

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Path, PathBuf},
    time::SystemTime,
};

type StateStamp = Option<(SystemTime, u64)>;

struct StateGuard(Vec<(PathBuf, StateStamp)>);

impl StateGuard {
    fn capture(paths: &[PathBuf]) -> io::Result<Self> {
        paths
            .iter()
            .map(|path| Ok((path.clone(), state_stamp(path)?)))
            .collect::<io::Result<Vec<_>>>()
            .map(Self)
    }

    fn check(&self) -> io::Result<()> {
        for (path, stamp) in &self.0 {
            if state_stamp(path)? != *stamp {
                return Err(io::Error::other("open projects or Session ledger changed; remaining stores were retained, rerun workspace.project_prune with dry_run=true"));
            }
        }
        Ok(())
    }
}

fn state_stamp(path: &Path) -> io::Result<StateStamp> {
    match fs::metadata(path) {
        Ok(metadata) => Ok(Some((metadata.modified()?, metadata.len()))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub scanned: usize,
    pub candidates: Vec<String>,
    pub retained: BTreeMap<String, usize>,
    pub manifest_hash: String,
    pub quarantine_dir: Option<PathBuf>,
    /// A read-only inventory can finish while the fleet changes. Apply always
    /// reloads protection and refuses changes during its own inspection/moves.
    pub protection_changed: bool,
    #[serde(skip)]
    fingerprints: BTreeMap<String, Vec<u8>>,
}

/// A plan is read-only. Missing roots alone never authorize deleting data.
pub fn inspect(
    projects: &Path,
    protected: &BTreeSet<String>,
    now: DateTime<Utc>,
) -> io::Result<Report> {
    let mut report = Report {
        scanned: 0,
        candidates: vec![],
        retained: BTreeMap::new(),
        manifest_hash: String::new(),
        quarantine_dir: None,
        protection_changed: false,
        fingerprints: BTreeMap::new(),
    };
    let mut digest = Sha256::new();
    digest.update(
        dunce::canonicalize(projects)
            .unwrap_or_else(|_| projects.to_path_buf())
            .as_os_str()
            .as_encoded_bytes(),
    );
    let mut entries = match fs::read_dir(projects) {
        Ok(entries) => entries.collect::<io::Result<Vec<_>>>()?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(report),
        Err(error) => return Err(error),
    };
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        report.scanned += 1;
        let id = entry.file_name().to_string_lossy().into_owned();
        let reason = if id.len() != 16 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            Err("unrecognized_store")
        } else if protected.contains(&id) {
            Err("open_project_or_session")
        } else {
            store_fingerprint(&entry.path(), now)
        };
        match reason {
            Ok(fingerprint) => {
                digest.update(id.as_bytes());
                digest.update(&fingerprint);
                report.fingerprints.insert(id.clone(), fingerprint);
                report.candidates.push(id);
            }
            Err(reason) => *report.retained.entry(reason.to_owned()).or_default() += 1,
        }
    }
    report.manifest_hash = hex::encode(digest.finalize());
    Ok(report)
}

/// Apply only a still-current dry-run manifest. Moves preserve the entire store.
fn quarantine(
    projects: &Path,
    protected: &BTreeSet<String>,
    now: DateTime<Utc>,
    manifest: &str,
    guard: Option<&StateGuard>,
) -> io::Result<Report> {
    if let Some(guard) = guard {
        guard.check()?;
    }
    let projects = dunce::canonicalize(projects)?;
    let mut plan = inspect(&projects, protected, now)?;
    if manifest.is_empty() || plan.manifest_hash != manifest {
        return Err(io::Error::other("project quarantine plan changed or manifest_hash is missing; run workspace.project_prune with dry_run=true and review the new manifest_hash"));
    }
    if plan.candidates.is_empty() {
        return Ok(plan);
    }
    if let Some(guard) = guard {
        guard.check()?;
    }
    let parent = projects
        .parent()
        .ok_or_else(|| io::Error::other("projects has no parent"))?;
    let quarantine_root = parent.join("project-quarantine");
    match fs::create_dir(&quarantine_root) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    if !fs::symlink_metadata(&quarantine_root)?.file_type().is_dir() {
        return Err(io::Error::other(
            "project-quarantine must be a real directory",
        ));
    }
    let destination = quarantine_root.join(uuid::Uuid::new_v4().to_string());
    fs::create_dir(&destination)?;
    plan.quarantine_dir = Some(destination.clone());
    // Publish recovery context before the first move, including on partial failure.
    fs::write(
        destination.join("manifest.json"),
        serde_json::to_vec_pretty(&plan)?,
    )?;
    for id in &plan.candidates {
        // Both absolute targets are constructed from a validated single hash component.
        // Recheck immediately before rename; a changed writer invalidates the plan.
        let source = projects.join(id);
        let target = destination.join(id);
        if source.parent() != Some(projects.as_path())
            || target.parent() != Some(destination.as_path())
        {
            return Err(io::Error::other(
                "project quarantine target escaped its store root",
            ));
        }
        if store_fingerprint(&source, now).ok().as_ref() != plan.fingerprints.get(id) {
            return Err(io::Error::other("project stores changed during quarantine; remaining stores were retained, inspect the recovery manifest and rerun the dry run"));
        }
        if let Some(guard) = guard {
            guard.check()?;
        }
        fs::rename(source, target)?;
    }
    Ok(plan)
}

/// Resolve only current/open roots. Historical endpoint paths are never rehashed here.
pub fn run(project_root: &Path, dry_run: bool, manifest: Option<&str>) -> io::Result<Report> {
    // Canonical Session writes atomically replace a child in this directory;
    // session.json changes when an open project changes. Detect both during
    // inventory and immediately before each move.
    let guard = StateGuard::capture(&[
        gwt_core::paths::gwt_session_state_path(),
        gwt_core::paths::gwt_sessions_dir(),
    ])?;
    let saved = crate::persistence::load_session_state(&gwt_core::paths::gwt_session_state_path())?;
    let mut roots = saved
        .tabs
        .into_iter()
        .map(|tab| tab.project_root)
        .collect::<Vec<_>>();
    roots.extend(
        saved
            .recent_projects
            .into_iter()
            .map(|project| project.path),
    );
    roots.push(project_root.to_path_buf());
    let sessions =
        match gwt_agent::session_ledger::load_sessions(&gwt_core::paths::gwt_sessions_dir()) {
            Ok(sessions) => sessions,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error),
        };
    for session in sessions {
        if !matches!(
            session.status,
            gwt_agent::AgentStatus::Stopped | gwt_agent::AgentStatus::Interrupted
        ) {
            roots.push(session.worktree_path);
        }
    }
    roots.sort();
    roots.dedup();
    let protected = roots
        .into_iter()
        .flat_map(|root| {
            let mut hashes = vec![gwt_core::repo_hash::compute_path_hash(&root)
                .as_str()
                .to_owned()];
            if root.is_dir() {
                hashes.push(
                    gwt_core::paths::project_scope_hash(&root)
                        .as_str()
                        .to_owned(),
                );
            }
            hashes
        })
        .collect();
    let projects = gwt_core::paths::gwt_projects_dir();
    if dry_run {
        let mut plan = inspect(&projects, &protected, Utc::now())?;
        plan.protection_changed = guard.check().is_err();
        Ok(plan)
    } else {
        quarantine(
            &projects,
            &protected,
            Utc::now(),
            manifest.unwrap_or_default(),
            Some(&guard),
        )
    }
}

fn store_fingerprint(store: &Path, now: DateTime<Utc>) -> Result<Vec<u8>, &'static str> {
    if store.join("pm").exists() || store.join("project-state/pm.json").exists() {
        return Err("pm_store");
    }
    let mut digest = Sha256::new();
    fingerprint_tree(store, store, now, &mut digest, 0)?;
    Ok(digest.finalize().to_vec())
}

fn fingerprint_tree(
    store: &Path,
    path: &Path,
    now: DateTime<Utc>,
    digest: &mut Sha256,
    depth: usize,
) -> Result<(), &'static str> {
    if depth > 4 {
        return Err("unknown_state");
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| "unreadable_state")?;
    let kind = metadata.file_type();
    if kind.is_symlink() {
        return Err("linked_state");
    }
    if DateTime::<Utc>::from(metadata.modified().map_err(|_| "unreadable_state")?)
        > now - chrono::Duration::days(1)
    {
        return Err("recent_state");
    }
    let relative = path.strip_prefix(store).map_err(|_| "unknown_state")?;
    digest.update(relative.as_os_str().as_encoded_bytes());
    if kind.is_dir() {
        if !relative.as_os_str().is_empty()
            && !["runtime", "runtime/daemon", "project-state", "workspace"]
                .iter()
                .any(|allowed| relative == Path::new(allowed))
        {
            return Err("unknown_state");
        }
        if relative == Path::new("runtime") {
            // Other runtime families can own live GUI processes without endpoints.
            if fs::read_dir(path)
                .map_err(|_| "unreadable_state")?
                .any(|entry| !entry.is_ok_and(|entry| entry.file_name() == "daemon"))
            {
                return Err("unknown_state");
            }
        }
        let mut entries = fs::read_dir(path)
            .map_err(|_| "unreadable_state")?
            .collect::<io::Result<Vec<_>>>()
            .map_err(|_| "unreadable_state")?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            fingerprint_tree(store, &entry.path(), now, digest, depth + 1)?;
        }
    } else if kind.is_file() {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("unknown_state")?;
        let known = relative == Path::new("workspace.json")
            || (relative.parent() == Some(Path::new("runtime/daemon"))
                && path
                    .extension()
                    .is_some_and(|extension| extension == "json"))
            || ((relative.parent() == Some(Path::new("project-state"))
                || relative.parent() == Some(Path::new("workspace")))
                && matches!(
                    name,
                    "current.json"
                        | "works.json"
                        | "work_items.json"
                        | "journal.jsonl"
                        | "work-events.jsonl"
                        | "work-events-closed.jsonl"
                        | "works.lock"
                        | "work_items.lock"
                ));
        if !known {
            return Err("unknown_state");
        }
        if metadata.len() > 1024 * 1024 {
            return Err("unknown_state");
        }
        let bytes = fs::read(path).map_err(|_| "unreadable_state")?;
        digest.update(&bytes);
        if path
            .extension()
            .is_some_and(|extension| extension == "lock")
            && bytes.is_empty()
        {
            return Ok(());
        }
        if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            let value: Value = serde_json::from_slice(&bytes).map_err(|_| "malformed_state")?;
            if !value.is_object() {
                return Err("unknown_state");
            }
            if name == "workspace.json"
                && (!value["viewport"].is_object() || !value["windows"].is_array())
            {
                return Err("unknown_state");
            }
            if name == "current.json" && !value["project_root"].is_string() {
                return Err("unknown_state");
            }
            if matches!(name, "works.json" | "work_items.json") && !value["work_items"].is_array() {
                return Err("unknown_state");
            }
            if relative.starts_with("runtime/daemon") {
                let endpoint: gwt_core::daemon::DaemonEndpoint =
                    serde_json::from_value(value.clone()).map_err(|_| "malformed_state")?;
                if !endpoint.scope.project_root.is_absolute() {
                    return Err("unknown_state");
                }
                if endpoint.has_live_owner(crate::process::is_process_alive) {
                    return Err("live_daemon");
                }
            }
            if references_existing_root(&value) {
                return Err("existing_project_root");
            }
        } else if path
            .extension()
            .is_some_and(|extension| extension == "jsonl")
        {
            for line in bytes
                .split(|byte| *byte == b'\n')
                .filter(|line| !line.is_empty())
            {
                let value: Value = serde_json::from_slice(line).map_err(|_| "malformed_state")?;
                if !value.is_object() {
                    return Err("unknown_state");
                }
                if references_existing_root(&value) {
                    return Err("existing_project_root");
                }
            }
        } else {
            return Err("unknown_state");
        }
    } else {
        return Err("unknown_state");
    }
    Ok(())
}

fn references_existing_root(value: &Value) -> bool {
    match value {
        Value::Object(fields) => fields.iter().any(|(key, value)| {
            if matches!(
                key.as_str(),
                "project_root" | "worktree_path" | "working_dir" | "cwd"
            ) {
                if let Some(root) = value.as_str().filter(|root| !root.is_empty()) {
                    match fs::metadata(root) {
                        Ok(_) => return true,
                        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                        Err(_) => return true,
                    }
                }
            }
            (key == "is_pm" && value.as_bool() == Some(true)) || references_existing_root(value)
        }),
        Value::Array(values) => values.iter().any(references_existing_root),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use std::{collections::BTreeSet, fs};

    fn fixture() -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let projects = temp.path().join("projects");
        fs::create_dir(&projects).unwrap();
        (temp, projects)
    }

    fn workspace(projects: &Path, id: &str) -> PathBuf {
        let store = projects.join(id);
        fs::create_dir(&store).unwrap();
        fs::write(
            store.join("workspace.json"),
            br#"{"viewport":{},"windows":[]}"#,
        )
        .unwrap();
        store
    }

    #[test]
    fn project_prune_dry_run_and_quarantine_preserve_bytes_and_require_manifest() {
        let (temp, projects) = fixture();
        let id = "0123456789abcdef";
        let store = workspace(&projects, id);
        let bytes = fs::read(store.join("workspace.json")).unwrap();
        let now = Utc::now() + Duration::days(2);
        let plan = inspect(&projects, &BTreeSet::new(), now).unwrap();
        assert_eq!(plan.candidates.len(), 1);
        assert!(store.exists());
        assert!(!temp.path().join("project-quarantine").exists());
        assert!(quarantine(&projects, &BTreeSet::new(), now, "wrong", None).is_err());
        let result =
            quarantine(&projects, &BTreeSet::new(), now, &plan.manifest_hash, None).unwrap();
        let moved = result.quarantine_dir.unwrap().join(id);
        assert!(!store.exists());
        assert_eq!(fs::read(moved.join("workspace.json")).unwrap(), bytes);
        assert!(moved.parent().unwrap().join("manifest.json").is_file());
    }

    #[test]
    fn project_prune_retains_live_pm_unknown_recent_and_referenced_stores() {
        let (_temp, projects) = fixture();
        let root = projects.parent().unwrap().join("real-project");
        fs::create_dir(&root).unwrap();
        let referenced = workspace(&projects, "0000000000000001");
        fs::create_dir(referenced.join("project-state")).unwrap();
        fs::write(
            referenced.join("project-state/current.json"),
            serde_json::to_vec(&serde_json::json!({"project_root": root})).unwrap(),
        )
        .unwrap();
        let pm = workspace(&projects, "0000000000000002");
        fs::create_dir(pm.join("pm")).unwrap();
        let unknown = workspace(&projects, "0000000000000003");
        fs::write(unknown.join("valuable-unknown-data"), b"keep").unwrap();
        let corrupt = workspace(&projects, "0000000000000004");
        fs::write(corrupt.join("workspace.json"), b"broken").unwrap();
        workspace(&projects, "0000000000000005");
        let protected = BTreeSet::from(["0000000000000005".to_owned()]);
        let plan = inspect(&projects, &protected, Utc::now() + Duration::days(2)).unwrap();
        assert!(plan.candidates.is_empty(), "{plan:?}");
        let recent = inspect(&projects, &BTreeSet::new(), Utc::now()).unwrap();
        assert!(recent.candidates.is_empty());
    }

    #[test]
    fn project_prune_refuses_changed_plan_and_keeps_live_daemon_owner() {
        let (_temp, projects) = fixture();
        let store = workspace(&projects, "0123456789abcdef");
        let now = Utc::now() + Duration::days(2);
        let plan = inspect(&projects, &BTreeSet::new(), now).unwrap();
        fs::write(
            store.join("workspace.json"),
            br#"{"viewport":{},"windows":[{}]}"#,
        )
        .unwrap();
        assert!(quarantine(&projects, &BTreeSet::new(), now, &plan.manifest_hash, None).is_err());
        let daemon = store.join("runtime/daemon");
        fs::create_dir_all(&daemon).unwrap();
        let scope = gwt_core::daemon::RuntimeScope::new(
            "repo",
            "tree",
            projects.join("missing"),
            gwt_core::daemon::RuntimeTarget::Host,
        )
        .unwrap();
        let endpoint = gwt_core::daemon::DaemonEndpoint::new(
            scope,
            std::process::id(),
            "bind".into(),
            "token".into(),
            "test".into(),
        );
        fs::write(
            daemon.join("endpoint.json"),
            serde_json::to_vec(&endpoint).unwrap(),
        )
        .unwrap();
        assert!(inspect(&projects, &BTreeSet::new(), now)
            .unwrap()
            .candidates
            .is_empty());
    }

    #[test]
    fn project_prune_retains_nested_unknown_state_and_refuses_changed_global_protection() {
        let (_temp, projects) = fixture();
        let store = workspace(&projects, "0123456789abcdef");
        let now = Utc::now() + Duration::days(2);
        let plan = inspect(&projects, &BTreeSet::new(), now).unwrap();
        let state = projects.parent().unwrap().join("session.json");
        fs::write(&state, b"before").unwrap();
        let guard = StateGuard::capture(std::slice::from_ref(&state)).unwrap();
        fs::write(&state, b"changed-open-projects").unwrap();
        assert!(quarantine(
            &projects,
            &BTreeSet::new(),
            now,
            &plan.manifest_hash,
            Some(&guard)
        )
        .is_err());
        assert!(store.is_dir());
        let unknown = store.join("project-state/customer-data");
        fs::create_dir_all(&unknown).unwrap();
        fs::write(unknown.join("works.json"), br#"{"work_items":[]}"#).unwrap();
        assert!(inspect(&projects, &BTreeSet::new(), now)
            .unwrap()
            .candidates
            .is_empty());
    }

    #[test]
    fn project_prune_uses_session_ledger_type_checks_for_legacy_preferences() {
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let sessions = home.path().join(".gwt/sessions");
        fs::create_dir_all(&sessions).unwrap();
        fs::write(
            sessions.join("legacy-project.toml"),
            "last_worktree_path = 'legacy-project'\n",
        )
        .unwrap();
        assert!(run(home.path(), true, None).is_ok());
    }
}
