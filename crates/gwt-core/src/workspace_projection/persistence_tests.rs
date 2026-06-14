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

/// Override `HOME` for the duration of a test so the home-side projection
/// writes (works.json, project-state) and the legacy migration sources
/// resolve under an isolated temp directory. Restores the previous value
/// on drop.
struct ScopedHome {
    previous_home: Option<std::ffi::OsString>,
}

impl ScopedHome {
    fn set(path: &Path) -> Self {
        let previous_home = std::env::var_os("HOME");
        std::env::set_var("HOME", path);
        Self { previous_home }
    }
}

impl Drop for ScopedHome {
    fn drop(&mut self) {
        match self.previous_home.as_ref() {
            Some(previous) => std::env::set_var("HOME", previous),
            None => std::env::remove_var("HOME"),
        }
    }
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
        discarded,
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
