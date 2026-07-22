use chrono::TimeZone;

use super::*;

#[cfg(windows)]
fn open_directory_for_mtime(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_READ_ATTRIBUTES: u32 = 0x0080;
    const FILE_WRITE_ATTRIBUTES: u32 = 0x0100;

    OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES | FILE_WRITE_ATTRIBUTES)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
}

#[cfg(not(windows))]
fn open_directory_for_mtime(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    std::fs::File::open(path)
}

// SPEC-2359 close-latency root fix: the works.json cache must stop
// re-parsing unchanged files, observe content changes, and never cache a
// synthesized fallback against a missing works.json.
#[test]
fn work_items_cache_reuses_unchanged_file_and_reparses_on_change() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let work_items_path = tmp.path().join("works.json");
    let current_path = tmp.path().join("current.json");
    let journal_path = tmp.path().join("journal.jsonl");
    let project_root = tmp.path().join("repo");
    std::fs::create_dir_all(&project_root).expect("repo dir");

    let now = chrono::Utc::now();
    let mut projection = super::WorkItemsProjection::empty(now);
    projection.apply_event(sample_work_event("work-1", now));
    super::save_workspace_work_items_projection_to_path(&work_items_path, &projection)
        .expect("save works.json");

    let mut cache = super::WorkItemsCache::new();
    let first = cache
        .load_or_synthesize_from_paths(
            &work_items_path,
            &current_path,
            &journal_path,
            &project_root,
        )
        .expect("first load");
    assert_eq!(first.work_items.len(), 1);
    assert_eq!(cache.parse_count, 1);

    let second = cache
        .load_or_synthesize_from_paths(
            &work_items_path,
            &current_path,
            &journal_path,
            &project_root,
        )
        .expect("second load");
    assert_eq!(second.work_items.len(), 1);
    assert_eq!(
        cache.parse_count, 1,
        "unchanged works.json must not re-parse"
    );

    // Grow the file (extra item) so mtime granularity cannot mask the change.
    projection.apply_event(sample_work_event(
        "work-2",
        now + chrono::Duration::seconds(1),
    ));
    super::save_workspace_work_items_projection_to_path(&work_items_path, &projection)
        .expect("resave works.json");
    let third = cache
        .load_or_synthesize_from_paths(
            &work_items_path,
            &current_path,
            &journal_path,
            &project_root,
        )
        .expect("third load");
    assert_eq!(third.work_items.len(), 2, "changed works.json must reload");
    assert_eq!(cache.parse_count, 2);
}

#[test]
fn work_items_loader_classifies_malformed_and_incompatible_json() {
    let temp = tempfile::tempdir().expect("tempdir");
    let malformed_path = temp.path().join("malformed.json");
    std::fs::write(&malformed_path, b"{\"work_items\":").expect("write malformed json");
    assert!(matches!(
        load_workspace_work_items_from_path(&malformed_path),
        Err(GwtError::JsonDecode {
            kind: JsonDecodeKind::Malformed,
            ..
        })
    ));

    let incompatible_path = temp.path().join("incompatible.json");
    let now = Utc.with_ymd_and_hms(2026, 7, 16, 7, 0, 0).unwrap();
    let mut projection = WorkItemsProjection::empty(now);
    let container = WorkspaceExecutionContainerRef {
        branch: Some("feature/future-loader".to_string()),
        worktree_path: Some(temp.path().join("future-loader")),
        pr_number: None,
        pr_url: None,
        pr_state: None,
    };
    let mut event = WorkEvent::new(WorkEventKind::Start, "work-future-loader", now);
    event.agent_session_id = Some("session-future-loader".to_string());
    event.agent_id = Some("codex".to_string());
    event.execution_container = Some(container.clone());
    projection.apply_event(event);
    let legacy_snapshot = projection.work_items[0].clone();
    projection.work_items[0].legacy_metadata_snapshot = Some(Box::new(legacy_snapshot));
    let mut duplicate_event = WorkEvent::new(
        WorkEventKind::Update,
        "work-future-loader",
        now + chrono::Duration::seconds(1),
    );
    duplicate_event.execution_container = Some(container.clone());
    projection.work_items[0].duplicate_event_containers.insert(
        "duplicate-event".to_string(),
        vec![
            DuplicateWorkEventProvenance::Event(Box::new(duplicate_event)),
            DuplicateWorkEventProvenance::LegacyContainer(container),
        ],
    );
    let compatible = serde_json::to_value(&projection).expect("projection json");
    let mut incompatible = compatible.clone();
    incompatible["work_items"][0]["status_category"] =
        serde_json::Value::String("future_state".to_string());
    std::fs::write(
        &incompatible_path,
        serde_json::to_vec_pretty(&incompatible).expect("incompatible json"),
    )
    .expect("write incompatible json");
    assert!(matches!(
        load_workspace_work_items_from_path(&incompatible_path),
        Err(GwtError::JsonDecode {
            kind: JsonDecodeKind::IncompatibleSchema,
            ..
        })
    ));

    let unknown_cases = [
        ("top-level", vec![]),
        ("work-item", vec!["work_items", "0"]),
        ("work-event", vec!["work_items", "0", "events", "0"]),
        ("work-agent", vec!["work_items", "0", "agents", "0"]),
        (
            "work-container",
            vec!["work_items", "0", "execution_containers", "0"],
        ),
        (
            "event-container",
            vec!["work_items", "0", "events", "0", "execution_container"],
        ),
        (
            "legacy-snapshot",
            vec!["work_items", "0", "legacy_metadata_snapshot"],
        ),
        (
            "duplicate-event",
            vec![
                "work_items",
                "0",
                "duplicate_event_containers",
                "duplicate-event",
                "0",
            ],
        ),
        (
            "duplicate-container",
            vec![
                "work_items",
                "0",
                "duplicate_event_containers",
                "duplicate-event",
                "1",
            ],
        ),
    ];
    for (label, path) in unknown_cases {
        let mut value = compatible.clone();
        let mut target = &mut value;
        for component in path {
            target = if let Ok(index) = component.parse::<usize>() {
                &mut target[index]
            } else {
                &mut target[component]
            };
        }
        target
            .as_object_mut()
            .expect("unknown field target must be an object")
            .insert(
                "future_schema_field".to_string(),
                serde_json::json!({ "preserve": true }),
            );
        let path = temp.path().join(format!("unknown-{label}.json"));
        let original = serde_json::to_vec_pretty(&value).expect("unknown field json");
        std::fs::write(&path, &original).expect("write unknown field json");

        assert!(matches!(
            load_workspace_work_items_from_path(&path),
            Err(GwtError::JsonDecode {
                kind: JsonDecodeKind::IncompatibleSchema,
                ..
            })
        ));
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }
}

// ---------------------------------------------------------------------
// SPEC-2359 T-820/T-821 (US-88 / FR-557): release A is a reader-first
// compatibility boundary. Direct Work-event JSONL records may carry a future
// kind or additive top-level fields and must roundtrip opaquely, while every
// projection / transaction / identity-container schema stays strict.
// ---------------------------------------------------------------------

const T820_MIXED_EVENT_LOG: &str = concat!(
    r#"{ "id":"event-known-start", "work_item_id":"work-mixed-version", "kind":"start", "title":"Known title", "status_category":"active", "updated_at":"2026-07-22T02:00:00Z" }"#,
    "\n",
    r#"{"id":"event-known-additive","work_item_id":"work-mixed-version","kind":"update","summary":"Known update with a future field","updated_at":"2026-07-22T02:01:00Z","future_event_field":{"nested":[1,"two",true]}}"#,
    "\n",
    r#"{ "id":"event-future-correction", "work_item_id":"work-mixed-version", "kind":"correction", "corrects_event_ids":["event-known-start"], "patch":{"title":"Corrected by release B"}, "updated_at":"2026-07-22T02:02:00Z" }"#,
    "\n",
);

fn t820_roundtrip_work_event_log(source: &Path, destination: &Path) -> Result<()> {
    let records = read_workspace_work_event_records_from_path(source)?;
    append_workspace_work_event_records_to_path(destination, &records)
}

#[test]
fn mixed_version_event_log_roundtrips_unknown_kind_and_fields_byte_exact() {
    let temp = tempfile::tempdir().expect("tempdir");
    let source = temp.path().join("source.jsonl");
    let destination = temp.path().join("roundtrip.jsonl");
    fs::write(&source, T820_MIXED_EVENT_LOG.as_bytes()).expect("write mixed event log");

    t820_roundtrip_work_event_log(&source, &destination)
        .expect("release A must roundtrip future event records opaquely");

    let roundtripped = fs::read(&destination).expect("read roundtripped log");
    assert_eq!(
        roundtripped,
        T820_MIXED_EVENT_LOG.as_bytes(),
        "unknown kind, unknown field values, key order, and original event bytes must survive"
    );
    let source_values = T820_MIXED_EVENT_LOG
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("source JSON value"))
        .collect::<Vec<_>>();
    let roundtrip_values = String::from_utf8(roundtripped)
        .expect("roundtrip UTF-8")
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("roundtrip JSON value"))
        .collect::<Vec<_>>();
    assert_eq!(
        roundtrip_values, source_values,
        "JSON values must be lossless"
    );
}

#[test]
fn mixed_version_event_reader_keeps_identity_and_container_schemas_strict() {
    let temp = tempfile::tempdir().expect("tempdir");
    let cases = [
        (
            "identity",
            concat!(
                r#"{"id":"event-bad-identity","work_item_id":{"future":"shape"},"kind":"update","updated_at":"2026-07-22T02:03:00Z"}"#,
                "\n"
            ),
        ),
        (
            "container",
            concat!(
                r#"{"id":"event-bad-container","work_item_id":"work-mixed-version","kind":"update","execution_container":{"branch":"work/mixed","future_container_field":true},"updated_at":"2026-07-22T02:04:00Z"}"#,
                "\n"
            ),
        ),
    ];

    for (label, body) in cases {
        let source = temp.path().join(format!("{label}-source.jsonl"));
        let destination = temp.path().join(format!("{label}-roundtrip.jsonl"));
        fs::write(&source, body).expect("write incompatible event");

        assert!(
            t820_roundtrip_work_event_log(&source, &destination).is_err(),
            "unknown {label} schema must fail closed instead of becoming opaque"
        );
        assert!(
            !destination.exists(),
            "unknown {label} schema must not partially write a destination log"
        );
    }
}

#[test]
fn mixed_version_event_log_keeps_known_projection_stable() {
    let temp = tempfile::tempdir().expect("tempdir");
    let events = temp.path().join("events.jsonl");
    let works = temp.path().join("works.json");
    let marker = temp.path().join("work-items-rebuild.json");
    fs::write(&events, T820_MIXED_EVENT_LOG.as_bytes()).expect("write mixed event log");
    let original = fs::read(&events).expect("snapshot mixed event log");

    assert_eq!(
        rebuild_work_items_from_events_paths(&works, &events, &marker)
            .expect("release A must safely read a mixed-version event log"),
        WorkItemsRebuildOutcome::Applied
    );

    let projection = load_workspace_work_items_from_path(&works)
        .expect("load rebuilt projection")
        .expect("rebuilt projection");
    let item = projection
        .work_items
        .iter()
        .find(|item| item.id == "work-mixed-version")
        .expect("known Work");
    assert_eq!(item.title, "Known title");
    assert_eq!(
        item.summary.as_deref(),
        Some("Known update with a future field")
    );
    assert_eq!(item.status_category, WorkspaceStatusCategory::Active);
    assert_eq!(
        item.events
            .iter()
            .map(|event| event.id.as_str())
            .collect::<Vec<_>>(),
        vec!["event-known-start", "event-known-additive"],
        "known event data is projected and an unknown kind remains opaque"
    );
    assert_eq!(
        fs::read(&events).expect("read preserved mixed event log"),
        original,
        "projection rebuild must never rewrite opaque event bytes"
    );
}

#[test]
fn tracked_event_rebuild_never_repairs_or_truncates_source_bytes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let known_event = r#"{"id":"event-byte-preserving","work_item_id":"work-byte-preserving","kind":"start","updated_at":"2026-07-22T02:00:00Z"}"#;

    let unterminated_events = temp.path().join("unterminated-events.jsonl");
    let unterminated_works = temp.path().join("unterminated-works.json");
    let unterminated_marker = temp.path().join("unterminated-marker.json");
    fs::write(&unterminated_events, known_event.as_bytes()).expect("write unterminated event");
    let unterminated_original = fs::read(&unterminated_events).expect("snapshot unterminated log");

    assert_eq!(
        rebuild_work_items_from_events_paths(
            &unterminated_works,
            &unterminated_events,
            &unterminated_marker,
        )
        .expect("a complete final JSON record does not require a newline"),
        WorkItemsRebuildOutcome::Applied
    );
    assert_eq!(
        fs::read(&unterminated_events).expect("read unterminated log"),
        unterminated_original,
        "a projection rebuild must not append a newline to tracked input"
    );

    let partial_events = temp.path().join("partial-events.jsonl");
    let partial_works = temp.path().join("partial-works.json");
    let partial_marker = temp.path().join("partial-marker.json");
    let partial_body = format!("{known_event}\n{{\"id\":\"partial-tail\"");
    fs::write(&partial_events, partial_body.as_bytes()).expect("write partial event log");
    let partial_original = fs::read(&partial_events).expect("snapshot partial log");

    assert!(
        rebuild_work_items_from_events_paths(&partial_works, &partial_events, &partial_marker)
            .is_err(),
        "a partial tracked record must fail without silently discarding evidence"
    );
    assert_eq!(
        fs::read(&partial_events).expect("read partial log"),
        partial_original,
        "a failed projection rebuild must preserve the partial tracked record byte-for-byte"
    );
    assert!(!partial_works.exists());
    assert!(!partial_marker.exists());
}

#[test]
fn release_a_typed_writer_cannot_construct_a_correction_event() {
    const {
        assert!(
            !WORK_EVENT_CORRECTION_WRITER_ENABLED,
            "release A must keep every production Correction writer disabled"
        );
    }
    assert!(
        serde_json::from_str::<WorkEventKind>(r#""correction""#).is_err(),
        "Correction must not become a release A WorkEventKind writer variant"
    );
    assert!(
        serde_json::from_str::<WorkEvent>(T820_MIXED_EVENT_LOG.lines().nth(2).unwrap()).is_err(),
        "the production typed writer must not accept opaque Correction JSON as a WorkEvent"
    );
}

#[test]
fn work_items_cache_never_caches_synthesized_fallback() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let work_items_path = tmp.path().join("works.json");
    let current_path = tmp.path().join("current.json");
    let journal_path = tmp.path().join("journal.jsonl");
    let project_root = tmp.path().join("repo");
    std::fs::create_dir_all(&project_root).expect("repo dir");

    let mut cache = super::WorkItemsCache::new();
    let synthesized = cache
        .load_or_synthesize_from_paths(
            &work_items_path,
            &current_path,
            &journal_path,
            &project_root,
        )
        .expect("synthesized load");
    assert!(synthesized.work_items.is_empty());

    // works.json appears afterwards: the next load must see it.
    let now = chrono::Utc::now();
    let mut projection = super::WorkItemsProjection::empty(now);
    projection.apply_event(sample_work_event("work-1", now));
    super::save_workspace_work_items_projection_to_path(&work_items_path, &projection)
        .expect("save works.json");
    let loaded = cache
        .load_or_synthesize_from_paths(
            &work_items_path,
            &current_path,
            &journal_path,
            &project_root,
        )
        .expect("post-create load");
    assert_eq!(loaded.work_items.len(), 1);
}

#[test]
fn load_workspace_work_items_backfills_progress_summary_from_legacy_events() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let work_items_path = tmp.path().join("works.json");
    let t1 = chrono::Utc.with_ymd_and_hms(2026, 6, 16, 10, 0, 0).unwrap();
    let t2 = chrono::Utc.with_ymd_and_hms(2026, 6, 16, 11, 0, 0).unwrap();

    let mut projection = super::WorkItemsProjection::empty(t1);
    let mut start = super::WorkEvent::new(super::WorkEventKind::Start, "legacy-work", t1);
    start.title = Some("Project Tabs UX".to_string());
    start.intent = Some("Compared browser-tab and built-in tab switching UX.".to_string());
    projection.apply_event(start);
    let mut update = super::WorkEvent::new(super::WorkEventKind::Update, "legacy-work", t2);
    update.summary =
        Some("Implemented project switcher and quiet Agent completion notifications.".to_string());
    projection.apply_event(update);

    projection.work_items[0].progress_summary = None;
    super::save_workspace_work_items_projection_to_path(&work_items_path, &projection)
        .expect("save legacy works.json");

    let loaded = super::load_workspace_work_items_from_path(&work_items_path)
        .expect("load works.json")
        .expect("items");
    let progress = loaded.work_items[0]
        .progress_summary
        .as_deref()
        .expect("loader should backfill derived progress_summary");
    assert!(progress.contains("Compared browser-tab"), "{progress}");
    assert!(
        progress.contains("Implemented project switcher"),
        "{progress}"
    );
}

#[test]
fn load_workspace_work_items_preserves_saved_progress_summary() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let work_items_path = tmp.path().join("works.json");
    let now = chrono::Utc.with_ymd_and_hms(2026, 6, 16, 10, 0, 0).unwrap();

    let mut projection = super::WorkItemsProjection::empty(now);
    let mut event = super::WorkEvent::new(super::WorkEventKind::Start, "saved-work", now);
    event.title = Some("Saved progress".to_string());
    event.intent = Some("Short latest focus".to_string());
    projection.apply_event(event);
    projection.work_items[0].progress_summary =
        Some("Human-written cumulative progress must stay intact.".to_string());
    super::save_workspace_work_items_projection_to_path(&work_items_path, &projection)
        .expect("save works.json");

    let loaded = super::load_workspace_work_items_from_path(&work_items_path)
        .expect("load works.json")
        .expect("items");
    assert_eq!(
        loaded.work_items[0].progress_summary.as_deref(),
        Some("Human-written cumulative progress must stay intact.")
    );
}

fn sample_work_event(work_id: &str, updated_at: chrono::DateTime<chrono::Utc>) -> super::WorkEvent {
    let mut event = super::WorkEvent::new(super::WorkEventKind::Start, work_id, updated_at);
    event.status_category = Some(super::WorkspaceStatusCategory::Active);
    event.title = Some(format!("title {work_id}"));
    event
}
/// SPEC-2359 Phase W-11 (US-58 / SC-228): the one-time reset clears
/// legacy title_summary / current_focus exactly once (version-guarded),
/// later runs are a no-op, and agent-authored values written after the
/// reset are preserved.
#[test]
fn reset_legacy_agent_identity_clears_once_and_preserves_later_values() {
    let temp = tempfile::tempdir().expect("tempdir");
    let current_path = temp.path().join("current.json");

    let mut projection = WorkspaceProjection::default_for_project(temp.path());
    projection.agents.push(WorkspaceAgentSummary {
        session_id: "sess-legacy".to_string(),
        window_id: None,
        agent_id: "codex".to_string(),
        display_name: "Codex".to_string(),
        status_category: WorkspaceStatusCategory::Active,
        current_focus: Some("/gwt-discussion 生プロンプト focus".to_string()),
        title_summary: Some("あなたの目的は何ですか".to_string()),
        worktree_path: None,
        branch: None,
        last_board_entry_id: None,
        last_board_entry_kind: None,
        coordination_scope: None,
        affiliation_status: WorkspaceAgentAffiliationStatus::Assigned,
        workspace_id: None,
        updated_at: Utc::now(),
    });
    save_workspace_projection_to_path(&current_path, &projection).expect("save");

    // First reset clears the legacy values and writes the marker.
    let applied = reset_legacy_agent_identity_at(&current_path).expect("reset");
    assert!(applied, "first reset should run and write the marker");
    let after = load_workspace_projection_from_path(&current_path)
        .expect("load")
        .expect("present");
    assert_eq!(after.agents[0].title_summary, None);
    assert_eq!(after.agents[0].current_focus, None);

    // The agent authors a real purpose after the migration.
    let mut authored = after;
    authored.agents[0].title_summary = Some("Agent タイトル目的化".to_string());
    save_workspace_projection_to_path(&current_path, &authored).expect("save authored");

    // Second reset is a no-op (marker guard) and preserves the agent value.
    let applied_again = reset_legacy_agent_identity_at(&current_path).expect("reset again");
    assert!(!applied_again, "marker must prevent a second clear");
    let preserved = load_workspace_projection_from_path(&current_path)
        .expect("load")
        .expect("present");
    assert_eq!(
        preserved.agents[0].title_summary.as_deref(),
        Some("Agent タイトル目的化"),
        "agent-authored title must survive later loads"
    );
}

#[test]
fn workspace_update_persists_current_summary_and_journal_entry() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project_root = temp.path().join("repo");
    let current_path = temp.path().join("workspace/current.json");
    let journal_path = temp.path().join("workspace/journal.jsonl");

    let entry = update_workspace_projection_with_journal_paths(
        &current_path,
        &journal_path,
        &project_root,
        WorkspaceProjectionUpdate {
            title: Some("Fix Active Work lifecycle".to_string()),
            status_category: Some(WorkspaceStatusCategory::Active),
            status_text: Some("Implementing lifecycle cleanup".to_string()),
            owner: Some("SPEC-2359".to_string()),
            next_action: Some("Run focused regression tests".to_string()),
            summary: Some("Workspace state is now the source for Active Work.".to_string()),
            progress_summary: None,
            agent_session_id: Some("session-1".to_string()),
            agent_current_focus: Some("Writing RED tests".to_string()),
            agent_title_summary: None,
        },
    )
    .expect("update workspace projection");

    let projection = load_workspace_projection_from_path(&current_path)
        .expect("load projection")
        .expect("projection");
    assert_eq!(projection.title, "Fix Active Work lifecycle");
    assert_eq!(projection.status_category, WorkspaceStatusCategory::Active);
    assert_eq!(projection.status_text, "Implementing lifecycle cleanup");
    assert_eq!(
        projection.summary.as_deref(),
        Some("Workspace state is now the source for Active Work.")
    );
    assert_eq!(
        projection.next_action.as_deref(),
        Some("Run focused regression tests")
    );

    let lines = std::fs::read_to_string(&journal_path).expect("journal");
    let entries = lines.lines().collect::<Vec<_>>();
    assert_eq!(entries.len(), 1);
    let journal: WorkspaceJournalEntry = serde_json::from_str(entries[0]).expect("journal entry");
    assert_eq!(journal.id, entry.id);
    assert_eq!(journal.owner.as_deref(), Some("SPEC-2359"));
    assert_eq!(
        journal.summary.as_deref(),
        Some("Workspace state is now the source for Active Work.")
    );
    assert_eq!(journal.agent_session_id.as_deref(), Some("session-1"));
    assert_eq!(
        journal.agent_current_focus.as_deref(),
        Some("Writing RED tests")
    );
}

#[test]
fn workspace_journal_event_targets_agent_branch_work_item() {
    let project_root = std::path::PathBuf::from("/repo/workspace-home");
    let worktree = std::path::PathBuf::from("/repo/workspace-home/work/20260617-0255");
    let updated_at = Utc.with_ymd_and_hms(2026, 6, 17, 5, 30, 0).unwrap();
    let mut projection = WorkspaceProjection::default_for_project(&project_root);
    projection.id = canonical_work_id(&project_root, Some("develop"), None).unwrap();
    projection.title = "Canonical root work".to_string();
    projection.git_details = Some(GitDetails {
        branch: Some("develop".to_string()),
        worktree_path: Some(project_root.join("develop")),
        base_branch: Some("origin/develop".to_string()),
        pr_number: None,
        pr_url: None,
        pr_state: None,
        pr_created_at: None,
        created_by_start_work: false,
        created_at: updated_at,
    });
    projection.agents.push(WorkspaceAgentSummary {
        session_id: "session-current".to_string(),
        window_id: None,
        agent_id: "codex".to_string(),
        display_name: "Codex".to_string(),
        status_category: WorkspaceStatusCategory::Active,
        current_focus: None,
        title_summary: None,
        worktree_path: Some(worktree.clone()),
        branch: Some("work/20260617-0255".to_string()),
        last_board_entry_id: None,
        last_board_entry_kind: None,
        coordination_scope: None,
        affiliation_status: WorkspaceAgentAffiliationStatus::Unassigned,
        workspace_id: None,
        updated_at,
    });
    let entry = WorkspaceJournalEntry {
        id: "journal-current".to_string(),
        project_root: project_root.clone(),
        title: None,
        status_category: Some(WorkspaceStatusCategory::Active),
        status_text: None,
        owner: Some("SPEC-2359".to_string()),
        next_action: None,
        summary: None,
        progress_summary: None,
        agent_session_id: Some("session-current".to_string()),
        agent_current_focus: Some("修正中".to_string()),
        agent_title_summary: Some("Workspace detail content".to_string()),
        updated_at,
    };

    let event = super::workspace_work_event_from_journal_entry(&projection, &entry);

    let expected_id = canonical_work_id(&project_root, Some("work/20260617-0255"), None).unwrap();
    assert_eq!(
        event.work_item_id, expected_id,
        "session-authored workspace.update must land on the session branch Work"
    );
    let container = event.execution_container.expect("execution container");
    assert_eq!(container.branch.as_deref(), Some("work/20260617-0255"));
    assert_eq!(container.worktree_path.as_deref(), Some(worktree.as_path()));
}

#[test]
fn workspace_journal_event_targets_explicitly_assigned_work_item() {
    let project_root = std::path::PathBuf::from("/repo/workspace-home");
    let worktree = std::path::PathBuf::from("/repo/workspace-home/work/20260617-0255");
    let updated_at = Utc.with_ymd_and_hms(2026, 7, 15, 5, 30, 0).unwrap();
    let mut projection = WorkspaceProjection::default_for_project(&project_root);
    projection.id = canonical_work_id(&project_root, Some("develop"), None).unwrap();
    projection.agents.push(WorkspaceAgentSummary {
        session_id: "session-current".to_string(),
        window_id: None,
        agent_id: "codex".to_string(),
        display_name: "Codex".to_string(),
        status_category: WorkspaceStatusCategory::Active,
        current_focus: None,
        title_summary: None,
        worktree_path: Some(worktree.clone()),
        branch: Some("work/20260617-0255".to_string()),
        last_board_entry_id: None,
        last_board_entry_kind: None,
        coordination_scope: None,
        affiliation_status: WorkspaceAgentAffiliationStatus::Assigned,
        workspace_id: Some("work-existing-joined".to_string()),
        updated_at,
    });
    let entry = WorkspaceJournalEntry {
        id: "journal-assigned".to_string(),
        project_root: project_root.clone(),
        title: None,
        status_category: Some(WorkspaceStatusCategory::Active),
        status_text: None,
        owner: Some("SPEC-2359".to_string()),
        next_action: None,
        summary: None,
        progress_summary: None,
        agent_session_id: Some("session-current".to_string()),
        agent_current_focus: Some("joined work update".to_string()),
        agent_title_summary: Some("Joined Work".to_string()),
        updated_at,
    };

    let event = super::workspace_work_event_from_journal_entry(&projection, &entry);

    assert_eq!(
        event.work_item_id, "work-existing-joined",
        "an explicit Session assignment must win over branch-derived identity"
    );
    assert_eq!(
        event
            .execution_container
            .as_ref()
            .and_then(|container| container.branch.as_deref()),
        Some("work/20260617-0255")
    );
}

#[test]
fn workspace_journal_event_keeps_latest_duplicate_assignment_after_update() {
    let temp = tempfile::tempdir().unwrap();
    let current = temp.path().join("state/current.json");
    let journal = temp.path().join("state/journal.jsonl");
    let works = temp.path().join("state/works.json");
    let events = temp.path().join("repo/.gwt/work/events.jsonl");
    let root = temp.path().join("repo");
    let old_at = Utc.with_ymd_and_hms(2026, 7, 15, 8, 0, 0).unwrap();
    let current_at = Utc.with_ymd_and_hms(2026, 7, 15, 9, 0, 0).unwrap();
    let mut projection = WorkspaceProjection::default_for_project(&root);
    let mut stale = assigned_agent("session-duplicate", "codex", "work-stale");
    stale.affiliation_status = WorkspaceAgentAffiliationStatus::Unassigned;
    stale.workspace_id = None;
    stale.branch = Some("feature/stale".to_string());
    stale.updated_at = old_at;
    let mut assigned = assigned_agent("session-duplicate", "codex", "work-current");
    assigned.branch = Some("feature/current".to_string());
    assigned.updated_at = current_at;
    projection.agents = vec![stale, assigned];
    save_workspace_projection_to_path(&current, &projection).unwrap();

    update_workspace_projection_with_journal_paths_at(
        &current,
        &journal,
        &root,
        WorkspaceProjectionUpdate {
            title: None,
            status_category: None,
            status_text: None,
            owner: None,
            next_action: None,
            summary: None,
            progress_summary: None,
            agent_session_id: Some("session-duplicate".to_string()),
            agent_current_focus: Some("latest update".to_string()),
            agent_title_summary: None,
        },
        current_at + chrono::Duration::hours(1),
    )
    .unwrap();
    let projection = load_workspace_projection_from_path(&current)
        .unwrap()
        .unwrap();
    let entry = load_recent_workspace_journal_entries_from_path(&journal, 1)
        .unwrap()
        .pop()
        .unwrap();
    let event = workspace_work_event_from_journal_entry_for_root(&projection, &entry, &root);
    assert_eq!(event.work_item_id, "work-current");
    assert_eq!(
        workspace_assignment_for_session(&projection, "session-duplicate"),
        WorkspaceSessionAssignment::Assigned("work-current".to_string())
    );
    assert!(!works.exists());
    assert!(!events.exists());
}

#[test]
fn workspace_state_transaction_does_not_publish_current_when_event_append_fails() {
    let temp = tempfile::tempdir().unwrap();
    let current = temp.path().join("state/current.json");
    let works = temp.path().join("state/works.json");
    let events = temp.path().join("events-is-a-directory");
    let root = temp.path().join("repo");
    std::fs::create_dir_all(&events).unwrap();
    let now = Utc.with_ymd_and_hms(2026, 7, 15, 11, 0, 0).unwrap();
    let mut initial = WorkspaceProjection::default_for_project(&root);
    initial.agents.push(WorkspaceAgentSummary {
        session_id: "session-failed-transaction".to_string(),
        window_id: None,
        agent_id: "codex".to_string(),
        display_name: "Codex".to_string(),
        status_category: WorkspaceStatusCategory::Active,
        current_focus: None,
        title_summary: None,
        worktree_path: Some(root.clone()),
        branch: Some("feature/failed-transaction".to_string()),
        last_board_entry_id: None,
        last_board_entry_kind: None,
        coordination_scope: None,
        affiliation_status: WorkspaceAgentAffiliationStatus::Unassigned,
        workspace_id: None,
        updated_at: now,
    });
    save_workspace_projection_to_path(&current, &initial).unwrap();

    let result =
        transact_workspace_state_at(&current, &works, &events, &root, |projection, _, _| {
            projection.assign_agent(
                "session-failed-transaction",
                "work-failed-transaction",
                None,
                None,
                now,
            );
            let mut event = WorkEvent::new(WorkEventKind::Start, "work-failed-transaction", now);
            event.agent_session_id = Some("session-failed-transaction".to_string());
            Ok(((), vec![event]))
        });
    assert!(result.is_err());

    let saved = load_workspace_projection_from_path(&current)
        .unwrap()
        .unwrap();
    assert_eq!(
        workspace_assignment_for_session(&saved, "session-failed-transaction"),
        WorkspaceSessionAssignment::Unassigned,
        "a failed event append must not publish assignment-only current state"
    );
    assert!(!works.exists());
}

#[test]
fn workspace_state_transaction_recovers_partial_commit_exactly_once() {
    let temp = tempfile::tempdir().unwrap();
    let current = temp.path().join("state/current.json");
    let works = temp.path().join("work-state/works.json");
    let events = temp.path().join("repo/.gwt/work/events.jsonl");
    let root = temp.path().join("repo");
    let now = Utc.with_ymd_and_hms(2026, 7, 15, 11, 30, 0).unwrap();
    let mut initial = WorkspaceProjection::default_for_project(&root);
    initial.agents.push(WorkspaceAgentSummary {
        session_id: "session-recovery".to_string(),
        window_id: None,
        agent_id: "codex".to_string(),
        display_name: "Codex".to_string(),
        status_category: WorkspaceStatusCategory::Active,
        current_focus: None,
        title_summary: None,
        worktree_path: Some(root.clone()),
        branch: Some("feature/recovery".to_string()),
        last_board_entry_id: None,
        last_board_entry_kind: None,
        coordination_scope: None,
        affiliation_status: WorkspaceAgentAffiliationStatus::Unassigned,
        workspace_id: None,
        updated_at: now,
    });
    save_workspace_projection_to_path(&current, &initial).unwrap();

    let mut recovered_current = initial;
    assert!(recovered_current.assign_agent("session-recovery", "work-recovery", None, None, now,));
    let mut event = WorkEvent::new(WorkEventKind::Start, "work-recovery", now);
    event.id = "event-recovery".to_string();
    event.agent_session_id = Some("session-recovery".to_string());
    let mut recovered_works = WorkItemsProjection::empty(now);
    recovered_works.apply_event(event.clone());
    let pending = PendingWorkspaceStateTransaction {
        version: WORKSPACE_STATE_TRANSACTION_VERSION,
        transaction_id: Some(Uuid::new_v4().to_string()),
        current_path: current.clone(),
        work_items_path: works.clone(),
        current_precondition: Some(workspace_state_file_fingerprint(&current).unwrap()),
        work_items_precondition: Some(workspace_state_file_fingerprint(&works).unwrap()),
        projection: recovered_current,
        work_items: Some(recovered_works),
        events_path: Some(events.clone()),
        events: vec![event.clone()],
        journal_path: None,
        journal_entries: Vec::new(),
    };
    write_atomic(
        &pending_workspace_state_transaction_path(&current),
        &serde_json::to_vec_pretty(&pending).unwrap(),
    )
    .unwrap();
    append_workspace_work_event_to_path(&events, &event).unwrap();

    transact_workspace_state_at(&current, &works, &events, &root, |projection, items, _| {
        assert_eq!(
            workspace_assignment_for_session(projection, "session-recovery"),
            WorkspaceSessionAssignment::Assigned("work-recovery".to_string())
        );
        assert!(items
            .work_items
            .iter()
            .any(|item| item.id == "work-recovery"));
        Ok(((), Vec::new()))
    })
    .unwrap();

    assert!(!pending_workspace_state_transaction_path(&current).exists());
    assert_eq!(
        std::fs::read_to_string(&events)
            .unwrap()
            .lines()
            .filter(|line| line.contains("event-recovery"))
            .count(),
        1,
        "recovery must not duplicate a durable event"
    );
    let saved = load_workspace_projection_from_path(&current)
        .unwrap()
        .unwrap();
    assert_eq!(
        workspace_assignment_for_session(&saved, "session-recovery"),
        WorkspaceSessionAssignment::Assigned("work-recovery".to_string())
    );
}

