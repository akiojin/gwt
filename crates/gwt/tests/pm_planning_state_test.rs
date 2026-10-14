//! SPEC #3431 T-109: the planning store is separate from execution state.

use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Barrier,
    },
    thread,
};

use gwt::pm_planning::{
    load_planning_state, mutate_planning_state, planning_state_path_for_repo, Goal, PmPlanningState,
};
use serde_json::{json, Value};

fn populated_goal(id: &str) -> Goal {
    Goal {
        id: id.to_string(),
        outcome: "Reliable project planning".to_string(),
        metric: "Accepted milestones".to_string(),
        target: "3".to_string(),
        horizon: "2026-Q4".to_string(),
        owner: "project-pm".to_string(),
        status: "active".to_string(),
    }
}

fn write_fixture(path: &Path, value: &Value) -> Vec<u8> {
    let bytes = serde_json::to_vec_pretty(value).expect("serialize planning fixture");
    fs::write(path, &bytes).expect("write planning fixture");
    bytes
}

fn corrupt_copies(path: &Path) -> Vec<PathBuf> {
    fs::read_dir(path.parent().expect("planning parent"))
        .expect("read planning directory")
        .map(|entry| entry.expect("read directory entry").path())
        .filter(|entry| {
            entry
                .file_name()
                .expect("backup filename")
                .to_string_lossy()
                .starts_with("pm-planning.json.corrupt-")
        })
        .collect()
}

#[test]
fn missing_planning_state_returns_defaults_without_creating_files() {
    let dir = tempfile::tempdir().expect("temp directory");
    let path = dir.path().join("project-state/pm-planning.json");

    let state = load_planning_state(&path).expect("missing state has safe defaults");

    assert_eq!(state.schema_version, 1);
    assert_eq!(state.revision, 0);
    assert!(state.goals.is_empty());
    assert!(state.roadmap_items.is_empty());
    assert!(state.milestones.is_empty());
    assert!(state.risks.is_empty());
    assert!(state.decisions.is_empty());
    assert!(state.recurring_items.is_empty());
    assert!(state.policies.is_empty());
    assert!(state.latest_forecasts.is_empty());
    assert!(state.audit_cursor.is_none());
    assert!(!path.exists());
    assert!(!path.parent().expect("planning parent").exists());
}

#[test]
fn populated_goal_and_audit_cursor_survive_commit_and_reload() {
    let dir = tempfile::tempdir().expect("temp directory");
    let path = dir.path().join("project-state/pm-planning.json");

    let (committed, receipt) = mutate_planning_state(&path, 0, |state| {
        state.goals.push(populated_goal("goal-1"));
        state.audit_cursor = Some("audit-17".to_string());
        Ok("goal registered")
    })
    .expect("commit initial planning state");
    let reloaded = load_planning_state(&path).expect("reload planning state");

    assert_eq!(receipt, "goal registered");
    assert_eq!(committed.revision, 1);
    assert_eq!(reloaded.revision, 1);
    assert_eq!(reloaded.schema_version, 1);
    assert_eq!(reloaded.goals.len(), 1);
    assert_eq!(
        serde_json::to_value(&reloaded.goals[0]).expect("serialize reloaded goal"),
        serde_json::to_value(populated_goal("goal-1")).expect("serialize expected goal")
    );
    assert_eq!(reloaded.audit_cursor.as_deref(), Some("audit-17"));
    assert_eq!(
        serde_json::to_value(committed).expect("serialize committed state"),
        serde_json::to_value(reloaded).expect("serialize reloaded state")
    );
}

#[test]
fn missing_legacy_fields_load_with_defaults_and_migrate_only_on_commit() {
    let dir = tempfile::tempdir().expect("temp directory");
    let path = dir.path().join("pm-planning.json");
    let before = write_fixture(&path, &json!({"revision": 7}));

    let loaded = load_planning_state(&path).expect("load missing legacy fields");
    assert_eq!(loaded.schema_version, 1);
    assert_eq!(loaded.revision, 7);
    assert!(loaded.goals.is_empty());
    assert!(loaded.recurring_items.is_empty());
    assert_eq!(
        fs::read(&path).expect("read untouched legacy state"),
        before
    );

    mutate_planning_state(&path, 7, |_| Ok(())).expect("commit migrated state");
    let persisted: Value = serde_json::from_slice(&fs::read(&path).expect("read migrated state"))
        .expect("valid migrated JSON");
    assert_eq!(persisted["schema_version"], 1);
    assert_eq!(persisted["revision"], 8);
    assert_eq!(persisted["goals"], json!([]));
}

