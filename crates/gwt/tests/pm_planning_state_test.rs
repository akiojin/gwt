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
    assert!(corrupt_copies(&path).is_empty());
}

#[test]
fn incompatible_valid_json_retries_do_not_create_corrupt_backups() {
    for document in [
        json!({"revision": 4, "unknown_policy": "keep me"}),
        json!(true),
        json!({"goals": [{"id": 123}]}),
    ] {
        let dir = tempfile::tempdir().expect("temp directory");
        let path = dir.path().join("pm-planning.json");
        let before = write_fixture(&path, &document);
        let calls = AtomicUsize::new(0);

        for _ in 0..2 {
            let error = mutate_planning_state(&path, 4, |_| {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .expect_err("incompatible data must fail closed");
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert_eq!(
                fs::read(&path).expect("retained incompatible bytes"),
                before
            );
            assert!(corrupt_copies(&path).is_empty(), "document: {document}");
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
}

fn plan_request(kind: &str, revision: u64, key: &str) -> gwt::pm_planning::PlanningApplyRequest {
    serde_json::from_value(json!({
        "action": {"kind": kind, "value": {"id": kind}},
        "expected_revision": revision, "reason": "Accepted project plan", "idempotency_key": key
    }))
    .unwrap()
}

#[test]
fn planning_apply_upserts_all_seven_records_with_atomic_receipts() {
    use gwt::pm_planning::apply_planning_action;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pm-planning.json");
    for (revision, kind) in [
        "goal",
        "milestone",
        "risk",
        "decision",
        "policy",
        "recurring",
        "link",
    ]
    .into_iter()
    .enumerate()
    {
        let receipt =
            apply_planning_action(&path, "pm-a", &plan_request(kind, revision as u64, kind))
                .unwrap();
        assert_eq!(receipt.actor, "pm-a");
        assert_eq!(receipt.requester, "pm-a");
        assert_eq!(receipt.action_kind, kind);
        assert_eq!(receipt.target_id, kind);
        assert_eq!(receipt.expected_revision, revision as u64);
        assert_eq!(receipt.resulting_revision, revision as u64 + 1);
        assert_eq!(receipt.outcome, "applied");
        let state = load_planning_state(&path).unwrap();
        assert_eq!(state.revision, receipt.resulting_revision);
        assert_eq!(state.audit_receipts.last(), Some(&receipt));
    }
    let mut request = plan_request("goal", 7, "replace-goal");
    if let gwt::pm_planning::PlanningAction::Goal(goal) = &mut request.action {
        goal.outcome = "Updated outcome".into();
    }
    apply_planning_action(&path, "pm-a", &request).unwrap();
    let state = load_planning_state(&path).unwrap();
    assert_eq!(state.revision, 8);
    assert_eq!(state.goals.len(), 1);
    assert_eq!(state.goals[0].outcome, "Updated outcome");
    assert_eq!(state.audit_receipts.len(), 8);
    assert_eq!(state.milestones.len(), 1);
    assert_eq!(state.risks.len(), 1);
    assert_eq!(state.decisions.len(), 1);
    assert_eq!(state.policies.len(), 1);
    assert_eq!(state.recurring_items.len(), 1);
    assert_eq!(state.roadmap_items.len(), 1);
}

#[test]
fn planning_change_receipts_keep_direct_impact_and_exact_history_on_replay() {
    use gwt::pm_planning::{apply_planning_action, PlanningApplyRequest};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pm-planning.json");
    write_fixture(
        &path,
        &json!({
            "milestones":[{"id":"m","target_date":"2026-12-01","health":"green","issues":[1]}],
            "roadmap_items":[
                {"id":"linked","milestone_id":"m","issues":[2,1]},
                {"id":"moving","category":"maintenance","horizon":"now","issues":[3,1]},
                {"id":"unrelated","milestone_id":"other","issues":[99]}
            ]
        }),
    );
    let request: PlanningApplyRequest = serde_json::from_value(json!({
        "action":{"kind":"milestone","value":{"id":"m","target_date":"2027-01-01",
            "health":"yellow","issues":[4]}},
        "expected_revision":0,"reason":"Adjust accepted delivery scope","idempotency_key":"move-date"
    })).unwrap();
    let receipt = apply_planning_action(&path, "pm-a", &request).unwrap();
    let value = serde_json::to_value(&receipt).unwrap();
    let impact = &value["change_impact"];
    assert_eq!(impact["record_created"], false);
    assert_eq!(
        impact["changes"]["target_date"],
        json!({"before":"2026-12-01","after":"2027-01-01"})
    );
    assert_eq!(
        impact["changes"]["health"],
        json!({"before":"green","after":"yellow"})
    );
    assert_eq!(
        impact["changes"]["issues"],
        json!({"before":[1],"after":[4]})
    );
    assert_eq!(impact["changes"].as_object().unwrap().len(), 3);
    assert_eq!(impact["directly_affected_issues"], json!([1, 2, 4]));
    assert_eq!(
        impact["missing_inputs"],
        json!(["dependency_graph", "active_owner_handoff"])
    );
    let mut link = plan_request("link", 1, "move-link");
    link.action = serde_json::from_value(json!({"kind":"link","value":{
        "id":"moving","category":"maintenance","horizon":"next","issues":[5,3]
    }}))
    .unwrap();
    let moved = serde_json::to_value(apply_planning_action(&path, "pm-a", &link).unwrap()).unwrap();
    assert_eq!(
        moved["change_impact"]["directly_affected_issues"],
        json!([1, 3, 5])
    );
    assert_eq!(
        moved["change_impact"]["changes"]["horizon"],
        json!({"before":"now","after":"next"})
    );
    let other = apply_planning_action(&path, "pm-a", &plan_request("risk", 2, "risk")).unwrap();
    assert!(serde_json::to_value(other)
        .unwrap()
        .get("change_impact")
        .is_none());
    let before = fs::read(&path).unwrap();
    assert_eq!(
        apply_planning_action(&path, "pm-a", &request).unwrap(),
        receipt
    );
    assert_eq!(fs::read(&path).unwrap(), before);
    link.expected_revision = 3;
    link.idempotency_key = "same-link".into();
    let same = serde_json::to_value(apply_planning_action(&path, "pm-a", &link).unwrap()).unwrap();
    assert_eq!(same["change_impact"]["changes"], json!({}));
    assert_eq!(same["change_impact"]["directly_affected_issues"], json!([]));
    let created = serde_json::to_value(
        apply_planning_action(&path, "pm-a", &plan_request("milestone", 4, "new")).unwrap(),
    )
    .unwrap();
    assert_eq!(created["change_impact"]["record_created"], true);
    assert_eq!(
        created["change_impact"]["changes"]["id"],
        json!({"before":null,"after":"milestone"})
    );
}

#[test]
fn goal_change_receipts_use_explicit_links_and_preserve_original_replay() {
    use gwt::pm_planning::{apply_planning_action, PlanningApplyRequest};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pm-planning.json");
    write_fixture(
        &path,
        &json!({
            "goals":[{"id":"g","target":"3","horizon":"quarter"}],
            "roadmap_items":[
                {"id":"a","goal_id":"g","milestone_id":"m","issues":[3,1]},
                {"id":"b","goal_id":"g","category":"maintenance","issues":[2,3]},
                {"id":"other","goal_id":"other","issues":[99]},
                {"id":"category","category":"maintenance","issues":[77]}
            ],
            "milestones":[{"id":"m","issues":[88]}]
        }),
    );
    let request: PlanningApplyRequest = serde_json::from_value(json!({
        "action":{"kind":"goal","value":{"id":"g","target":"5","horizon":"year"}},
        "expected_revision":0,"reason":"Revise accepted goal","idempotency_key":"goal-target"
    }))
    .unwrap();
    let receipt = apply_planning_action(&path, "pm-a", &request).unwrap();
    let value = serde_json::to_value(&receipt).unwrap();
    assert_eq!(value["change_impact"]["record_created"], false);
    assert_eq!(
        value["change_impact"]["changes"],
        json!({
            "target":{"before":"3","after":"5"},
            "horizon":{"before":"quarter","after":"year"}
        })
    );
    assert_eq!(
        value["change_impact"]["directly_affected_issues"],
        json!([1, 2, 3])
    );
    assert_eq!(
        value["change_impact"]["missing_inputs"],
        json!(["dependency_graph", "active_owner_handoff"])
    );

    let link: PlanningApplyRequest = serde_json::from_value(json!({
        "action":{"kind":"link","value":{"id":"a","goal_id":"g","issues":[42]}},
        "expected_revision":1,"reason":"Adjust linked scope","idempotency_key":"goal-link"
    }))
    .unwrap();
    apply_planning_action(&path, "pm-a", &link).unwrap();
    let bytes = fs::read(&path).unwrap();
    assert_eq!(
        apply_planning_action(&path, "pm-a", &request).unwrap(),
        receipt
    );
    assert_eq!(fs::read(&path).unwrap(), bytes);

    let mut same = request;
    same.expected_revision = 2;
    same.idempotency_key = "same-goal".into();
    let unchanged =
        serde_json::to_value(apply_planning_action(&path, "pm-a", &same).unwrap()).unwrap();
    assert_eq!(unchanged["change_impact"]["changes"], json!({}));
    assert_eq!(
        unchanged["change_impact"]["directly_affected_issues"],
        json!([])
    );
    let created = serde_json::to_value(
        apply_planning_action(&path, "pm-a", &plan_request("goal", 3, "new-goal")).unwrap(),
    )
    .unwrap();
    assert_eq!(created["change_impact"]["record_created"], true);
    assert_eq!(
        created["change_impact"]["changes"]["id"],
        json!({"before":null,"after":"goal"})
    );
    assert_eq!(
        created["change_impact"]["directly_affected_issues"],
        json!([])
    );
}

#[test]
fn planning_change_history_redacts_legacy_values_and_accepts_old_receipts() {
    use gwt::pm_planning::apply_planning_action;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pm-planning.json");
    write_fixture(
        &path,
        &json!({"milestones":[{"id":"milestone",
        "exit_criteria":["Bearer example0123456789"]}]}),
    );
    let receipt =
        apply_planning_action(&path, "pm-a", &plan_request("milestone", 0, "redact")).unwrap();
    let mut value = serde_json::to_value(receipt).unwrap();
    let captured = value["change_impact"]["changes"]["exit_criteria"]["before"].to_string();
    assert!(captured.contains("REDACTED"), "{captured}");
    assert!(!captured.contains("example0123456789"));
    value.as_object_mut().unwrap().remove("change_impact");
    let mut legacy = serde_json::to_value(load_planning_state(&path).unwrap()).unwrap();
    legacy["audit_receipts"] = json!([value]);
    write_fixture(&path, &legacy);
    let loaded = load_planning_state(&path).unwrap();
    assert!(serde_json::to_value(&loaded.audit_receipts[0])
        .unwrap()
        .get("change_impact")
        .is_none());

    write_fixture(
        &path,
        &json!({"goals":[{"id":"goal",
        "target":"Bearer example0123456789"}]}),
    );
    let receipt =
        apply_planning_action(&path, "pm-a", &plan_request("goal", 0, "redact-goal")).unwrap();
    let value = serde_json::to_value(receipt).unwrap();
    let captured = value["change_impact"]["changes"]["target"]["before"].to_string();
    assert!(captured.contains("REDACTED"), "{captured}");
    assert!(!fs::read_to_string(&path)
        .unwrap()
        .contains("example0123456789"));
}

#[test]
fn planning_apply_replays_exact_request_after_later_writes_and_refuses_conflicts() {
    use gwt::pm_planning::apply_planning_action;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pm-planning.json");
    let request = plan_request("goal", 0, "first");
    let receipt = apply_planning_action(&path, "pm-a", &request).unwrap();
    apply_planning_action(&path, "pm-a", &plan_request("risk", 1, "second")).unwrap();
    let bytes = fs::read(&path).unwrap();
    assert_eq!(
        apply_planning_action(&path, "pm-a", &request).unwrap(),
        receipt
    );
    let mut conflict = request.clone();
    conflict.reason = "Different rationale".into();
    assert!(apply_planning_action(&path, "pm-a", &conflict).is_err());
    assert!(apply_planning_action(&path, "other-pm", &request).is_err());
    assert!(apply_planning_action(&path, "pm-a", &plan_request("risk", 0, "stale")).is_err());
    assert_eq!(fs::read(&path).unwrap(), bytes);
}

#[test]
fn planning_apply_rejects_credentials_in_records_and_redacts_audit_reason() {
    use gwt::pm_planning::{apply_planning_action, PlanningAction};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pm-planning.json");
    let token = "ghp_0123456789abcdefghij";
    let mut request = plan_request("goal", 0, "redacted-audit");
    for credential in [
        token,
        "OPENAI_API_KEY=sk-example0123456789",
        "Bearer example0123456789",
    ] {
        if let PlanningAction::Goal(goal) = &mut request.action {
            goal.outcome = credential.into();
        }
        assert!(
            apply_planning_action(&path, "pm-a", &request).is_err(),
            "refuse known credential form"
        );
        assert!(!path.exists());
    }
    request = plan_request("goal", 0, "redacted-audit");
    request.reason = format!("Rotate token={token}\nAuthorization: Bearer {token}\nOPENAI_API_KEY=sk-example0123456789\nBearer example0123456789");
    let receipt = apply_planning_action(&path, "pm-a", &request).unwrap();
    assert!(receipt.reason.contains("***redacted***"));
    let bytes = fs::read_to_string(&path).unwrap();
    assert!(!bytes.contains(token));
    assert!(!bytes.contains("sk-example0123456789"));
    assert!(!bytes.contains("example0123456789"));
    assert!(!bytes.contains("Accepted project plan"));
    assert_eq!(
        apply_planning_action(&path, "pm-a", &request).unwrap(),
        receipt
    );
}

#[test]
fn planning_apply_concurrent_exact_retry_publishes_once() {
    use gwt::pm_planning::apply_planning_action;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pm-planning.json");
    let barrier = Arc::new(Barrier::new(2));
    let workers: Vec<_> = (0..2)
        .map(|_| {
            let path = path.clone();
            let barrier = barrier.clone();
            thread::spawn(move || {
                barrier.wait();
                apply_planning_action(&path, "pm-a", &plan_request("goal", 0, "shared"))
            })
        })
        .collect();
    let receipts: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap().unwrap())
        .collect();
    assert_eq!(receipts[0], receipts[1]);
    let state = load_planning_state(&path).unwrap();
    assert_eq!(state.revision, 1);
    assert_eq!(state.goals.len(), 1);
    assert_eq!(state.audit_receipts.len(), 1);
}