fn write_pending_transaction_markers(transaction: &PendingWorkspaceStateTransaction) {
    let bytes = serde_json::to_vec_pretty(transaction).unwrap();
    for marker_path in pending_workspace_state_transaction_paths(transaction) {
        write_atomic(&marker_path, &bytes).unwrap();
    }
}

#[test]
fn ordinary_current_writer_recovers_pending_transaction_before_mutating() {
    let temp = tempfile::tempdir().unwrap();
    let current = temp.path().join("current-state/current.json");
    let works = temp.path().join("work-state/works.json");
    let events = temp.path().join("repo/.gwt/work/events.jsonl");
    let root = temp.path().join("repo");
    let now = Utc.with_ymd_and_hms(2026, 7, 15, 13, 0, 0).unwrap();
    let mut initial = WorkspaceProjection::default_for_project(&root);
    initial.agents.push(assigned_agent(
        "session-pending-current",
        "codex",
        "work-before-pending",
    ));
    initial.agents[0].affiliation_status = WorkspaceAgentAffiliationStatus::Unassigned;
    initial.agents[0].workspace_id = None;
    save_workspace_projection_to_path(&current, &initial).unwrap();

    let mut pending_current = initial;
    assert!(pending_current.assign_agent(
        "session-pending-current",
        "work-from-pending",
        None,
        None,
        now,
    ));
    let mut pending_event = WorkEvent::new(WorkEventKind::Start, "work-from-pending", now);
    pending_event.id = "event-from-pending".to_string();
    pending_event.agent_session_id = Some("session-pending-current".to_string());
    let mut pending_works = WorkItemsProjection::empty(now);
    pending_works.apply_event(pending_event.clone());
    write_pending_transaction_markers(&PendingWorkspaceStateTransaction {
        version: WORKSPACE_STATE_TRANSACTION_VERSION,
        transaction_id: Some(Uuid::new_v4().to_string()),
        current_path: current.clone(),
        work_items_path: works.clone(),
        current_precondition: Some(workspace_state_file_fingerprint(&current).unwrap()),
        work_items_precondition: Some(workspace_state_file_fingerprint(&works).unwrap()),
        projection: pending_current,
        work_items: Some(pending_works),
        events_path: Some(events.clone()),
        events: vec![pending_event],
        journal_path: None,
        journal_entries: Vec::new(),
    });

    mutate_workspace_projection_at(&current, &root, |projection| {
        projection.summary = Some("newer ordinary mutation".to_string());
        Ok(())
    })
    .unwrap();

    let saved = load_workspace_projection_from_path(&current)
        .unwrap()
        .unwrap();
    assert_eq!(
        workspace_assignment_for_session(&saved, "session-pending-current"),
        WorkspaceSessionAssignment::Assigned("work-from-pending".to_string())
    );
    assert_eq!(saved.summary.as_deref(), Some("newer ordinary mutation"));
    assert!(load_workspace_work_items_from_path(&works)
        .unwrap()
        .unwrap()
        .work_items
        .iter()
        .any(|item| item.id == "work-from-pending"));
    assert!(!pending_workspace_state_transaction_path(&current).exists());
    assert!(!works
        .with_file_name("pending-state-transaction.json")
        .exists());
}

#[test]
fn ordinary_work_event_writer_recovers_pending_transaction_before_appending() {
    let temp = tempfile::tempdir().unwrap();
    let current = temp.path().join("current-state/current.json");
    let works = temp.path().join("work-state/works.json");
    let events = temp.path().join("repo/.gwt/work/events.jsonl");
    let root = temp.path().join("repo");
    let now = Utc.with_ymd_and_hms(2026, 7, 15, 13, 30, 0).unwrap();
    let initial = WorkspaceProjection::default_for_project(&root);
    let mut pending_current = initial.clone();
    pending_current.summary = Some("pending current".to_string());
    let mut pending_event = WorkEvent::new(WorkEventKind::Start, "work-from-pending", now);
    pending_event.id = "event-from-pending".to_string();
    let mut pending_works = WorkItemsProjection::empty(now);
    pending_works.apply_event(pending_event.clone());
    write_pending_transaction_markers(&PendingWorkspaceStateTransaction {
        version: WORKSPACE_STATE_TRANSACTION_VERSION,
        transaction_id: Some(Uuid::new_v4().to_string()),
        current_path: current.clone(),
        work_items_path: works.clone(),
        current_precondition: Some(workspace_state_file_fingerprint(&current).unwrap()),
        work_items_precondition: Some(workspace_state_file_fingerprint(&works).unwrap()),
        projection: pending_current,
        work_items: Some(pending_works),
        events_path: Some(events.clone()),
        events: vec![pending_event],
        journal_path: None,
        journal_entries: Vec::new(),
    });

    let mut newer_event = WorkEvent::new(
        WorkEventKind::Start,
        "work-from-ordinary-writer",
        now + chrono::Duration::minutes(1),
    );
    newer_event.id = "event-from-ordinary-writer".to_string();
    record_workspace_work_event_paths(&works, &events, newer_event).unwrap();

    let saved = load_workspace_work_items_from_path(&works)
        .unwrap()
        .unwrap();
    assert!(saved
        .work_items
        .iter()
        .any(|item| item.id == "work-from-pending"));
    assert!(saved
        .work_items
        .iter()
        .any(|item| item.id == "work-from-ordinary-writer"));
    assert_eq!(
        load_workspace_projection_from_path(&current)
            .unwrap()
            .unwrap()
            .summary
            .as_deref(),
        Some("pending current")
    );
    assert!(!pending_workspace_state_transaction_path(&current).exists());
    assert!(!works
        .with_file_name("pending-state-transaction.json")
        .exists());
}

#[test]
fn one_sided_pending_marker_never_overwrites_a_later_work_event() {
    let temp = tempfile::tempdir().unwrap();
    let current = temp.path().join("current-state/current.json");
    let works = temp.path().join("work-state/works.json");
    let events = temp.path().join("repo/.gwt/work/events.jsonl");
    let root = temp.path().join("repo");
    let now = Utc.with_ymd_and_hms(2026, 7, 15, 13, 45, 0).unwrap();
    let initial_current = WorkspaceProjection::default_for_project(&root);
    let initial_works = WorkItemsProjection::empty(now);
    save_workspace_projection_to_path(&current, &initial_current).unwrap();
    save_workspace_work_items_projection_to_path(&works, &initial_works).unwrap();
    let current_precondition = workspace_state_file_fingerprint(&current).unwrap();
    let work_items_precondition = workspace_state_file_fingerprint(&works).unwrap();

    let mut committed_current = initial_current;
    committed_current.summary = Some("committed transaction".to_string());
    let mut committed_event = WorkEvent::new(WorkEventKind::Start, "work-committed", now);
    committed_event.id = "event-committed".to_string();
    let mut committed_works = initial_works;
    committed_works.apply_event(committed_event.clone());
    save_workspace_projection_to_path(&current, &committed_current).unwrap();
    save_workspace_work_items_projection_to_path(&works, &committed_works).unwrap();

    let pending = PendingWorkspaceStateTransaction {
        version: WORKSPACE_STATE_TRANSACTION_VERSION,
        transaction_id: Some(Uuid::new_v4().to_string()),
        current_path: current.clone(),
        work_items_path: works.clone(),
        current_precondition: Some(current_precondition),
        work_items_precondition: Some(work_items_precondition),
        projection: committed_current,
        work_items: Some(committed_works),
        events_path: Some(events.clone()),
        events: vec![committed_event],
        journal_path: None,
        journal_entries: Vec::new(),
    };
    write_pending_transaction_markers(&pending);
    std::fs::remove_file(works.with_file_name("pending-state-transaction.json")).unwrap();

    let mut later_event = WorkEvent::new(
        WorkEventKind::Start,
        "work-after-commit",
        now + chrono::Duration::minutes(1),
    );
    later_event.id = "event-after-commit".to_string();
    record_workspace_work_event_paths(&works, &events, later_event).unwrap();

    mutate_workspace_projection_at(&current, &root, |projection| {
        projection.status_text = "later current mutation".to_string();
        Ok(())
    })
    .unwrap();

    let saved = load_workspace_work_items_from_path(&works)
        .unwrap()
        .unwrap();
    assert!(saved
        .work_items
        .iter()
        .any(|item| item.id == "work-after-commit"));
}

#[test]
fn coordinator_only_pending_transaction_is_discovered_by_work_writer() {
    let temp = tempfile::tempdir().unwrap();
    let current = temp.path().join("current-state/current.json");
    let works = temp.path().join("work-state/works.json");
    let events = temp.path().join("repo/.gwt/work/events.jsonl");
    let root = temp.path().join("repo");
    let now = Utc.with_ymd_and_hms(2026, 7, 15, 13, 50, 0).unwrap();
    let mut initial_current = WorkspaceProjection::default_for_project(&root);
    initial_current.agents.push(assigned_agent(
        "session-coordinator-only",
        "codex",
        "work-before-coordinator",
    ));
    initial_current.agents[0].affiliation_status = WorkspaceAgentAffiliationStatus::Unassigned;
    initial_current.agents[0].workspace_id = None;
    let initial_works = WorkItemsProjection::empty(now);
    save_workspace_projection_to_path(&current, &initial_current).unwrap();
    save_workspace_work_items_projection_to_path(&works, &initial_works).unwrap();

    let mut pending_current = initial_current;
    assert!(pending_current.assign_agent(
        "session-coordinator-only",
        "work-from-coordinator",
        None,
        None,
        now,
    ));
    let mut pending_event = WorkEvent::new(WorkEventKind::Start, "work-from-coordinator", now);
    pending_event.id = "event-from-coordinator".to_string();
    pending_event.agent_session_id = Some("session-coordinator-only".to_string());
    let mut pending_works = initial_works;
    pending_works.apply_event(pending_event.clone());
    let pending = PendingWorkspaceStateTransaction {
        version: WORKSPACE_STATE_TRANSACTION_VERSION,
        transaction_id: Some(Uuid::new_v4().to_string()),
        current_path: current.clone(),
        work_items_path: works.clone(),
        current_precondition: Some(workspace_state_file_fingerprint(&current).unwrap()),
        work_items_precondition: Some(workspace_state_file_fingerprint(&works).unwrap()),
        projection: pending_current,
        work_items: Some(pending_works),
        events_path: Some(events.clone()),
        events: vec![pending_event],
        journal_path: None,
        journal_entries: Vec::new(),
    };
    let coordinator = pending_workspace_state_transaction_coordinator_path(&pending).unwrap();
    write_atomic(&coordinator, &serde_json::to_vec_pretty(&pending).unwrap()).unwrap();

    let mut later_event = WorkEvent::new(
        WorkEventKind::Start,
        "work-after-coordinator",
        now + chrono::Duration::minutes(1),
    );
    later_event.id = "event-after-coordinator".to_string();
    record_workspace_work_event_paths(&works, &events, later_event).unwrap();

    let saved_current = load_workspace_projection_from_path(&current)
        .unwrap()
        .unwrap();
    assert_eq!(
        workspace_assignment_for_session(&saved_current, "session-coordinator-only"),
        WorkspaceSessionAssignment::Assigned("work-from-coordinator".to_string())
    );
    let saved_works = load_workspace_work_items_from_path(&works)
        .unwrap()
        .unwrap();
    assert!(saved_works
        .work_items
        .iter()
        .any(|item| item.id == "work-from-coordinator"));
    assert!(saved_works
        .work_items
        .iter()
        .any(|item| item.id == "work-after-coordinator"));
    assert!(!coordinator.exists());
}

#[test]
fn coordinator_created_while_writer_waits_for_lock_is_recovered_before_operation() {
    use fs2::FileExt;

    let _guard = lock_test_env();
    let temp = tempfile::tempdir().unwrap();
    let gwt_home = temp.path().join("gwt-home");
    let _home = ScopedHome::set(&gwt_home);
    let current = temp.path().join("current-state/current.json");
    let works = temp.path().join("work-state/works.json");
    let events = temp.path().join("repo/.gwt/work/events.jsonl");
    let root = temp.path().join("repo");
    let now = Utc.with_ymd_and_hms(2026, 7, 16, 11, 0, 0).unwrap();

    let mut initial_current = WorkspaceProjection::default_for_project(&root);
    initial_current
        .agents
        .push(unassigned_agent("session-lock-wait-coordinator", "codex"));
    let initial_works = WorkItemsProjection::empty(now);
    save_workspace_projection_to_path(&current, &initial_current).unwrap();
    save_workspace_work_items_projection_to_path(&works, &initial_works).unwrap();

    let base_lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(works.with_extension("lock"))
        .unwrap();
    base_lock.lock_exclusive().unwrap();

    let works_for_writer = works.clone();
    let events_for_writer = events.clone();
    let gwt_home_for_writer = gwt_home.clone();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let writer = std::thread::spawn(move || {
        let _home = ScopedHome::set(&gwt_home_for_writer);
        started_tx.send(()).unwrap();
        let mut later = WorkEvent::new(
            WorkEventKind::Start,
            "work-after-lock-wait",
            now + chrono::Duration::minutes(1),
        );
        later.id = "event-after-lock-wait".to_string();
        record_workspace_work_event_paths(&works_for_writer, &events_for_writer, later)
    });
    started_rx.recv().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(
        !writer.is_finished(),
        "writer must be waiting for the base lock"
    );

    let mut pending_current = initial_current;
    assert!(pending_current.assign_agent(
        "session-lock-wait-coordinator",
        "work-from-lock-wait-coordinator",
        None,
        None,
        now,
    ));
    let mut pending_event =
        WorkEvent::new(WorkEventKind::Start, "work-from-lock-wait-coordinator", now);
    pending_event.id = "event-from-lock-wait-coordinator".to_string();
    pending_event.agent_session_id = Some("session-lock-wait-coordinator".to_string());
    let mut pending_works = initial_works;
    pending_works.apply_event(pending_event.clone());
    let pending = PendingWorkspaceStateTransaction {
        version: WORKSPACE_STATE_TRANSACTION_VERSION,
        transaction_id: Some(Uuid::new_v4().to_string()),
        current_path: current.clone(),
        work_items_path: works.clone(),
        current_precondition: Some(workspace_state_file_fingerprint(&current).unwrap()),
        work_items_precondition: Some(workspace_state_file_fingerprint(&works).unwrap()),
        projection: pending_current,
        work_items: Some(pending_works),
        events_path: Some(events.clone()),
        events: vec![pending_event],
        journal_path: None,
        journal_entries: Vec::new(),
    };
    let coordinator = pending_workspace_state_transaction_coordinator_path(&pending).unwrap();
    write_atomic(&coordinator, &serde_json::to_vec_pretty(&pending).unwrap()).unwrap();

    FileExt::unlock(&base_lock).unwrap();
    writer.join().unwrap().unwrap();

    let saved_current = load_workspace_projection_from_path(&current)
        .unwrap()
        .unwrap();
    assert_eq!(
        workspace_assignment_for_session(&saved_current, "session-lock-wait-coordinator"),
        WorkspaceSessionAssignment::Assigned("work-from-lock-wait-coordinator".to_string())
    );
    let saved_works = load_workspace_work_items_from_path(&works)
        .unwrap()
        .unwrap();
    assert!(saved_works
        .work_items
        .iter()
        .any(|item| item.id == "work-from-lock-wait-coordinator"));
    assert!(saved_works
        .work_items
        .iter()
        .any(|item| item.id == "work-after-lock-wait"));
    assert!(!coordinator.exists());
}