#[test]
fn schema_zero_preserves_legacy_goal_and_migrates_on_next_commit() {
    let dir = tempfile::tempdir().expect("temp directory");
    let path = dir.path().join("pm-planning.json");
    let before = write_fixture(
        &path,
        &json!({
            "schema_version": 0,
            "revision": 2,
            "goals": [populated_goal("legacy-goal")]
        }),
    );

    let loaded = load_planning_state(&path).expect("load schema zero");
    assert_eq!(loaded.schema_version, 1);
    assert_eq!(loaded.goals[0].id, "legacy-goal");
    assert_eq!(fs::read(&path).expect("read original schema zero"), before);

    let (committed, ()) =
        mutate_planning_state(&path, 2, |_| Ok(())).expect("commit schema migration");
    assert_eq!(committed.schema_version, 1);
    assert_eq!(committed.revision, 3);
    let reloaded = load_planning_state(&path).expect("reload migrated legacy state");
    assert_eq!(reloaded.goals[0].id, "legacy-goal");
    let persisted: Value = serde_json::from_slice(&fs::read(&path).expect("read migration"))
        .expect("valid migrated JSON");
    assert_eq!(persisted["schema_version"], 1);
}

#[test]
fn future_schema_is_rejected_without_invoking_mutation_or_changing_bytes() {
    let dir = tempfile::tempdir().expect("temp directory");
    let path = dir.path().join("pm-planning.json");
    let before = write_fixture(&path, &json!({"schema_version": 2, "revision": 9}));
    let calls = AtomicUsize::new(0);

    assert!(load_planning_state(&path).is_err());
    assert!(mutate_planning_state(&path, 9, |_| {
        calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    })
    .is_err());

    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(fs::read(&path).expect("read future schema"), before);
}

