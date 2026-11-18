//! Inactive, source-qualified snapshots for the common Agent registry migration.
//!
//! Nothing in this module grants admission or changes project ownership. The
//! complete roster/profile/assignment cutover must reconcile these observations
//! against their source fingerprints before using any alias as a live identity.

use std::{
    collections::{BTreeMap, HashSet},
    fs, io,
};

use gwt_core::repo_hash::ProjectKey;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// An immutable observation. Owner numbers and assignments remain scoped to
/// the source store; they must never be interpreted as current global claims.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportedProject {
    /// Preserve even unknown fields and legacy profile spellings verbatim.
    pub preferences_json: String,
    pub preferences_sha256: String,
    pub roster_revision: u64,
    /// Legacy member UUID -> future global UUID, including retired members.
    /// The immutable ledger is shared by current and historical snapshots.
    pub member_aliases: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub previous_snapshots: Vec<ImportSourceSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportSourceSnapshot {
    pub preferences_json: String,
    pub preferences_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ImportPhase {
    Inactive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InactiveImports {
    schema_version: u32,
    phase: ImportPhase,
    /// Exact project-store keys, including legacy stores. Never merge by name
    /// or infer that two different stores represent the same repository/person.
    pub sources: BTreeMap<String, ImportedProject>,
}

impl Default for InactiveImports {
    fn default() -> Self {
        Self {
            schema_version: 1,
            phase: ImportPhase::Inactive,
            sources: BTreeMap::new(),
        }
    }
}

pub fn path() -> std::path::PathBuf {
    gwt_core::paths::gwt_home().join("machine-state/agent-registry-import.json")
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn fingerprint(raw: &str) -> String {
    hex::encode(Sha256::digest(raw.as_bytes()))
}

fn source_roster(raw: &str) -> io::Result<Option<crate::agent_names::AgentRoster>> {
    let value: serde_json::Value = serde_json::from_str(raw).map_err(io::Error::other)?;
    let fields = value
        .as_object()
        .ok_or_else(|| invalid("Agent import source preferences must be an object"))?;
    if fields.get("schema_version").is_some_and(|v| v != 1) {
        return Err(invalid(
            "Unsupported Agent import source preferences schema",
        ));
    }
    let Some(value) = fields.get("agent_roster").filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let roster: crate::agent_names::AgentRoster =
        serde_json::from_value(value.clone()).map_err(io::Error::other)?;
    if roster.schema_version != 1 {
        return Err(invalid("Inactive import requires the legacy roster schema"));
    }
    roster.validate().map_err(invalid)?;
    Ok(Some(roster))
}

/// Read only the inactive file. Missing state is distinct from an empty import.
/// Validate all snapshots/aliases before a later import may extend the file.
pub fn load() -> io::Result<Option<InactiveImports>> {
    let raw = match fs::read(path()) {
        Ok(raw) => raw,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let stored: InactiveImports = serde_json::from_slice(&raw).map_err(io::Error::other)?;
    if stored.schema_version != 1 {
        return Err(invalid("Unsupported inactive Agent import schema"));
    }
    let mut global_ids = HashSet::new();
    for (key, source) in &stored.sources {
        ProjectKey::parse(key).map_err(|error| invalid(error.to_string()))?;
        let roster = source_roster(&source.preferences_json)?
            .ok_or_else(|| invalid("Imported Agent source has no roster"))?;
        if fingerprint(&source.preferences_json) != source.preferences_sha256
            || roster.revision != source.roster_revision
        {
            return Err(invalid(format!(
                "Agent import snapshot/alias mismatch: {key}"
            )));
        }
        let mut member_ids: HashSet<_> = roster
            .members
            .into_iter()
            .map(|member| member.agent_member_id)
            .collect();
        for snapshot in &source.previous_snapshots {
            if fingerprint(&snapshot.preferences_json) != snapshot.preferences_sha256 {
                return Err(invalid(format!(
                    "Agent import historical fingerprint mismatch: {key}"
                )));
            }
            let previous = source_roster(&snapshot.preferences_json)?
                .ok_or_else(|| invalid(format!("Imported Agent history has no roster: {key}")))?;
            member_ids.extend(
                previous
                    .members
                    .into_iter()
                    .map(|member| member.agent_member_id),
            );
        }
        if member_ids.len() != source.member_aliases.len() {
            return Err(invalid(format!(
                "Agent import snapshot/alias mismatch: {key}"
            )));
        }
        for member_id in member_ids {
            let alias = source
                .member_aliases
                .get(&member_id)
                .ok_or_else(|| invalid(format!("Agent import alias missing: {key}/{member_id}")))?;
            let id = uuid::Uuid::parse_str(alias).map_err(|error| invalid(error.to_string()))?;
            if !global_ids.insert(id) {
                return Err(invalid("Agent import has duplicate global identities"));
            }
        }
    }
    Ok(Some(stored))
}

fn save(stored: &InactiveImports) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(stored).map_err(io::Error::other)?;
    crate::issue_monitor::durable_atomic_write(&path(), &bytes)
}

pub(crate) fn lock_for_registry() -> io::Result<gwt_core::operation_deadline::NamedFileLock> {
    let path = path();
    gwt_config::home::assert_test_path(&path);
    fs::create_dir_all(path.parent().expect("machine state parent"))?;
    gwt_core::operation_deadline::NamedFileLock::acquire(
        &path.with_extension("lock"),
        "inactive Agent imports",
    )
}

/// Caller holds source -> registry -> inactive locks. Preserve the existing
/// ledger instead of reentering the public source-locking import operations.
pub(crate) fn stage_locked(
    stored: &mut InactiveImports,
    key: &str,
    raw: &str,
    refresh: bool,
) -> io::Result<ImportedProject> {
    ProjectKey::parse(key).map_err(|error| invalid(error.to_string()))?;
    let roster = source_roster(raw)?.ok_or_else(|| invalid("Agent source has no roster"))?;
    let observed = fingerprint(raw);
    let mut source = match stored.sources.get(key) {
        Some(source) if source.preferences_sha256 == observed => return Ok(source.clone()),
        Some(source) if refresh => {
            let mut source = source.clone();
            source.previous_snapshots.push(ImportSourceSnapshot {
                preferences_json: source.preferences_json.clone(),
                preferences_sha256: source.preferences_sha256.clone(),
            });
            source
        }
        Some(source) => return Err(invalid(format!(
            "Agent import source {key} changed: imported {}, observed {observed}; reconcile before registry cutover",
            source.preferences_sha256,
        ))),
        None => ImportedProject { preferences_json: String::new(), preferences_sha256: String::new(),
            roster_revision: 0, member_aliases: BTreeMap::new(), previous_snapshots: Vec::new() },
    };
    source.preferences_json = raw.into();
    source.preferences_sha256 = observed;
    source.roster_revision = roster.revision;
    for member in roster.members {
        source
            .member_aliases
            .entry(member.agent_member_id)
            .or_insert_with(|| uuid::Uuid::new_v4().to_string());
    }
    stored.sources.insert(key.into(), source.clone());
    save(stored)?;
    Ok(source)
}

fn missing_source(source: &ProjectKey) -> io::Result<Option<ImportedProject>> {
    let _registry = crate::agent_registry::inactive_import_guard()?;
    if let Some(previous) = load()?.and_then(|mut stored| stored.sources.remove(source.as_str())) {
        return Err(invalid(format!(
            "Agent import source {} missing: imported {}; reconcile before registry cutover",
            source, previous.preferences_sha256
        )));
    }
    Ok(None)
}

/// Explicitly stage one existing source without initializing or normalizing it.
/// This API is deliberately not wired to launch, UI, or a background scanner.
/// Missing/rosterless projects remain unimported. Changed sources require an
/// explicit reconciliation at cutover, rather than silently losing provenance.
pub fn import_project(source: &ProjectKey) -> io::Result<Option<ImportedProject>> {
    let prefs_path =
        gwt_core::paths::gwt_project_dir(source).join("project-state/issue-monitor.json");
    match fs::metadata(&prefs_path) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return missing_source(source),
        Err(error) => return Err(error),
    }
    // Same stable sibling inode as the current project writer. Never use a
    // roster "Read" mutation here: it can initialize, normalize, or clean up.
    let source_lock_path = prefs_path.with_extension("lock");
    gwt_config::home::assert_test_path(&source_lock_path);
    let _source_lock = gwt_core::operation_deadline::NamedFileLock::acquire(
        &source_lock_path,
        "Agent import source preferences",
    )?;
    let raw = match fs::read_to_string(&prefs_path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return missing_source(source),
        Err(error) => return Err(error),
    };
    let observed_hash = fingerprint(&raw);
    let _registry = crate::agent_registry::inactive_import_guard()?;
    let import_path = path();
    gwt_config::home::assert_test_path(&import_path);
    fs::create_dir_all(import_path.parent().expect("machine state parent"))?;
    // Fixed order: source prefs -> inactive import. No code under this lock
    // acquires another project's prefs, Session lease, or machine-policy lock.
    let _import_lock = gwt_core::operation_deadline::NamedFileLock::acquire(
        &import_path.with_extension("lock"),
        "inactive Agent imports",
    )?;
    let mut stored = load()?.unwrap_or_default();
    if let Some(previous) = stored.sources.get(source.as_str()) {
        if previous.preferences_sha256 != observed_hash {
            return Err(invalid(format!(
                "Agent import source {} changed: imported {}, observed {}; reconcile before registry cutover",
                source, previous.preferences_sha256, observed_hash
            )));
        }
        let previous = previous.clone();
        // A previous rename may have committed before directory sync failed.
        // Re-establish durability on retry without generating another alias.
        save(&stored)?;
        return Ok(Some(previous));
    }
    let Some(roster) = source_roster(&raw)? else {
        return Ok(None);
    };
    let imported = ImportedProject {
        preferences_json: raw,
        preferences_sha256: observed_hash,
        roster_revision: roster.revision,
        member_aliases: roster
            .members
            .into_iter()
            .map(|member| (member.agent_member_id, uuid::Uuid::new_v4().to_string()))
            .collect(),
        previous_snapshots: Vec::new(),
    };
    stored
        .sources
        .insert(source.as_str().to_string(), imported.clone());
    // Aliases and the complete source snapshot become visible together.
    save(&stored)?;
    Ok(Some(imported))
}

/// Explicitly refresh an inactive observation, retaining all prior bytes and
/// aliases. Both fingerprints must describe the caller's inspected transition.
/// Only an exact immediate replay may use its already-committed predecessor.
/// This does not activate a registry or rebind any live Session/assignment.
pub fn reconcile_project(
    source: &ProjectKey,
    expected_import_fingerprint: &str,
    expected_source_fingerprint: &str,
) -> io::Result<ImportedProject> {
    let prefs_path =
        gwt_core::paths::gwt_project_dir(source).join("project-state/issue-monitor.json");
    let source_lock_path = prefs_path.with_extension("lock");
    gwt_config::home::assert_test_path(&source_lock_path);
    let _source_lock = gwt_core::operation_deadline::NamedFileLock::acquire(
        &source_lock_path,
        "Agent reconciliation source preferences",
    )?;
    let raw = fs::read_to_string(&prefs_path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("Agent reconciliation source {source} unreadable: {error}"),
        )
    })?;
    let observed = fingerprint(&raw);
    if observed != expected_source_fingerprint {
        return Err(invalid(format!(
            "Agent source {source} fingerprint changed: expected {expected_source_fingerprint}, observed {observed}; inspect source before reconciling"
        )));
    }
    let _registry = crate::agent_registry::inactive_import_guard()?;
    let import_path = path();
    gwt_config::home::assert_test_path(&import_path);
    fs::create_dir_all(import_path.parent().expect("machine state parent"))?;
    // Same source -> import order as import_project; never acquire Session,
    // machine-policy or another project's locks under the import lock.
    let _import_lock = gwt_core::operation_deadline::NamedFileLock::acquire(
        &import_path.with_extension("lock"),
        "inactive Agent imports",
    )?;
    let mut stored = load()?.unwrap_or_default();
    let previous = stored.sources.get(source.as_str()).ok_or_else(|| {
        invalid(format!(
            "Agent source {source} is not imported; import it first"
        ))
    })?;
    let replay = previous.preferences_sha256 == observed
        && previous
            .previous_snapshots
            .last()
            .is_some_and(|snapshot| snapshot.preferences_sha256 == expected_import_fingerprint);
    if previous.preferences_sha256 != expected_import_fingerprint && !replay {
        return Err(invalid(format!(
            "Agent import {source} fingerprint changed: expected {expected_import_fingerprint}, current {}; reload import before reconciling",
            previous.preferences_sha256
        )));
    }
    if previous.preferences_sha256 == observed {
        let result = previous.clone();
        // Retry may follow a committed rename whose parent fsync failed.
        save(&stored)?;
        return Ok(result);
    }
    let roster = source_roster(&raw)?.ok_or_else(|| {
        invalid(format!(
            "Agent reconciliation source {source} has no roster"
        ))
    })?;
    let mut reconciled = previous.clone();
    reconciled.previous_snapshots.push(ImportSourceSnapshot {
        preferences_json: previous.preferences_json.clone(),
        preferences_sha256: previous.preferences_sha256.clone(),
    });
    reconciled.preferences_json = raw;
    reconciled.preferences_sha256 = observed;
    reconciled.roster_revision = roster.revision;
    for member in roster.members {
        reconciled
            .member_aliases
            .entry(member.agent_member_id)
            .or_insert_with(|| uuid::Uuid::new_v4().to_string());
    }
    stored
        .sources
        .insert(source.as_str().to_string(), reconciled.clone());
    save(&stored)?;
    Ok(reconciled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{fs, path::PathBuf};

    const A: &str = "aaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbb";
    const MEMBER: &str = "b4263a32-14e9-4bb0-ab78-ec3d4816e811";

    fn source(key: &str, model: &str) -> PathBuf {
        let project = gwt_core::paths::gwt_projects_dir().join(key);
        let path = project.join("project-state/issue-monitor.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut prefs = serde_json::to_value(crate::IssueMonitorPrefs::default()).unwrap();
        prefs["agent_roster"] = json!({"schema_version": 1, "revision": 7, "members": [{
            "agent_member_id": MEMBER, "agent_id": "codex", "name": "Midori",
            "enabled": true, "created_at": "2026-10-01T00:00:00Z",
            "origin_session_id": "original-session", "last_issue_number": 42,
            "assignment": {"reservation_id": "original-reservation",
                "session_id": (key == A).then_some("original-session"),
                "issue_number": 42, "spec_number": null, "delivery_id": "pending-delivery",
                "host_id": "unknown-host", "host_pid": 0, "host_started_at": null}
        }]});
        prefs["launch_profiles"] = json!([{"agent_id": "codex", "agent_member_id": MEMBER,
            "model": model, "reasoning": "custom-reasoning", "codex_fast_mode": true,
            "future_profile_field": {"retain": true}}]);
        prefs["future_prefs_field"] = json!({"keep": "exact bytes"});
        fs::write(&path, serde_json::to_string_pretty(&prefs).unwrap()).unwrap();
        fs::create_dir_all(gwt_core::paths::gwt_sessions_dir()).unwrap();
        fs::write(
            gwt_core::paths::gwt_sessions_dir().join("original-session.toml"),
            "immutable history",
        )
        .unwrap();
        fs::write(project.join("agent-names.jsonl"), "immutable name history").unwrap();
        path
    }

    fn import(key: &str) -> ImportedProject {
        import_project(&ProjectKey::parse(key).unwrap())
            .unwrap()
            .expect("source roster must be imported")
    }

    #[test]
    fn collisions_keep_source_identity_profiles_and_unknown_assignment_without_source_writes() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let first = source(A, "custom-a");
        let second = source(B, "custom-b");
        let a_bytes = fs::read_to_string(&first).unwrap();
        let b_bytes = fs::read_to_string(&second).unwrap();
        let (a, b) = std::thread::scope(|scope| {
            let a = scope.spawn(|| {
                let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
                import(A)
            });
            let b = scope.spawn(|| {
                let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
                import(B)
            });
            (a.join().unwrap(), b.join().unwrap())
        });
        assert_ne!(a.member_aliases[MEMBER], b.member_aliases[MEMBER]);
        assert_ne!(a.member_aliases[MEMBER], MEMBER);
        assert_eq!(a.preferences_json, a_bytes);
        assert_eq!(b.preferences_json, b_bytes);
        assert_eq!(a.roster_revision, 7);
        assert_eq!(load().unwrap().unwrap().sources.len(), 2);
        for (path, bytes) in [(first, a_bytes), (second, b_bytes)] {
            assert_eq!(fs::read_to_string(&path).unwrap(), bytes);
            let project = path.parent().unwrap().parent().unwrap();
            assert_eq!(
                fs::read_to_string(
                    gwt_core::paths::gwt_sessions_dir().join("original-session.toml")
                )
                .unwrap(),
                "immutable history"
            );
            assert_eq!(
                fs::read_to_string(project.join("agent-names.jsonl")).unwrap(),
                "immutable name history"
            );
        }
    }

    #[test]
    fn committed_import_reloads_and_replays_without_changing_aliases_or_bytes() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        source(A, "custom-a");
        let first = import(A);
        let bytes = fs::read(path()).unwrap();
        assert_eq!(import(A), first);
        assert_eq!(fs::read(path()).unwrap(), bytes);
        assert_eq!(load().unwrap().unwrap().sources[A], first);
    }

    #[test]
    fn source_changes_refuse_with_fingerprints_and_preserve_both_states() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        source(A, "original");
        let first = import(A);
        let bytes = fs::read(path()).unwrap();
        let changed_path = source(A, "changed");
        let changed = fs::read(&changed_path).unwrap();
        let error = import_project(&ProjectKey::parse(A).unwrap())
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(A) && error.contains(&first.preferences_sha256),
            "{error}"
        );
        assert!(
            error.contains("reconcile") && error.contains("observed"),
            "{error}"
        );
        assert_eq!(fs::read(path()).unwrap(), bytes);
        assert_eq!(fs::read(&changed_path).unwrap(), changed);
        fs::write(&changed_path, "{").unwrap();
        let error = import_project(&ProjectKey::parse(A).unwrap())
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(A)
                && error.contains(&first.preferences_sha256)
                && error.contains("observed")
                && error.contains("reconcile"),
            "{error}"
        );
        fs::write(&changed_path, r#"{"schema_version":1,"agent_roster":null}"#).unwrap();
        let error = import_project(&ProjectKey::parse(A).unwrap())
            .unwrap_err()
            .to_string();
        assert!(error.contains("reconcile"), "{error}");
        fs::remove_file(&changed_path).unwrap();
        let error = import_project(&ProjectKey::parse(A).unwrap())
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("missing") && error.contains("reconcile"),
            "{error}"
        );
        assert_eq!(fs::read(path()).unwrap(), bytes);
    }

    #[test]
    fn source_disappearing_while_waiting_for_its_writer_lock_uses_missing_source_diagnostic() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let prefs_path = source(A, "custom-a");
        let first = import(A);
        let lock_path = prefs_path.with_extension("lock");
        let writer =
            gwt_core::operation_deadline::NamedFileLock::acquire(&lock_path, "test source writer")
                .unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            let reader = scope.spawn(|| {
                let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
                let _contention =
                    gwt_core::operation_deadline::ScopedNamedLockContentionObserver::set(
                        &lock_path,
                        move || tx.send(()).unwrap(),
                    );
                import_project(&ProjectKey::parse(A).unwrap())
            });
            gwt_core::test_support::recv_event(&rx, "import reader reached the source writer lock");
            fs::remove_file(prefs_path).unwrap();
            drop(writer);
            let error = reader.join().unwrap().unwrap_err().to_string();
            assert!(
                error.contains(A)
                    && error.contains(&first.preferences_sha256)
                    && error.contains("missing")
                    && error.contains("reconcile"),
                "{error}"
            );
        });
        assert_eq!(load().unwrap().unwrap().sources[A], first);
    }

    #[test]
    fn interrupted_next_source_resumes_without_reimporting_completed_source() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        source(A, "custom-a");
        let a = import(A);
        let second = source(B, "custom-b");
        fs::write(&second, "{").unwrap();
        let bytes = fs::read(path()).unwrap();
        assert!(import_project(&ProjectKey::parse(B).unwrap()).is_err());
        assert_eq!(fs::read(path()).unwrap(), bytes);
        source(B, "custom-b");
        import(B);
        assert_eq!(import(A), a);
        assert_eq!(load().unwrap().unwrap().sources.len(), 2);
    }

    #[test]
    fn retry_after_committed_rename_sync_failure_retains_one_alias() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        source(A, "custom-a");
        fs::create_dir_all(path().parent().unwrap()).unwrap();
        let _fail = gwt_core::test_support::ScopedEnvVar::set(
            "GWT_TEST_FAIL_ISSUE_MONITOR_PREFS_PARENT_SYNC_ONCE",
            path(),
        );
        fs::write(path().with_extension("parent-sync-fail-once"), "fail").unwrap();
        assert!(import_project(&ProjectKey::parse(A).unwrap()).is_err());
        let committed = load().unwrap().unwrap().sources[A].clone();
        assert_eq!(import(A), committed);
        assert_eq!(load().unwrap().unwrap().sources[A], committed);
    }

    #[test]
    fn missing_roster_and_invalid_import_state_never_mark_source_complete() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        assert!(import_project(&ProjectKey::parse(A).unwrap())
            .unwrap()
            .is_none());
        assert!(!path().exists());
        let prefs = source(A, "custom-a");
        fs::write(&prefs, "{}").unwrap();
        assert!(import_project(&ProjectKey::parse(A).unwrap())
            .unwrap()
            .is_none());
        fs::write(&prefs, r#"{"schema_version":2}"#).unwrap();
        assert!(import_project(&ProjectKey::parse(A).unwrap()).is_err());
        source(A, "custom-a");
        import(A);
        let mut stored: serde_json::Value =
            serde_json::from_slice(&fs::read(path()).unwrap()).unwrap();
        let digest = stored["sources"][A]["preferences_sha256"].clone();
        stored["sources"][A]["preferences_sha256"] = json!("tampered");
        fs::write(path(), serde_json::to_vec(&stored).unwrap()).unwrap();
        assert!(load().is_err());
        stored["sources"][A]["preferences_sha256"] = digest;
        stored["sources"][A]["member_aliases"] = json!({});
        fs::write(path(), serde_json::to_vec(&stored).unwrap()).unwrap();
        assert!(load().is_err());
        stored["sources"][A]["member_aliases"] =
            json!({(MEMBER): uuid::Uuid::new_v4().to_string()});
        stored["phase"] = json!("active");
        let broken = serde_json::to_vec(&stored).unwrap();
        fs::write(path(), &broken).unwrap();
        assert!(load().is_err());
        assert!(import_project(&ProjectKey::parse(A).unwrap()).is_err());
        assert_eq!(fs::read(path()).unwrap(), broken);
    }

    #[test]
    fn reconciliation_preserves_snapshots_and_retired_aliases_without_source_writes() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let prefs = source(A, "original");
        source(B, "other-project");
        let original = import(A);
        let other = import(B);
        let mut changed: serde_json::Value =
            serde_json::from_str(&original.preferences_json).unwrap();
        let mut added = changed["agent_roster"]["members"][0].clone();
        let new_id = uuid::Uuid::new_v4().to_string();
        added["agent_member_id"] = json!(new_id);
        added["name"] = json!("Other");
        added["assignment"] = serde_json::Value::Null;
        changed["agent_roster"]["members"]
            .as_array_mut()
            .unwrap()
            .push(added);
        changed["launch_profiles"][0]["model"] = json!("changed-profile");
        let raw = serde_json::to_string_pretty(&changed).unwrap();
        fs::write(&prefs, &raw).unwrap();
        let observed = fingerprint(&raw);
        let key = ProjectKey::parse(A).unwrap();
        let reconciled = reconcile_project(&key, &original.preferences_sha256, &observed).unwrap();
        assert_eq!(
            reconciled.member_aliases[MEMBER],
            original.member_aliases[MEMBER]
        );
        assert_ne!(
            reconciled.member_aliases[&new_id],
            other.member_aliases[MEMBER]
        );
        assert_eq!(reconciled.preferences_json, raw);
        let bytes = fs::read(path()).unwrap();
        assert_eq!(
            reconcile_project(&key, &original.preferences_sha256, &observed).unwrap(),
            reconciled
        );
        assert_eq!(fs::read(path()).unwrap(), bytes);
        assert!(reconcile_project(&key, "stale", &observed).is_err());
        assert_eq!(
            reconcile_project(&key, &observed, &observed).unwrap(),
            reconciled
        );
        let state: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            state["sources"][A]["previous_snapshots"][0]["preferences_json"],
            original.preferences_json
        );
        assert_eq!(fs::read_to_string(&prefs).unwrap(), raw);
        // A removed then restored source UUID keeps its original global alias.
        let original_member = changed["agent_roster"]["members"]
            .as_array_mut()
            .unwrap()
            .remove(0);
        let removed = serde_json::to_string(&changed).unwrap();
        fs::write(&prefs, &removed).unwrap();
        let retired = reconcile_project(&key, &observed, &fingerprint(&removed)).unwrap();
        assert_eq!(
            retired.member_aliases[MEMBER],
            original.member_aliases[MEMBER]
        );
        assert_eq!(load().unwrap().unwrap().sources[A], retired);
        changed["agent_roster"]["members"]
            .as_array_mut()
            .unwrap()
            .push(original_member);
        let restored = serde_json::to_string(&changed).unwrap();
        fs::write(&prefs, &restored).unwrap();
        let restored =
            reconcile_project(&key, &retired.preferences_sha256, &fingerprint(&restored)).unwrap();
        assert_eq!(restored.member_aliases, reconciled.member_aliases);
        assert_eq!(load().unwrap().unwrap().sources[B], other);
        assert_eq!(
            fs::read_to_string(gwt_core::paths::gwt_sessions_dir().join("original-session.toml"))
                .unwrap(),
            "immutable history"
        );
        let mut state = serde_json::to_value(load().unwrap().unwrap()).unwrap();
        state["sources"][A]["previous_snapshots"][0]["preferences_sha256"] = json!("tampered");
        fs::write(path(), serde_json::to_vec(&state).unwrap()).unwrap();
        assert!(load().is_err());
    }

    #[test]
    fn reconciliation_requires_both_fingerprints_and_an_existing_valid_source() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let prefs = source(A, "original");
        let key = ProjectKey::parse(A).unwrap();
        let first = import(A);
        let bytes = fs::read(path()).unwrap();
        source(A, "changed");
        let observed = fingerprint(&fs::read_to_string(&prefs).unwrap());
        for (old, current) in [
            ("stale", observed.as_str()),
            (first.preferences_sha256.as_str(), "stale"),
        ] {
            let error = reconcile_project(&key, old, current)
                .unwrap_err()
                .to_string();
            assert!(
                error.contains(A) && error.contains("fingerprint"),
                "{error}"
            );
            assert_eq!(fs::read(path()).unwrap(), bytes);
        }
        for raw in ["{", "{}", r#"{"schema_version":2}"#] {
            fs::write(&prefs, raw).unwrap();
            assert!(reconcile_project(&key, &first.preferences_sha256, &fingerprint(raw)).is_err());
            assert_eq!(fs::read(path()).unwrap(), bytes);
            assert_eq!(fs::read_to_string(&prefs).unwrap(), raw);
        }
        fs::remove_file(&prefs).unwrap();
        assert!(reconcile_project(&key, &first.preferences_sha256, &observed).is_err());
        let b_prefs = source(B, "unimported");
        let b_observed = fingerprint(&fs::read_to_string(b_prefs).unwrap());
        let b = ProjectKey::parse(B).unwrap();
        let error = reconcile_project(&b, &first.preferences_sha256, &b_observed)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(B) && error.contains("not imported"),
            "{error}"
        );
        assert_eq!(fs::read(path()).unwrap(), bytes);
    }

    #[test]
    fn reconciliation_retries_before_and_after_commit_without_duplicate_history_or_aliases() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        source(A, "original");
        let first = import(A);
        let bytes = fs::read(path()).unwrap();
        let prefs = source(A, "changed");
        let mut changed: serde_json::Value =
            serde_json::from_slice(&fs::read(&prefs).unwrap()).unwrap();
        let mut added = changed["agent_roster"]["members"][0].clone();
        added["agent_member_id"] = json!(uuid::Uuid::new_v4().to_string());
        added["name"] = json!("Added");
        changed["agent_roster"]["members"]
            .as_array_mut()
            .unwrap()
            .push(added);
        let raw = serde_json::to_string(&changed).unwrap();
        fs::write(prefs, &raw).unwrap();
        let observed = fingerprint(&raw);
        let key = ProjectKey::parse(A).unwrap();
        {
            let now = std::time::Instant::now();
            let _clock = gwt_core::operation_deadline::ScopedOperationClock::set(now);
            let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(now);
            let error = reconcile_project(&key, &first.preferences_sha256, &observed).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        }
        assert_eq!(fs::read(path()).unwrap(), bytes);
        let _fail = gwt_core::test_support::ScopedEnvVar::set(
            "GWT_TEST_FAIL_ISSUE_MONITOR_PREFS_PARENT_SYNC_ONCE",
            path(),
        );
        fs::write(path().with_extension("parent-sync-fail-once"), "fail").unwrap();
        assert!(reconcile_project(&key, &first.preferences_sha256, &observed).is_err());
        let committed = load().unwrap().unwrap().sources[A].clone();
        assert_eq!(committed.preferences_sha256, observed);
        assert_eq!(
            committed.member_aliases[MEMBER],
            first.member_aliases[MEMBER]
        );
        assert_eq!(committed.member_aliases.len(), 2);
        let bytes = fs::read(path()).unwrap();
        assert_eq!(
            reconcile_project(&key, &first.preferences_sha256, &observed).unwrap(),
            committed
        );
        assert_eq!(fs::read(path()).unwrap(), bytes);
    }
}