#[test]
fn coordinator_only_incompatible_transaction_is_preserved_and_blocks_writer() {
    let _guard = lock_test_env();
    let temp = tempfile::tempdir().unwrap();
    let gwt_home = temp.path().join("gwt-home");
    let _home = ScopedHome::set(&gwt_home);
    let current = temp.path().join("current-state/current.json");
    let works = current.with_file_name("works.json");
    let root = temp.path().join("repo");
    let now = Utc.with_ymd_and_hms(2026, 7, 16, 12, 0, 0).unwrap();
    let transaction = PendingWorkspaceStateTransaction {
        version: WORKSPACE_STATE_TRANSACTION_VERSION,
        transaction_id: Some("coordinator-only-incompatible".to_string()),
        current_path: current.clone(),
        work_items_path: works,
        current_precondition: Some("missing".to_string()),
        work_items_precondition: Some("missing".to_string()),
        projection: WorkspaceProjection::default_for_project(&root),
        work_items: Some(WorkItemsProjection::empty(now)),
        events_path: None,
        events: Vec::new(),
        journal_path: None,
        journal_entries: Vec::new(),
    };
    let coordinator = pending_workspace_state_transaction_coordinator_path(&transaction).unwrap();
    let mut value = serde_json::to_value(&transaction).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .insert("future_schema_field".to_string(), serde_json::json!(true));
    let original = serde_json::to_vec_pretty(&value).unwrap();
    write_atomic(&coordinator, &original).unwrap();
    let operation_ran = std::sync::atomic::AtomicBool::new(false);

    let result = mutate_workspace_projection_at(&current, &root, |_| {
        operation_ran.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    });

    assert!(result.is_err());
    assert!(!operation_ran.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(
        std::fs::read(&coordinator).expect("incompatible coordinator must remain"),
        original
    );
    assert!(!current.exists(), "writer must not mutate project state");
}

#[test]
fn incompatible_global_coordinator_does_not_block_an_unrelated_project_writer() {
    let _guard = lock_test_env();
    let temp = tempfile::tempdir().unwrap();
    let gwt_home = temp.path().join("gwt-home");
    let _home = ScopedHome::set(&gwt_home);
    let current_a = temp.path().join("project-a/current.json");
    let works_a = current_a.with_file_name("works.json");
    let root_a = temp.path().join("repo-a");
    let transaction = PendingWorkspaceStateTransaction {
        version: WORKSPACE_STATE_TRANSACTION_VERSION,
        transaction_id: Some("project-a-incompatible".to_string()),
        current_path: current_a,
        work_items_path: works_a,
        current_precondition: Some("missing".to_string()),
        work_items_precondition: Some("missing".to_string()),
        projection: WorkspaceProjection::default_for_project(&root_a),
        work_items: None,
        events_path: None,
        events: Vec::new(),
        journal_path: None,
        journal_entries: Vec::new(),
    };
    let coordinator = pending_workspace_state_transaction_coordinator_path(&transaction).unwrap();
    let mut value = serde_json::to_value(&transaction).unwrap();
    value["projection"]["future_project_a_field"] = serde_json::json!(true);
    let original = serde_json::to_vec_pretty(&value).unwrap();
    write_atomic(&coordinator, &original).unwrap();

    let current_b = temp.path().join("project-b/current.json");
    let root_b = temp.path().join("repo-b");
    mutate_workspace_projection_at(&current_b, &root_b, |projection| {
        projection.summary = Some("project B remains writable".to_string());
        Ok(())
    })
    .expect("an unrelated project must not inspect project A's full transaction schema");

    assert_eq!(
        load_workspace_projection_from_path(&current_b)
            .unwrap()
            .unwrap()
            .summary
            .as_deref(),
        Some("project B remains writable")
    );
    assert_eq!(
        std::fs::read(&coordinator).expect("project A marker must remain"),
        original
    );
}

#[test]
fn split_root_coordinator_uses_writable_gwt_state_and_is_discoverable_by_both_writers() {
    let _guard = lock_test_env();
    let temp = tempfile::tempdir().unwrap();
    let gwt_home = temp.path().join("gwt-home");
    let _home = ScopedHome::set(&gwt_home);
    let now = Utc.with_ymd_and_hms(2026, 7, 16, 9, 0, 0).unwrap();

    #[cfg(not(windows))]
    let (current, works) = (
        PathBuf::from("/split-current-state/project/current.json"),
        PathBuf::from("/split-work-state/project/works.json"),
    );
    #[cfg(windows)]
    let (current, works) = (
        PathBuf::from(r"C:\split-current-state\project\current.json"),
        PathBuf::from(r"D:\split-work-state\project\works.json"),
    );

    #[cfg(not(windows))]
    assert_eq!(
        common_path_ancestor(current.parent().unwrap(), works.parent().unwrap()).as_deref(),
        Some(Path::new("/")),
        "fixture roots must meet only at the filesystem root"
    );
    #[cfg(windows)]
    assert!(
        common_path_ancestor(current.parent().unwrap(), works.parent().unwrap()).is_none(),
        "fixture roots must reside on different volumes"
    );

    let transaction = PendingWorkspaceStateTransaction {
        version: WORKSPACE_STATE_TRANSACTION_VERSION,
        transaction_id: Some("split-root-writable-coordinator".to_string()),
        current_path: current.clone(),
        work_items_path: works.clone(),
        current_precondition: None,
        work_items_precondition: None,
        projection: WorkspaceProjection::default_for_project(current.parent().unwrap()),
        work_items: Some(WorkItemsProjection::empty(now)),
        events_path: None,
        events: Vec::new(),
        journal_path: None,
        journal_entries: Vec::new(),
    };
    let coordinator = pending_workspace_state_transaction_coordinator_path(&transaction)
        .expect("split roots still require a coordinator");
    assert!(
        coordinator.starts_with(&gwt_home),
        "coordinator must use writable GWT state, got {}",
        coordinator.display()
    );
    write_atomic(
        &coordinator,
        &serde_json::to_vec_pretty(&transaction).unwrap(),
    )
    .unwrap();

    for (writer, lock_target) in [
        ("current", current.with_file_name("works.json")),
        ("Work", works.clone()),
    ] {
        let discovered =
            discover_pending_workspace_state_transaction_coordinators(&[lock_target]).unwrap();
        assert!(
            discovered.contains(&coordinator),
            "{writer} writer must discover the split-root coordinator"
        );
    }
}

#[test]
fn pending_recovery_repairs_partial_jsonl_tails_before_exact_once_append() {
    let temp = tempfile::tempdir().unwrap();
    let current = temp.path().join("state/current.json");
    let works = temp.path().join("state/works.json");
    let events = temp.path().join("repo/.gwt/work/events.jsonl");
    let journal = temp.path().join("state/journal.jsonl");
    let root = temp.path().join("repo");
    let now = Utc.with_ymd_and_hms(2026, 7, 15, 14, 0, 0).unwrap();
    let pending_current = WorkspaceProjection::default_for_project(&root);
    let mut event = WorkEvent::new(WorkEventKind::Start, "work-partial-jsonl", now);
    event.id = "event-partial-jsonl".to_string();
    let mut pending_works = WorkItemsProjection::empty(now);
    pending_works.apply_event(event.clone());
    let journal_entry = WorkspaceJournalEntry {
        id: "journal-partial-jsonl".to_string(),
        project_root: root.clone(),
        title: None,
        status_category: Some(WorkspaceStatusCategory::Active),
        status_text: None,
        owner: None,
        next_action: None,
        summary: Some("recover partial journal".to_string()),
        progress_summary: None,
        agent_session_id: None,
        agent_current_focus: None,
        agent_title_summary: None,
        updated_at: now,
    };
    write_pending_transaction_markers(&PendingWorkspaceStateTransaction {
        version: WORKSPACE_STATE_TRANSACTION_VERSION,
        transaction_id: Some(Uuid::new_v4().to_string()),
        current_path: current.clone(),
        work_items_path: works.clone(),
        current_precondition: Some(workspace_state_file_fingerprint(&current).unwrap()),
        work_items_precondition: Some(workspace_state_file_fingerprint(&works).unwrap()),
        projection: pending_current,
        work_items: Some(pending_works),
        events_path: Some(events.clone()),
        events: vec![event],
        journal_path: Some(journal.clone()),
        journal_entries: vec![journal_entry],
    });
    std::fs::create_dir_all(events.parent().unwrap()).unwrap();
    std::fs::write(&events, br#"{"id":"event-partial"#).unwrap();
    std::fs::write(&journal, br#"{"id":"journal-partial"#).unwrap();

    transact_workspace_state_at(&current, &works, &events, &root, |_, _, _| {
        Ok(((), Vec::new()))
    })
    .unwrap();

    let event_lines = std::fs::read_to_string(&events).unwrap();
    let journal_lines = std::fs::read_to_string(&journal).unwrap();
    assert_eq!(
        event_lines
            .lines()
            .map(serde_json::from_str::<serde_json::Value>)
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap()
            .iter()
            .filter(|value| value["id"] == "event-partial-jsonl")
            .count(),
        1
    );
    assert_eq!(
        journal_lines
            .lines()
            .map(serde_json::from_str::<serde_json::Value>)
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap()
            .iter()
            .filter(|value| value["id"] == "journal-partial-jsonl")
            .count(),
        1
    );
}

#[test]
fn corrupt_pending_transaction_is_quarantined_and_retry_can_write() {
    let temp = tempfile::tempdir().unwrap();
    let current = temp.path().join("state/current.json");
    let root = temp.path().join("repo");
    let marker = pending_workspace_state_transaction_path(&current);
    std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
    std::fs::write(&marker, b"{").unwrap();

    let first = mutate_workspace_projection_at(&current, &root, |projection| {
        projection.summary = Some("must wait for a clean retry".to_string());
        Ok(())
    });
    assert!(first.is_err(), "the corrupt WAL must fail closed once");
    assert!(!marker.exists(), "the corrupt WAL must be quarantined");
    assert!(std::fs::read_dir(marker.parent().unwrap())
        .unwrap()
        .filter_map(std::result::Result::ok)
        .any(|entry| entry
            .file_name()
            .to_string_lossy()
            .starts_with("pending-state-transaction.json.corrupt-")));

    mutate_workspace_projection_at(&current, &root, |projection| {
        projection.summary = Some("clean retry".to_string());
        Ok(())
    })
    .unwrap();
    assert_eq!(
        load_workspace_projection_from_path(&current)
            .unwrap()
            .unwrap()
            .summary
            .as_deref(),
        Some("clean retry")
    );
}

#[test]
fn future_pending_transaction_is_preserved_and_blocks_writer() {
    let temp = tempfile::tempdir().unwrap();
    let current = temp.path().join("state/current.json");
    let works = current.with_file_name("works.json");
    let root = temp.path().join("repo");
    let marker = pending_workspace_state_transaction_path(&current);
    let now = Utc.with_ymd_and_hms(2026, 7, 16, 8, 0, 0).unwrap();
    let transaction = PendingWorkspaceStateTransaction {
        version: WORKSPACE_STATE_TRANSACTION_VERSION + 1,
        transaction_id: Some("future-transaction".to_string()),
        current_path: current.clone(),
        work_items_path: works,
        current_precondition: Some("missing".to_string()),
        work_items_precondition: Some("missing".to_string()),
        projection: WorkspaceProjection::default_for_project(&root),
        work_items: Some(WorkItemsProjection::empty(now)),
        events_path: None,
        events: Vec::new(),
        journal_path: None,
        journal_entries: Vec::new(),
    };
    write_atomic(&marker, &serde_json::to_vec_pretty(&transaction).unwrap()).unwrap();
    let original = std::fs::read(&marker).unwrap();
    let operation_ran = std::sync::atomic::AtomicBool::new(false);

    let result = mutate_workspace_projection_at(&current, &root, |_| {
        operation_ran.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    });

    assert!(result.is_err());
    assert!(!operation_ran.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(
        std::fs::read(&marker).expect("future marker must remain in place"),
        original
    );
    assert!(!current.exists(), "writer must not mutate project state");
}

#[test]
fn nested_unknown_pending_transaction_payload_is_preserved_and_blocks_writer() {
    let temp = tempfile::tempdir().unwrap();
    let current = temp.path().join("state/current.json");
    let works = current.with_file_name("works.json");
    let root = temp.path().join("repo");
    let marker = pending_workspace_state_transaction_path(&current);
    let now = Utc.with_ymd_and_hms(2026, 7, 16, 8, 0, 0).unwrap();
    let mut projection = WorkspaceProjection::default_for_project(&root);
    projection
        .agents
        .push(assigned_agent("session-nested", "codex", "work-nested"));
    projection.linked_issues.push(WorkspaceIssueLink {
        number: 2359,
        title: Some("Work projection".to_string()),
        url: None,
    });
    projection.linked_prs.push(WorkspacePrLink {
        number: 3292,
        title: Some("Work projection follow-up".to_string()),
        url: None,
        state: Some("merged".to_string()),
    });
    let mut transaction_event = WorkEvent::new(WorkEventKind::Update, "work-nested", now);
    transaction_event.agent_session_id = Some("session-nested".to_string());
    transaction_event.execution_container = Some(WorkspaceExecutionContainerRef {
        branch: Some("work/nested".to_string()),
        worktree_path: Some(root.join("work/nested")),
        pr_number: None,
        pr_url: None,
        pr_state: None,
    });
    let transaction = PendingWorkspaceStateTransaction {
        version: WORKSPACE_STATE_TRANSACTION_VERSION,
        transaction_id: Some("nested-incompatible".to_string()),
        current_path: current.clone(),
        work_items_path: works,
        current_precondition: Some("missing".to_string()),
        work_items_precondition: Some("missing".to_string()),
        projection,
        work_items: Some(WorkItemsProjection::empty(now)),
        events_path: None,
        events: vec![transaction_event],
        journal_path: Some(temp.path().join("state/journal.jsonl")),
        journal_entries: vec![WorkspaceJournalEntry {
            id: "journal-nested".to_string(),
            project_root: root.clone(),
            title: None,
            status_category: None,
            status_text: None,
            owner: None,
            next_action: None,
            summary: None,
            progress_summary: None,
            agent_session_id: None,
            agent_current_focus: None,
            agent_title_summary: None,
            updated_at: now,
        }],
    };

    for (label, path) in [
        ("projection", vec!["projection", "future_projection_field"]),
        (
            "agent",
            vec!["projection", "agents", "0", "future_agent_field"],
        ),
        (
            "journal",
            vec!["journal_entries", "0", "future_journal_field"],
        ),
        (
            "linked issue",
            vec!["projection", "linked_issues", "0", "future_issue_field"],
        ),
        (
            "linked PR",
            vec!["projection", "linked_prs", "0", "future_pr_field"],
        ),
        (
            "transaction event",
            vec!["events", "0", "future_event_field"],
        ),
        (
            "transaction event container",
            vec![
                "events",
                "0",
                "execution_container",
                "future_container_field",
            ],
        ),
    ] {
        let mut value = serde_json::to_value(&transaction).unwrap();
        let mut target = &mut value;
        for component in &path[..path.len() - 1] {
            target = match target {
                serde_json::Value::Object(object) => object.get_mut(*component).unwrap(),
                serde_json::Value::Array(array) => {
                    array.get_mut(component.parse::<usize>().unwrap()).unwrap()
                }
                _ => unreachable!(),
            };
        }
        target
            .as_object_mut()
            .unwrap()
            .insert(path.last().unwrap().to_string(), serde_json::json!(true));
        write_atomic(&marker, &serde_json::to_vec_pretty(&value).unwrap()).unwrap();
        let original = std::fs::read(&marker).unwrap();
        let operation_ran = std::sync::atomic::AtomicBool::new(false);

        let result = mutate_workspace_projection_at(&current, &root, |_| {
            operation_ran.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        });

        assert!(result.is_err(), "nested {label} field must be incompatible");
        assert!(
            !operation_ran.load(std::sync::atomic::Ordering::SeqCst),
            "writer must not run for nested {label} incompatibility"
        );
        assert_eq!(
            std::fs::read(&marker).expect("incompatible marker must remain"),
            original,
            "nested {label} incompatibility must preserve marker bytes"
        );
        assert!(!current.exists(), "writer must not mutate project state");
    }
}

#[test]
fn unreadable_pending_transaction_is_preserved_and_blocks_writer() {
    let temp = tempfile::tempdir().unwrap();
    let current = temp.path().join("state/current.json");
    let root = temp.path().join("repo");
    let marker = pending_workspace_state_transaction_path(&current);
    std::fs::create_dir_all(&marker).unwrap();
    let operation_ran = std::sync::atomic::AtomicBool::new(false);

    let result = mutate_workspace_projection_at(&current, &root, |_| {
        operation_ran.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    });

    assert!(result.is_err());
    assert!(!operation_ran.load(std::sync::atomic::Ordering::SeqCst));
    assert!(
        marker.is_dir(),
        "an unreadable marker must remain in place for a compatible reader or operator"
    );
    assert!(!current.exists(), "writer must not mutate project state");
}

#[test]
fn legacy_journal_read_migration_never_clobbers_canonical_writer_data() {
    let temp = tempfile::tempdir().unwrap();
    let legacy = temp.path().join("legacy/journal.jsonl");
    let canonical = temp.path().join("canonical/journal.jsonl");
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    std::fs::create_dir_all(canonical.parent().unwrap()).unwrap();
    std::fs::write(&legacy, b"legacy\n").unwrap();
    std::fs::write(&canonical, b"writer\n").unwrap();

    copy_legacy_workspace_file_if_needed(&legacy, &canonical).unwrap();

    assert_eq!(std::fs::read(&canonical).unwrap(), b"writer\n");
}

#[test]
fn journal_update_does_not_publish_current_when_journal_append_fails() {
    let temp = tempfile::tempdir().unwrap();
    let current = temp.path().join("state/current.json");
    let journal = temp.path().join("journal-is-a-directory");
    let root = temp.path().join("repo");
    std::fs::create_dir_all(&journal).unwrap();
    let initial = WorkspaceProjection::default_for_project(&root);
    save_workspace_projection_to_path(&current, &initial).unwrap();

    let result = update_workspace_projection_with_journal_paths_at(
        &current,
        &journal,
        &root,
        WorkspaceProjectionUpdate {
            title: None,
            status_category: None,
            status_text: None,
            owner: None,
            next_action: None,
            summary: Some("must not leak".to_string()),
            progress_summary: None,
            agent_session_id: None,
            agent_current_focus: None,
            agent_title_summary: None,
        },
        Utc.with_ymd_and_hms(2026, 7, 15, 12, 0, 0).unwrap(),
    );
    assert!(result.is_err());
    let saved = load_workspace_projection_from_path(&current)
        .unwrap()
        .unwrap();
    assert_eq!(saved.summary, None);
}

#[test]
fn legacy_journal_copy_waits_for_workspace_transaction_lock() {
    use fs2::FileExt;

    let temp = tempfile::tempdir().unwrap();
    let project_state_root = temp.path().join("project-state");
    let work_event_root = temp.path().join("work-events");
    let current = gwt_workspace_projection_path_for_repo_path(&project_state_root);
    let works = gwt_workspace_work_items_path_for_repo_path(&work_event_root);
    let legacy = legacy_workspace_journal_path_for_repo_path(&project_state_root);
    let canonical = gwt_workspace_journal_path_for_repo_path(&project_state_root);
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    let legacy_entry = WorkspaceJournalEntry {
        id: "legacy".to_string(),
        project_root: project_state_root.clone(),
        title: None,
        status_category: Some(WorkspaceStatusCategory::Active),
        status_text: None,
        owner: None,
        next_action: None,
        summary: Some("legacy journal".to_string()),
        progress_summary: None,
        agent_session_id: None,
        agent_current_focus: None,
        agent_title_summary: None,
        updated_at: Utc.with_ymd_and_hms(2026, 7, 15, 8, 0, 0).unwrap(),
    };
    std::fs::write(
        &legacy,
        format!("{}\n", serde_json::to_string(&legacy_entry).unwrap()),
    )
    .unwrap();
    let initial = WorkspaceProjection::default_for_project(&project_state_root);
    save_workspace_projection_to_path(&current, &initial).unwrap();

    std::fs::create_dir_all(works.parent().unwrap()).unwrap();
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(works.with_extension("lock"))
        .unwrap();
    lock.lock_exclusive().unwrap();

    let state = project_state_root.clone();
    let events = work_event_root.clone();
    let handle = std::thread::spawn(move || {
        update_workspace_projection_with_journal_for_work_event_root(
            &state,
            &events,
            WorkspaceProjectionUpdate {
                title: None,
                status_category: None,
                status_text: None,
                owner: None,
                next_action: None,
                summary: Some("locked update".to_string()),
                progress_summary: None,
                agent_session_id: None,
                agent_current_focus: None,
                agent_title_summary: None,
            },
            TrackedWorkEventPolicy::Persist,
        )
    });
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(
        !canonical.exists(),
        "legacy journal migration must not run before the transaction lock"
    );
    FileExt::unlock(&lock).unwrap();
    handle.join().unwrap().unwrap();
    assert!(canonical.exists());
}

#[test]
fn workspace_journal_event_carries_progress_summary_separately_from_status_summary() {
    let project_root = std::path::PathBuf::from("/repo/workspace-home");
    let updated_at = Utc.with_ymd_and_hms(2026, 6, 17, 6, 0, 0).unwrap();
    let mut projection = WorkspaceProjection::default_for_project(&project_root);
    projection.id = canonical_work_id(&project_root, Some("develop"), None).unwrap();
    projection.title = "Workspace detail context".to_string();
    projection.summary = Some("Previous latest status".to_string());
    projection.progress_summary = Some("Previous cumulative progress".to_string());

    let entry = WorkspaceJournalEntry {
        id: "journal-progress-summary".to_string(),
        project_root: project_root.clone(),
        title: None,
        status_category: Some(WorkspaceStatusCategory::Active),
        status_text: None,
        owner: Some("SPEC-3075".to_string()),
        next_action: None,
        summary: Some("Latest status snapshot".to_string()),
        progress_summary: Some(
            "Investigated resume persistence, confirmed hook timing, and wired the field."
                .to_string(),
        ),
        agent_session_id: None,
        agent_current_focus: None,
        agent_title_summary: None,
        updated_at,
    };

    let event = super::workspace_work_event_from_journal_entry(&projection, &entry);

    assert_eq!(event.summary.as_deref(), Some("Latest status snapshot"));
    assert_eq!(
        event.progress_summary.as_deref(),
        Some("Investigated resume persistence, confirmed hook timing, and wired the field.")
    );
}

#[test]
fn board_status_event_updates_summary_without_overwriting_progress_summary() {
    let started_at = Utc.with_ymd_and_hms(2026, 6, 17, 6, 5, 0).unwrap();
    let updated_at = Utc.with_ymd_and_hms(2026, 6, 17, 6, 10, 0).unwrap();
    let mut projection = WorkItemsProjection::empty(started_at);
    let mut start = WorkEvent::new(WorkEventKind::Start, "work-progress", started_at);
    start.title = Some("Workspace detail context".to_string());
    start.summary = Some("Initial status".to_string());
    start.progress_summary =
        Some("Implemented resume normalization and isolated the WorkEvent root issue.".to_string());
    projection.apply_event(start);

    let mut board_update = WorkEvent::new(WorkEventKind::Update, "work-progress", updated_at);
    board_update.summary = Some("Posted a short Board status update.".to_string());
    projection.apply_event(board_update);

    let item = projection
        .work_items
        .iter()
        .find(|item| item.id == "work-progress")
        .expect("work item");
    assert_eq!(
        item.summary.as_deref(),
        Some("Posted a short Board status update.")
    );
    assert_eq!(
        item.progress_summary.as_deref(),
        Some("Implemented resume normalization and isolated the WorkEvent root issue.")
    );
}

#[test]
fn workspace_journal_event_records_to_agent_worktree_log() {
    let _guard = crate::test_support::env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = tempfile::tempdir().expect("home");
    let _home = ScopedHome::set(home.path());
    let temp = tempfile::tempdir().expect("tempdir");
    let project_state_root = temp.path().join("workspace-home");
    let worktree = temp.path().join("workspace-home/work/20260617-0255");
    init_test_git_repo(&worktree);
    let mut projection = WorkspaceProjection::default_for_project(&project_state_root);
    projection.id = canonical_work_id(&project_state_root, Some("develop"), None).unwrap();
    projection.git_details = Some(GitDetails {
        branch: Some("develop".to_string()),
        worktree_path: Some(project_state_root.clone()),
        base_branch: None,
        pr_number: None,
        pr_state: None,
        pr_url: None,
        pr_created_at: None,
        created_by_start_work: false,
        created_at: Utc::now(),
    });
    projection.agents.push(WorkspaceAgentSummary {
        session_id: "session-current".to_string(),
        window_id: None,
        agent_id: "codex".to_string(),
        display_name: "Codex".to_string(),
        status_category: WorkspaceStatusCategory::Active,
        current_focus: None,
        title_summary: None,
        worktree_path: Some(worktree.clone()),
        branch: Some("work/20260617-0255".to_string()),
        last_board_entry_id: None,
        last_board_entry_kind: None,
        coordination_scope: None,
        affiliation_status: WorkspaceAgentAffiliationStatus::Unassigned,
        workspace_id: None,
        updated_at: Utc::now(),
    });
    save_workspace_projection(&project_state_root, &projection).expect("seed canonical projection");

    update_workspace_projection_with_journal_for_work_event_root(
        &project_state_root,
        &worktree,
        WorkspaceProjectionUpdate {
            title: Some("Workspace".to_string()),
            status_category: Some(WorkspaceStatusCategory::Active),
            status_text: None,
            owner: Some("SPEC-2359".to_string()),
            next_action: None,
            summary: Some("Canonical journal update".to_string()),
            progress_summary: None,
            agent_session_id: Some("session-current".to_string()),
            agent_current_focus: Some("修正中".to_string()),
            agent_title_summary: Some("Workspace detail content".to_string()),
        },
        TrackedWorkEventPolicy::Persist,
    )
    .expect("workspace update");

    let canonical_journal = gwt_workspace_journal_path_for_repo_path(&project_state_root);
    assert!(
        canonical_journal.is_file(),
        "summary journal stays in the project-state root"
    );

    let worktree_events = gwt_repo_local_work_events_path(&worktree);
    let events_text = fs::read_to_string(&worktree_events).expect("worktree events log");
    let lines: Vec<&str> = events_text.lines().collect();
    assert_eq!(lines.len(), 1, "one event is recorded in the worktree log");
    let event: WorkEvent = serde_json::from_str(lines[0]).expect("event json");
    let expected_id = canonical_work_id(&worktree, Some("work/20260617-0255"), None).unwrap();
    assert_eq!(event.work_item_id, expected_id);
    assert_eq!(
        event
            .execution_container
            .as_ref()
            .and_then(|container| container.branch.as_deref()),
        Some("work/20260617-0255")
    );

    let project_state_events = gwt_repo_local_work_events_path(&project_state_root);
    assert!(
        !project_state_events.exists(),
        "session-authored Work event must not be written to the project-state root"
    );
}

// ---------------------------------------------------------------------
// SPEC-2359 T-812 (US-86 / FR-550 / FR-551): a Session-bound sparse
// mutation is authorized against one resolved Work target. The shared
// current projection is ambient state and must never supply that target's
// identity or omitted mutable fields.
// ---------------------------------------------------------------------

const T812_SESSION_ID: &str = "session-bound-update";
const T812_TARGET_WORK_ID: &str = "work-session-bound-target";
const T812_TARGET_BRANCH: &str = "work/session-bound-target";
const T812_TARGET_TITLE: &str = "Target Work title";
const T812_TARGET_INTENT: &str = "Target Work intent";
const T812_TARGET_SUMMARY: &str = "Target Work status";
const T812_TARGET_PROGRESS: &str = "Target Work cumulative progress";
const T812_TARGET_OWNER: &str = "SPEC-target-owner";

const T812_FOREIGN_TITLE: &str = "Foreign shared-current title";
const T812_FOREIGN_SUMMARY: &str = "Foreign shared-current summary";
const T812_FOREIGN_PROGRESS: &str = "Foreign shared-current progress";
const T812_FOREIGN_OWNER: &str = "SPEC-foreign-owner";

type T812ResolvedMutationTarget = SessionBoundWorkspaceMutationTarget;

#[derive(Debug)]
struct T812Fixture {
    target: T812ResolvedMutationTarget,
    current_path: PathBuf,
    work_items_path: PathBuf,
    events_path: PathBuf,
    journal_path: PathBuf,
}

#[derive(Debug, PartialEq, Eq)]
struct T812WorkspaceStateBytes {
    current: Option<Vec<u8>>,
    work_items: Option<Vec<u8>>,
    events: Option<Vec<u8>>,
    journal: Option<Vec<u8>>,
}

impl T812Fixture {
    fn state_bytes(&self) -> T812WorkspaceStateBytes {
        T812WorkspaceStateBytes {
            current: t812_optional_file_bytes(&self.current_path),
            work_items: t812_optional_file_bytes(&self.work_items_path),
            events: t812_optional_file_bytes(&self.events_path),
            journal: t812_optional_file_bytes(&self.journal_path),
        }
    }
}

fn t812_optional_file_bytes(path: &Path) -> Option<Vec<u8>> {
    match fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => panic!("read {}: {error}", path.display()),
    }
}

fn t812_seed_session_bound_fixture(temp: &Path) -> T812Fixture {
    let project_state_root = temp.join("workspace-home");
    let work_event_root = temp.join("worktree");
    fs::create_dir_all(&project_state_root).expect("project-state root");
    init_test_git_repo(&work_event_root);

    let seeded_at = Utc.with_ymd_and_hms(2026, 7, 22, 1, 0, 0).unwrap();
    let mut current = WorkspaceProjection::default_for_project(&project_state_root);
    current.id = "work-foreign-shared-current".to_string();
    current.title = T812_FOREIGN_TITLE.to_string();
    current.status_category = WorkspaceStatusCategory::Blocked;
    current.status_text = "Foreign shared-current blocked status".to_string();
    current.summary = Some(T812_FOREIGN_SUMMARY.to_string());
    current.progress_summary = Some(T812_FOREIGN_PROGRESS.to_string());
    current.owner = Some(T812_FOREIGN_OWNER.to_string());
    current.next_action = Some("Foreign shared-current next action".to_string());
    current.agents.push(WorkspaceAgentSummary {
        session_id: T812_SESSION_ID.to_string(),
        window_id: Some("parent-window".to_string()),
        agent_id: "codex".to_string(),
        display_name: "Codex parent".to_string(),
        status_category: WorkspaceStatusCategory::Active,
        current_focus: None,
        title_summary: None,
        worktree_path: Some(work_event_root.clone()),
        branch: Some(T812_TARGET_BRANCH.to_string()),
        last_board_entry_id: None,
        last_board_entry_kind: None,
        coordination_scope: None,
        affiliation_status: WorkspaceAgentAffiliationStatus::Assigned,
        workspace_id: Some(T812_TARGET_WORK_ID.to_string()),
        updated_at: seeded_at,
    });
    save_workspace_projection(&project_state_root, &current).expect("seed shared current");

    let target_container = WorkspaceExecutionContainerRef {
        branch: Some(T812_TARGET_BRANCH.to_string()),
        worktree_path: Some(work_event_root.clone()),
        pr_number: None,
        pr_url: None,
        pr_state: None,
    };
    let mut target_start = WorkEvent::new(
        WorkEventKind::Start,
        T812_TARGET_WORK_ID,
        seeded_at - chrono::Duration::minutes(1),
    );
    target_start.title = Some(T812_TARGET_TITLE.to_string());
    target_start.intent = Some(T812_TARGET_INTENT.to_string());
    target_start.summary = Some(T812_TARGET_SUMMARY.to_string());
    target_start.progress_summary = Some(T812_TARGET_PROGRESS.to_string());
    target_start.status_category = Some(WorkspaceStatusCategory::Idle);
    target_start.owner = Some(T812_TARGET_OWNER.to_string());
    target_start.execution_container = Some(target_container);

    let mut work_items = WorkItemsProjection::empty(target_start.updated_at);
    assert_eq!(
        work_items.apply_event(target_start.clone()),
        WorkEventApplyOutcome::Applied
    );
    let work_items_path = gwt_workspace_work_items_path_for_repo_path(&work_event_root);
    save_workspace_work_items_projection_to_path(&work_items_path, &work_items)
        .expect("seed target Work");

    let events_path =
        repo_local_work_events_path_with_migration(&work_event_root).expect("resolve event log");
    append_workspace_work_event_to_path(&events_path, &target_start).expect("seed target event");

    let journal_path = gwt_workspace_journal_path_for_repo_path(&project_state_root);
    append_workspace_journal_entry_to_path(
        &journal_path,
        &WorkspaceJournalEntry {
            id: "journal-foreign-shared-current".to_string(),
            project_root: project_state_root.clone(),
            title: Some(T812_FOREIGN_TITLE.to_string()),
            status_category: Some(WorkspaceStatusCategory::Blocked),
            status_text: Some("Foreign shared-current blocked status".to_string()),
            owner: Some(T812_FOREIGN_OWNER.to_string()),
            next_action: Some("Foreign shared-current next action".to_string()),
            summary: Some(T812_FOREIGN_SUMMARY.to_string()),
            progress_summary: Some(T812_FOREIGN_PROGRESS.to_string()),
            agent_session_id: None,
            agent_current_focus: None,
            agent_title_summary: None,
            updated_at: seeded_at,
        },
    )
    .expect("seed shared-current journal");

    T812Fixture {
        current_path: gwt_workspace_projection_path_for_repo_path(&project_state_root),
        target: T812ResolvedMutationTarget {
            project_state_root,
            work_event_root: work_event_root.clone(),
            session_id: T812_SESSION_ID.to_string(),
            branch_identity: T812_TARGET_BRANCH.to_string(),
            worktree_identity: work_event_root,
            work_id: T812_TARGET_WORK_ID.to_string(),
        },
        work_items_path,
        events_path,
        journal_path,
    }
}

/// Keep the T-812 scenarios focused on persistence semantics; the gwt-layer
/// Session-ledger/runtime revalidation callback is covered at its integration
/// boundary and is intentionally a no-op in these core fixtures.
fn t812_apply_resolved_workspace_update(
    target: &T812ResolvedMutationTarget,
    update: WorkspaceProjectionUpdate,
) -> Result<WorkspaceJournalEntry> {
    update_workspace_projection_with_journal_for_resolved_work_target(
        target,
        update,
        TrackedWorkEventPolicy::Persist,
        |_, _| Ok(()),
        |_, _| Ok(()),
    )
}

fn t812_read_events(path: &Path) -> Vec<WorkEvent> {
    fs::read_to_string(path)
        .expect("read Work event log")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("parse Work event"))
        .collect()
}

fn t812_assert_rejected_without_mutation(
    result: &Result<WorkspaceJournalEntry>,
    before: &T812WorkspaceStateBytes,
    after: &T812WorkspaceStateBytes,
    scenario: &str,
) {
    assert!(
        result.is_err(),
        "{scenario} must reject the complete transaction, got {result:?}"
    );
    assert_eq!(
        after, before,
        "{scenario} must leave current, works, event log, and journal byte-equivalent"
    );
}

#[test]
fn session_bound_sparse_update_does_not_inherit_foreign_shared_current_fields() {
    let _guard = lock_test_env();
    let home = tempfile::tempdir().expect("home");
    let _home = ScopedHome::set(home.path());
    let temp = tempfile::tempdir().expect("tempdir");
    let fixture = t812_seed_session_bound_fixture(temp.path());

    t812_apply_resolved_workspace_update(
        &fixture.target,
        WorkspaceProjectionUpdate {
            title: None,
            status_category: None,
            status_text: None,
            owner: None,
            next_action: Some("Run target-only verification".to_string()),
            summary: None,
            progress_summary: None,
            agent_session_id: Some(T812_SESSION_ID.to_string()),
            agent_current_focus: None,
            agent_title_summary: None,
        },
    )
    .expect("valid Session-bound sparse update");

    let events = t812_read_events(&fixture.events_path);
    let event = events.last().expect("sparse Work event");
    assert_eq!(event.work_item_id, T812_TARGET_WORK_ID);
    assert_eq!(event.kind, WorkEventKind::Update);
    assert_eq!(event.title, None, "omitted title must remain sparse");
    assert_eq!(
        event.intent, None,
        "foreign shared-current summary must not become target intent"
    );
    assert_eq!(event.summary, None, "omitted summary must remain sparse");
    assert_eq!(
        event.progress_summary, None,
        "omitted progress must remain sparse"
    );
    assert_eq!(
        event.status_category, None,
        "omitted status must remain sparse"
    );
    assert_eq!(
        event.owner.as_deref(),
        Some(T812_TARGET_OWNER),
        "owner is target-owned identity, never shared-current metadata"
    );
    assert_eq!(
        event.next_action.as_deref(),
        Some("Run target-only verification")
    );
    assert_eq!(event.agent_session_id.as_deref(), Some(T812_SESSION_ID));
    assert_eq!(
        event
            .execution_container
            .as_ref()
            .and_then(|container| container.branch.as_deref()),
        Some(T812_TARGET_BRANCH)
    );

    let work_items = load_workspace_work_items_from_path(&fixture.work_items_path)
        .expect("load Work projection")
        .expect("Work projection");
    let target = work_items
        .work_items
        .iter()
        .find(|item| item.id == T812_TARGET_WORK_ID)
        .expect("target Work");
    assert_eq!(target.title, T812_TARGET_TITLE);
    assert_eq!(target.intent.as_deref(), Some(T812_TARGET_INTENT));
    assert_eq!(target.summary.as_deref(), Some(T812_TARGET_SUMMARY));
    assert_eq!(
        target.progress_summary.as_deref(),
        Some(T812_TARGET_PROGRESS)
    );
    assert_eq!(target.status_category, WorkspaceStatusCategory::Idle);
    assert_eq!(target.owner.as_deref(), Some(T812_TARGET_OWNER));
}

#[test]
fn session_bound_foreign_current_agent_refresh_advances_projection_timestamp() {
    let _guard = lock_test_env();
    let home = tempfile::tempdir().expect("home");
    let _home = ScopedHome::set(home.path());
    let temp = tempfile::tempdir().expect("tempdir");
    let fixture = t812_seed_session_bound_fixture(temp.path());
    let before = load_workspace_projection_from_path(&fixture.current_path)
        .expect("load shared current before update")
        .expect("shared current before update");

    t812_apply_resolved_workspace_update(
        &fixture.target,
        WorkspaceProjectionUpdate {
            title: None,
            status_category: None,
            status_text: None,
            owner: None,
            next_action: None,
            summary: None,
            progress_summary: None,
            agent_session_id: Some(T812_SESSION_ID.to_string()),
            agent_current_focus: Some("Verify the target Work only".to_string()),
            agent_title_summary: None,
        },
    )
    .expect("valid Session-bound Agent refresh");

    let after = load_workspace_projection_from_path(&fixture.current_path)
        .expect("load shared current after update")
        .expect("shared current after update");
    let agent = after
        .latest_agent_for_session(T812_SESSION_ID)
        .expect("target Session Agent row");

    assert_eq!(
        agent.current_focus.as_deref(),
        Some("Verify the target Work only")
    );
    assert_eq!(
        after.updated_at, agent.updated_at,
        "a persisted Agent refresh must advance the parent projection timestamp"
    );
    assert!(
        after.updated_at > before.updated_at,
        "the parent projection timestamp must make the Agent refresh observable"
    );
    assert_eq!(after.id, before.id);
    assert_eq!(after.title, before.title);
    assert_eq!(after.status_category, before.status_category);
    assert_eq!(after.owner, before.owner);
}

#[test]
fn session_bound_update_rejects_explicit_owner_conflict_without_mutation() {
    let _guard = lock_test_env();
    let home = tempfile::tempdir().expect("home");
    let _home = ScopedHome::set(home.path());
    let temp = tempfile::tempdir().expect("tempdir");
    let fixture = t812_seed_session_bound_fixture(temp.path());
    let before = fixture.state_bytes();

    let result = t812_apply_resolved_workspace_update(
        &fixture.target,
        WorkspaceProjectionUpdate {
            title: None,
            status_category: None,
            status_text: None,
            owner: Some("SPEC-conflicting-explicit-owner".to_string()),
            next_action: None,
            summary: Some("must not commit".to_string()),
            progress_summary: None,
            agent_session_id: Some(T812_SESSION_ID.to_string()),
            agent_current_focus: None,
            agent_title_summary: None,
        },
    );
    let after = fixture.state_bytes();

    t812_assert_rejected_without_mutation(&result, &before, &after, "explicit owner conflict");
}

#[test]
fn session_bound_update_runs_pre_persist_hook_before_any_surface_mutation() {
    let _guard = lock_test_env();
    let home = tempfile::tempdir().expect("home");
    let _home = ScopedHome::set(home.path());
    let temp = tempfile::tempdir().expect("tempdir");
    let fixture = t812_seed_session_bound_fixture(temp.path());
    let before = fixture.state_bytes();

    let result = update_workspace_projection_with_journal_for_resolved_work_target(
        &fixture.target,
        WorkspaceProjectionUpdate {
            title: None,
            status_category: Some(WorkspaceStatusCategory::Done),
            status_text: None,
            owner: None,
            next_action: None,
            summary: Some("must remain in memory only".to_string()),
            progress_summary: None,
            agent_session_id: Some(T812_SESSION_ID.to_string()),
            agent_current_focus: None,
            agent_title_summary: None,
        },
        TrackedWorkEventPolicy::Persist,
        |_, _| Ok(()),
        |event, entry| {
            assert_eq!(event.kind, WorkEventKind::Done);
            assert_eq!(event.work_item_id, T812_TARGET_WORK_ID);
            assert_eq!(event.agent_session_id.as_deref(), Some(T812_SESSION_ID));
            assert_eq!(entry.status_category, Some(WorkspaceStatusCategory::Done));
            Err(GwtError::Other(
                "synthetic write-ahead reservation failure".to_string(),
            ))
        },
    );
    let after = fixture.state_bytes();

    t812_assert_rejected_without_mutation(
        &result,
        &before,
        &after,
        "pre-persist reservation failure",
    );
}

#[test]
fn session_bound_update_accepts_same_session_subordinate_helper() {
    const RAW_PROVIDER_ACTOR_ID: &str = "provider-thread-private-sentinel-86";

    let _guard = lock_test_env();
    let home = tempfile::tempdir().expect("home");
    let _home = ScopedHome::set(home.path());
    let temp = tempfile::tempdir().expect("tempdir");
    let fixture = t812_seed_session_bound_fixture(temp.path());
    let mut current = load_workspace_projection_from_path(&fixture.current_path)
        .expect("load shared current")
        .expect("shared current");
    let mut helper = current.agents[0].clone();
    helper.window_id = Some(RAW_PROVIDER_ACTOR_ID.to_string());
    helper.display_name = "Codex subordinate helper".to_string();
    helper.updated_at += chrono::Duration::seconds(1);
    current.agents.push(helper);
    save_workspace_projection_to_path(&fixture.current_path, &current)
        .expect("seed same-Session helper");

    t812_apply_resolved_workspace_update(
        &fixture.target,
        WorkspaceProjectionUpdate {
            title: None,
            status_category: None,
            status_text: None,
            owner: None,
            next_action: None,
            summary: Some("Same-Session helper status".to_string()),
            progress_summary: None,
            agent_session_id: Some(T812_SESSION_ID.to_string()),
            agent_current_focus: None,
            agent_title_summary: None,
        },
    )
    .expect("same-Session subordinate helper must be authorized");

    let events = t812_read_events(&fixture.events_path);
    let event = events.last().expect("helper Work event");
    assert_eq!(event.work_item_id, T812_TARGET_WORK_ID);
    assert_eq!(event.agent_session_id.as_deref(), Some(T812_SESSION_ID));
    assert!(
        !serde_json::to_string(event)
            .expect("serialize helper event")
            .contains(RAW_PROVIDER_ACTOR_ID),
        "provider actor id is inspection-only and must not enter tracked Work events"
    );
}

#[test]
fn session_bound_update_revalidates_assignment_after_lock_wait_without_mutation() {
    use fs2::FileExt;

    const REASSIGNED_WORK_ID: &str = "work-reassigned-while-waiting";
    const REASSIGNED_BRANCH: &str = "work/reassigned-while-waiting";

    let _guard = lock_test_env();
    let home = tempfile::tempdir().expect("home");
    let _home = ScopedHome::set(home.path());
    let temp = tempfile::tempdir().expect("tempdir");
    let fixture = t812_seed_session_bound_fixture(temp.path());
    let reassigned_worktree = temp.path().join("reassigned-worktree");
    fs::create_dir_all(&reassigned_worktree).expect("reassigned worktree");

    let reassigned_at = Utc.with_ymd_and_hms(2026, 7, 22, 1, 5, 0).unwrap();
    let reassigned_container = WorkspaceExecutionContainerRef {
        branch: Some(REASSIGNED_BRANCH.to_string()),
        worktree_path: Some(reassigned_worktree.clone()),
        pr_number: None,
        pr_url: None,
        pr_state: None,
    };
    let mut reassigned_start = WorkEvent::new(
        WorkEventKind::Start,
        REASSIGNED_WORK_ID,
        reassigned_at - chrono::Duration::seconds(1),
    );
    reassigned_start.title = Some("Reassigned Work".to_string());
    reassigned_start.owner = Some("SPEC-reassigned-owner".to_string());
    reassigned_start.status_category = Some(WorkspaceStatusCategory::Active);
    reassigned_start.execution_container = Some(reassigned_container);
    let mut work_items = load_workspace_work_items_from_path(&fixture.work_items_path)
        .expect("load Work projection")
        .expect("Work projection");
    assert_eq!(
        work_items.apply_event(reassigned_start.clone()),
        WorkEventApplyOutcome::Applied
    );
    save_workspace_work_items_projection_to_path(&fixture.work_items_path, &work_items)
        .expect("seed reassignment target");
    append_workspace_work_event_to_path(&fixture.events_path, &reassigned_start)
        .expect("seed reassignment event");

    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(fixture.work_items_path.with_extension("lock"))
        .expect("open Work transaction lock");
    lock.lock_exclusive().expect("hold Work transaction lock");

    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let thread_home = home.path().to_path_buf();
    let target = fixture.target.clone();
    let handle = std::thread::spawn(move || {
        let _home = ScopedHome::set(&thread_home);
        started_tx.send(()).expect("signal mutation start");
        t812_apply_resolved_workspace_update(
            &target,
            WorkspaceProjectionUpdate {
                title: None,
                status_category: None,
                status_text: None,
                owner: None,
                next_action: None,
                summary: Some("must not land on reassigned Work".to_string()),
                progress_summary: None,
                agent_session_id: Some(T812_SESSION_ID.to_string()),
                agent_current_focus: None,
                agent_title_summary: None,
            },
        )
    });
    started_rx.recv().expect("mutation thread started");
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(
        !handle.is_finished(),
        "mutation must be waiting on the Work transaction lock"
    );

    let mut reassigned_current = load_workspace_projection_from_path(&fixture.current_path)
        .expect("load current for reassignment")
        .expect("current for reassignment");
    let agent = reassigned_current
        .latest_agent_for_session_mut(T812_SESSION_ID)
        .expect("assigned Session");
    agent.workspace_id = Some(REASSIGNED_WORK_ID.to_string());
    agent.branch = Some(REASSIGNED_BRANCH.to_string());
    agent.worktree_path = Some(reassigned_worktree);
    agent.updated_at = reassigned_at;
    reassigned_current.updated_at = reassigned_at;
    save_workspace_projection_to_path_unlocked(&fixture.current_path, &reassigned_current)
        .expect("commit concurrent reassignment");
    let after_reassignment = fixture.state_bytes();

    FileExt::unlock(&lock).expect("release Work transaction lock");
    let result = handle.join().expect("join mutation thread");
    let after_attempt = fixture.state_bytes();

    t812_assert_rejected_without_mutation(
        &result,
        &after_reassignment,
        &after_attempt,
        "stale resolved target after lock-time reassignment",
    );
}

/// Issue #3278: `TrackedWorkEventPolicy::SkipTracked` must leave the git-tracked
/// `events.jsonl` byte-for-byte unchanged (so a settled/merged worktree stays
/// clean) while the machine-local projection still records the coordination
/// update.
#[test]
fn skip_tracked_policy_leaves_committed_events_log_untouched() {
    let _guard = crate::test_support::env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = tempfile::tempdir().expect("home");
    let _home = ScopedHome::set(home.path());
    let temp = tempfile::tempdir().expect("tempdir");
    let project_state_root = temp.path().join("workspace-home");
    let worktree = temp.path().join("workspace-home/work/20260722-0100");
    init_test_git_repo(&worktree);

    let mut projection = WorkspaceProjection::default_for_project(&project_state_root);
    projection.id = canonical_work_id(&project_state_root, Some("develop"), None).unwrap();
    projection.agents.push(WorkspaceAgentSummary {
        session_id: "session-current".to_string(),
        window_id: None,
        agent_id: "codex".to_string(),
        display_name: "Codex".to_string(),
        status_category: WorkspaceStatusCategory::Active,
        current_focus: None,
        title_summary: None,
        worktree_path: Some(worktree.clone()),
        branch: Some("work/20260722-0100".to_string()),
        last_board_entry_id: None,
        last_board_entry_kind: None,
        coordination_scope: None,
        affiliation_status: WorkspaceAgentAffiliationStatus::Unassigned,
        workspace_id: None,
        updated_at: Utc::now(),
    });
    save_workspace_projection(&project_state_root, &projection).expect("seed projection");

    let update = |focus: &str| WorkspaceProjectionUpdate {
        title: None,
        status_category: None,
        status_text: None,
        owner: None,
        next_action: None,
        summary: None,
        progress_summary: None,
        agent_session_id: Some("session-current".to_string()),
        agent_current_focus: Some(focus.to_string()),
        agent_title_summary: None,
    };

    // Active work: the event is appended to the tracked log (bundled into commits).
    update_workspace_projection_with_journal_for_work_event_root(
        &project_state_root,
        &worktree,
        update("active focus"),
        TrackedWorkEventPolicy::Persist,
    )
    .expect("persist update");

    let events_path = gwt_repo_local_work_events_path(&worktree);
    let after_persist = fs::read_to_string(&events_path).expect("events log");
    assert_eq!(
        after_persist.lines().count(),
        1,
        "active update appends one tracked event"
    );

    // Settled work: a coordination-only update must not touch the tracked log.
    update_workspace_projection_with_journal_for_work_event_root(
        &project_state_root,
        &worktree,
        update("post-completion focus"),
        TrackedWorkEventPolicy::SkipTracked,
    )
    .expect("skip-tracked update");

    assert_eq!(
        fs::read_to_string(&events_path).expect("events log"),
        after_persist,
        "SkipTracked must leave the committed events.jsonl byte-for-byte unchanged"
    );

    let saved = load_workspace_projection(&project_state_root)
        .expect("load projection")
        .expect("projection exists");
    let agent = saved
        .agents
        .iter()
        .find(|agent| agent.session_id == "session-current")
        .expect("agent row");
    assert_eq!(
        agent.current_focus.as_deref(),
        Some("post-completion focus"),
        "machine-local projection must still reflect the coordination update"
    );
}

#[test]
fn workspace_work_items_synthesize_from_legacy_current_and_journal_without_rewrite() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project_root = temp.path().join("repo");
    let current_path = temp.path().join("workspace/current.json");
    let journal_path = temp.path().join("workspace/journal.jsonl");
    let work_items_path = temp.path().join("workspace/work_items.json");
    let first_at = Utc.with_ymd_and_hms(2026, 5, 11, 2, 0, 0).unwrap();
    let second_at = Utc.with_ymd_and_hms(2026, 5, 11, 2, 5, 0).unwrap();

    let mut projection = WorkspaceProjection::default_for_project(&project_root);
    projection.id = "workspace-current".to_string();
    projection.title = "Workspace WorkItem history".to_string();
    projection.status_category = WorkspaceStatusCategory::Active;
    projection.status_text = "Implementing WorkItem projection".to_string();
    projection.summary = Some("Legacy current state remains readable.".to_string());
    projection.owner = Some("SPEC-2359".to_string());
    projection.board_refs.push("board-legacy-1".to_string());
    save_workspace_projection_to_path(&current_path, &projection).expect("save legacy projection");

    append_workspace_journal_entry_to_path(
        &journal_path,
        &WorkspaceJournalEntry {
            id: "journal-start".to_string(),
            project_root: project_root.clone(),
            title: Some("Workspace WorkItem history".to_string()),
            status_category: Some(WorkspaceStatusCategory::Active),
            status_text: Some("Started".to_string()),
            owner: Some("SPEC-2359".to_string()),
            next_action: None,
            summary: Some("Started from legacy journal.".to_string()),
            progress_summary: None,
            agent_session_id: Some("session-legacy".to_string()),
            agent_current_focus: Some("Implement lifecycle events".to_string()),
            agent_title_summary: Some("WorkItem history".to_string()),
            updated_at: first_at,
        },
    )
    .expect("append first journal");
    append_workspace_journal_entry_to_path(
        &journal_path,
        &WorkspaceJournalEntry {
            id: "journal-update".to_string(),
            project_root: project_root.clone(),
            title: None,
            status_category: Some(WorkspaceStatusCategory::Blocked),
            status_text: Some("Waiting for coordination decision".to_string()),
            owner: Some("SPEC-2359".to_string()),
            next_action: Some("Post Board handoff".to_string()),
            summary: Some("Blocked state from legacy journal.".to_string()),
            progress_summary: None,
            agent_session_id: Some("session-legacy".to_string()),
            agent_current_focus: None,
            agent_title_summary: Some("WorkItem history".to_string()),
            updated_at: second_at,
        },
    )
    .expect("append second journal");

    let synthesized = load_or_synthesize_workspace_work_items_from_paths(
        &work_items_path,
        &current_path,
        &journal_path,
        &project_root,
    )
    .expect("synthesize work items");

    assert_eq!(synthesized.work_items.len(), 1);
    let item = &synthesized.work_items[0];
    assert_eq!(item.id, "workspace-current");
    assert_eq!(item.title, "Work WorkItem history");
    assert_eq!(item.status_category, WorkspaceStatusCategory::Active);
    assert_eq!(item.owner.as_deref(), Some("SPEC-2359"));
    assert_eq!(item.board_refs, vec!["board-legacy-1".to_string()]);
    assert_eq!(item.events.len(), 2);
    assert_eq!(
        item.events[0].summary.as_deref(),
        Some("Started from legacy journal.")
    );
    assert_eq!(item.events[1].kind, WorkEventKind::Blocked);
    assert!(
        !work_items_path.exists(),
        "legacy migration must be read-only until a real WorkItem event is recorded"
    );
}

#[test]
fn workspace_update_persists_agent_title_summary_separately_from_focus() {
    let updated_at = Utc.with_ymd_and_hms(2026, 5, 7, 2, 30, 0).unwrap();
    let mut projection = WorkspaceProjection::default_for_project("/repo");
    projection.agents.push(WorkspaceAgentSummary {
        session_id: "session-1".to_string(),
        window_id: Some("tab-1::agent-1".to_string()),
        agent_id: "codex".to_string(),
        display_name: "Codex".to_string(),
        status_category: WorkspaceStatusCategory::Active,
        current_focus: None,
        title_summary: None,
        worktree_path: None,
        branch: None,
        last_board_entry_id: None,
        last_board_entry_kind: None,
        coordination_scope: None,
        affiliation_status: WorkspaceAgentAffiliationStatus::Assigned,
        workspace_id: None,
        updated_at,
    });

    let journal = projection.apply_update(
        WorkspaceProjectionUpdate {
            title: None,
            status_category: None,
            status_text: None,
            owner: None,
            next_action: None,
            summary: None,
            progress_summary: None,
            agent_session_id: Some("session-1".to_string()),
            agent_current_focus: Some(
                "Implementing title-summary support across Board and Workspace".to_string(),
            ),
            agent_title_summary: Some("Title summary support".to_string()),
        },
        updated_at,
    );

    assert_eq!(
        projection.agents[0].current_focus.as_deref(),
        Some("Implementing title-summary support across Board and Workspace")
    );
    assert_eq!(
        projection.agents[0].title_summary.as_deref(),
        Some("Title summary support")
    );
    assert_eq!(
        journal.agent_current_focus.as_deref(),
        Some("Implementing title-summary support across Board and Workspace")
    );
    assert_eq!(
        journal.agent_title_summary.as_deref(),
        Some("Title summary support")
    );
}

#[test]
fn recent_workspace_journal_entries_load_newest_first_with_limit() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project_root = temp.path().join("repo");
    let current_path = temp.path().join("workspace/current.json");
    let journal_path = temp.path().join("workspace/journal.jsonl");
    let first_at = Utc.with_ymd_and_hms(2026, 5, 7, 1, 0, 0).unwrap();
    let second_at = Utc.with_ymd_and_hms(2026, 5, 7, 1, 5, 0).unwrap();

    update_workspace_projection_with_journal_paths_at(
        &current_path,
        &journal_path,
        &project_root,
        WorkspaceProjectionUpdate {
            title: Some("Workspace Overview".to_string()),
            status_category: Some(WorkspaceStatusCategory::Active),
            status_text: Some("Drafting overview".to_string()),
            owner: Some("SPEC-2359".to_string()),
            next_action: None,
            summary: Some("First summary".to_string()),
            progress_summary: None,
            agent_session_id: None,
            agent_current_focus: None,
            agent_title_summary: None,
        },
        first_at,
    )
    .expect("first update");
    update_workspace_projection_with_journal_paths_at(
        &current_path,
        &journal_path,
        &project_root,
        WorkspaceProjectionUpdate {
            title: None,
            status_category: Some(WorkspaceStatusCategory::Idle),
            status_text: Some("Ready for review".to_string()),
            owner: Some("SPEC-2359".to_string()),
            next_action: Some("Review Workspace Overview".to_string()),
            summary: Some("Second summary".to_string()),
            progress_summary: None,
            agent_session_id: None,
            agent_current_focus: None,
            agent_title_summary: None,
        },
        second_at,
    )
    .expect("second update");

    let recent = load_recent_workspace_journal_entries_from_path(&journal_path, 1)
        .expect("recent journal entries");

    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].summary.as_deref(), Some("Second summary"));
    assert_eq!(recent[0].updated_at, second_at);
}