#[test]
fn corrupt_json_fails_closed_and_preserves_unique_forensic_copies() {
    let dir = tempfile::tempdir().expect("temp directory");
    let path = dir.path().join("pm-planning.json");
    let before = b"{\"schema_version\":1,\"revision\":";
    fs::write(&path, before).expect("write corrupt planning state");
    let calls = AtomicUsize::new(0);

    assert!(load_planning_state(&path).is_err());
    let initial_copies = corrupt_copies(&path).len();
    for attempt in 1..=2 {
        assert!(mutate_planning_state(&path, 0, |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .is_err());
        assert_eq!(
            fs::read(&path).expect("read original corrupt bytes"),
            before
        );
        let copies = corrupt_copies(&path);
        assert_eq!(copies.len(), initial_copies + attempt);
        for copy in copies {
            assert_eq!(fs::read(copy).expect("read forensic copy"), before);
        }
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn each_successful_commit_advances_revision_once() {
    let dir = tempfile::tempdir().expect("temp directory");
    let path = dir.path().join("pm-planning.json");

    for expected_revision in 0..3 {
        let (committed, observed_revision) =
            mutate_planning_state(&path, expected_revision, |state| Ok(state.revision))
                .expect("commit planning revision");
        assert_eq!(observed_revision, expected_revision);
        assert_eq!(committed.revision, expected_revision + 1);
        assert_eq!(
            load_planning_state(&path)
                .expect("reload revision")
                .revision,
            expected_revision + 1
        );
    }
}

#[test]
fn stale_expected_revision_never_runs_closure_or_overwrites_winner() {
    let dir = tempfile::tempdir().expect("temp directory");
    let path = dir.path().join("pm-planning.json");
    mutate_planning_state(&path, 0, |state| {
        state.goals.push(populated_goal("winner"));
        Ok(())
    })
    .expect("commit winning revision");
    let before = fs::read(&path).expect("read winning bytes");
    let calls = AtomicUsize::new(0);

    assert!(mutate_planning_state(&path, 0, |state| {
        calls.fetch_add(1, Ordering::SeqCst);
        state.goals.clear();
        Ok(())
    })
    .is_err());

    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(fs::read(&path).expect("read retained winner"), before);
    assert_eq!(
        load_planning_state(&path).expect("reload winner").goals[0].id,
        "winner"
    );
}

#[test]
fn failed_mutation_leaves_existing_state_and_revision_unchanged() {
    let dir = tempfile::tempdir().expect("temp directory");
    let path = dir.path().join("pm-planning.json");
    mutate_planning_state(&path, 0, |state| {
        state.goals.push(populated_goal("retained"));
        Ok(())
    })
    .expect("commit retained state");
    let before = fs::read(&path).expect("read retained bytes");

    let result = mutate_planning_state(&path, 1, |state| {
        state.goals.clear();
        Err::<(), _>(io::Error::other("planning change refused"))
    });

    assert!(result.is_err());
    assert_eq!(fs::read(&path).expect("read failed mutation state"), before);
    let loaded = load_planning_state(&path).expect("reload retained state");
    assert_eq!(loaded.revision, 1);
    assert_eq!(loaded.goals[0].id, "retained");
}

#[test]
fn failed_initial_mutation_does_not_publish_default_state() {
    let dir = tempfile::tempdir().expect("temp directory");
    let path = dir.path().join("project-state/pm-planning.json");

    let result = mutate_planning_state(&path, 0, |state| {
        state.goals.push(populated_goal("uncommitted"));
        Err::<(), _>(io::Error::other("initial planning change refused"))
    });

    assert!(result.is_err());
    assert!(!path.exists());
    assert_eq!(
        load_planning_state(&path).expect("safe defaults").revision,
        0
    );
}

#[test]
fn revision_overflow_refuses_commit_without_replacing_original_state() {
    let dir = tempfile::tempdir().expect("temp directory");
    let path = dir.path().join("pm-planning.json");
    let before = write_fixture(&path, &json!({"schema_version": 1, "revision": u64::MAX}));

    assert!(mutate_planning_state(&path, u64::MAX, |state| {
        state.goals.push(populated_goal("overflow"));
        Ok(())
    })
    .is_err());

    assert_eq!(fs::read(&path).expect("read overflow revision"), before);
    assert_eq!(
        load_planning_state(&path)
            .expect("reload retained maximum revision")
            .revision,
        u64::MAX
    );
}

#[test]
fn concurrent_writers_with_same_expected_revision_have_exactly_one_winner() {
    let dir = tempfile::tempdir().expect("temp directory");
    let path = dir.path().join("project-state/pm-planning.json");
    let barrier = Arc::new(Barrier::new(3));
    let calls = Arc::new(AtomicUsize::new(0));
    let mut workers = Vec::new();

    for id in ["first", "second"] {
        let path = path.clone();
        let barrier = Arc::clone(&barrier);
        let calls = Arc::clone(&calls);
        workers.push(thread::spawn(move || {
            barrier.wait();
            mutate_planning_state(&path, 0, |state| {
                calls.fetch_add(1, Ordering::SeqCst);
                state.goals.push(populated_goal(id));
                Ok(())
            })
        }));
    }

    barrier.wait();
    let outcomes: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().expect("planning writer completes"))
        .collect();
    assert_eq!(outcomes.iter().filter(|outcome| outcome.is_ok()).count(), 1);
    assert_eq!(
        outcomes.iter().filter(|outcome| outcome.is_err()).count(),
        1
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let loaded = load_planning_state(&path).expect("load concurrent winner");
    assert_eq!(loaded.revision, 1);
    assert_eq!(loaded.goals.len(), 1);
    assert!(["first", "second"].contains(&loaded.goals[0].id.as_str()));
}

#[test]
fn atomic_commit_replaces_file_without_overwriting_existing_hardlink() {
    let dir = tempfile::tempdir().expect("temp directory");
    let path = dir.path().join("pm-planning.json");
    mutate_planning_state(&path, 0, |state| {
        state.goals.push(populated_goal("before"));
        Ok(())
    })
    .expect("commit initial state");
    let before = fs::read(&path).expect("read initial bytes");
    let old_link = dir.path().join("previous-planning-state.json");
    fs::hard_link(&path, &old_link).expect("retain old state inode");

    mutate_planning_state(&path, 1, |state| {
        state.goals[0].status = "completed".to_string();
        Ok(())
    })
    .expect("atomically replace state");

    assert_eq!(fs::read(old_link).expect("read old inode"), before);
    assert_ne!(fs::read(&path).expect("read replacement bytes"), before);
    let loaded = load_planning_state(&path).expect("reload replacement");
    assert_eq!(loaded.revision, 2);
    assert_eq!(loaded.goals[0].status, "completed");
}

#[test]
fn planning_path_uses_existing_canonical_project_state_directory() {
    let repo = tempfile::tempdir().expect("isolated project path");
    let expected = gwt_core::paths::gwt_project_dir_for_repo_path(repo.path())
        .join("project-state/pm-planning.json");

    assert_eq!(planning_state_path_for_repo(repo.path()), expected);
    assert_ne!(
        planning_state_path_for_repo(repo.path()),
        gwt::pm_registry::pm_prefs_path_for_repo_path(repo.path())
    );
}

#[test]
fn default_state_serializes_all_typed_record_collections() {
    let value = serde_json::to_value(PmPlanningState::default()).expect("serialize default state");

    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["revision"], 0);
    for key in [
        "goals",
        "roadmap_items",
        "milestones",
        "risks",
        "decisions",
        "recurring_items",
        "policies",
        "latest_forecasts",
    ] {
        assert_eq!(value[key], json!([]), "default collection {key}");
    }
}

#[test]
fn mutation_cannot_rewrite_store_managed_schema_or_revision() {
    let dir = tempfile::tempdir().expect("temp directory");
    let path = dir.path().join("pm-planning.json");
    mutate_planning_state(&path, 0, |_| Ok(())).expect("initial commit");
    let before = fs::read(&path).expect("initial bytes");

    for (schema, revision) in [(2, 1), (1, 100)] {
        assert!(mutate_planning_state(&path, 1, |state| {
            state.schema_version = schema;
            state.revision = revision;
            Ok(())
        })
        .is_err());
        assert_eq!(fs::read(&path).expect("retained state"), before);
    }
}

#[test]
fn unrecognized_data_is_not_silently_dropped_on_commit() {
    let dir = tempfile::tempdir().expect("temp directory");
    let path = dir.path().join("pm-planning.json");
    let before = write_fixture(&path, &json!({"revision": 4, "unknown_policy": "keep me"}));

    assert!(load_planning_state(&path).is_err());
    assert!(mutate_planning_state(&path, 4, |_| Ok(())).is_err());
    assert_eq!(fs::read(&path).expect("retained unknown data"), before);
}