fn assigned_agent(session_id: &str, agent_id: &str, workspace_id: &str) -> WorkspaceAgentSummary {
    WorkspaceAgentSummary {
        session_id: session_id.into(),
        window_id: None,
        agent_id: agent_id.into(),
        display_name: agent_id.into(),
        status_category: WorkspaceStatusCategory::Active,
        current_focus: None,
        title_summary: None,
        worktree_path: None,
        branch: None,
        last_board_entry_id: None,
        last_board_entry_kind: None,
        coordination_scope: None,
        affiliation_status: WorkspaceAgentAffiliationStatus::Assigned,
        workspace_id: Some(workspace_id.into()),
        updated_at: Utc::now(),
    }
}

fn unassigned_agent(session_id: &str, agent_id: &str) -> WorkspaceAgentSummary {
    let mut a = assigned_agent(session_id, agent_id, "_unused");
    a.affiliation_status = WorkspaceAgentAffiliationStatus::Unassigned;
    a.workspace_id = None;
    a
}

fn lock_test_env() -> std::sync::MutexGuard<'static, ()> {
    crate::test_support::env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[test]
fn resolve_workspace_id_for_session_returns_assigned_workspace_id() {
    let _guard = lock_test_env();
    let dir = tempfile::tempdir().unwrap();
    let mut projection = WorkspaceProjection::default_for_project(dir.path());
    projection
        .agents
        .push(assigned_agent("sess-A", "codex", "ws-1"));
    save_workspace_projection(dir.path(), &projection).unwrap();

    assert_eq!(
        resolve_workspace_id_for_session(dir.path(), "sess-A"),
        Some("ws-1".into())
    );
}

#[test]
fn resolve_workspace_id_for_session_returns_none_for_unassigned_agent() {
    let _guard = lock_test_env();
    let dir = tempfile::tempdir().unwrap();
    let mut projection = WorkspaceProjection::default_for_project(dir.path());
    projection.agents.push(unassigned_agent("sess-B", "codex"));
    save_workspace_projection(dir.path(), &projection).unwrap();

    assert_eq!(resolve_workspace_id_for_session(dir.path(), "sess-B"), None);
}

#[test]
fn resolve_workspace_id_for_session_uses_latest_duplicate_agent_row() {
    let _guard = lock_test_env();
    let dir = tempfile::tempdir().unwrap();
    let older = Utc.with_ymd_and_hms(2026, 7, 15, 7, 0, 0).unwrap();
    let newer = Utc.with_ymd_and_hms(2026, 7, 15, 8, 0, 0).unwrap();
    let mut projection = WorkspaceProjection::default_for_project(dir.path());
    let mut old_assignment = assigned_agent("sess-duplicate", "codex", "work-old");
    old_assignment.updated_at = older;
    let mut new_assignment = assigned_agent("sess-duplicate", "codex", "work-new");
    new_assignment.updated_at = newer;
    projection.agents.extend([old_assignment, new_assignment]);
    save_workspace_projection(dir.path(), &projection).unwrap();

    assert_eq!(
        try_resolve_workspace_id_for_session(dir.path(), "sess-duplicate").unwrap(),
        Some("work-new".to_string())
    );

    projection.agents[0].updated_at = newer;
    save_workspace_projection(dir.path(), &projection).unwrap();
    let stable_assignment =
        try_resolve_workspace_id_for_session(dir.path(), "sess-duplicate").unwrap();
    projection.agents.reverse();
    save_workspace_projection(dir.path(), &projection).unwrap();
    assert_eq!(
        try_resolve_workspace_id_for_session(dir.path(), "sess-duplicate").unwrap(),
        stable_assignment,
        "equal timestamps use a stable row-content tie-break independent of projection order"
    );
}

#[test]
fn resolve_workspace_id_for_session_does_not_fall_back_past_latest_unassigned_row() {
    let _guard = lock_test_env();
    let dir = tempfile::tempdir().unwrap();
    let older = Utc.with_ymd_and_hms(2026, 7, 15, 7, 0, 0).unwrap();
    let newer = Utc.with_ymd_and_hms(2026, 7, 15, 8, 0, 0).unwrap();
    let mut projection = WorkspaceProjection::default_for_project(dir.path());
    let mut old_assignment = assigned_agent("sess-duplicate", "codex", "work-old");
    old_assignment.updated_at = older;
    let mut current_unassigned = unassigned_agent("sess-duplicate", "codex");
    current_unassigned.updated_at = newer;
    projection
        .agents
        .extend([old_assignment, current_unassigned]);
    save_workspace_projection(dir.path(), &projection).unwrap();

    assert_eq!(
        try_resolve_workspace_id_for_session(dir.path(), "sess-duplicate").unwrap(),
        None
    );
}

#[test]
fn resolve_workspace_id_for_session_returns_none_when_session_missing() {
    let _guard = lock_test_env();
    let dir = tempfile::tempdir().unwrap();
    let projection = WorkspaceProjection::default_for_project(dir.path());
    save_workspace_projection(dir.path(), &projection).unwrap();

    assert_eq!(
        resolve_workspace_id_for_session(dir.path(), "sess-missing"),
        None
    );
}

#[test]
fn session_workspace_resolvers_preserve_strict_and_best_effort_error_semantics() {
    let _guard = lock_test_env();
    let dir = tempfile::tempdir().unwrap();
    let path = gwt_workspace_projection_path_for_repo_path(dir.path());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, "{").unwrap();

    assert!(try_resolve_workspace_id_for_session(dir.path(), "sess-corrupt").is_err());
    assert_eq!(
        resolve_workspace_id_for_session(dir.path(), "sess-corrupt"),
        None
    );
}

#[test]
fn resolve_workspace_id_for_mention_session_matches_session_id() {
    let _guard = lock_test_env();
    let dir = tempfile::tempdir().unwrap();
    let mut projection = WorkspaceProjection::default_for_project(dir.path());
    projection
        .agents
        .push(assigned_agent("sess-C", "codex", "ws-2"));
    save_workspace_projection(dir.path(), &projection).unwrap();

    assert_eq!(
        resolve_workspace_id_for_mention(dir.path(), "session", "sess-C"),
        Some("ws-2".into())
    );
}

#[test]
fn resolve_workspace_id_for_mention_agent_matches_display_or_agent_id() {
    let _guard = lock_test_env();
    let dir = tempfile::tempdir().unwrap();
    let mut projection = WorkspaceProjection::default_for_project(dir.path());
    projection
        .agents
        .push(assigned_agent("sess-D", "codex", "ws-3"));
    save_workspace_projection(dir.path(), &projection).unwrap();

    assert_eq!(
        resolve_workspace_id_for_mention(dir.path(), "agent", "codex"),
        Some("ws-3".into())
    );
    assert_eq!(
        resolve_workspace_id_for_mention(dir.path(), "agent", "Codex"),
        Some("ws-3".into()),
        "case-insensitive display-name match"
    );
}

#[test]
fn resolve_workspace_id_for_mention_uses_latest_duplicate_session_row() {
    let dir = tempfile::tempdir().unwrap();
    let mut projection = WorkspaceProjection::default_for_project(dir.path());
    let old_at = Utc.with_ymd_and_hms(2026, 7, 15, 8, 0, 0).unwrap();
    let new_at = old_at + chrono::Duration::hours(1);
    let mut stale = assigned_agent("sess-mention-duplicate", "codex", "work-stale");
    stale.updated_at = old_at;
    let mut current = assigned_agent("sess-mention-duplicate", "codex", "work-current");
    current.updated_at = new_at;
    projection.agents = vec![stale, current];
    save_workspace_projection(dir.path(), &projection).unwrap();

    assert_eq!(
        resolve_workspace_id_for_mention(dir.path(), "session", "sess-mention-duplicate"),
        Some("work-current".to_string())
    );
    assert_eq!(
        resolve_workspace_id_for_mention(dir.path(), "agent", "codex"),
        Some("work-current".to_string())
    );

    projection.agents[1].affiliation_status = WorkspaceAgentAffiliationStatus::Unassigned;
    projection.agents[1].workspace_id = None;
    projection.agents[1].updated_at = new_at + chrono::Duration::hours(1);
    save_workspace_projection(dir.path(), &projection).unwrap();
    assert_eq!(
        resolve_workspace_id_for_mention(dir.path(), "session", "sess-mention-duplicate"),
        None
    );
    assert_eq!(
        resolve_workspace_id_for_mention(dir.path(), "agent", "codex"),
        None
    );
}

#[test]
fn resolve_workspace_id_for_mention_returns_none_for_unassigned_target() {
    let _guard = lock_test_env();
    let dir = tempfile::tempdir().unwrap();
    let mut projection = WorkspaceProjection::default_for_project(dir.path());
    projection.agents.push(unassigned_agent("sess-E", "codex"));
    save_workspace_projection(dir.path(), &projection).unwrap();

    assert_eq!(
        resolve_workspace_id_for_mention(dir.path(), "session", "sess-E"),
        None
    );
    assert_eq!(
        resolve_workspace_id_for_mention(dir.path(), "agent", "codex"),
        None
    );
}

#[test]
fn resolve_workspace_id_for_mention_user_or_branch_kind_returns_none() {
    let _guard = lock_test_env();
    let dir = tempfile::tempdir().unwrap();
    let mut projection = WorkspaceProjection::default_for_project(dir.path());
    projection
        .agents
        .push(assigned_agent("sess-F", "codex", "ws-4"));
    save_workspace_projection(dir.path(), &projection).unwrap();

    assert_eq!(
        resolve_workspace_id_for_mention(dir.path(), "user", "akiojin"),
        None
    );
    assert_eq!(
        resolve_workspace_id_for_mention(dir.path(), "branch", "feature/x"),
        None
    );
}

#[test]
fn classify_workspace_projections_deletes_empty_default_projection() {
    let _guard = lock_test_env();
    let home = tempfile::tempdir().expect("home");
    let _home = ScopedHome::set(home.path());
    let repo = tempfile::tempdir().expect("repo");
    let projection = WorkspaceProjection::default_for_project(repo.path());
    save_workspace_projection(repo.path(), &projection).expect("save projection");

    let plan = classify_workspace_projections(
        &home.path().join(".gwt/projects"),
        &WorkspaceRetentionConfig::default(),
        Utc::now(),
        |_| false,
    );

    assert_eq!(plan.len(), 1);
    assert_eq!(plan[0].workspace_id, projection.id);
    assert_eq!(plan[0].stale_reason, Some(StaleReason::EmptyProjection));
    assert_eq!(plan[0].action, PruneAction::Delete);
}

#[test]
fn classify_workspace_projections_deletes_empty_projection_with_agent_stub() {
    let _guard = lock_test_env();
    let home = tempfile::tempdir().expect("home");
    let _home = ScopedHome::set(home.path());
    let repo = tempfile::tempdir().expect("repo");
    let mut projection = WorkspaceProjection::default_for_project(repo.path());
    projection
        .agents
        .push(unassigned_agent("sess-stub", "codex"));
    save_workspace_projection(repo.path(), &projection).expect("save projection");

    let plan = classify_workspace_projections(
        &home.path().join(".gwt/projects"),
        &WorkspaceRetentionConfig::default(),
        Utc::now(),
        |_| false,
    );

    assert_eq!(plan.len(), 1);
    assert_eq!(plan[0].stale_reason, Some(StaleReason::EmptyProjection));
    assert_eq!(plan[0].action, PruneAction::Delete);
}

#[test]
fn classify_workspace_projections_keeps_projection_with_agent_worktree() {
    let _guard = lock_test_env();
    let home = tempfile::tempdir().expect("home");
    let _home = ScopedHome::set(home.path());
    let repo = tempfile::tempdir().expect("repo");
    let worktree = tempfile::tempdir().expect("worktree");
    let mut projection = WorkspaceProjection::default_for_project(repo.path());
    let mut agent = unassigned_agent("sess-real", "codex");
    agent.worktree_path = Some(worktree.path().to_path_buf());
    projection.agents.push(agent);
    save_workspace_projection(repo.path(), &projection).expect("save projection");

    let plan = classify_workspace_projections(
        &home.path().join(".gwt/projects"),
        &WorkspaceRetentionConfig::default(),
        Utc::now(),
        |_| false,
    );

    assert_eq!(plan.len(), 1);
    assert_eq!(plan[0].stale_reason, None);
    assert!(matches!(
        plan[0].action,
        PruneAction::Skip {
            reason: PruneSkipReason::NotStale
        }
    ));
}

#[test]
fn apply_prune_plan_removes_empty_project_dir_after_projection_delete() {
    let _guard = lock_test_env();
    let home = tempfile::tempdir().expect("home");
    let _home = ScopedHome::set(home.path());
    let repo = tempfile::tempdir().expect("repo");
    let projection = WorkspaceProjection::default_for_project(repo.path());
    save_workspace_projection(repo.path(), &projection).expect("save projection");

    let project_dir = gwt_project_dir_for_repo_path(repo.path());
    let plan = classify_workspace_projections(
        &home.path().join(".gwt/projects"),
        &WorkspaceRetentionConfig::default(),
        Utc::now(),
        |_| false,
    );

    let summary = apply_prune_plan(&plan, false).expect("apply prune");

    assert_eq!(summary.deleted, 1);
    assert!(
        !project_dir.exists(),
        "empty project dir should be removed after deleting its only projection"
    );
}

// SPEC-2359 US-37 / T-236..T-239: auto-done emit helper and retroactive migration scanner.

#[test]
fn auto_done_emit_helper_appends_single_done_event_and_marks_work_item_done() {
    let temp = tempfile::tempdir().expect("tempdir");
    let work_items_path = temp.path().join("workspace/work_items.json");
    let events_path = temp.path().join("workspace/work_events.jsonl");
    let started_at = Utc.with_ymd_and_hms(2026, 5, 13, 1, 0, 0).unwrap();
    let done_at = Utc.with_ymd_and_hms(2026, 5, 13, 2, 0, 0).unwrap();

    let mut start = WorkEvent::new(WorkEventKind::Start, "wi-auto-done", started_at);
    start.title = Some("Auto-done test work".to_string());
    start.status_category = Some(WorkspaceStatusCategory::Active);
    record_workspace_work_event_paths(&work_items_path, &events_path, start)
        .expect("record start event");

    let emitted = emit_workspace_done_event_if_absent_paths(
        &work_items_path,
        &events_path,
        "wi-auto-done",
        done_at,
    )
    .expect("emit done");
    assert!(emitted, "first call must append a Done event");

    let projection = load_workspace_work_items_from_path(&work_items_path)
        .expect("load work items")
        .expect("work items");
    assert_eq!(projection.work_items.len(), 1);
    let item = &projection.work_items[0];
    assert_eq!(item.id, "wi-auto-done");
    assert_eq!(item.status_category, WorkspaceStatusCategory::Done);
    assert_eq!(item.completed_at, Some(done_at));

    let events_text = std::fs::read_to_string(&events_path).expect("read events");
    let done_lines = events_text
        .lines()
        .filter(|line| line.contains("\"kind\":\"done\"") && line.contains("wi-auto-done"))
        .count();
    assert_eq!(done_lines, 1, "exactly one Done event must be persisted");
}

#[test]
fn auto_done_emit_helper_is_idempotent_per_work_item_id() {
    let temp = tempfile::tempdir().expect("tempdir");
    let work_items_path = temp.path().join("workspace/work_items.json");
    let events_path = temp.path().join("workspace/work_events.jsonl");
    let started_at = Utc.with_ymd_and_hms(2026, 5, 13, 1, 0, 0).unwrap();
    let first_done_at = Utc.with_ymd_and_hms(2026, 5, 13, 2, 0, 0).unwrap();
    let second_done_at = Utc.with_ymd_and_hms(2026, 5, 13, 3, 0, 0).unwrap();

    let mut start = WorkEvent::new(WorkEventKind::Start, "wi-idempotent", started_at);
    start.status_category = Some(WorkspaceStatusCategory::Active);
    record_workspace_work_event_paths(&work_items_path, &events_path, start)
        .expect("record start event");

    let first = emit_workspace_done_event_if_absent_paths(
        &work_items_path,
        &events_path,
        "wi-idempotent",
        first_done_at,
    )
    .expect("first emit");
    let second = emit_workspace_done_event_if_absent_paths(
        &work_items_path,
        &events_path,
        "wi-idempotent",
        second_done_at,
    )
    .expect("second emit");

    assert!(first, "first call must append Done");
    assert!(!second, "second call must be a noop");

    let events_text = std::fs::read_to_string(&events_path).expect("read events");
    let done_lines = events_text
        .lines()
        .filter(|line| line.contains("\"kind\":\"done\"") && line.contains("wi-idempotent"))
        .count();
    assert_eq!(done_lines, 1, "Done event must not be duplicated");

    let projection = load_workspace_work_items_from_path(&work_items_path)
        .expect("load work items")
        .expect("work items");
    let item = &projection.work_items[0];
    assert_eq!(item.completed_at, Some(first_done_at));
}

#[test]
fn terminal_emit_does_not_create_a_missing_work_item() {
    let temp = tempfile::tempdir().expect("tempdir");
    let work_items_path = temp.path().join("workspace/works.json");
    let events_path = temp.path().join("workspace/work-events-closed.jsonl");
    save_workspace_work_items_projection_to_path(
        &work_items_path,
        &WorkItemsProjection::empty(Utc::now()),
    )
    .unwrap();

    assert!(!emit_workspace_done_event_if_absent_paths(
        &work_items_path,
        &events_path,
        "work-missing",
        Utc::now(),
    )
    .unwrap());
    let projection = load_workspace_work_items_from_path(&work_items_path)
        .unwrap()
        .unwrap();
    assert!(projection.work_items.is_empty());
    assert!(!events_path.exists());
}

#[test]
fn terminal_retry_recovers_durable_event_without_appending_a_second_close() {
    for (kind, label) in [
        (WorkEventKind::Done, "done"),
        (WorkEventKind::Discard, "discard"),
    ] {
        let temp = tempfile::tempdir().expect("tempdir");
        let work_items_path = temp.path().join("workspace/works.json");
        let shared_events_path = temp.path().join("shared/events.jsonl");
        let close_events_path = temp.path().join("local/work-events-closed.jsonl");
        let started_at = Utc.with_ymd_and_hms(2026, 7, 15, 7, 0, 0).unwrap();
        let closed_at = Utc.with_ymd_and_hms(2026, 7, 15, 8, 0, 0).unwrap();
        let work_id = format!("work-durable-{label}");
        let mut start = WorkEvent::new(WorkEventKind::Start, &work_id, started_at);
        start.status_category = Some(WorkspaceStatusCategory::Active);
        record_workspace_work_event_paths(&work_items_path, &shared_events_path, start).unwrap();

        let mut durable = WorkEvent::new(kind, &work_id, closed_at);
        durable.id = format!("evt-durable-{label}");
        if kind == WorkEventKind::Done {
            durable.status_category = Some(WorkspaceStatusCategory::Done);
        }
        append_workspace_work_event_to_path(&close_events_path, &durable).unwrap();

        let emitted = match kind {
            WorkEventKind::Done => emit_workspace_done_event_if_absent_paths(
                &work_items_path,
                &close_events_path,
                &work_id,
                closed_at + chrono::Duration::minutes(1),
            ),
            WorkEventKind::Discard => emit_workspace_discard_event_if_absent_paths(
                &work_items_path,
                &close_events_path,
                &work_id,
                closed_at + chrono::Duration::minutes(1),
            ),
            _ => unreachable!(),
        }
        .unwrap();

        assert!(!emitted, "retry must reuse the durable {label} event");
        let projection = load_workspace_work_items_from_path(&work_items_path)
            .unwrap()
            .unwrap();
        let item = projection
            .work_items
            .iter()
            .find(|item| item.id == work_id)
            .unwrap();
        assert!(item.is_terminal());
        assert_eq!(item.discarded, kind == WorkEventKind::Discard);
        let close_lines = std::fs::read_to_string(&close_events_path)
            .unwrap()
            .lines()
            .count();
        assert_eq!(close_lines, 1, "retry must not append a second {label}");
    }
}

#[test]
fn terminal_retry_refolds_durable_close_before_a_later_saved_heartbeat() {
    for (kind, label) in [
        (WorkEventKind::Done, "done"),
        (WorkEventKind::Discard, "discard"),
    ] {
        let temp = tempfile::tempdir().expect("tempdir");
        let work_items_path = temp.path().join("workspace/works.json");
        let shared_events_path = temp.path().join("shared/events.jsonl");
        let close_events_path = temp.path().join("local/work-events-closed.jsonl");
        let started_at = Utc.with_ymd_and_hms(2026, 7, 15, 7, 0, 0).unwrap();
        let closed_at = Utc.with_ymd_and_hms(2026, 7, 15, 8, 0, 0).unwrap();
        let heartbeat_at = Utc.with_ymd_and_hms(2026, 7, 15, 9, 0, 0).unwrap();
        let retry_at = Utc.with_ymd_and_hms(2026, 7, 15, 10, 0, 0).unwrap();
        let work_id = format!("work-heartbeat-recovery-{label}");
        let mut start = WorkEvent::new(WorkEventKind::Start, &work_id, started_at);
        start.status_category = Some(WorkspaceStatusCategory::Active);
        record_workspace_work_event_paths(&work_items_path, &shared_events_path, start).unwrap();

        let mut durable = WorkEvent::new(kind, &work_id, closed_at);
        durable.id = format!("evt-heartbeat-durable-{label}");
        if kind == WorkEventKind::Done {
            durable.status_category = Some(WorkspaceStatusCategory::Done);
        }
        append_workspace_work_event_to_path(&close_events_path, &durable).unwrap();

        let mut projection = load_workspace_work_items_from_path(&work_items_path)
            .unwrap()
            .unwrap();
        let mut heartbeat = WorkEvent::new(WorkEventKind::Update, &work_id, heartbeat_at);
        heartbeat.id = format!("evt-later-heartbeat-{label}");
        heartbeat.status_category = Some(WorkspaceStatusCategory::Active);
        projection.apply_event(heartbeat);
        save_workspace_work_items_projection_to_path(&work_items_path, &projection).unwrap();

        let emitted = match kind {
            WorkEventKind::Done => emit_workspace_done_event_if_absent_paths(
                &work_items_path,
                &close_events_path,
                &work_id,
                retry_at,
            ),
            WorkEventKind::Discard => emit_workspace_discard_event_if_absent_paths(
                &work_items_path,
                &close_events_path,
                &work_id,
                retry_at,
            ),
            _ => unreachable!(),
        }
        .unwrap();

        assert!(
            !emitted,
            "retry must recover the earlier durable {label} before the later heartbeat"
        );
        let projection = load_workspace_work_items_from_path(&work_items_path)
            .unwrap()
            .unwrap();
        let item = projection
            .work_items
            .iter()
            .find(|item| item.id == work_id)
            .unwrap();
        assert!(item.is_terminal());
        assert_eq!(item.discarded, kind == WorkEventKind::Discard);
        assert_eq!(
            std::fs::read_to_string(&close_events_path)
                .unwrap()
                .lines()
                .count(),
            1,
            "retry must not append a replacement {label}"
        );
    }
}

#[test]
fn terminal_retry_rejects_unknown_or_additive_machine_local_event_schema() {
    let cases = [
        (
            "unknown-kind",
            concat!(
                r#"{"id":"evt-local-correction","work_item_id":"work-local-strict","kind":"correction","updated_at":"2026-07-22T02:00:00Z"}"#,
                "\n"
            ),
        ),
        (
            "additive-field",
            concat!(
                r#"{"id":"evt-local-done","work_item_id":"work-local-strict","kind":"done","future_event_field":true,"updated_at":"2026-07-22T02:01:00Z"}"#,
                "\n"
            ),
        ),
    ];

    for (label, body) in cases {
        let temp = tempfile::tempdir().expect("tempdir");
        let work_items_path = temp.path().join("workspace/works.json");
        let shared_events_path = temp.path().join("shared/events.jsonl");
        let close_events_path = temp.path().join("local/work-events-closed.jsonl");
        let started_at = Utc.with_ymd_and_hms(2026, 7, 22, 1, 0, 0).unwrap();
        let retry_at = Utc.with_ymd_and_hms(2026, 7, 22, 3, 0, 0).unwrap();
        let mut start = WorkEvent::new(WorkEventKind::Start, "work-local-strict", started_at);
        start.status_category = Some(WorkspaceStatusCategory::Active);
        record_workspace_work_event_paths(&work_items_path, &shared_events_path, start).unwrap();
        fs::create_dir_all(close_events_path.parent().unwrap()).expect("create local state dir");
        fs::write(&close_events_path, body.as_bytes()).expect("write future lifecycle event");
        let original_projection = fs::read(&work_items_path).expect("snapshot projection");
        let original_close_log = fs::read(&close_events_path).expect("snapshot close log");

        assert!(
            emit_workspace_done_event_if_absent_paths(
                &work_items_path,
                &close_events_path,
                "work-local-strict",
                retry_at,
            )
            .is_err(),
            "machine-local {label} schema must fail closed during terminal recovery"
        );
        assert_eq!(
            fs::read(&work_items_path).expect("read preserved projection"),
            original_projection,
            "failed recovery must not mutate the saved projection"
        );
        assert_eq!(
            fs::read(&close_events_path).expect("read preserved close log"),
            original_close_log,
            "failed recovery must not append a conflicting terminal event"
        );
    }
}

#[test]
fn terminal_retry_repairs_partial_durable_lifecycle_tail_before_strict_replay() {
    use std::io::Write as _;

    for (kind, label) in [
        (WorkEventKind::Done, "done"),
        (WorkEventKind::Discard, "discard"),
    ] {
        let temp = tempfile::tempdir().expect("tempdir");
        let work_items_path = temp.path().join("workspace/works.json");
        let shared_events_path = temp.path().join("shared/events.jsonl");
        let close_events_path = temp.path().join("local/work-events-closed.jsonl");
        let started_at = Utc.with_ymd_and_hms(2026, 7, 16, 7, 0, 0).unwrap();
        let closed_at = Utc.with_ymd_and_hms(2026, 7, 16, 8, 0, 0).unwrap();
        let retry_at = Utc.with_ymd_and_hms(2026, 7, 16, 9, 0, 0).unwrap();
        let work_id = format!("work-partial-durable-{label}");
        let mut start = WorkEvent::new(WorkEventKind::Start, &work_id, started_at);
        start.status_category = Some(WorkspaceStatusCategory::Active);
        record_workspace_work_event_paths(&work_items_path, &shared_events_path, start).unwrap();

        let mut durable = WorkEvent::new(kind, &work_id, closed_at);
        durable.id = format!("evt-partial-durable-{label}");
        if kind == WorkEventKind::Done {
            durable.status_category = Some(WorkspaceStatusCategory::Done);
        }
        append_workspace_work_event_to_path(&close_events_path, &durable).unwrap();
        let mut close_log = std::fs::OpenOptions::new()
            .append(true)
            .open(&close_events_path)
            .unwrap();
        close_log.write_all(br#"{"id":"partial-tail"#).unwrap();
        close_log.sync_all().unwrap();
        drop(close_log);

        let emitted = match kind {
            WorkEventKind::Done => emit_workspace_done_event_if_absent_paths(
                &work_items_path,
                &close_events_path,
                &work_id,
                retry_at,
            ),
            WorkEventKind::Discard => emit_workspace_discard_event_if_absent_paths(
                &work_items_path,
                &close_events_path,
                &work_id,
                retry_at,
            ),
            _ => unreachable!(),
        }
        .expect("retry must repair a partial lifecycle tail before strict replay");

        assert!(!emitted, "retry must reuse the durable {label} event");
        let projection = load_workspace_work_items_from_path(&work_items_path)
            .unwrap()
            .unwrap();
        let item = projection
            .work_items
            .iter()
            .find(|item| item.id == work_id)
            .unwrap();
        assert!(item.is_terminal());
        assert_eq!(item.discarded, kind == WorkEventKind::Discard);

        let repaired = std::fs::read(&close_events_path).unwrap();
        assert_eq!(repaired.last(), Some(&b'\n'));
        let durable_events = repaired
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice::<WorkEvent>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(durable_events.len(), 1, "retry must not duplicate {label}");
        assert_eq!(durable_events[0].id, durable.id);
    }
}

#[test]
fn terminal_emit_waiting_for_lock_does_not_recreate_a_removed_target() {
    use fs2::FileExt;

    let temp = tempfile::tempdir().expect("tempdir");
    let work_items_path = temp.path().join("workspace/works.json");
    let shared_events_path = temp.path().join("shared/events.jsonl");
    let close_events_path = temp.path().join("local/work-events-closed.jsonl");
    let now = Utc.with_ymd_and_hms(2026, 7, 15, 8, 0, 0).unwrap();
    let mut start = WorkEvent::new(WorkEventKind::Start, "work-removed", now);
    start.status_category = Some(WorkspaceStatusCategory::Active);
    record_workspace_work_event_paths(&work_items_path, &shared_events_path, start).unwrap();
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(work_items_path.with_extension("lock"))
        .unwrap();
    lock.lock_exclusive().unwrap();

    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let works = work_items_path.clone();
    let closes = close_events_path.clone();
    let handle = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        emit_workspace_done_event_if_absent_paths(
            &works,
            &closes,
            "work-removed",
            now + chrono::Duration::minutes(1),
        )
    });
    started_rx.recv().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(100));
    save_workspace_work_items_projection_to_path(
        &work_items_path,
        &WorkItemsProjection::empty(now),
    )
    .unwrap();
    FileExt::unlock(&lock).unwrap();

    assert!(!handle.join().unwrap().unwrap());
    let projection = load_workspace_work_items_from_path(&work_items_path)
        .unwrap()
        .unwrap();
    assert!(projection.work_items.is_empty());
    assert!(!close_events_path.exists());
}

#[test]
fn session_terminal_resolution_waits_for_lock_and_uses_latest_assignment() {
    use fs2::FileExt;

    let temp = tempfile::tempdir().expect("tempdir");
    let state_dir = temp.path().join("workspace");
    let current_path = state_dir.join("current.json");
    let work_items_path = state_dir.join("works.json");
    let shared_events_path = temp.path().join("shared/events.jsonl");
    let close_events_path = state_dir.join("work-events-closed.jsonl");
    let started_at = Utc.with_ymd_and_hms(2026, 7, 15, 7, 0, 0).unwrap();
    let closed_at = Utc.with_ymd_and_hms(2026, 7, 15, 8, 0, 0).unwrap();
    let session_id = "session-assignment-race";

    for work_id in ["work-assignment-a", "work-assignment-b"] {
        let mut start = WorkEvent::new(WorkEventKind::Start, work_id, started_at);
        start.status_category = Some(WorkspaceStatusCategory::Active);
        record_workspace_work_event_paths(&work_items_path, &shared_events_path, start).unwrap();
    }

    let mut current = WorkspaceProjection::default_for_project(temp.path());
    current
        .agents
        .push(assigned_agent(session_id, "codex", "work-assignment-a"));
    save_workspace_projection_to_path(&current_path, &current).unwrap();

    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(work_items_path.with_extension("lock"))
        .unwrap();
    lock.lock_exclusive().unwrap();

    let current_for_thread = current_path.clone();
    let works_for_thread = work_items_path.clone();
    let closes_for_thread = close_events_path.clone();
    let handle = std::thread::spawn(move || {
        emit_workspace_done_event_for_session_paths(
            &current_for_thread,
            &works_for_thread,
            &closes_for_thread,
            session_id,
            "work-session-assignment-race",
            closed_at,
        )
    });
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(
        !handle.is_finished(),
        "session resolution must wait for the Work transaction lock"
    );

    current.agents.clear();
    current
        .agents
        .push(assigned_agent(session_id, "codex", "work-assignment-b"));
    let bytes = serde_json::to_vec_pretty(&current).unwrap();
    write_atomic(&current_path, &bytes).unwrap();
    FileExt::unlock(&lock).unwrap();

    assert!(handle.join().unwrap().unwrap());
    let projection = load_workspace_work_items_from_path(&work_items_path)
        .unwrap()
        .unwrap();
    let work_a = projection
        .work_items
        .iter()
        .find(|item| item.id == "work-assignment-a")
        .unwrap();
    let work_b = projection
        .work_items
        .iter()
        .find(|item| item.id == "work-assignment-b")
        .unwrap();
    assert!(!work_a.is_terminal());
    assert!(work_b.is_terminal());
}

#[test]
fn session_terminal_resolution_waits_for_split_current_assignment_lock() {
    use fs2::FileExt;

    let temp = tempfile::tempdir().expect("tempdir");
    let current_path = temp.path().join("project-state/current.json");
    let work_items_path = temp.path().join("work-event-state/works.json");
    let shared_events_path = temp.path().join("shared/events.jsonl");
    let close_events_path = temp
        .path()
        .join("work-event-state/work-events-closed.jsonl");
    let started_at = Utc.with_ymd_and_hms(2026, 7, 15, 7, 0, 0).unwrap();
    let closed_at = Utc.with_ymd_and_hms(2026, 7, 15, 8, 0, 0).unwrap();
    let session_id = "session-split-assignment-race";

    for work_id in ["work-split-assignment-a", "work-split-assignment-b"] {
        let mut start = WorkEvent::new(WorkEventKind::Start, work_id, started_at);
        start.status_category = Some(WorkspaceStatusCategory::Active);
        record_workspace_work_event_paths(&work_items_path, &shared_events_path, start).unwrap();
    }

    let mut current = WorkspaceProjection::default_for_project(temp.path());
    current.agents.push(assigned_agent(
        session_id,
        "codex",
        "work-split-assignment-a",
    ));
    save_workspace_projection_to_path(&current_path, &current).unwrap();

    let current_lock_path = current_path.with_file_name("works.lock");
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(current_lock_path)
        .unwrap();
    lock.lock_exclusive().unwrap();

    let current_for_thread = current_path.clone();
    let works_for_thread = work_items_path.clone();
    let closes_for_thread = close_events_path.clone();
    let handle = std::thread::spawn(move || {
        emit_workspace_done_event_for_session_paths(
            &current_for_thread,
            &works_for_thread,
            &closes_for_thread,
            session_id,
            "work-session-split-assignment-race",
            closed_at,
        )
    });
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(
        !handle.is_finished(),
        "split-root completion must wait for the current assignment writer"
    );

    current.agents.clear();
    current.agents.push(assigned_agent(
        session_id,
        "codex",
        "work-split-assignment-b",
    ));
    let bytes = serde_json::to_vec_pretty(&current).unwrap();
    write_atomic(&current_path, &bytes).unwrap();
    FileExt::unlock(&lock).unwrap();

    assert!(handle.join().unwrap().unwrap());
    let projection = load_workspace_work_items_from_path(&work_items_path)
        .unwrap()
        .unwrap();
    assert!(!projection
        .work_items
        .iter()
        .find(|item| item.id == "work-split-assignment-a")
        .unwrap()
        .is_terminal());
    assert!(projection
        .work_items
        .iter()
        .find(|item| item.id == "work-split-assignment-b")
        .unwrap()
        .is_terminal());
}

#[test]
fn session_terminal_wrappers_migrate_legacy_work_items_before_first_close() {
    let _guard = lock_test_env();
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let _home = ScopedHome::set(&home);
    let started_at = Utc.with_ymd_and_hms(2026, 7, 16, 7, 0, 0).unwrap();
    let closed_at = Utc.with_ymd_and_hms(2026, 7, 16, 8, 0, 0).unwrap();

    for (kind, label) in [
        (WorkEventKind::Done, "done"),
        (WorkEventKind::Discard, "discard"),
    ] {
        let repo = temp.path().join(format!("repo-{label}"));
        std::fs::create_dir_all(&repo).unwrap();
        let canonical = gwt_workspace_work_items_path_for_repo_path(&repo);
        let legacy = legacy_workspace_work_items_path_for_repo_path(&repo);
        let close_events = gwt_workspace_work_events_closed_path_for_repo_path(&repo);
        let work_id = format!("work-legacy-first-{label}");
        let session_id = format!("session-legacy-first-{label}");
        let mut projection = WorkItemsProjection::empty(started_at);
        let mut start = WorkEvent::new(WorkEventKind::Start, &work_id, started_at);
        start.agent_session_id = Some(session_id.clone());
        start.status_category = Some(WorkspaceStatusCategory::Active);
        projection.apply_event(start);
        save_workspace_work_items_projection_to_path(&legacy, &projection).unwrap();
        assert!(!canonical.exists(), "fixture must be legacy-only");

        let emitted = match kind {
            WorkEventKind::Done => emit_workspace_done_event_for_session(
                &repo,
                &repo,
                &session_id,
                &work_id,
                closed_at,
            ),
            WorkEventKind::Discard => emit_workspace_discard_event_for_session(
                &repo,
                &repo,
                &session_id,
                &work_id,
                closed_at,
            ),
            _ => unreachable!(),
        }
        .unwrap();

        assert!(emitted, "first {label} call must close the migrated Work");
        let migrated = load_workspace_work_items_from_path(&canonical)
            .unwrap()
            .expect("legacy Work must migrate to canonical state");
        let item = migrated
            .work_items
            .iter()
            .find(|item| item.id == work_id)
            .expect("migrated Work");
        assert!(item.is_terminal());
        assert_eq!(item.discarded, kind == WorkEventKind::Discard);
        let terminal_events = std::fs::read_to_string(&close_events)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<WorkEvent>(line).unwrap())
            .filter(|event| event.kind == kind && event.work_item_id == work_id)
            .count();
        assert_eq!(terminal_events, 1, "first {label} call emits once");
    }
}

#[test]
fn session_terminal_outcome_classifies_assigned_work_under_the_dual_lock() {
    let temp = tempfile::tempdir().expect("tempdir");
    let current_path = temp.path().join("project-state/current.json");
    let work_items_path = temp.path().join("work-state/works.json");
    let events_path = temp.path().join("work-state/work-events-closed.jsonl");
    let shared_events_path = temp.path().join("shared/events.jsonl");
    let now = Utc.with_ymd_and_hms(2026, 7, 16, 8, 0, 0).unwrap();
    let session_id = "session-terminal-outcome";

    let mut current = WorkspaceProjection::default_for_project(temp.path());
    current
        .agents
        .push(assigned_agent(session_id, "codex", "work-assigned-missing"));
    save_workspace_projection_to_path(&current_path, &current).unwrap();
    assert_eq!(
        emit_workspace_done_event_for_session_outcome_paths(
            &current_path,
            &work_items_path,
            &events_path,
            session_id,
            "work-session-terminal-outcome",
            now,
        )
        .unwrap(),
        WorkspaceTerminalEventOutcome::AssignedWorkMissing("work-assigned-missing".to_string())
    );

    for work_id in ["work-matching", "work-wrong", "work-ambiguous"] {
        let mut start = WorkEvent::new(WorkEventKind::Start, work_id, now);
        start.status_category = Some(WorkspaceStatusCategory::Active);
        record_workspace_work_event_paths(&work_items_path, &shared_events_path, start).unwrap();
    }
    current.agents.clear();
    current
        .agents
        .push(assigned_agent(session_id, "codex", "work-matching"));
    save_workspace_projection_to_path(&current_path, &current).unwrap();
    emit_workspace_done_event_if_absent_paths(&work_items_path, &events_path, "work-matching", now)
        .unwrap();
    assert_eq!(
        emit_workspace_done_event_for_session_outcome_paths(
            &current_path,
            &work_items_path,
            &events_path,
            session_id,
            "legacy",
            now,
        )
        .unwrap(),
        WorkspaceTerminalEventOutcome::AlreadyMatching
    );

    current.agents.clear();
    current
        .agents
        .push(assigned_agent(session_id, "codex", "work-wrong"));
    save_workspace_projection_to_path(&current_path, &current).unwrap();
    emit_workspace_discard_event_if_absent_paths(&work_items_path, &events_path, "work-wrong", now)
        .unwrap();
    assert_eq!(
        emit_workspace_done_event_for_session_outcome_paths(
            &current_path,
            &work_items_path,
            &events_path,
            session_id,
            "legacy",
            now,
        )
        .unwrap(),
        WorkspaceTerminalEventOutcome::WrongTerminal
    );

    let mut works = load_workspace_work_items_from_path(&work_items_path)
        .unwrap()
        .unwrap();
    let ambiguous = works
        .work_items
        .iter_mut()
        .find(|item| item.id == "work-ambiguous")
        .unwrap();
    ambiguous.status_category = WorkspaceStatusCategory::Done;
    ambiguous.discarded = true;
    save_workspace_work_items_projection_to_path(&work_items_path, &works).unwrap();
    current.agents.clear();
    current
        .agents
        .push(assigned_agent(session_id, "codex", "work-ambiguous"));
    save_workspace_projection_to_path(&current_path, &current).unwrap();
    assert_eq!(
        emit_workspace_done_event_for_session_outcome_paths(
            &current_path,
            &work_items_path,
            &events_path,
            session_id,
            "legacy",
            now,
        )
        .unwrap(),
        WorkspaceTerminalEventOutcome::AmbiguousTerminal
    );
}

#[test]
fn concurrent_done_and_discard_append_only_one_terminal_event() {
    use fs2::FileExt;
    use std::sync::{Arc, Barrier};

    let temp = tempfile::tempdir().expect("tempdir");
    let work_items_path = temp.path().join("workspace/works.json");
    let events_path = temp.path().join("workspace/work-events-closed.jsonl");
    let started_at = Utc.with_ymd_and_hms(2026, 7, 15, 7, 0, 0).unwrap();
    let closed_at = Utc.with_ymd_and_hms(2026, 7, 15, 8, 0, 0).unwrap();
    let mut start = WorkEvent::new(WorkEventKind::Start, "work-racing-close", started_at);
    start.status_category = Some(WorkspaceStatusCategory::Active);
    record_workspace_work_event_paths(&work_items_path, &events_path, start).unwrap();

    let lock_path = work_items_path.with_extension("lock");
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)
        .unwrap();
    lock.lock_exclusive().unwrap();

    let barrier = Arc::new(Barrier::new(3));
    let works = Arc::new(work_items_path.clone());
    let events = Arc::new(events_path.clone());
    let (done_result, discard_result) = std::thread::scope(|scope| {
        let done_barrier = Arc::clone(&barrier);
        let done_works = Arc::clone(&works);
        let done_events = Arc::clone(&events);
        let done = scope.spawn(move || {
            done_barrier.wait();
            emit_workspace_done_event_if_absent_paths(
                &done_works,
                &done_events,
                "work-racing-close",
                closed_at,
            )
            .unwrap()
        });
        let discard_barrier = Arc::clone(&barrier);
        let discard_works = Arc::clone(&works);
        let discard_events = Arc::clone(&events);
        let discard = scope.spawn(move || {
            discard_barrier.wait();
            emit_workspace_discard_event_if_absent_paths(
                &discard_works,
                &discard_events,
                "work-racing-close",
                closed_at,
            )
            .unwrap()
        });
        barrier.wait();
        std::thread::sleep(std::time::Duration::from_millis(100));
        FileExt::unlock(&lock).unwrap();
        (done.join().unwrap(), discard.join().unwrap())
    });

    assert_eq!(usize::from(done_result) + usize::from(discard_result), 1);
    let terminal_lines = std::fs::read_to_string(&events_path)
        .unwrap()
        .lines()
        .filter(|line| line.contains("\"kind\":\"done\"") || line.contains("\"kind\":\"discard\""))
        .count();
    assert_eq!(terminal_lines, 1);
}

#[test]
fn work_event_batch_appends_and_projects_pause_with_all_board_refs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let work_items_path = temp.path().join("workspace/works.json");
    let events_path = temp.path().join("workspace/work-events-closed.jsonl");
    let now = Utc.with_ymd_and_hms(2026, 7, 15, 8, 0, 0).unwrap();
    let mut pause = WorkEvent::new(WorkEventKind::Pause, "work-batch", now);
    pause.id = "evt-pause-batch".to_string();
    let mut first_ref = WorkEvent::new(WorkEventKind::Update, "work-batch", now);
    first_ref.id = "evt-board-first".to_string();
    first_ref.board_entry_id = Some("board-first".to_string());
    let mut second_ref = WorkEvent::new(WorkEventKind::Update, "work-batch", now);
    second_ref.id = "evt-board-second".to_string();
    second_ref.board_entry_id = Some("board-second".to_string());

    record_workspace_work_events_paths(
        &work_items_path,
        &events_path,
        vec![first_ref, second_ref, pause],
    )
    .unwrap();

    let lines = std::fs::read_to_string(&events_path)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect::<Vec<_>>();
    assert_eq!(lines.len(), 3);
    assert!(lines[0].contains("evt-board-first"));
    assert!(lines[1].contains("evt-board-second"));
    assert!(lines[2].contains("evt-pause-batch"));
    let projection = load_workspace_work_items_from_path(&work_items_path)
        .unwrap()
        .unwrap();
    let item = projection
        .work_items
        .iter()
        .find(|item| item.id == "work-batch")
        .unwrap();
    assert_eq!(item.status_category, WorkspaceStatusCategory::Idle);
    assert_eq!(item.board_refs, vec!["board-first", "board-second"]);
}

#[test]
fn rejected_pause_rejects_the_entire_board_reference_batch() {
    let temp = tempfile::tempdir().expect("tempdir");
    let work_items_path = temp.path().join("workspace/works.json");
    let shared_events_path = temp.path().join("shared/events.jsonl");
    let close_events_path = temp.path().join("local/work-events-closed.jsonl");
    let t0 = Utc.with_ymd_and_hms(2026, 7, 15, 7, 0, 0).unwrap();
    let t1 = Utc.with_ymd_and_hms(2026, 7, 15, 8, 0, 0).unwrap();

    let mut owner = WorkEvent::new(WorkEventKind::Start, "work-owner", t0);
    owner.agent_session_id = Some("session-owner".to_string());
    owner.execution_container = Some(WorkspaceExecutionContainerRef {
        branch: Some("feature/owner".to_string()),
        worktree_path: Some("/repo/feature/owner".into()),
        pr_number: None,
        pr_url: None,
        pr_state: None,
    });
    record_workspace_work_event_paths(&work_items_path, &shared_events_path, owner).unwrap();
    let mut target = WorkEvent::new(WorkEventKind::Start, "work-target", t0);
    target.execution_container = Some(WorkspaceExecutionContainerRef {
        branch: Some("feature/target".to_string()),
        worktree_path: Some("/repo/feature/target".into()),
        pr_number: None,
        pr_url: None,
        pr_state: None,
    });
    record_workspace_work_event_paths(&work_items_path, &shared_events_path, target).unwrap();
    let before = std::fs::read(&work_items_path).unwrap();

    record_workspace_work_paused_event_paths(
        &work_items_path,
        &close_events_path,
        "work-target",
        Some("Rejected pause"),
        None,
        None,
        &["board-orphan".to_string()],
        Some(WorkspaceExecutionContainerRef {
            branch: Some("feature/target".to_string()),
            worktree_path: Some("/repo/feature/target".into()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        }),
        Some("session-owner"),
        t1,
    )
    .unwrap();

    assert_eq!(std::fs::read(&work_items_path).unwrap(), before);
    assert!(
        !close_events_path.exists(),
        "a rejected Pause must not leave orphan Board refs"
    );
}

#[test]
fn auto_done_emit_helper_appends_once_after_explicit_reopen() {
    let temp = tempfile::tempdir().expect("tempdir");
    let work_items_path = temp.path().join("workspace/work_items.json");
    let events_path = temp.path().join("workspace/work_events.jsonl");
    let started_at = Utc.with_ymd_and_hms(2026, 7, 15, 1, 0, 0).unwrap();
    let first_done_at = Utc.with_ymd_and_hms(2026, 7, 15, 2, 0, 0).unwrap();
    let reopened_at = Utc.with_ymd_and_hms(2026, 7, 15, 3, 0, 0).unwrap();
    let second_done_at = Utc.with_ymd_and_hms(2026, 7, 15, 4, 0, 0).unwrap();

    let mut start = WorkEvent::new(WorkEventKind::Start, "wi-reopened", started_at);
    start.status_category = Some(WorkspaceStatusCategory::Active);
    record_workspace_work_event_paths(&work_items_path, &events_path, start).unwrap();
    assert!(emit_workspace_done_event_if_absent_paths(
        &work_items_path,
        &events_path,
        "wi-reopened",
        first_done_at,
    )
    .unwrap());

    let mut reopen = WorkEvent::new(WorkEventKind::Resume, "wi-reopened", reopened_at);
    reopen.status_category = Some(WorkspaceStatusCategory::Active);
    record_workspace_work_event_paths(&work_items_path, &events_path, reopen).unwrap();

    assert!(emit_workspace_done_event_if_absent_paths(
        &work_items_path,
        &events_path,
        "wi-reopened",
        second_done_at,
    )
    .unwrap());
    assert!(!emit_workspace_done_event_if_absent_paths(
        &work_items_path,
        &events_path,
        "wi-reopened",
        second_done_at + chrono::Duration::minutes(1),
    )
    .unwrap());

    let projection = load_workspace_work_items_from_path(&work_items_path)
        .unwrap()
        .unwrap();
    let item = projection
        .work_items
        .iter()
        .find(|item| item.id == "wi-reopened")
        .unwrap();
    assert_eq!(item.status_category, WorkspaceStatusCategory::Done);
    assert_eq!(item.completed_at, Some(second_done_at));
    assert_eq!(
        item.events
            .iter()
            .filter(|event| event.kind == WorkEventKind::Done)
            .count(),
        2
    );
}

#[test]
fn retroactive_auto_done_scan_marks_eligible_merged_work_branch_workitems() {
    let temp = tempfile::tempdir().expect("tempdir");
    let current_path = temp.path().join("workspace/current.json");
    let work_items_path = temp.path().join("workspace/work_items.json");
    let events_path = temp.path().join("workspace/work_events.jsonl");
    let started_at = Utc.with_ymd_and_hms(2026, 5, 13, 1, 0, 0).unwrap();
    let now = Utc.with_ymd_and_hms(2026, 5, 13, 9, 0, 0).unwrap();

    let mut eligible = WorkEvent::new(WorkEventKind::Start, "wi-eligible", started_at);
    eligible.status_category = Some(WorkspaceStatusCategory::Active);
    eligible.execution_container = Some(WorkspaceExecutionContainerRef {
        branch: Some("work/20260513-0100".to_string()),
        worktree_path: None,
        pr_number: Some(1),
        pr_url: None,
        pr_state: Some("merged".to_string()),
    });
    record_workspace_work_event_paths(&work_items_path, &events_path, eligible)
        .expect("record eligible start");

    let mut non_work = WorkEvent::new(WorkEventKind::Start, "wi-non-work-branch", started_at);
    non_work.status_category = Some(WorkspaceStatusCategory::Active);
    non_work.execution_container = Some(WorkspaceExecutionContainerRef {
        branch: Some("feature/manual".to_string()),
        worktree_path: None,
        pr_number: Some(2),
        pr_url: None,
        pr_state: Some("merged".to_string()),
    });
    record_workspace_work_event_paths(&work_items_path, &events_path, non_work)
        .expect("record non-work start");

    let mut not_merged = WorkEvent::new(WorkEventKind::Start, "wi-not-merged", started_at);
    not_merged.status_category = Some(WorkspaceStatusCategory::Active);
    not_merged.execution_container = Some(WorkspaceExecutionContainerRef {
        branch: Some("work/20260513-0200".to_string()),
        worktree_path: None,
        pr_number: Some(3),
        pr_url: None,
        pr_state: Some("open".to_string()),
    });
    record_workspace_work_event_paths(&work_items_path, &events_path, not_merged)
        .expect("record not-merged start");

    let count =
        retroactive_auto_done_scan_paths(&current_path, &work_items_path, &events_path, now)
            .expect("retroactive scan");
    assert_eq!(count, 1, "only the eligible WorkItem must be auto-Done'd");

    let projection = load_workspace_work_items_from_path(&work_items_path)
        .expect("load work items")
        .expect("work items");
    let eligible_item = projection
        .work_items
        .iter()
        .find(|item| item.id == "wi-eligible")
        .expect("eligible item");
    assert_eq!(eligible_item.status_category, WorkspaceStatusCategory::Done);
    assert_eq!(eligible_item.completed_at, Some(now));

    let non_work_item = projection
        .work_items
        .iter()
        .find(|item| item.id == "wi-non-work-branch")
        .expect("non-work item");
    assert_eq!(
        non_work_item.status_category,
        WorkspaceStatusCategory::Active,
        "non-work/ branch must not be auto-Done'd",
    );

    let not_merged_item = projection
        .work_items
        .iter()
        .find(|item| item.id == "wi-not-merged")
        .expect("not-merged item");
    assert_eq!(
        not_merged_item.status_category,
        WorkspaceStatusCategory::Active,
        "WorkItem without merged PR must not be auto-Done'd",
    );
}

#[test]
fn retroactive_auto_done_scan_is_idempotent_across_invocations() {
    let temp = tempfile::tempdir().expect("tempdir");
    let current_path = temp.path().join("workspace/current.json");
    let work_items_path = temp.path().join("workspace/work_items.json");
    let events_path = temp.path().join("workspace/work_events.jsonl");
    let started_at = Utc.with_ymd_and_hms(2026, 5, 13, 1, 0, 0).unwrap();
    let first_run = Utc.with_ymd_and_hms(2026, 5, 13, 9, 0, 0).unwrap();
    let second_run = Utc.with_ymd_and_hms(2026, 5, 13, 10, 0, 0).unwrap();

    let mut eligible = WorkEvent::new(WorkEventKind::Start, "wi-twice", started_at);
    eligible.status_category = Some(WorkspaceStatusCategory::Active);
    eligible.execution_container = Some(WorkspaceExecutionContainerRef {
        branch: Some("work/20260513-0100".to_string()),
        worktree_path: None,
        pr_number: Some(7),
        pr_url: None,
        pr_state: Some("merged".to_string()),
    });
    record_workspace_work_event_paths(&work_items_path, &events_path, eligible)
        .expect("record start");

    let first =
        retroactive_auto_done_scan_paths(&current_path, &work_items_path, &events_path, first_run)
            .expect("first scan");
    let second =
        retroactive_auto_done_scan_paths(&current_path, &work_items_path, &events_path, second_run)
            .expect("second scan");

    assert_eq!(first, 1, "first scan must emit Done");
    assert_eq!(second, 0, "second scan must be noop");

    let events_text = std::fs::read_to_string(&events_path).expect("read events");
    let done_lines = events_text
        .lines()
        .filter(|line| line.contains("\"kind\":\"done\"") && line.contains("wi-twice"))
        .count();
    assert_eq!(done_lines, 1);
}

// SPEC-2359 US-37 / T-241: cleanup hook auto-done by branch match.

#[test]
fn emit_workspace_done_event_for_branch_emits_done_when_branch_matches() {
    let temp = tempfile::tempdir().expect("tempdir");
    let current_path = temp.path().join("workspace/current.json");
    let work_items_path = temp.path().join("workspace/work_items.json");
    let events_path = temp.path().join("workspace/work_events.jsonl");
    let now = Utc.with_ymd_and_hms(2026, 5, 13, 12, 0, 0).unwrap();
    let project_root = temp.path().join("repo");
    std::fs::create_dir_all(&project_root).expect("create repo");

    let mut projection = WorkspaceProjection::default_for_project(&project_root);
    projection.id = "wi-cleanup-target".to_string();
    projection.git_details = Some(GitDetails {
        branch: Some("work/auto-done-branch".to_string()),
        worktree_path: None,
        base_branch: Some("origin/develop".to_string()),
        pr_number: None,
        pr_state: None,
        pr_url: None,
        pr_created_at: None,
        created_by_start_work: true,
        created_at: now,
    });
    save_workspace_projection_to_path(&current_path, &projection).expect("save projection");

    let mut start = WorkEvent::new(
        WorkEventKind::Start,
        "wi-cleanup-target",
        Utc.with_ymd_and_hms(2026, 5, 13, 1, 0, 0).unwrap(),
    );
    start.status_category = Some(WorkspaceStatusCategory::Active);
    record_workspace_work_event_paths(&work_items_path, &events_path, start).expect("seed start");

    let emitted = emit_workspace_done_event_for_branch_paths(
        &current_path,
        &work_items_path,
        &events_path,
        "work/auto-done-branch",
        now,
    )
    .expect("emit");
    assert!(emitted, "branch match must trigger Done emit");

    let work_items = load_workspace_work_items_from_path(&work_items_path)
        .expect("load")
        .expect("work items");
    let item = work_items
        .work_items
        .iter()
        .find(|item| item.id == "wi-cleanup-target")
        .expect("item");
    assert_eq!(item.status_category, WorkspaceStatusCategory::Done);
    assert_eq!(item.completed_at, Some(now));
}

#[test]
fn emit_workspace_done_event_for_branch_is_noop_when_branch_does_not_match() {
    let temp = tempfile::tempdir().expect("tempdir");
    let current_path = temp.path().join("workspace/current.json");
    let work_items_path = temp.path().join("workspace/work_items.json");
    let events_path = temp.path().join("workspace/work_events.jsonl");
    let now = Utc.with_ymd_and_hms(2026, 5, 13, 12, 0, 0).unwrap();
    let project_root = temp.path().join("repo");
    std::fs::create_dir_all(&project_root).expect("create repo");

    let mut projection = WorkspaceProjection::default_for_project(&project_root);
    projection.id = "wi-different-branch".to_string();
    projection.git_details = Some(GitDetails {
        branch: Some("work/current-branch".to_string()),
        worktree_path: None,
        base_branch: None,
        pr_number: None,
        pr_state: None,
        pr_url: None,
        pr_created_at: None,
        created_by_start_work: true,
        created_at: now,
    });
    save_workspace_projection_to_path(&current_path, &projection).expect("save projection");

    let mut start = WorkEvent::new(
        WorkEventKind::Start,
        "wi-different-branch",
        Utc.with_ymd_and_hms(2026, 5, 13, 1, 0, 0).unwrap(),
    );
    start.status_category = Some(WorkspaceStatusCategory::Active);
    record_workspace_work_event_paths(&work_items_path, &events_path, start).expect("seed start");

    let emitted = emit_workspace_done_event_for_branch_paths(
        &current_path,
        &work_items_path,
        &events_path,
        "work/different-branch",
        now,
    )
    .expect("emit");
    assert!(!emitted, "non-matching branch must not trigger Done");

    let work_items = load_workspace_work_items_from_path(&work_items_path)
        .expect("load")
        .expect("work items");
    let item = work_items
        .work_items
        .iter()
        .find(|item| item.id == "wi-different-branch")
        .expect("item");
    assert_eq!(item.status_category, WorkspaceStatusCategory::Active);
}

// SPEC-2359 US-37 / T-242: retroactive migration startup robustness.

#[test]
fn retroactive_auto_done_scan_returns_zero_when_work_items_file_missing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let current_path = temp.path().join("workspace/current.json");
    let work_items_path = temp.path().join("workspace/work_items.json");
    let events_path = temp.path().join("workspace/work_events.jsonl");
    let now = Utc.with_ymd_and_hms(2026, 5, 13, 12, 0, 0).unwrap();

    assert!(!work_items_path.exists());
    assert!(!current_path.exists());
    let count =
        retroactive_auto_done_scan_paths(&current_path, &work_items_path, &events_path, now)
            .expect("scan with missing files must not error");
    assert_eq!(count, 0);
    assert!(
        !events_path.exists(),
        "missing inputs must skip without writing events"
    );
}

// SPEC-2359 US-37 / FR-119 current.json fallback (upgrade path).

#[test]
fn retroactive_auto_done_scan_emits_done_from_current_projection_when_work_items_missing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let current_path = temp.path().join("workspace/current.json");
    let work_items_path = temp.path().join("workspace/work_items.json");
    let events_path = temp.path().join("workspace/work_events.jsonl");
    let now = Utc.with_ymd_and_hms(2026, 5, 13, 12, 0, 0).unwrap();
    let project_root = temp.path().join("repo");
    std::fs::create_dir_all(&project_root).expect("create repo");

    let mut projection = WorkspaceProjection::default_for_project(&project_root);
    projection.id = "wi-current-merged".to_string();
    projection.git_details = Some(GitDetails {
        branch: Some("work/20260513-0100".to_string()),
        worktree_path: None,
        base_branch: Some("origin/develop".to_string()),
        pr_number: Some(42),
        pr_state: Some("MERGED".to_string()),
        pr_url: None,
        pr_created_at: None,
        created_by_start_work: true,
        created_at: now,
    });
    save_workspace_projection_to_path(&current_path, &projection).expect("save projection");

    let count =
        retroactive_auto_done_scan_paths(&current_path, &work_items_path, &events_path, now)
            .expect("scan");
    assert_eq!(
        count, 1,
        "current.json with merged work/* + start_work must trigger one Done emit",
    );

    let work_items = load_workspace_work_items_from_path(&work_items_path)
        .expect("load work items")
        .expect("work items projection created via emit");
    let item = work_items
        .work_items
        .iter()
        .find(|item| item.id == "wi-current-merged")
        .expect("WorkItem created from emit");
    assert_eq!(item.status_category, WorkspaceStatusCategory::Done);
    assert_eq!(item.completed_at, Some(now));

    let second =
        retroactive_auto_done_scan_paths(&current_path, &work_items_path, &events_path, now)
            .expect("second scan");
    assert_eq!(
        second, 0,
        "second scan must be noop after Done event exists"
    );
}

#[test]
fn retroactive_auto_done_scan_skips_current_projection_without_start_work_flag() {
    let temp = tempfile::tempdir().expect("tempdir");
    let current_path = temp.path().join("workspace/current.json");
    let work_items_path = temp.path().join("workspace/work_items.json");
    let events_path = temp.path().join("workspace/work_events.jsonl");
    let now = Utc.with_ymd_and_hms(2026, 5, 13, 12, 0, 0).unwrap();
    let project_root = temp.path().join("repo");
    std::fs::create_dir_all(&project_root).expect("create repo");

    let mut projection = WorkspaceProjection::default_for_project(&project_root);
    projection.id = "wi-manual-branch".to_string();
    projection.git_details = Some(GitDetails {
        branch: Some("work/20260513-0200".to_string()),
        worktree_path: None,
        base_branch: None,
        pr_number: Some(43),
        pr_state: Some("merged".to_string()),
        pr_url: None,
        pr_created_at: None,
        created_by_start_work: false,
        created_at: now,
    });
    save_workspace_projection_to_path(&current_path, &projection).expect("save projection");

    let count =
        retroactive_auto_done_scan_paths(&current_path, &work_items_path, &events_path, now)
            .expect("scan");
    assert_eq!(
        count, 0,
        "non-start_work workspaces must be excluded from current.json fallback",
    );
    assert!(
        !events_path.exists(),
        "no work_events should be written for ineligible current projection",
    );
}

fn make_stale_projection(updated_at: DateTime<Utc>) -> WorkspaceProjection {
    let mut projection = WorkspaceProjection::default_for_project("/repo");
    projection.updated_at = updated_at;
    projection
}

#[test]
fn stale_reason_returns_none_for_fresh_active_workspace() {
    let now = Utc::now();
    let projection = make_stale_projection(now);
    let config = WorkspaceRetentionConfig::default();
    assert_eq!(
        workspace_projection_stale_reason(&projection, &config, now),
        None,
    );
}

#[test]
fn stale_reason_detects_missing_worktree() {
    let now = Utc::now();
    let mut projection = make_stale_projection(now);
    projection.git_details = Some(GitDetails {
        branch: Some("work/test".to_string()),
        worktree_path: Some(PathBuf::from(
            "/nonexistent/path/__stale_reason_should_not_exist_xyz__",
        )),
        base_branch: None,
        pr_number: None,
        pr_state: None,
        pr_url: None,
        pr_created_at: None,
        created_by_start_work: true,
        created_at: now,
    });
    let config = WorkspaceRetentionConfig::default();
    assert_eq!(
        workspace_projection_stale_reason(&projection, &config, now),
        Some(StaleReason::WorktreeMissing),
    );
}

#[test]
fn stale_reason_detects_pr_merged_via_git_details() {
    let now = Utc::now();
    let mut projection = make_stale_projection(now);
    projection.git_details = Some(GitDetails {
        branch: Some("work/test".to_string()),
        worktree_path: None,
        base_branch: None,
        pr_number: Some(123),
        pr_state: Some("merged".to_string()),
        pr_url: None,
        pr_created_at: None,
        created_by_start_work: true,
        created_at: now,
    });
    let config = WorkspaceRetentionConfig::default();
    assert_eq!(
        workspace_projection_stale_reason(&projection, &config, now),
        Some(StaleReason::PrClosed),
    );
}

#[test]
fn stale_reason_detects_pr_closed_via_linked_prs() {
    let now = Utc::now();
    let mut projection = make_stale_projection(now);
    projection.linked_prs.push(WorkspacePrLink {
        number: 456,
        title: None,
        url: None,
        state: Some("Closed".to_string()),
    });
    let config = WorkspaceRetentionConfig::default();
    assert_eq!(
        workspace_projection_stale_reason(&projection, &config, now),
        Some(StaleReason::PrClosed),
    );
}

#[test]
fn stale_reason_detects_time_threshold() {
    let now = Utc::now();
    let projection = make_stale_projection(now - chrono::Duration::days(40));
    let config = WorkspaceRetentionConfig::default();
    assert_eq!(
        workspace_projection_stale_reason(&projection, &config, now),
        Some(StaleReason::TimeThreshold),
    );
}

#[test]
fn stale_reason_returns_compound_when_multiple_conditions_hold() {
    let now = Utc::now();
    let mut projection = make_stale_projection(now - chrono::Duration::days(40));
    projection.git_details = Some(GitDetails {
        branch: Some("work/test".to_string()),
        worktree_path: None,
        base_branch: None,
        pr_number: Some(789),
        pr_state: Some("merged".to_string()),
        pr_url: None,
        pr_created_at: None,
        created_by_start_work: true,
        created_at: now,
    });
    let config = WorkspaceRetentionConfig::default();
    assert_eq!(
        workspace_projection_stale_reason(&projection, &config, now),
        Some(StaleReason::Compound),
    );
}

#[test]
fn workspace_retention_config_default_uses_30_60_days() {
    let config = WorkspaceRetentionConfig::default();
    assert_eq!(config.archive_after_days, 30);
    assert_eq!(config.delete_after_archive_days, 60);
}

#[test]
fn stale_reason_as_str_matches_snake_case_serde() {
    assert_eq!(StaleReason::WorktreeMissing.as_str(), "worktree_missing");
    assert_eq!(StaleReason::PrClosed.as_str(), "pr_closed");
    assert_eq!(StaleReason::TimeThreshold.as_str(), "time_threshold");
    assert_eq!(StaleReason::Compound.as_str(), "compound");
}

fn write_projection_at(workspace_dir: &Path, projection: &WorkspaceProjection) {
    std::fs::create_dir_all(workspace_dir).expect("create workspace dir");
    let current = workspace_dir.join("current.json");
    save_workspace_projection_to_path(&current, projection).expect("save projection");
}

fn make_classify_projection(
    id: &str,
    project_root: &Path,
    updated_at: DateTime<Utc>,
    lifecycle: WorkspaceLifecycleStage,
) -> WorkspaceProjection {
    let mut projection = WorkspaceProjection::default_for_project(project_root);
    projection.id = id.to_string();
    projection.updated_at = updated_at;
    projection.lifecycle_stage = lifecycle;
    projection
}

#[test]
fn classify_workspace_projections_returns_empty_for_missing_scan_root() {
    let scan_root = PathBuf::from("/nonexistent/projects/scan-root-xyz");
    let now = Utc::now();
    let result = classify_workspace_projections(
        &scan_root,
        &WorkspaceRetentionConfig::default(),
        now,
        |_| false,
    );
    assert!(result.is_empty());
}

#[test]
fn classify_workspace_projections_classifies_stale_active_as_archive() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let scan_root = tmp.path().to_path_buf();
    let project_dir = scan_root.join("abc123");
    let workspace_dir = project_dir.join("workspace");
    let now = Utc::now();
    let projection = make_classify_projection(
        "ws-archive-me",
        &project_dir,
        now - chrono::Duration::days(40),
        WorkspaceLifecycleStage::Active,
    );
    write_projection_at(&workspace_dir, &projection);

    let result = classify_workspace_projections(
        &scan_root,
        &WorkspaceRetentionConfig::default(),
        now,
        |_| false,
    );
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].workspace_id, "ws-archive-me");
    assert_eq!(result[0].action, PruneAction::Archive);
    assert_eq!(result[0].stale_reason, Some(StaleReason::TimeThreshold));
}

#[test]
fn classify_workspace_projections_classifies_archived_beyond_threshold_as_delete() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let scan_root = tmp.path().to_path_buf();
    let project_dir = scan_root.join("def456");
    let workspace_dir = project_dir.join("workspace");
    let now = Utc::now();
    let projection = make_classify_projection(
        "ws-delete-me",
        &project_dir,
        now - chrono::Duration::days(90),
        WorkspaceLifecycleStage::Archived,
    );
    write_projection_at(&workspace_dir, &projection);

    let result = classify_workspace_projections(
        &scan_root,
        &WorkspaceRetentionConfig::default(),
        now,
        |_| false,
    );
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].action, PruneAction::Delete);
}

#[test]
fn classify_workspace_projections_skips_archived_too_soon() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let scan_root = tmp.path().to_path_buf();
    let project_dir = scan_root.join("ghi789");
    let workspace_dir = project_dir.join("workspace");
    let now = Utc::now();
    let projection = make_classify_projection(
        "ws-keep-archived",
        &project_dir,
        now - chrono::Duration::days(10),
        WorkspaceLifecycleStage::Archived,
    );
    write_projection_at(&workspace_dir, &projection);

    let result = classify_workspace_projections(
        &scan_root,
        &WorkspaceRetentionConfig::default(),
        now,
        |_| false,
    );
    assert_eq!(result.len(), 1);
    assert_eq!(
        result[0].action,
        PruneAction::Skip {
            reason: PruneSkipReason::ArchivedTooSoon,
        }
    );
}

#[test]
fn classify_workspace_projections_skips_active_session() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let scan_root = tmp.path().to_path_buf();
    let project_dir = scan_root.join("jkl012");
    let workspace_dir = project_dir.join("workspace");
    let now = Utc::now();
    let projection = make_classify_projection(
        "ws-active",
        &project_dir,
        now - chrono::Duration::days(40),
        WorkspaceLifecycleStage::Active,
    );
    write_projection_at(&workspace_dir, &projection);

    let result = classify_workspace_projections(
        &scan_root,
        &WorkspaceRetentionConfig::default(),
        now,
        |_| true, // every workspace has an active session
    );
    assert_eq!(result.len(), 1);
    assert_eq!(
        result[0].action,
        PruneAction::Skip {
            reason: PruneSkipReason::ActiveAgent,
        }
    );
}

#[test]
fn apply_prune_plan_dry_run_counts_without_filesystem_change() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let scan_root = tmp.path().to_path_buf();
    let project_dir = scan_root.join("dry-run-test");
    let workspace_dir = project_dir.join("workspace");
    let now = Utc::now();
    let projection = make_classify_projection(
        "ws-dry",
        &project_dir,
        now - chrono::Duration::days(40),
        WorkspaceLifecycleStage::Active,
    );
    write_projection_at(&workspace_dir, &projection);

    let plan = classify_workspace_projections(
        &scan_root,
        &WorkspaceRetentionConfig::default(),
        now,
        |_| false,
    );
    let summary = apply_prune_plan(&plan, true).expect("dry run summary");
    assert_eq!(summary.archived, 1);
    assert_eq!(summary.deleted, 0);
    assert_eq!(summary.skipped, 0);

    let loaded = load_workspace_projection_from_path(&workspace_dir.join("current.json"))
        .expect("load")
        .expect("present");
    assert_eq!(
        loaded.lifecycle_stage,
        WorkspaceLifecycleStage::Active,
        "dry-run must not mutate lifecycle_stage",
    );
}

#[test]
fn apply_prune_plan_archives_then_deletes() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let scan_root = tmp.path().to_path_buf();

    let now = Utc::now();
    let archive_dir = scan_root.join("archive-target").join("workspace");
    let archive_projection = make_classify_projection(
        "ws-arch",
        &scan_root.join("archive-target"),
        now - chrono::Duration::days(40),
        WorkspaceLifecycleStage::Active,
    );
    write_projection_at(&archive_dir, &archive_projection);

    let delete_dir = scan_root.join("delete-target").join("workspace");
    let delete_projection = make_classify_projection(
        "ws-del",
        &scan_root.join("delete-target"),
        now - chrono::Duration::days(90),
        WorkspaceLifecycleStage::Archived,
    );
    write_projection_at(&delete_dir, &delete_projection);

    let plan = classify_workspace_projections(
        &scan_root,
        &WorkspaceRetentionConfig::default(),
        now,
        |_| false,
    );
    let summary = apply_prune_plan(&plan, false).expect("apply prune");
    assert_eq!(summary.archived, 1);
    assert_eq!(summary.deleted, 1);

    let loaded = load_workspace_projection_from_path(&archive_dir.join("current.json"))
        .expect("load")
        .expect("present");
    assert_eq!(loaded.lifecycle_stage, WorkspaceLifecycleStage::Archived);

    assert!(
        !delete_dir.exists(),
        "delete target workspace dir should have been removed",
    );
}

#[test]
fn rebuild_work_items_from_events_recovers_done_after_subsequent_update() {
    // SPEC-2359 US-37: Existing work_items.json files written with the
    // legacy apply_event semantics may show status=active even though
    // work_events.jsonl contains a Done event. Replaying events through
    // the fixed apply_event must restore the Done terminal state.
    let temp = tempfile::tempdir().expect("tempdir");
    let events_path = temp.path().join("work_events.jsonl");
    let work_items_path = temp.path().join("work_items.json");
    let marker_path = temp.path().join("work_items.migration.json");

    let work_item_id = "wi-recovered";
    let t1 = Utc.with_ymd_and_hms(2026, 5, 10, 10, 0, 0).unwrap();
    let t2 = Utc.with_ymd_and_hms(2026, 5, 10, 11, 0, 0).unwrap();
    let mut done_event = WorkEvent::new(WorkEventKind::Done, work_item_id, t1);
    done_event.status_category = Some(WorkspaceStatusCategory::Done);
    done_event.title = Some("Recovered work".to_string());
    append_workspace_work_event_to_path(&events_path, &done_event).expect("append done");
    let update_event = WorkEvent::new(WorkEventKind::Update, work_item_id, t2);
    append_workspace_work_event_to_path(&events_path, &update_event).expect("append update");

    let outcome =
        rebuild_work_items_from_events_paths(&work_items_path, &events_path, &marker_path)
            .expect("rebuild");
    assert_eq!(outcome, WorkItemsRebuildOutcome::Applied);

    let projection = load_workspace_work_items_from_path(&work_items_path)
        .expect("load")
        .expect("present");
    let item = projection
        .work_items
        .iter()
        .find(|it| it.id == work_item_id)
        .expect("recovered item exists");
    assert_eq!(item.status_category, WorkspaceStatusCategory::Done);
    assert_eq!(item.completed_at, Some(t1));

    // Re-running is idempotent (marker prevents rebuild).
    let outcome_again =
        rebuild_work_items_from_events_paths(&work_items_path, &events_path, &marker_path)
            .expect("rebuild idempotent");
    assert_eq!(outcome_again, WorkItemsRebuildOutcome::AlreadyMigrated);
}

#[test]
fn emit_workspace_discard_event_if_absent_is_idempotent_for_terminal_work() {
    // SPEC-2359 Phase W-12 Slice 4 (FR-352): a re-close of an already
    // discarded (or already Done) Work is a noop.
    let temp = tempfile::tempdir().expect("tempdir");
    let work_items_path = temp.path().join("work_items.json");
    let events_path = temp.path().join("work_events.jsonl");
    let work_item_id = "wi-discard-idem";
    let t1 = Utc.with_ymd_and_hms(2026, 6, 4, 10, 0, 0).unwrap();
    let t2 = Utc.with_ymd_and_hms(2026, 6, 4, 11, 0, 0).unwrap();

    let mut start = WorkEvent::new(WorkEventKind::Start, work_item_id, t1);
    start.status_category = Some(WorkspaceStatusCategory::Active);
    record_workspace_work_event_paths(&work_items_path, &events_path, start).expect("record start");

    assert!(
        emit_workspace_discard_event_if_absent_paths(
            &work_items_path,
            &events_path,
            work_item_id,
            t2
        )
        .expect("first discard"),
        "first discard appends a new event"
    );
    assert!(
        !emit_workspace_discard_event_if_absent_paths(
            &work_items_path,
            &events_path,
            work_item_id,
            t2
        )
        .expect("second discard"),
        "re-discarding a terminal Work is a noop"
    );

    let projection = load_workspace_work_items_from_path(&work_items_path)
        .expect("load")
        .expect("present");
    let item = projection
        .work_items
        .iter()
        .find(|it| it.id == work_item_id)
        .expect("item exists");
    assert!(item.discarded);
    let discard_events = item
        .events
        .iter()
        .filter(|e| e.kind == WorkEventKind::Discard)
        .count();
    assert_eq!(discard_events, 1, "only one Discard event is recorded");
}

#[test]
fn record_workspace_work_event_does_not_persist_rejected_session_conflict() {
    let temp = tempfile::tempdir().expect("tempdir");
    let work_items_path = temp.path().join("work_items.json");
    let events_path = temp.path().join("work_events.jsonl");
    let t0 = Utc.with_ymd_and_hms(2026, 7, 15, 7, 0, 0).unwrap();
    let t1 = Utc.with_ymd_and_hms(2026, 7, 15, 8, 0, 0).unwrap();

    let mut owner = WorkEvent::new(WorkEventKind::Start, "work-owner", t0);
    owner.agent_session_id = Some("session-owner".to_string());
    owner.execution_container = Some(WorkspaceExecutionContainerRef {
        branch: Some("work/issue-3272".to_string()),
        worktree_path: Some("/repo/work/issue-3272".into()),
        pr_number: None,
        pr_url: None,
        pr_state: None,
    });
    record_workspace_work_event_paths(&work_items_path, &events_path, owner).expect("record owner");

    let mut target = WorkEvent::new(WorkEventKind::Start, "work-target", t0);
    target.agent_session_id = Some("session-target".to_string());
    target.execution_container = Some(WorkspaceExecutionContainerRef {
        branch: Some("feature/spec-3273".to_string()),
        worktree_path: Some("/repo/feature/spec-3273".into()),
        pr_number: None,
        pr_url: None,
        pr_state: None,
    });
    record_workspace_work_event_paths(&work_items_path, &events_path, target)
        .expect("record target");

    let before_projection = std::fs::read(&work_items_path).expect("read projection before");
    let before_events = std::fs::read(&events_path).expect("read events before");

    let mut stray = WorkEvent::new(WorkEventKind::Update, "work-target", t1);
    stray.id = "event-rejected-stray".to_string();
    stray.agent_session_id = Some("session-owner".to_string());
    stray.status_category = Some(WorkspaceStatusCategory::Active);
    stray.execution_container = Some(WorkspaceExecutionContainerRef {
        branch: Some("feature/foreign".to_string()),
        worktree_path: Some("/repo/feature/foreign".into()),
        pr_number: None,
        pr_url: None,
        pr_state: None,
    });
    record_workspace_work_event_paths(&work_items_path, &events_path, stray)
        .expect("reject stray without failing caller");

    assert_eq!(
        std::fs::read(&work_items_path).expect("read projection after"),
        before_projection,
        "rejected event must not rewrite the projection"
    );
    assert_eq!(
        std::fs::read(&events_path).expect("read events after"),
        before_events,
        "rejected event must not enter the normal event log"
    );
    assert!(!std::fs::read_to_string(&events_path)
        .expect("event log text")
        .contains("event-rejected-stray"));
}

#[test]
fn record_workspace_work_event_does_not_advance_projection_when_append_fails() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let work_items_path = tmp.path().join("works.json");
    let events_path = tmp.path().join("events-as-directory");
    let t0 = chrono::Utc.with_ymd_and_hms(2026, 7, 15, 7, 0, 0).unwrap();
    let t1 = chrono::Utc.with_ymd_and_hms(2026, 7, 15, 8, 0, 0).unwrap();
    let mut projection = super::WorkItemsProjection::empty(t0);
    projection.apply_event(sample_work_event("work-durable", t0));
    super::save_workspace_work_items_projection_to_path(&work_items_path, &projection)
        .expect("seed projection");
    std::fs::create_dir(&events_path).expect("events path directory");
    let before = std::fs::read(&work_items_path).expect("projection bytes");

    let mut done = super::WorkEvent::new(super::WorkEventKind::Done, "work-durable", t1);
    done.status_category = Some(super::WorkspaceStatusCategory::Done);
    let error = super::record_workspace_work_event_paths(&work_items_path, &events_path, done)
        .expect_err("append must fail for a directory path");

    assert!(!error.to_string().is_empty());
    assert_eq!(
        std::fs::read(&work_items_path).expect("projection after failure"),
        before,
        "the durable event log must advance before works.json"
    );
}

#[test]
fn record_workspace_work_event_waits_for_the_projection_transaction_lock() {
    use fs2::FileExt as _;
    use std::fs::OpenOptions;
    use std::sync::mpsc::TryRecvError;
    use std::time::Duration;

    let tmp = tempfile::tempdir().expect("tempdir");
    let work_items_path = tmp.path().join("works.json");
    let events_path = tmp.path().join("events.jsonl");
    let lock_path = work_items_path.with_extension("lock");
    let t0 = chrono::Utc.with_ymd_and_hms(2026, 7, 15, 7, 0, 0).unwrap();
    let mut projection = super::WorkItemsProjection::empty(t0);
    projection.apply_event(sample_work_event("work-locked", t0));
    super::save_workspace_work_items_projection_to_path(&work_items_path, &projection)
        .expect("seed projection");

    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .expect("open lock");
    lock.lock_exclusive().expect("hold transaction lock");

    let (tx, rx) = std::sync::mpsc::channel();
    let writer_works = work_items_path.clone();
    let writer_events = events_path.clone();
    let writer = std::thread::spawn(move || {
        let event = sample_work_event("work-locked", t0 + chrono::Duration::minutes(1));
        tx.send(super::record_workspace_work_event_paths(
            &writer_works,
            &writer_events,
            event,
        ))
        .unwrap();
    });

    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        rx.try_recv().unwrap_err(),
        TryRecvError::Empty,
        "writer must not enter the projection transaction while the lock is held"
    );
    lock.unlock().expect("release transaction lock");
    rx.recv_timeout(Duration::from_secs(2))
        .expect("writer result")
        .expect("writer succeeds after lock release");
    writer.join().expect("writer thread");
}

/// SPEC-2359 Phase W-12 Slice 5a (FR-350): recording a Pause event persists
/// the Work in the history as a non-Done (incomplete) item keyed by the
/// session-derived id, carrying the branch / worktree execution container so
/// the Work surface can render the retained Paused row.
#[test]
fn record_workspace_work_paused_event_retains_incomplete_history_item() {
    let temp = tempfile::tempdir().expect("tempdir");
    let work_items_path = temp.path().join("works.json");
    let events_path = temp.path().join("work-events.jsonl");
    let container = WorkspaceExecutionContainerRef {
        branch: Some("work/paused".to_string()),
        worktree_path: Some(temp.path().join("work/paused")),
        pr_number: None,
        pr_url: None,
        pr_state: None,
    };

    super::record_workspace_work_paused_event_paths(
        &work_items_path,
        &events_path,
        "work-session-session-paused",
        Some("Paused persistence"),
        Some("agent stopped"),
        Some("SPEC-2359"),
        &["board-1".to_string()],
        Some(container),
        Some("session-paused"),
        Utc::now(),
    )
    .expect("record paused event");

    let projection = super::load_workspace_work_items_from_path(&work_items_path)
        .expect("load work items")
        .expect("work items present");
    let item = projection
        .work_items
        .iter()
        .find(|item| item.id == "work-session-session-paused")
        .expect("paused work item");
    assert!(item.is_incomplete(), "paused Work must stay non-Done");
    assert_ne!(item.status_category, WorkspaceStatusCategory::Done);
    assert_eq!(item.completed_at, None);
    assert_eq!(item.title, "Paused persistence");
    assert_eq!(item.execution_containers.len(), 1);
    assert_eq!(
        item.execution_containers[0].branch.as_deref(),
        Some("work/paused")
    );
    assert!(item.board_refs.iter().any(|value| value == "board-1"));
    assert!(item
        .events
        .iter()
        .any(|event| event.kind == WorkEventKind::Pause));
}

/// SPEC-2359 Phase W-12 Slice 5a (FR-350): a Pause event carries no explicit
/// status, so the Done-preservation in `apply_event` keeps an already-closed
/// (Done) Work terminal — agent stop must never reopen a closed Work.
#[test]
fn record_workspace_work_paused_event_does_not_reopen_done_work() {
    let temp = tempfile::tempdir().expect("tempdir");
    let work_items_path = temp.path().join("works.json");
    let events_path = temp.path().join("work-events.jsonl");
    let now = Utc::now();
    let mut done = WorkEvent::new(WorkEventKind::Done, "work-session-x", now);
    done.status_category = Some(WorkspaceStatusCategory::Done);
    super::record_workspace_work_event_paths(&work_items_path, &events_path, done)
        .expect("record done");

    super::record_workspace_work_paused_event_paths(
        &work_items_path,
        &events_path,
        "work-session-x",
        Some("Closed Work"),
        None,
        None,
        &[],
        None,
        Some("session-x"),
        now + chrono::Duration::seconds(1),
    )
    .expect("record paused event");

    let projection = super::load_workspace_work_items_from_path(&work_items_path)
        .expect("load work items")
        .expect("work items present");
    let item = projection
        .work_items
        .iter()
        .find(|item| item.id == "work-session-x")
        .expect("work item");
    assert_eq!(item.status_category, WorkspaceStatusCategory::Done);
    assert!(item.completed_at.is_some());
}

// ---------------------------------------------------------------------
// SPEC-2359 Phase W-12 Slice 5b (FR-353 / FR-355 / FR-358): the Work
// event log persistent core is repo-local and git-tracked.
// ---------------------------------------------------------------------

/// Override `gwt_home()` for the duration of a test so the home-side projection
/// writes (works.json, project-state) and the legacy migration sources
/// resolve under an isolated temp directory.
struct ScopedHome {
    _home: crate::test_support::ScopedGwtHome,
}

impl ScopedHome {
    fn set(path: &Path) -> Self {
        Self {
            _home: crate::test_support::ScopedGwtHome::set(path),
        }
    }
}

#[test]
fn legacy_work_items_migration_waits_for_project_lock_and_preserves_canonical_writer() {
    use fs2::FileExt;

    let _guard = lock_test_env();
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let _home = ScopedHome::set(&home);
    let canonical = gwt_workspace_work_items_path_for_repo_path(&repo);
    let legacy = legacy_workspace_work_items_path_for_repo_path(&repo);
    let t0 = Utc.with_ymd_and_hms(2026, 7, 15, 7, 0, 0).unwrap();
    let t1 = Utc.with_ymd_and_hms(2026, 7, 15, 8, 0, 0).unwrap();
    let mut legacy_projection = WorkItemsProjection::empty(t0);
    legacy_projection.apply_event(sample_work_event("work-legacy", t0));
    save_workspace_work_items_projection_to_path(&legacy, &legacy_projection).unwrap();

    std::fs::create_dir_all(canonical.parent().unwrap()).unwrap();
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(canonical.with_extension("lock"))
        .unwrap();
    lock.lock_exclusive().unwrap();

    let (tx, rx) = std::sync::mpsc::channel();
    let thread_home = home.clone();
    let thread_repo = repo.clone();
    let thread_canonical = canonical.clone();
    let handle = std::thread::spawn(move || {
        let _home = ScopedHome::set(&thread_home);
        tx.send(migrate_legacy_workspace_work_items(
            &thread_repo,
            &thread_canonical,
        ))
        .unwrap();
    });
    let early = rx.recv_timeout(std::time::Duration::from_millis(100)).ok();

    let mut writer_projection = WorkItemsProjection::empty(t1);
    writer_projection.apply_event(sample_work_event("work-writer", t1));
    save_workspace_work_items_projection_to_path(&canonical, &writer_projection).unwrap();
    FileExt::unlock(&lock).unwrap();
    let completed_while_locked = early.is_some();
    let migrated = early
        .unwrap_or_else(|| rx.recv().unwrap())
        .unwrap()
        .unwrap();
    handle.join().unwrap();

    assert!(
        !completed_while_locked,
        "migration must not enter while the canonical project lock is held"
    );
    assert!(migrated
        .work_items
        .iter()
        .any(|item| item.id == "work-writer"));
    let canonical_projection = load_workspace_work_items_from_path(&canonical)
        .unwrap()
        .unwrap();
    assert!(canonical_projection
        .work_items
        .iter()
        .any(|item| item.id == "work-writer"));
    assert!(!canonical_projection
        .work_items
        .iter()
        .any(|item| item.id == "work-legacy"));
}

#[test]
fn legacy_current_migration_waits_for_project_lock_and_preserves_canonical_writer() {
    use fs2::FileExt;

    let _guard = lock_test_env();
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let _home = ScopedHome::set(&home);
    let canonical = gwt_workspace_projection_path_for_repo_path(&repo);
    let legacy = legacy_workspace_projection_path_for_repo_path(&repo);
    let mut legacy_projection = WorkspaceProjection::default_for_project(&repo);
    legacy_projection.title = "Legacy current".to_string();
    save_workspace_projection_to_path(&legacy, &legacy_projection).unwrap();

    std::fs::create_dir_all(canonical.parent().unwrap()).unwrap();
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(canonical.with_file_name("works.lock"))
        .unwrap();
    lock.lock_exclusive().unwrap();

    let (tx, rx) = std::sync::mpsc::channel();
    let thread_home = home.clone();
    let thread_repo = repo.clone();
    let thread_canonical = canonical.clone();
    let handle = std::thread::spawn(move || {
        let _home = ScopedHome::set(&thread_home);
        tx.send(migrate_legacy_workspace_projection(
            &thread_repo,
            &thread_canonical,
        ))
        .unwrap();
    });
    let early = rx.recv_timeout(std::time::Duration::from_millis(100)).ok();

    let mut writer_projection = WorkspaceProjection::default_for_project(&repo);
    writer_projection.title = "Canonical writer".to_string();
    save_workspace_projection_to_path_unlocked(&canonical, &writer_projection).unwrap();
    FileExt::unlock(&lock).unwrap();
    let completed_while_locked = early.is_some();
    let migrated = early
        .unwrap_or_else(|| rx.recv().unwrap())
        .unwrap()
        .unwrap();
    handle.join().unwrap();

    assert!(
        !completed_while_locked,
        "legacy current migration must serialize before checking canonical existence"
    );
    assert_eq!(migrated.title, "Canonical writer");
    assert_eq!(
        load_workspace_projection_from_path(&canonical)
            .unwrap()
            .unwrap()
            .title,
        "Canonical writer"
    );
}

#[test]
fn existing_projection_mutation_migrates_legacy_current_before_updating() {
    let _guard = lock_test_env();
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let _home = ScopedHome::set(&home);
    let canonical = gwt_workspace_projection_path_for_repo_path(&repo);
    let legacy = legacy_workspace_projection_path_for_repo_path(&repo);
    let mut projection = WorkspaceProjection::default_for_project(&repo);
    projection.title = "Legacy current".to_string();
    save_workspace_projection_to_path(&legacy, &projection).unwrap();

    let result = mutate_existing_workspace_projection(&repo, |projection| {
        projection.summary = Some("Updated after migration".to_string());
        Ok(projection.id.clone())
    })
    .unwrap();

    assert_eq!(result, Some(projection.id));
    let saved = load_workspace_projection_from_path(&canonical)
        .unwrap()
        .expect("legacy current must be migrated before mutation");
    assert_eq!(saved.title, "Legacy current");
    assert_eq!(saved.summary.as_deref(), Some("Updated after migration"));
}

#[test]
fn current_update_locks_before_read_and_preserves_a_waiting_writer() {
    use fs2::FileExt;

    let temp = tempfile::tempdir().unwrap();
    let state_dir = temp.path().join("workspace");
    let current = state_dir.join("current.json");
    let journal = state_dir.join("journal.jsonl");
    let project_root = temp.path().join("repo");
    let mut initial = WorkspaceProjection::default_for_project(&project_root);
    initial.title = "Initial".to_string();
    save_workspace_projection_to_path(&current, &initial).unwrap();

    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(current.with_file_name("works.lock"))
        .unwrap();
    lock.lock_exclusive().unwrap();

    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let current_for_thread = current.clone();
    let journal_for_thread = journal.clone();
    let root_for_thread = project_root.clone();
    let handle = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        update_workspace_projection_with_journal_paths_at(
            &current_for_thread,
            &journal_for_thread,
            &root_for_thread,
            WorkspaceProjectionUpdate {
                title: None,
                status_category: None,
                status_text: None,
                owner: None,
                next_action: None,
                summary: Some("Concurrent update".to_string()),
                progress_summary: None,
                agent_session_id: None,
                agent_current_focus: None,
                agent_title_summary: None,
            },
            Utc.with_ymd_and_hms(2026, 7, 15, 10, 0, 0).unwrap(),
        )
    });
    started_rx.recv().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(100));

    let mut writer = WorkspaceProjection::default_for_project(&project_root);
    writer.title = "Canonical writer".to_string();
    save_workspace_projection_to_path_unlocked(&current, &writer).unwrap();
    FileExt::unlock(&lock).unwrap();
    handle.join().unwrap().unwrap();

    let projection = load_workspace_projection_from_path(&current)
        .unwrap()
        .unwrap();
    assert_eq!(projection.title, "Canonical writer");
    assert_eq!(projection.summary.as_deref(), Some("Concurrent update"));
}

#[test]
fn atomic_projection_mutation_locks_before_read_and_preserves_a_waiting_writer() {
    use fs2::FileExt;

    let temp = tempfile::tempdir().unwrap();
    let current = temp.path().join("state/current.json");
    let project_root = temp.path().join("repo");
    let mut initial = WorkspaceProjection::default_for_project(&project_root);
    initial.title = "Initial".to_string();
    save_workspace_projection_to_path(&current, &initial).unwrap();

    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(current.with_file_name("works.lock"))
        .unwrap();
    lock.lock_exclusive().unwrap();

    let current_for_thread = current.clone();
    let root_for_thread = project_root.clone();
    let handle = std::thread::spawn(move || {
        mutate_workspace_projection_at(&current_for_thread, &root_for_thread, |projection| {
            projection.summary = Some("Atomic update".to_string());
            Ok(())
        })
    });
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(
        !handle.is_finished(),
        "the mutation must acquire the project lock before loading current.json"
    );

    let mut writer = WorkspaceProjection::default_for_project(&project_root);
    writer.title = "Canonical writer".to_string();
    save_workspace_projection_to_path_unlocked(&current, &writer).unwrap();
    FileExt::unlock(&lock).unwrap();
    handle.join().unwrap().unwrap();

    let projection = load_workspace_projection_from_path(&current)
        .unwrap()
        .unwrap();
    assert_eq!(projection.title, "Canonical writer");
    assert_eq!(projection.summary.as_deref(), Some("Atomic update"));
}

#[test]
fn workspace_state_transaction_persists_assignment_and_work_event_under_one_lock() {
    use fs2::FileExt;

    let temp = tempfile::tempdir().unwrap();
    let current = temp.path().join("state/current.json");
    let works = temp.path().join("state/works.json");
    let events = temp.path().join("repo/.gwt/work/events.jsonl");
    let project_root = temp.path().join("repo");
    let now = Utc.with_ymd_and_hms(2026, 7, 15, 11, 0, 0).unwrap();
    let mut initial = WorkspaceProjection::default_for_project(&project_root);
    initial.agents.push(WorkspaceAgentSummary {
        session_id: "session-atomic".to_string(),
        window_id: None,
        agent_id: "codex".to_string(),
        display_name: "Codex".to_string(),
        status_category: WorkspaceStatusCategory::Active,
        current_focus: None,
        title_summary: None,
        worktree_path: Some(project_root.join("feature/spec-2359")),
        branch: Some("feature/spec-2359".to_string()),
        last_board_entry_id: None,
        last_board_entry_kind: None,
        coordination_scope: None,
        affiliation_status: WorkspaceAgentAffiliationStatus::Unassigned,
        workspace_id: None,
        updated_at: now,
    });
    save_workspace_projection_to_path(&current, &initial).unwrap();

    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(works.with_extension("lock"))
        .unwrap();
    lock.lock_exclusive().unwrap();

    let current_for_thread = current.clone();
    let works_for_thread = works.clone();
    let events_for_thread = events.clone();
    let root_for_thread = project_root.clone();
    let handle = std::thread::spawn(move || {
        transact_workspace_state_at(
            &current_for_thread,
            &works_for_thread,
            &events_for_thread,
            &root_for_thread,
            |projection, _work_items, _work_items_persisted| {
                let agent = projection
                    .agents
                    .iter_mut()
                    .find(|agent| agent.session_id == "session-atomic")
                    .unwrap();
                agent.affiliation_status = WorkspaceAgentAffiliationStatus::Assigned;
                agent.workspace_id = Some("work-atomic".to_string());
                let mut event = WorkEvent::new(WorkEventKind::Start, "work-atomic", now);
                event.agent_session_id = Some("session-atomic".to_string());
                event.execution_container = Some(WorkspaceExecutionContainerRef {
                    branch: Some("feature/spec-2359".to_string()),
                    worktree_path: Some(root_for_thread.join("feature/spec-2359")),
                    pr_number: None,
                    pr_url: None,
                    pr_state: None,
                });
                Ok(((), vec![event]))
            },
        )
    });
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(
        !handle.is_finished(),
        "assignment and Work creation must wait on the same project lock"
    );

    let mut writer = initial;
    writer.summary = Some("Writer state".to_string());
    save_workspace_projection_to_path_unlocked(&current, &writer).unwrap();
    FileExt::unlock(&lock).unwrap();
    handle.join().unwrap().unwrap();

    let projection = load_workspace_projection_from_path(&current)
        .unwrap()
        .unwrap();
    assert_eq!(projection.summary.as_deref(), Some("Writer state"));
    assert_eq!(
        workspace_assignment_for_session(&projection, "session-atomic"),
        WorkspaceSessionAssignment::Assigned("work-atomic".to_string())
    );
    let work_items = load_workspace_work_items_from_path(&works)
        .unwrap()
        .unwrap();
    assert!(work_items
        .work_items
        .iter()
        .any(|item| item.id == "work-atomic"));
}

#[test]
fn legacy_multi_branch_decomposition_waits_for_project_lock() {
    use fs2::FileExt;

    let temp = tempfile::tempdir().unwrap();
    let project_root = temp.path().join("repo");
    let work_items_path = temp.path().join("state/works.json");
    std::fs::create_dir_all(&project_root).unwrap();
    let t0 = Utc.with_ymd_and_hms(2026, 7, 15, 7, 0, 0).unwrap();
    let t1 = Utc.with_ymd_and_hms(2026, 7, 15, 8, 0, 0).unwrap();
    let mut legacy = WorkItemsProjection::empty(t0);
    for (id, branch) in [("evt-a", "work/a"), ("evt-b", "work/b")] {
        let mut event = WorkEvent::new(WorkEventKind::Update, "work-legacy-mega", t0);
        event.id = id.to_string();
        event.execution_container = Some(WorkspaceExecutionContainerRef {
            branch: Some(branch.to_string()),
            worktree_path: Some(project_root.join(branch)),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        });
        legacy.apply_event(event);
    }
    save_workspace_work_items_projection_to_path(&work_items_path, &legacy).unwrap();

    std::fs::create_dir_all(work_items_path.parent().unwrap()).unwrap();
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(work_items_path.with_extension("lock"))
        .unwrap();
    lock.lock_exclusive().unwrap();
    let works = work_items_path.clone();
    let root = project_root.clone();
    let handle =
        std::thread::spawn(move || decompose_legacy_multi_branch_work_items_paths(&works, &root));
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(
        !handle.is_finished(),
        "decomposition must wait for the project transaction lock"
    );

    let mut writer = WorkItemsProjection::empty(t1);
    writer.apply_event(sample_work_event("work-newer-writer", t1));
    save_workspace_work_items_projection_to_path(&work_items_path, &writer).unwrap();
    FileExt::unlock(&lock).unwrap();
    assert_eq!(handle.join().unwrap().unwrap(), 0);
    let saved = load_workspace_work_items_from_path(&work_items_path)
        .unwrap()
        .unwrap();
    assert!(saved
        .work_items
        .iter()
        .any(|item| item.id == "work-newer-writer"));
}

#[test]
fn resume_owner_repair_waits_for_project_lock() {
    use fs2::FileExt;

    let temp = tempfile::tempdir().unwrap();
    let work_items_path = temp.path().join("state/works.json");
    let current_path = temp.path().join("state/current.json");
    let t0 = Utc.with_ymd_and_hms(2026, 7, 15, 7, 0, 0).unwrap();
    let t1 = Utc.with_ymd_and_hms(2026, 7, 15, 8, 0, 0).unwrap();
    save_workspace_work_items_projection_to_path(&work_items_path, &WorkItemsProjection::empty(t0))
        .unwrap();

    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(work_items_path.with_extension("lock"))
        .unwrap();
    lock.lock_exclusive().unwrap();
    let works = work_items_path.clone();
    let current = current_path.clone();
    let handle = std::thread::spawn(move || repair_resume_owner_bleed_paths(&works, &current, t1));
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(
        !handle.is_finished(),
        "resume-owner repair must wait for the project transaction lock"
    );

    let mut writer = WorkItemsProjection::empty(t1);
    writer.apply_event(sample_work_event("work-repair-writer", t1));
    save_workspace_work_items_projection_to_path(&work_items_path, &writer).unwrap();
    FileExt::unlock(&lock).unwrap();
    handle.join().unwrap().unwrap();
    let saved = load_workspace_work_items_from_path(&work_items_path)
        .unwrap()
        .unwrap();
    assert!(saved
        .work_items
        .iter()
        .any(|item| item.id == "work-repair-writer"));
}

#[test]
fn resume_owner_repair_waits_for_split_current_writer_lock() {
    use fs2::FileExt;

    let temp = tempfile::tempdir().unwrap();
    let work_items_path = temp.path().join("work-state/works.json");
    let current_path = temp.path().join("current-state/current.json");
    let now = Utc.timestamp_opt(30_000, 0).unwrap();
    let mut works = WorkItemsProjection::empty(now);
    for (index, branch) in ["work/a", "work/b", "work/c"].iter().enumerate() {
        let work_id = format!("work-split-repair-{index}");
        works.apply_event(bleed_backfill_event(&work_id, branch, index as i64));
        works.apply_event(bleed_resume_event(&work_id, index as i64));
    }
    save_workspace_work_items_projection_to_path(&work_items_path, &works).unwrap();

    let mut current = WorkspaceProjection::default_for_project(temp.path());
    current.title = "gwt-manage-pr".to_string();
    current.owner = Some("SPEC-2359".to_string());
    current.summary = Some("stale current state".to_string());
    save_workspace_projection_to_path(&current_path, &current).unwrap();

    let current_lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(current_path.with_file_name("works.lock"))
        .unwrap();
    current_lock.lock_exclusive().unwrap();
    let works_for_thread = work_items_path.clone();
    let current_for_thread = current_path.clone();
    let handle = std::thread::spawn(move || {
        repair_resume_owner_bleed_paths(&works_for_thread, &current_for_thread, now)
    });
    std::thread::sleep(std::time::Duration::from_millis(100));
    let finished_while_current_locked = handle.is_finished();

    current.owner = Some("SPEC-newer-writer".to_string());
    current.summary = Some("newer current writer".to_string());
    save_workspace_projection_to_path_unlocked(&current_path, &current).unwrap();
    FileExt::unlock(&current_lock).unwrap();
    handle.join().unwrap().unwrap();

    assert!(
        !finished_while_current_locked,
        "resume-owner repair must wait for the split current writer lock"
    );
    let saved = load_workspace_projection_from_path(&current_path)
        .unwrap()
        .unwrap();
    assert_eq!(saved.owner.as_deref(), Some("SPEC-newer-writer"));
    assert_eq!(saved.summary.as_deref(), Some("newer current writer"));
}

#[test]
fn repo_local_event_migration_waits_for_project_lock_and_preserves_writer_file() {
    use fs2::FileExt;

    let _guard = lock_test_env();
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let repo = temp.path().join("repo");
    init_test_git_repo(&repo);
    let _home = ScopedHome::set(&home);
    let home_events = gwt_workspace_work_events_path_for_repo_path(&repo);
    let repo_events = gwt_repo_local_work_events_path(&repo);
    append_workspace_work_event_to_path(
        &home_events,
        &start_event(
            "work-legacy-event",
            Utc.with_ymd_and_hms(2026, 7, 15, 7, 0, 0).unwrap(),
        ),
    )
    .unwrap();

    let work_items_path = gwt_workspace_work_items_path_for_repo_path(&repo);
    std::fs::create_dir_all(work_items_path.parent().unwrap()).unwrap();
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(work_items_path.with_extension("lock"))
        .unwrap();
    lock.lock_exclusive().unwrap();
    let thread_repo = repo.clone();
    let thread_home = home.clone();
    let handle = std::thread::spawn(move || {
        let _home = ScopedHome::set(&thread_home);
        repo_local_work_events_path_with_migration(&thread_repo)
    });
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(
        !handle.is_finished(),
        "first-use event migration must wait for the project transaction lock"
    );

    std::fs::create_dir_all(repo_events.parent().unwrap()).unwrap();
    std::fs::write(&repo_events, "writer-event\n").unwrap();
    FileExt::unlock(&lock).unwrap();
    assert_eq!(handle.join().unwrap().unwrap(), repo_events);
    assert_eq!(
        std::fs::read_to_string(&repo_events).unwrap(),
        "writer-event\n"
    );
}

fn init_test_git_repo(path: &Path) {
    std::fs::create_dir_all(path).expect("create repo dir");
    let output = crate::process::hidden_command("git")
        .args(["init", path.to_str().unwrap()])
        .output()
        .expect("git init");
    assert!(output.status.success(), "git init failed");
    for args in [
        ["config", "user.email", "test@example.com"],
        ["config", "user.name", "Test User"],
    ] {
        let mut cmd = crate::process::hidden_command("git");
        cmd.args(args).current_dir(path);
        crate::process::scrub_git_env(&mut cmd);
        assert!(cmd.output().expect("git config").status.success());
    }
}

fn start_event(work_item_id: &str, at: DateTime<Utc>) -> WorkEvent {
    let mut event = WorkEvent::new(WorkEventKind::Start, work_item_id, at);
    event.status_category = Some(WorkspaceStatusCategory::Active);
    event.title = Some("Repo-local work".to_string());
    event
}

#[test]
fn record_workspace_work_event_writes_to_repo_local_events_log() {
    let _guard = crate::test_support::env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = tempfile::tempdir().expect("home");
    let _home = ScopedHome::set(home.path());
    let workspace = tempfile::tempdir().expect("workspace");
    let repo = workspace.path().join("repo");
    init_test_git_repo(&repo);

    let t1 = Utc.with_ymd_and_hms(2026, 6, 5, 10, 0, 0).unwrap();
    record_workspace_work_event(&repo, start_event("wi-repo-local", t1)).expect("record event");

    // The event must land in the repo-local, git-tracked event log.
    let repo_local = repo.join(".gwt").join("work").join("events.jsonl");
    assert!(
        repo_local.is_file(),
        "event must be written to repo-local .gwt/work/events.jsonl"
    );
    let body = std::fs::read_to_string(&repo_local).expect("read events");
    assert!(body.contains("wi-repo-local"), "event payload present");

    // The home Project State event log must NOT be written for new events.
    let home_events = gwt_workspace_work_events_path_for_repo_path(&repo);
    assert!(
        !home_events.exists(),
        "home project-state event log must not receive new events"
    );
}

#[test]
fn record_workspace_work_event_adds_union_merge_gitattribute_idempotently() {
    let _guard = crate::test_support::env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = tempfile::tempdir().expect("home");
    let _home = ScopedHome::set(home.path());
    let workspace = tempfile::tempdir().expect("workspace");
    let repo = workspace.path().join("repo");
    init_test_git_repo(&repo);
    // Seed a pre-existing .gitattributes to confirm we append, not clobber.
    std::fs::write(repo.join(".gitattributes"), "*.sh text eol=lf\n").expect("seed gitattributes");

    let t1 = Utc.with_ymd_and_hms(2026, 6, 5, 10, 0, 0).unwrap();
    record_workspace_work_event(&repo, start_event("wi-attr", t1)).expect("record 1");
    record_workspace_work_event(
        &repo,
        start_event("wi-attr-2", t1 + chrono::Duration::seconds(1)),
    )
    .expect("record 2");

    let attributes =
        std::fs::read_to_string(repo.join(".gitattributes")).expect("read gitattributes");
    let union_lines = attributes
        .lines()
        .filter(|line| line.trim() == "**/.gwt/work/events.jsonl merge=union")
        .count();
    assert_eq!(
        union_lines, 1,
        "union-merge entry must be added exactly once"
    );
    assert!(
        attributes.contains("*.sh text eol=lf"),
        "pre-existing gitattributes content must be preserved"
    );
}

#[test]
fn migrates_home_events_into_repo_local_once_then_skips() {
    let _guard = crate::test_support::env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = tempfile::tempdir().expect("home");
    let _home = ScopedHome::set(home.path());
    let workspace = tempfile::tempdir().expect("workspace");
    let repo = workspace.path().join("repo");
    init_test_git_repo(&repo);

    // Seed the home Project State event log with a historical event so the
    // one-time migration has something to copy.
    let home_events = gwt_workspace_work_events_path_for_repo_path(&repo);
    let t0 = Utc.with_ymd_and_hms(2026, 6, 1, 9, 0, 0).unwrap();
    append_workspace_work_event_to_path(&home_events, &start_event("wi-historical", t0))
        .expect("seed home events");

    let repo_local = repo.join(".gwt").join("work").join("events.jsonl");
    assert!(!repo_local.exists(), "precondition: repo-local absent");

    // First record triggers migration: the historical event is copied in,
    // then the new event is appended.
    let t1 = Utc.with_ymd_and_hms(2026, 6, 5, 10, 0, 0).unwrap();
    record_workspace_work_event(&repo, start_event("wi-new", t1)).expect("record new");

    let body = std::fs::read_to_string(&repo_local).expect("read repo-local");
    assert!(
        body.contains("wi-historical"),
        "migration must copy the home historical event into the repo-local log"
    );
    assert!(
        body.contains("wi-new"),
        "the new event is appended after migration"
    );

    // Mutate the home log AFTER migration. Because the repo-local file now
    // exists, the home source must never be read again (idempotent skip).
    append_workspace_work_event_to_path(
        &home_events,
        &start_event("wi-home-after-migration", t1 + chrono::Duration::seconds(5)),
    )
    .expect("append post-migration home event");

    record_workspace_work_event(
        &repo,
        start_event("wi-second", t1 + chrono::Duration::seconds(10)),
    )
    .expect("record second");

    let body2 = std::fs::read_to_string(&repo_local).expect("read repo-local again");
    assert!(
        !body2.contains("wi-home-after-migration"),
        "once repo-local exists the home source must not be migrated again"
    );
    assert!(body2.contains("wi-second"), "second new event appended");
}

#[test]
fn rebuild_work_items_uses_repo_local_events_after_migration() {
    let _guard = crate::test_support::env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = tempfile::tempdir().expect("home");
    let _home = ScopedHome::set(home.path());
    let workspace = tempfile::tempdir().expect("workspace");
    let repo = workspace.path().join("repo");
    init_test_git_repo(&repo);

    // A Done then later Update in the home log; rebuild must replay through
    // the repo-local log and recover the terminal Done state (regression
    // coverage that the repo-local path drives the existing rebuild).
    let home_events = gwt_workspace_work_events_path_for_repo_path(&repo);
    let t1 = Utc.with_ymd_and_hms(2026, 6, 1, 10, 0, 0).unwrap();
    let t2 = Utc.with_ymd_and_hms(2026, 6, 1, 11, 0, 0).unwrap();
    let mut done = WorkEvent::new(WorkEventKind::Done, "wi-rebuild", t1);
    done.status_category = Some(WorkspaceStatusCategory::Done);
    append_workspace_work_event_to_path(&home_events, &done).expect("seed done");
    let update = WorkEvent::new(WorkEventKind::Update, "wi-rebuild", t2);
    append_workspace_work_event_to_path(&home_events, &update).expect("seed update");

    let outcome = rebuild_work_items_from_events_for_repo(&repo).expect("rebuild");
    assert_eq!(outcome, WorkItemsRebuildOutcome::Applied);

    // The rebuild must have migrated and replayed the repo-local log.
    let repo_local = repo.join(".gwt").join("work").join("events.jsonl");
    assert!(
        repo_local.is_file(),
        "rebuild migrates events into repo-local log"
    );

    let projection =
        load_workspace_work_items_from_path(&gwt_workspace_work_items_path_for_repo_path(&repo))
            .expect("load")
            .expect("present");
    let item = projection
        .work_items
        .iter()
        .find(|it| it.id == "wi-rebuild")
        .expect("rebuilt item");
    assert_eq!(
        item.status_category,
        WorkspaceStatusCategory::Done,
        "Done terminal state recovered via repo-local replay"
    );
}

fn backfill_source(branch: Option<&str>, worktree_path: &Path) -> WorktreeReconcileSource {
    WorktreeReconcileSource {
        branch: branch.map(str::to_string),
        worktree_path: worktree_path.to_path_buf(),
    }
}

fn seeded_work_item(
    id: &str,
    branch: Option<&str>,
    status: WorkspaceStatusCategory,
    discarded: bool,
    at: DateTime<Utc>,
) -> WorkItem {
    WorkItem {
        id: id.to_string(),
        title: id.to_string(),
        intent: None,
        summary: None,
        progress_summary: None,
        status_category: status,
        owner: None,
        created_at: at,
        updated_at: at,
        completed_at: None,
        agents: Vec::new(),
        execution_containers: branch
            .map(|branch| {
                vec![WorkspaceExecutionContainerRef {
                    branch: Some(branch.to_string()),
                    worktree_path: None,
                    pr_number: None,
                    pr_url: None,
                    pr_state: None,
                }]
            })
            .unwrap_or_default(),
        board_refs: Vec::new(),
        related_work_item_ids: Vec::new(),
        events: Vec::new(),
        legacy_metadata_snapshot: None,
        legacy_metadata_authoritative: false,
        legacy_metadata_snapshot_at: None,
        duplicate_event_containers: Default::default(),
        discarded,
        discarded_at: discarded.then_some(at),
    }
}

/// SPEC-2359 Phase W-15 (FR-379/FR-380): a real worktree without any
/// matching record gets a Backfill event recorded into the worktree's own
/// repo-local event log and surfaces as an Idle (-> Paused) work item with
/// title = branch name and the canonical branch-derived work id.
#[test]
fn backfill_records_work_item_for_worktree_without_record() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project_root = temp.path().join("repo");
    let worktree = temp.path().join("repo-wt");
    fs::create_dir_all(&worktree).expect("worktree dir");
    let work_items_path = temp.path().join("works.json");
    let now = Utc.with_ymd_and_hms(2026, 6, 10, 12, 0, 0).unwrap();

    let backfilled = reconcile_worktree_work_items_paths(
        &work_items_path,
        &project_root,
        &[backfill_source(Some("work/foo"), &worktree)],
        now,
    )
    .expect("reconcile");
    assert_eq!(backfilled, 1);

    let projection = load_workspace_work_items_from_path(&work_items_path)
        .expect("load works")
        .expect("projection exists");
    assert_eq!(projection.work_items.len(), 1);
    let item = &projection.work_items[0];
    let expected_id = canonical_work_id(&project_root, Some("work/foo"), None).unwrap();
    assert_eq!(item.id, expected_id);
    assert_eq!(item.title, "work/foo");
    assert_eq!(
        item.status_category,
        WorkspaceStatusCategory::Idle,
        "backfill surfaces as Idle (rendered Paused without live agent)"
    );
    assert!(item
        .execution_containers
        .iter()
        .any(|container| container.branch.as_deref() == Some("work/foo")
            && container.worktree_path.as_deref() == Some(worktree.as_path())));

    let events_path = gwt_repo_local_work_events_path(&worktree);
    let events_text = fs::read_to_string(&events_path).expect("worktree events log");
    let lines: Vec<&str> = events_text.lines().collect();
    assert_eq!(lines.len(), 1, "exactly one backfill event line");
    let event: WorkEvent = serde_json::from_str(lines[0]).expect("event json");
    assert_eq!(event.kind, WorkEventKind::Backfill);
    assert_eq!(
        event.status_category, None,
        "backfill must not carry an explicit status so apply_event terminal \
             preservation keeps closed items closed when the event is re-ingested"
    );
}

/// SPEC-2359 Phase W-15 (SC-255): repeated reconcile over the same sources
/// is idempotent — no duplicate work items and no duplicate event lines.
#[test]
fn backfill_is_idempotent_across_repeated_reconcile() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project_root = temp.path().join("repo");
    let worktree = temp.path().join("repo-wt");
    fs::create_dir_all(&worktree).expect("worktree dir");
    let work_items_path = temp.path().join("works.json");
    let now = Utc.with_ymd_and_hms(2026, 6, 10, 12, 0, 0).unwrap();
    let sources = [backfill_source(Some("work/foo"), &worktree)];

    let first = reconcile_worktree_work_items_paths(&work_items_path, &project_root, &sources, now)
        .expect("first reconcile");
    let second =
        reconcile_worktree_work_items_paths(&work_items_path, &project_root, &sources, now)
            .expect("second reconcile");
    assert_eq!((first, second), (1, 0));

    let projection = load_workspace_work_items_from_path(&work_items_path)
        .expect("load works")
        .expect("projection exists");
    assert_eq!(projection.work_items.len(), 1);
    let events_text =
        fs::read_to_string(gwt_repo_local_work_events_path(&worktree)).expect("events log");
    assert_eq!(events_text.lines().count(), 1);
}

/// SPEC-2359 Phase W-15 (FR-380 idempotency): a worktree whose branch is
/// already covered by a session-keyed record (work-session-<uuid>) must not
/// be backfilled, including when the recorded branch carries an origin/
/// prefix (canonical branch identity comparison).
#[test]
fn backfill_skips_worktree_already_covered_by_session_record() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project_root = temp.path().join("repo");
    let worktree = temp.path().join("repo-wt");
    fs::create_dir_all(&worktree).expect("worktree dir");
    let work_items_path = temp.path().join("works.json");
    let now = Utc.with_ymd_and_hms(2026, 6, 10, 12, 0, 0).unwrap();

    let projection = WorkItemsProjection {
        updated_at: now,
        work_items: vec![seeded_work_item(
            "work-session-abc",
            Some("origin/work/foo"),
            WorkspaceStatusCategory::Active,
            false,
            now,
        )],
    };
    save_workspace_work_items_projection_to_path(&work_items_path, &projection)
        .expect("seed works");

    let backfilled = reconcile_worktree_work_items_paths(
        &work_items_path,
        &project_root,
        &[backfill_source(Some("work/foo"), &worktree)],
        now,
    )
    .expect("reconcile");
    assert_eq!(backfilled, 0);
    assert!(
        !gwt_repo_local_work_events_path(&worktree).exists(),
        "no backfill event log should be created for a covered branch"
    );
}

/// SPEC-2359 Phase W-15 (FR-381): a detached worktree (no branch) is never
/// backfilled.
#[test]
fn backfill_skips_detached_worktree_without_branch() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project_root = temp.path().join("repo");
    let worktree = temp.path().join("repo-wt");
    fs::create_dir_all(&worktree).expect("worktree dir");
    let work_items_path = temp.path().join("works.json");
    let now = Utc.with_ymd_and_hms(2026, 6, 10, 12, 0, 0).unwrap();

    let backfilled = reconcile_worktree_work_items_paths(
        &work_items_path,
        &project_root,
        &[backfill_source(None, &worktree)],
        now,
    )
    .expect("reconcile");
    assert_eq!(backfilled, 0);
    assert!(load_workspace_work_items_from_path(&work_items_path)
        .expect("load works")
        .is_none());
}

/// SPEC-2359 Phase W-15 (US-61 preservation): a terminal record (Done or
/// discarded) matching the worktree's branch is skipped entirely — no
/// event is appended and the terminal status never regresses.
#[test]
fn backfill_does_not_reopen_terminal_record() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project_root = temp.path().join("repo");
    let done_worktree = temp.path().join("repo-done");
    let discarded_worktree = temp.path().join("repo-discarded");
    fs::create_dir_all(&done_worktree).expect("worktree dir");
    fs::create_dir_all(&discarded_worktree).expect("worktree dir");
    let work_items_path = temp.path().join("works.json");
    let now = Utc.with_ymd_and_hms(2026, 6, 10, 12, 0, 0).unwrap();

    let projection = WorkItemsProjection {
        updated_at: now,
        work_items: vec![
            seeded_work_item(
                "work-session-done",
                Some("work/done"),
                WorkspaceStatusCategory::Done,
                false,
                now,
            ),
            seeded_work_item(
                "work-session-discarded",
                Some("work/discarded"),
                WorkspaceStatusCategory::Idle,
                true,
                now,
            ),
        ],
    };
    save_workspace_work_items_projection_to_path(&work_items_path, &projection)
        .expect("seed works");

    let backfilled = reconcile_worktree_work_items_paths(
        &work_items_path,
        &project_root,
        &[
            backfill_source(Some("work/done"), &done_worktree),
            backfill_source(Some("work/discarded"), &discarded_worktree),
        ],
        now,
    )
    .expect("reconcile");
    assert_eq!(backfilled, 0);

    let reloaded = load_workspace_work_items_from_path(&work_items_path)
        .expect("load works")
        .expect("projection exists");
    assert_eq!(reloaded.work_items.len(), 2);
    assert_eq!(
        reloaded.work_items[0].status_category,
        WorkspaceStatusCategory::Done
    );
    assert!(reloaded.work_items[1].discarded);
    assert!(!gwt_repo_local_work_events_path(&done_worktree).exists());
    assert!(!gwt_repo_local_work_events_path(&discarded_worktree).exists());
}

/// SPEC-2359 Phase W-15 (FR-380): the Backfill kind serializes as
/// snake_case "backfill" on the wire and round-trips.
#[test]
fn backfill_event_kind_serializes_as_snake_case() {
    let json = serde_json::to_string(&WorkEventKind::Backfill).expect("serialize");
    assert_eq!(json, "\"backfill\"");
    let parsed: WorkEventKind = serde_json::from_str("\"backfill\"").expect("parse");
    assert_eq!(parsed, WorkEventKind::Backfill);
}

// --- SPEC-2359 Phase W-14 (US-70 / FR-375, SC-251): transition service ---

fn legacy_event(
    work_item_id: &str,
    branch: Option<&str>,
    title: Option<&str>,
    session: Option<&str>,
    at: DateTime<Utc>,
) -> WorkEvent {
    let mut event = WorkEvent::new(WorkEventKind::Update, work_item_id, at);
    event.title = title.map(str::to_string);
    event.agent_session_id = session.map(str::to_string);
    event.execution_container = branch.map(|branch| WorkspaceExecutionContainerRef {
        branch: Some(branch.to_string()),
        worktree_path: None,
        pr_number: None,
        pr_url: None,
        pr_state: None,
    });
    event
}

/// SPEC-2359 Phase W-16 (FR-393): a legacy mega-item whose events span
/// multiple branches is decomposed into canonical branch-keyed items.
/// Titles/agents follow each branch's events; the legacy item disappears;
/// a second run is a no-op (idempotent).
#[test]
fn legacy_multi_branch_work_item_is_decomposed_per_branch() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project_root = temp.path().join("repo");
    let work_items_path = temp.path().join("works.json");
    let t0 = Utc.with_ymd_and_hms(2026, 6, 10, 10, 0, 0).unwrap();
    let t1 = Utc.with_ymd_and_hms(2026, 6, 10, 11, 0, 0).unwrap();
    let t2 = Utc.with_ymd_and_hms(2026, 6, 10, 12, 0, 0).unwrap();

    let mega_id = "0c14f2ab-9f9a-4e79-94ab-db590cf88343";
    let mut projection = WorkItemsProjection::empty(t0);
    projection.apply_event(legacy_event(
        mega_id,
        Some("develop"),
        Some("develop での調査"),
        Some("sess-dev-1"),
        t0,
    ));
    projection.apply_event(legacy_event(
        mega_id,
        Some("work/foo"),
        Some("foo の実装"),
        Some("sess-foo-1"),
        t1,
    ));
    projection.apply_event(legacy_event(
        mega_id,
        Some("origin/develop"),
        Some("develop PR 監視"),
        Some("sess-dev-2"),
        t2,
    ));
    // Branchless heartbeat: dropped with the legacy shell on decomposition.
    projection.apply_event(legacy_event(mega_id, None, None, None, t2));
    save_workspace_work_items_projection_to_path(&work_items_path, &projection)
        .expect("seed works");

    let decomposed =
        decompose_legacy_multi_branch_work_items_paths(&work_items_path, &project_root)
            .expect("decompose");
    assert_eq!(decomposed, 1, "one legacy mega-item decomposed");

    let reloaded = load_workspace_work_items_from_path(&work_items_path)
        .expect("load works")
        .expect("projection exists");
    let develop_id = canonical_work_id(&project_root, Some("develop"), None).unwrap();
    let foo_id = canonical_work_id(&project_root, Some("work/foo"), None).unwrap();
    assert!(
        reloaded.work_items.iter().all(|item| item.id != mega_id),
        "legacy mega-item must be removed"
    );
    let develop = reloaded
        .work_items
        .iter()
        .find(|item| item.id == develop_id)
        .expect("develop item");
    assert_eq!(
        develop.title, "develop PR 監視",
        "last develop event title wins (origin/develop normalizes to develop)"
    );
    assert_eq!(develop.events.len(), 2);
    let develop_sessions: Vec<_> = develop
        .agents
        .iter()
        .map(|agent| agent.session_id.as_str())
        .collect();
    assert!(develop_sessions.contains(&"sess-dev-1"));
    assert!(develop_sessions.contains(&"sess-dev-2"));
    let foo = reloaded
        .work_items
        .iter()
        .find(|item| item.id == foo_id)
        .expect("work/foo item");
    assert_eq!(foo.title, "foo の実装");
    assert_eq!(foo.agents.len(), 1);

    let second = decompose_legacy_multi_branch_work_items_paths(&work_items_path, &project_root)
        .expect("second run");
    assert_eq!(
        second, 0,
        "idempotent: canonical items are not re-decomposed"
    );
}

/// SPEC-2359 Phase W-16 (FR-393): single-branch items (the normal
/// work-session shape) are left untouched by the decomposition.
#[test]
fn single_branch_work_items_are_not_decomposed() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project_root = temp.path().join("repo");
    let work_items_path = temp.path().join("works.json");
    let t0 = Utc.with_ymd_and_hms(2026, 6, 10, 10, 0, 0).unwrap();

    let mut projection = WorkItemsProjection::empty(t0);
    projection.apply_event(legacy_event(
        "work-session-abc",
        Some("work/bar"),
        Some("bar の作業"),
        Some("sess-bar"),
        t0,
    ));
    save_workspace_work_items_projection_to_path(&work_items_path, &projection)
        .expect("seed works");

    let decomposed =
        decompose_legacy_multi_branch_work_items_paths(&work_items_path, &project_root)
            .expect("decompose");
    assert_eq!(decomposed, 0);
    let reloaded = load_workspace_work_items_from_path(&work_items_path)
        .expect("load works")
        .expect("projection exists");
    assert_eq!(reloaded.work_items.len(), 1);
    assert_eq!(reloaded.work_items[0].id, "work-session-abc");
}

/// SPEC-2359 Phase W-16 (FR-403 follow-up): a Backfill event is a
/// synthetic materialization marker, not activity. Re-applying one (e.g.
/// replaying a duplicated backfill line) must not advance an existing
/// item's `updated_at` — otherwise hundreds of rows collapse onto the
/// replay instant and the recency sort degenerates.
#[test]
fn backfill_event_does_not_bump_updated_at_of_existing_item() {
    let t_old = Utc.with_ymd_and_hms(2026, 5, 18, 9, 15, 0).unwrap();
    let t_backfill = Utc.with_ymd_and_hms(2026, 6, 10, 6, 19, 47).unwrap();
    let mut projection = WorkItemsProjection::empty(t_old);
    let mut start = WorkEvent::new(WorkEventKind::Update, "work-x", t_old);
    start.title = Some("作業中".to_string());
    projection.apply_event(start);
    assert_eq!(projection.work_items[0].updated_at, t_old);

    let mut backfill = WorkEvent::new(WorkEventKind::Backfill, "work-x", t_backfill);
    backfill.title = Some("work/x".to_string());
    projection.apply_event(backfill);

    let item = &projection.work_items[0];
    assert_eq!(
        item.updated_at, t_old,
        "backfill must not advance an existing item's updated_at"
    );
    assert_eq!(
        item.title, "作業中",
        "backfill must not overwrite a real title"
    );

    // A brand-new item still gets the backfill time as its baseline.
    let mut fresh = WorkEvent::new(WorkEventKind::Backfill, "work-new", t_backfill);
    fresh.title = Some("work/new".to_string());
    projection.apply_event(fresh);
    let fresh_item = projection
        .work_items
        .iter()
        .find(|item| item.id == "work-new")
        .expect("new item");
    assert_eq!(fresh_item.updated_at, t_backfill);
}

/// SPEC-2359 Phase W-16 (FR-403 follow-up): a backfilled worktree's
/// baseline timestamp is the worktree directory's mtime (its last real
/// activity), not "now" — otherwise every freshly materialized old
/// worktree floods the top of the recency-sorted list.
#[test]
fn backfill_uses_worktree_mtime_as_baseline_timestamp() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project_root = temp.path().join("repo");
    let worktree = temp.path().join("repo-old");
    fs::create_dir_all(&worktree).expect("worktree dir");
    let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_750_000_000); // 2025-06-15ish
    let dir = open_directory_for_mtime(&worktree).expect("open dir");
    dir.set_times(fs::FileTimes::new().set_modified(old))
        .expect("set mtime");
    let work_items_path = temp.path().join("works.json");
    let now = Utc.with_ymd_and_hms(2026, 6, 10, 12, 0, 0).unwrap();

    reconcile_worktree_work_items_paths(
        &work_items_path,
        &project_root,
        &[WorktreeReconcileSource {
            branch: Some("work/old".to_string()),
            worktree_path: worktree.clone(),
        }],
        now,
    )
    .expect("reconcile");

    let projection = load_workspace_work_items_from_path(&work_items_path)
        .expect("load works")
        .expect("projection exists");
    let item = &projection.work_items[0];
    assert!(
        item.updated_at < Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        "baseline must be the worktree mtime (2025), not now: {}",
        item.updated_at
    );
}

/// SPEC-2359 Phase W-16 (FR-403 follow-up): for a git worktree the
/// backfill baseline is the HEAD committer time — directory mtime is
/// polluted by unrelated writes (e.g. the backfill itself creating
/// `.gwt/`), which collapsed mid-list ordering onto one instant.
#[test]
fn backfill_uses_head_commit_time_for_git_worktrees() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project_root = temp.path().join("repo");
    let worktree = temp.path().join("repo-wt");
    fs::create_dir_all(&worktree).expect("worktree dir");
    for args in [
        ["init", "-q"].as_slice(),
        ["config", "user.email", "t@example.com"].as_slice(),
        ["config", "user.name", "T"].as_slice(),
    ] {
        let output = crate::process::hidden_command("git")
            .args(args)
            .current_dir(&worktree)
            .output()
            .expect("git");
        assert!(output.status.success());
    }
    let mut commit = crate::process::hidden_command("git");
    commit
        .args(["commit", "--allow-empty", "-m", "old"])
        .env("GIT_COMMITTER_DATE", "2025-06-15T00:00:00Z")
        .env("GIT_AUTHOR_DATE", "2025-06-15T00:00:00Z")
        .current_dir(&worktree);
    assert!(commit.output().expect("commit").status.success());

    let work_items_path = temp.path().join("works.json");
    let now = Utc.with_ymd_and_hms(2026, 6, 11, 12, 0, 0).unwrap();
    reconcile_worktree_work_items_paths(
        &work_items_path,
        &project_root,
        &[WorktreeReconcileSource {
            branch: Some("work/old".to_string()),
            worktree_path: worktree.clone(),
        }],
        now,
    )
    .expect("reconcile");

    let projection = load_workspace_work_items_from_path(&work_items_path)
        .expect("load works")
        .expect("projection exists");
    assert_eq!(
        projection.work_items[0].updated_at,
        Utc.with_ymd_and_hms(2025, 6, 15, 0, 0, 0).unwrap(),
        "git worktree baseline is the HEAD committer time"
    );
}

// #3065: the resume context source lookup — a work item is found by
// canonical branch identity (local or origin/ prefixed), by worktree
// path, and misses cleanly for unknown containers.
#[test]
fn find_work_item_for_container_matches_branch_worktree_and_id() {
    let project_root = std::path::Path::new("/repo");
    let now = Utc.timestamp_opt(9_000, 0).unwrap();
    let mut projection = WorkItemsProjection::empty(now);
    let work_id = canonical_work_id(project_root, Some("work/foo"), None).expect("canonical id");
    let mut event = WorkEvent::new(WorkEventKind::Backfill, work_id.clone(), now);
    event.title = Some("work/foo".to_string());
    event.execution_container = Some(WorkspaceExecutionContainerRef {
        branch: Some("work/foo".to_string()),
        worktree_path: Some(PathBuf::from("/wt/foo")),
        pr_number: None,
        pr_url: None,
        pr_state: None,
    });
    projection.apply_event(event);

    let by_branch = find_work_item_for_container(&projection, project_root, Some("work/foo"), None)
        .expect("matched by branch");
    assert_eq!(by_branch.id, work_id);
    let by_remote =
        find_work_item_for_container(&projection, project_root, Some("origin/work/foo"), None)
            .expect("matched by remote-prefixed branch");
    assert_eq!(by_remote.id, work_id);
    let by_worktree = find_work_item_for_container(
        &projection,
        project_root,
        None,
        Some(std::path::Path::new("/wt/foo")),
    )
    .expect("matched by worktree path");
    assert_eq!(by_worktree.id, work_id);
    assert!(
        find_work_item_for_container(&projection, project_root, Some("work/other"), None).is_none()
    );
}

#[test]
fn work_item_latest_next_action_reads_most_recent_event() {
    let now = Utc.timestamp_opt(9_100, 0).unwrap();
    let later = Utc.timestamp_opt(9_200, 0).unwrap();
    let mut projection = WorkItemsProjection::empty(now);
    let mut first = WorkEvent::new(WorkEventKind::Start, "work-x", now);
    first.next_action = Some("older action".to_string());
    projection.apply_event(first);
    let mut second = WorkEvent::new(WorkEventKind::Update, "work-x", later);
    second.next_action = Some("newer action".to_string());
    projection.apply_event(second);

    assert_eq!(
        projection.work_items[0].latest_next_action(),
        Some("newer action")
    );
}

fn bleed_test_agent(session_id: &str, workspace_id: &str) -> WorkspaceAgentSummary {
    WorkspaceAgentSummary {
        session_id: session_id.to_string(),
        window_id: None,
        agent_id: "codex".to_string(),
        display_name: "Codex".to_string(),
        status_category: WorkspaceStatusCategory::Active,
        current_focus: None,
        title_summary: None,
        worktree_path: None,
        branch: None,
        last_board_entry_id: None,
        last_board_entry_kind: None,
        coordination_scope: None,
        affiliation_status: WorkspaceAgentAffiliationStatus::Assigned,
        workspace_id: Some(workspace_id.to_string()),
        updated_at: Utc.timestamp_opt(1_000, 0).unwrap(),
    }
}

fn bleed_resume_event(work_id: &str, seq: i64) -> WorkEvent {
    let mut event = WorkEvent::new(
        WorkEventKind::Resume,
        work_id,
        Utc.timestamp_opt(20_000 + seq, 0).unwrap(),
    );
    event.title = Some("gwt-manage-pr".to_string());
    event.owner = Some("SPEC-2359".to_string());
    event.summary = Some("765 active agents".to_string());
    event.next_action = Some("merged build re-check".to_string());
    event.status_category = Some(WorkspaceStatusCategory::Active);
    event.agent_session_id = Some(format!("sess-{seq}"));
    event
}

fn bleed_backfill_event(work_id: &str, branch: &str, seq: i64) -> WorkEvent {
    let mut event = WorkEvent::new(
        WorkEventKind::Backfill,
        work_id,
        Utc.timestamp_opt(10_000 + seq, 0).unwrap(),
    );
    event.title = Some(branch.to_string());
    event.execution_container = Some(WorkspaceExecutionContainerRef {
        branch: Some(branch.to_string()),
        worktree_path: Some(PathBuf::from(format!("/wt/{branch}"))),
        pr_number: None,
        pr_url: None,
        pr_state: None,
    });
    event
}

// #3065: the repair detects the bleed signature — an identical
// (title, owner, next_action) resume payload stamped onto 3+ distinct
// work items — sanitizes every event carrying the contaminated
// (title, owner) identity (resume AND pause/update stamps), re-folds the
// items, and clears the contaminated shared current projection even when
// its next_action has drifted. Idempotent: a second run is a no-op.
#[test]
fn repair_resume_owner_bleed_sanitizes_cross_item_stamp() {
    let temp = tempfile::tempdir().expect("tempdir");
    let work_items_path = temp.path().join("works.json");
    let current_path = temp.path().join("current.json");
    let now = Utc.timestamp_opt(30_000, 0).unwrap();

    let mut projection = WorkItemsProjection::empty(now);
    for (index, branch) in ["work/a", "work/b", "work/c"].iter().enumerate() {
        let work_id = format!("work-{}-0000000{index}", branch.replace('/', "-"));
        projection.apply_event(bleed_backfill_event(&work_id, branch, index as i64));
        projection.apply_event(bleed_resume_event(&work_id, index as i64));
    }
    // A pause stamp carrying the same contaminated identity on a fourth
    // item (the work-session-* leak path) is sanitized by the pair rule.
    let mut pause = WorkEvent::new(
        WorkEventKind::Pause,
        "work-session-sess-dead",
        Utc.timestamp_opt(20_900, 0).unwrap(),
    );
    pause.title = Some("gwt-manage-pr".to_string());
    pause.owner = Some("SPEC-2359".to_string());
    projection.apply_event(pause);
    // An update stamp with an agent-authored title but the contaminated
    // owner (the update/done leak) loses only its owner; the title and
    // the rest of the payload survive.
    let mut update = WorkEvent::new(
        WorkEventKind::Update,
        "work-work-d-00000003",
        Utc.timestamp_opt(20_950, 0).unwrap(),
    );
    update.title = Some("agent authored title".to_string());
    update.owner = Some("SPEC-2359".to_string());
    update.status_category = Some(WorkspaceStatusCategory::Active);
    projection.apply_event(update);
    let event_ids_before: std::collections::BTreeSet<String> = projection
        .work_items
        .iter()
        .flat_map(|item| item.events.iter().map(|event| event.id.clone()))
        .collect();
    save_workspace_work_items_projection_to_path(&work_items_path, &projection)
        .expect("save works");

    let mut current = WorkspaceProjection::default_for_project("/repo");
    current.id = "work-work-a-00000000".to_string();
    current.title = "gwt-manage-pr".to_string();
    current.owner = Some("SPEC-2359".to_string());
    // next_action drifted after the stamps were written; the pair rule
    // must still clear the identity.
    current.next_action = Some("Check Board for latest updates".to_string());
    current.status_text = "883 active agents".to_string();
    for seq in 0..5 {
        current.agents.push(bleed_test_agent(
            &format!("dead-{seq}"),
            &format!("work-other-{seq}"),
        ));
    }
    save_workspace_projection_to_path(&current_path, &current).expect("save current");

    let report =
        repair_resume_owner_bleed_paths(&work_items_path, &current_path, now).expect("repair");
    assert_eq!(
        report.sanitized_events, 5,
        "3 resume stamps + 1 pause stamp + 1 owner-only update stamp"
    );
    assert!(report.cleared_current, "current.json identity cleared");

    let repaired = load_workspace_work_items_from_path(&work_items_path)
        .expect("load works")
        .expect("projection exists");
    for item in &repaired.work_items {
        assert_eq!(item.owner, None, "owner cleared for {}", item.id);
        if item.id == "work-work-d-00000003" {
            assert_eq!(
                item.title, "agent authored title",
                "owner-only sanitize keeps the agent-authored title"
            );
        } else if item.id != "work-session-sess-dead" {
            assert!(
                item.title.starts_with("work/"),
                "title restored to branch name, got {}",
                item.title
            );
        }
    }
    let event_ids_after: std::collections::BTreeSet<String> = repaired
        .work_items
        .iter()
        .flat_map(|item| item.events.iter().map(|event| event.id.clone()))
        .collect();
    assert_eq!(
        event_ids_before, event_ids_after,
        "sanitized events keep their ids so the intake dedup still skips them"
    );

    let repaired_current = load_workspace_projection_from_path(&current_path)
        .expect("load current")
        .expect("current exists");
    assert_eq!(repaired_current.owner, None);
    assert_eq!(repaired_current.next_action, None);
    assert!(repaired_current.agents.is_empty(), "dead agents purged");

    let second = repair_resume_owner_bleed_paths(&work_items_path, &current_path, now)
        .expect("repair rerun");
    assert_eq!(second.sanitized_events, 0, "second run is a no-op");
    assert!(!second.cleared_current);
}

// #3065: two work items legitimately sharing the same owner/title (e.g.
// two branches working one SPEC) stay untouched — the signature requires
// 3+ distinct work items.
#[test]
fn repair_resume_owner_bleed_keeps_legitimate_duplicates_below_threshold() {
    let temp = tempfile::tempdir().expect("tempdir");
    let work_items_path = temp.path().join("works.json");
    let current_path = temp.path().join("current.json");
    let now = Utc.timestamp_opt(31_000, 0).unwrap();

    let mut projection = WorkItemsProjection::empty(now);
    for (index, branch) in ["work/a", "work/b"].iter().enumerate() {
        let work_id = format!("work-{}-0000000{index}", branch.replace('/', "-"));
        projection.apply_event(bleed_backfill_event(&work_id, branch, index as i64));
        projection.apply_event(bleed_resume_event(&work_id, index as i64));
    }
    save_workspace_work_items_projection_to_path(&work_items_path, &projection)
        .expect("save works");

    let report =
        repair_resume_owner_bleed_paths(&work_items_path, &current_path, now).expect("repair");
    assert_eq!(report.sanitized_events, 0);

    let untouched = load_workspace_work_items_from_path(&work_items_path)
        .expect("load works")
        .expect("projection exists");
    assert!(
        untouched
            .work_items
            .iter()
            .all(|item| item.owner.as_deref() == Some("SPEC-2359")),
        "below-threshold duplicates keep their owner"
    );
}

// SPEC-3075 FR-003/FR-004: a Board status post carries the Work *status*
// (a point-in-time snapshot), not its *purpose* (identity). Its body must
// never become the Work title; only `summary` may carry the body. This is the
// structural fix for "the summary is a status snapshot, not what the Work is".
#[test]
fn board_status_body_without_title_summary_keeps_work_identity() {
    let mut projection = WorkspaceProjection::default_for_project("/repo");
    projection.title = "Workspace 要約の目的第一化".to_string();

    let entry = crate::coordination::BoardEntry::new(
        crate::coordination::AuthorKind::Agent,
        "agent-1",
        crate::coordination::BoardEntryKind::Status,
        "現在の状態: PR #3007 をマージ完了。",
        None,
        None,
        Vec::new(),
        Vec::new(),
    );

    let event = workspace_work_event_from_board_entry(&projection, &entry);

    // identity (purpose) is preserved; the status body lives only in summary.
    assert_eq!(
        event.title.as_deref(),
        Some("Workspace 要約の目的第一化"),
        "Board status body must not overwrite the Work identity (title)"
    );
    assert_eq!(
        event.summary.as_deref(),
        Some("現在の状態: PR #3007 をマージ完了。"),
        "Board body is retained as status summary only"
    );
}

// SPEC-3075: when the agent declares a purpose via `title_summary`, that is the
// identity — still independent of the status body.
#[test]
fn board_entry_title_summary_is_the_work_identity() {
    let mut projection = WorkspaceProjection::default_for_project("/repo");
    projection.title = "old projection title".to_string();

    let mut entry = crate::coordination::BoardEntry::new(
        crate::coordination::AuthorKind::Agent,
        "agent-1",
        crate::coordination::BoardEntryKind::Status,
        "現在の状態: テスト実行中。",
        None,
        None,
        Vec::new(),
        Vec::new(),
    );
    entry.title_summary = Some("認証機能の実装".to_string());

    let event = workspace_work_event_from_board_entry(&projection, &entry);

    assert_eq!(event.title.as_deref(), Some("認証機能の実装"));
    assert_eq!(event.intent.as_deref(), Some("認証機能の実装"));
    assert_eq!(event.summary.as_deref(), Some("現在の状態: テスト実行中。"));
}

// SPEC-2359 T-814 (US-86 / FR-551 / FR-552): Board history is durable on its
// own, but a Board entry may mutate Work state only after its origin resolves
// to one existing, current project+Session target. These fixtures stay at the
// end of this file so T-812 can evolve its independent transaction tests.
const SESSION_BOUND_BOARD_CURRENT_WORK_ID: &str = "work-session-bound-board-current";
const SESSION_BOUND_BOARD_TARGET_WORK_ID: &str = "work-session-bound-board-target";
const SESSION_BOUND_BOARD_VALID_SESSION_ID: &str = "session-bound-board-valid";
const SESSION_BOUND_BOARD_STALE_SESSION_ID: &str = "session-bound-board-stale";
const SESSION_BOUND_BOARD_FOREIGN_SESSION_ID: &str = "session-bound-board-foreign";

struct SessionBoundBoardFixture {
    project_root: PathBuf,
    current_path: PathBuf,
    work_items_path: PathBuf,
    events_path: PathBuf,
    journal_path: PathBuf,
    base_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionBoundBoardWorkBytes {
    current: Option<Vec<u8>>,
    work_items: Option<Vec<u8>>,
    events: Option<Vec<u8>>,
    journal: Option<Vec<u8>>,
}

fn session_bound_board_optional_bytes(path: &Path) -> Option<Vec<u8>> {
    match fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => panic!("read {}: {error}", path.display()),
    }
}

fn session_bound_board_work_bytes(
    fixture: &SessionBoundBoardFixture,
) -> SessionBoundBoardWorkBytes {
    SessionBoundBoardWorkBytes {
        current: session_bound_board_optional_bytes(&fixture.current_path),
        work_items: session_bound_board_optional_bytes(&fixture.work_items_path),
        events: session_bound_board_optional_bytes(&fixture.events_path),
        journal: session_bound_board_optional_bytes(&fixture.journal_path),
    }
}

fn session_bound_board_container(
    project_root: &Path,
    branch: &str,
) -> WorkspaceExecutionContainerRef {
    WorkspaceExecutionContainerRef {
        branch: Some(branch.to_string()),
        worktree_path: Some(project_root.join(branch.replace('/', "-"))),
        pr_number: None,
        pr_url: None,
        pr_state: None,
    }
}

fn session_bound_board_assigned_agent(
    project_root: &Path,
    session_id: &str,
    work_id: &str,
    branch: &str,
    updated_at: DateTime<Utc>,
) -> WorkspaceAgentSummary {
    let mut agent = assigned_agent(session_id, "codex", work_id);
    agent.branch = Some(branch.to_string());
    agent.worktree_path = Some(project_root.join(branch.replace('/', "-")));
    agent.updated_at = updated_at;
    agent
}

fn session_bound_board_seed_fixture(temp: &Path) -> SessionBoundBoardFixture {
    let project_root = temp.join("repo");
    let current_path = temp.join("state/current.json");
    let work_items_path = temp.join("state/works.json");
    let events_path = project_root.join(".gwt/work/events.jsonl");
    let journal_path = temp.join("state/journal.jsonl");
    let base_at = Utc.with_ymd_and_hms(2026, 7, 22, 8, 0, 0).unwrap();
    fs::create_dir_all(&project_root).expect("create Board test project root");

    let current_branch = "work/session-bound-board-current";
    let target_branch = "work/session-bound-board-target";
    let foreign_branch = "work/session-bound-board-foreign";
    let mut current = WorkspaceProjection::default_for_project(&project_root);
    current.id = SESSION_BOUND_BOARD_CURRENT_WORK_ID.to_string();
    current.title = "Unrelated shared current Work".to_string();
    current.summary = Some("Shared current summary must stay unchanged".to_string());
    current.owner = Some("SPEC-shared-current".to_string());
    current.updated_at = base_at;
    current.git_details = Some(GitDetails {
        branch: Some(current_branch.to_string()),
        worktree_path: Some(project_root.join("current-worktree")),
        base_branch: Some("develop".to_string()),
        pr_number: None,
        pr_url: None,
        pr_state: None,
        pr_created_at: None,
        created_by_start_work: false,
        created_at: base_at,
    });
    current.agents = vec![
        session_bound_board_assigned_agent(
            &project_root,
            SESSION_BOUND_BOARD_VALID_SESSION_ID,
            SESSION_BOUND_BOARD_TARGET_WORK_ID,
            target_branch,
            base_at,
        ),
        session_bound_board_assigned_agent(
            &project_root,
            SESSION_BOUND_BOARD_STALE_SESSION_ID,
            SESSION_BOUND_BOARD_TARGET_WORK_ID,
            target_branch,
            base_at + chrono::Duration::hours(2),
        ),
        session_bound_board_assigned_agent(
            &project_root,
            SESSION_BOUND_BOARD_FOREIGN_SESSION_ID,
            "work-session-bound-board-foreign-missing",
            foreign_branch,
            base_at,
        ),
    ];
    save_workspace_projection_to_path(&current_path, &current).expect("seed shared current");

    let mut current_start = WorkEvent::new(
        WorkEventKind::Start,
        SESSION_BOUND_BOARD_CURRENT_WORK_ID,
        base_at,
    );
    current_start.id = "event-session-bound-board-current".to_string();
    current_start.title = Some("Unrelated shared current Work".to_string());
    current_start.execution_container =
        Some(session_bound_board_container(&project_root, current_branch));

    let mut target_start = WorkEvent::new(
        WorkEventKind::Start,
        SESSION_BOUND_BOARD_TARGET_WORK_ID,
        base_at,
    );
    target_start.id = "event-session-bound-board-target".to_string();
    target_start.title = Some("Board target Work".to_string());
    target_start.agent_session_id = Some(SESSION_BOUND_BOARD_VALID_SESSION_ID.to_string());
    target_start.execution_container =
        Some(session_bound_board_container(&project_root, target_branch));

    let mut stale_agent_attach = WorkEvent::new(
        WorkEventKind::Update,
        SESSION_BOUND_BOARD_TARGET_WORK_ID,
        base_at,
    );
    stale_agent_attach.id = "event-session-bound-board-stale-agent".to_string();
    stale_agent_attach.agent_session_id = Some(SESSION_BOUND_BOARD_STALE_SESSION_ID.to_string());
    stale_agent_attach.execution_container =
        Some(session_bound_board_container(&project_root, target_branch));

    record_workspace_work_events_paths(
        &work_items_path,
        &events_path,
        vec![current_start, target_start, stale_agent_attach],
    )
    .expect("seed Board Work targets");

    SessionBoundBoardFixture {
        project_root,
        current_path,
        work_items_path,
        events_path,
        journal_path,
        base_at,
    }
}

fn session_bound_board_entry(
    id: &str,
    origin_session_id: Option<&str>,
    origin_branch: &str,
    updated_at: DateTime<Utc>,
) -> crate::coordination::BoardEntry {
    let mut entry = crate::coordination::BoardEntry::new(
        crate::coordination::AuthorKind::Agent,
        "agent-board-origin",
        crate::coordination::BoardEntryKind::Status,
        format!("Board milestone {id}"),
        None,
        None,
        Vec::new(),
        vec!["SPEC-board-target".to_string()],
    );
    entry.id = id.to_string();
    entry.created_at = updated_at;
    entry.updated_at = updated_at;
    entry.origin_session_id = origin_session_id.map(str::to_string);
    entry.origin_branch = Some(origin_branch.to_string());
    entry.origin_agent_id = Some("agent-board-origin".to_string());
    entry
}

fn session_bound_board_origin_cases(
    fixture: &SessionBoundBoardFixture,
) -> Vec<(&'static str, crate::coordination::BoardEntry)> {
    let target_branch = "work/session-bound-board-target";
    vec![
        (
            "valid",
            session_bound_board_entry(
                "board-session-bound-valid",
                Some(SESSION_BOUND_BOARD_VALID_SESSION_ID),
                target_branch,
                fixture.base_at + chrono::Duration::hours(3),
            ),
        ),
        (
            "missing",
            session_bound_board_entry(
                "board-session-bound-missing",
                None,
                target_branch,
                fixture.base_at + chrono::Duration::hours(3),
            ),
        ),
        (
            "invalid",
            session_bound_board_entry(
                "board-session-bound-invalid",
                Some("../unsafe-session"),
                target_branch,
                fixture.base_at + chrono::Duration::hours(3),
            ),
        ),
        (
            "foreign",
            session_bound_board_entry(
                "board-session-bound-foreign",
                Some(SESSION_BOUND_BOARD_FOREIGN_SESSION_ID),
                "work/session-bound-board-foreign",
                fixture.base_at + chrono::Duration::hours(3),
            ),
        ),
        (
            "stale",
            session_bound_board_entry(
                "board-session-bound-stale",
                Some(SESSION_BOUND_BOARD_STALE_SESSION_ID),
                target_branch,
                fixture.base_at + chrono::Duration::hours(1),
            ),
        ),
    ]
}

fn session_bound_board_candidate_work_event(
    projection: &WorkspaceProjection,
    work_items: &WorkItemsProjection,
    entry: &crate::coordination::BoardEntry,
) -> Option<WorkEvent> {
    resolve_workspace_work_event_from_board_entry(projection, work_items, entry)
}

fn session_bound_board_apply_entry(
    fixture: &SessionBoundBoardFixture,
    entry: &crate::coordination::BoardEntry,
) -> Result<()> {
    transact_workspace_state_at(
        &fixture.current_path,
        &fixture.work_items_path,
        &fixture.events_path,
        &fixture.project_root,
        |projection, work_items, _| {
            let Some(event) =
                session_bound_board_candidate_work_event(projection, work_items, entry)
            else {
                return Ok(((), Vec::new()));
            };
            let state_cutoff = work_items
                .work_items
                .iter()
                .find(|item| item.id == event.work_item_id)
                .map(|item| item.updated_at);
            if event.work_item_id == projection.id {
                projection.record_board_milestone_with_state_cutoff(entry, state_cutoff);
            }
            Ok(((), vec![event]))
        },
    )
}

#[test]
fn session_bound_board_origin_matrix_resolves_only_valid_target() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fixture = session_bound_board_seed_fixture(temp.path());
    let projection = load_workspace_projection_from_path(&fixture.current_path)
        .expect("load current")
        .expect("current exists");
    let work_items = load_workspace_work_items_from_path(&fixture.work_items_path)
        .expect("load Work items")
        .expect("Work items exist");

    let actual = session_bound_board_origin_cases(&fixture)
        .iter()
        .map(|(case, entry)| {
            (
                *case,
                session_bound_board_candidate_work_event(&projection, &work_items, entry)
                    .map(|event| event.work_item_id),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        actual,
        vec![
            (
                "valid",
                Some(SESSION_BOUND_BOARD_TARGET_WORK_ID.to_string())
            ),
            ("missing", None),
            ("invalid", None),
            ("foreign", None),
            ("stale", None),
        ],
        "only a current origin resolving to an existing target Work may produce a Work event"
    );
}

#[test]
fn session_bound_board_origin_without_target_container_remains_board_only() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fixture = session_bound_board_seed_fixture(temp.path());
    let projection = load_workspace_projection_from_path(&fixture.current_path)
        .expect("load current")
        .expect("current exists");
    let mut work_items = load_workspace_work_items_from_path(&fixture.work_items_path)
        .expect("load Work items")
        .expect("Work items exist");
    let target = work_items
        .work_items
        .iter_mut()
        .find(|item| item.id == SESSION_BOUND_BOARD_TARGET_WORK_ID)
        .expect("target Work item");
    target.execution_containers.clear();
    let entry = session_bound_board_origin_cases(&fixture).remove(0).1;

    assert!(
        session_bound_board_candidate_work_event(&projection, &work_items, &entry).is_none(),
        "a target without a matching execution container must remain Board-only"
    );
}

#[test]
fn session_bound_board_invalid_origins_remain_board_only_with_byte_equivalent_work_state() {
    for (case, entry_index) in [("missing", 1), ("invalid", 2), ("foreign", 3), ("stale", 4)] {
        let temp = tempfile::tempdir().expect("tempdir");
        let fixture = session_bound_board_seed_fixture(temp.path());
        let entry = session_bound_board_origin_cases(&fixture)
            .remove(entry_index)
            .1;
        let before = session_bound_board_work_bytes(&fixture);

        let result = session_bound_board_apply_entry(&fixture, &entry);
        let after = session_bound_board_work_bytes(&fixture);

        assert!(result.is_ok(), "{case} Board-only route must be a no-op");
        assert!(
            after == before,
            "{case} Board origin must stay Board-only without a Work event, projection mutation, or partial ref"
        );
    }
}

#[test]
fn session_bound_board_valid_origin_updates_target_work_only() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fixture = session_bound_board_seed_fixture(temp.path());
    let entry = session_bound_board_origin_cases(&fixture).remove(0).1;
    let current_before = fs::read(&fixture.current_path).expect("current before");

    session_bound_board_apply_entry(&fixture, &entry).expect("valid Board origin");

    assert!(
        fs::read(&fixture.current_path).expect("current after") == current_before,
        "a valid origin targeting another Work must not mutate shared current"
    );
    let work_items = load_workspace_work_items_from_path(&fixture.work_items_path)
        .expect("load Work items")
        .expect("Work items exist");
    let current = work_items
        .work_items
        .iter()
        .find(|item| item.id == SESSION_BOUND_BOARD_CURRENT_WORK_ID)
        .expect("shared current Work item");
    let target = work_items
        .work_items
        .iter()
        .find(|item| item.id == SESSION_BOUND_BOARD_TARGET_WORK_ID)
        .expect("target Work item");
    assert!(!current.board_refs.contains(&entry.id));
    assert!(target.board_refs.contains(&entry.id));
}

#[test]
fn session_bound_board_store_rejects_cross_work_non_attach_mutation_byte_equivalent() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fixture = session_bound_board_seed_fixture(temp.path());
    let before = session_bound_board_work_bytes(&fixture);
    let result = transact_workspace_state_at(
        &fixture.current_path,
        &fixture.work_items_path,
        &fixture.events_path,
        &fixture.project_root,
        |_projection, _work_items, _| {
            let mut event = WorkEvent::new(
                WorkEventKind::Update,
                SESSION_BOUND_BOARD_TARGET_WORK_ID,
                fixture.base_at + chrono::Duration::hours(4),
            );
            event.id = "event-session-bound-board-cross-work".to_string();
            event.title = Some("Cross-Work contamination".to_string());
            event.board_entry_id = Some("board-session-bound-cross-work".to_string());
            // Deliberately no agent_session_id: this is a non-attach mutation
            // and therefore bypasses the existing stray-attach guard.
            Ok(((), vec![event]))
        },
    );
    let after = session_bound_board_work_bytes(&fixture);

    assert!(
        result.is_err(),
        "the store must reject a non-attach event targeting a Work other than the transaction principal"
    );
    assert!(
        after == before,
        "a rejected cross-Work mutation must preserve current/work/event/journal bytes"
    );
}

#[test]
fn session_bound_board_store_rejects_orphan_board_ref_byte_equivalent() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fixture = session_bound_board_seed_fixture(temp.path());
    let before = session_bound_board_work_bytes(&fixture);
    let result = transact_workspace_state_at(
        &fixture.current_path,
        &fixture.work_items_path,
        &fixture.events_path,
        &fixture.project_root,
        |projection, _work_items, _| {
            projection
                .board_refs
                .push("board-session-bound-orphan".to_string());
            Ok(((), Vec::new()))
        },
    );
    let after = session_bound_board_work_bytes(&fixture);

    assert!(
        result.is_err(),
        "a Board ref without its authorized Work event must reject atomically"
    );
    assert!(
        after == before,
        "a rejected orphan Board ref must preserve current/work/event/journal bytes"
    );
}

#[test]
fn session_bound_board_event_uses_canonical_agent_metadata_without_provider_actor_id() {
    const RAW_PROVIDER_ACTOR_ID: &str = "provider-thread-private-sentinel-board-86";

    let temp = tempfile::tempdir().expect("tempdir");
    let fixture = session_bound_board_seed_fixture(temp.path());
    let projection = load_workspace_projection_from_path(&fixture.current_path)
        .expect("load current")
        .expect("current exists");
    let work_items = load_workspace_work_items_from_path(&fixture.work_items_path)
        .expect("load Work items")
        .expect("Work items exist");
    let mut entry = session_bound_board_origin_cases(&fixture).remove(0).1;
    entry.origin_agent_id = Some(RAW_PROVIDER_ACTOR_ID.to_string());

    let event = session_bound_board_candidate_work_event(&projection, &work_items, &entry)
        .expect("valid Board origin");

    assert_eq!(event.agent_id.as_deref(), Some("codex"));
    assert_eq!(event.display_name.as_deref(), Some("codex"));
    assert!(
        !serde_json::to_string(&event)
            .expect("serialize Board Work event")
            .contains(RAW_PROVIDER_ACTOR_ID),
        "untrusted provider actor metadata must not enter the tracked Work event"
    );
}

#[test]
fn session_bound_board_store_rejects_reassignment_then_update_authority_escalation() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fixture = session_bound_board_seed_fixture(temp.path());
    let before = session_bound_board_work_bytes(&fixture);
    let foreign_work_id = "work-session-bound-board-reassigned-in-closure";
    let result = transact_workspace_state_at(
        &fixture.current_path,
        &fixture.work_items_path,
        &fixture.events_path,
        &fixture.project_root,
        |projection, _work_items, _| {
            assert!(projection.assign_agent(
                SESSION_BOUND_BOARD_VALID_SESSION_ID,
                foreign_work_id,
                None,
                None,
                fixture.base_at + chrono::Duration::hours(4),
            ));
            let mut event = WorkEvent::new(
                WorkEventKind::Update,
                foreign_work_id,
                fixture.base_at + chrono::Duration::hours(4),
            );
            event.agent_session_id = Some(SESSION_BOUND_BOARD_VALID_SESSION_ID.to_string());
            event.summary = Some("post-update assignment must not grant authority".to_string());
            Ok(((), vec![event]))
        },
    );
    let after = session_bound_board_work_bytes(&fixture);

    assert!(
        result.is_err(),
        "an Update must use pre-transaction assignment authority"
    );
    assert!(
        after == before,
        "rejected in-closure reassignment must preserve current/work/event/journal bytes"
    );
}
