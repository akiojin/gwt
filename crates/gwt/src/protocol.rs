use gwt_agent::{ClaudeCodeOpenaiCompatInput, CustomCodingAgent, PresetDefinition};
use gwt_core::{
    coordination::{BoardEntry, BoardEntryKind},
    logging::LogEvent,
    notes::MemoNote,
};
use serde::{Deserialize, Serialize};

use crate::{
    branch_cleanup::BranchCleanupResultEntry,
    branch_list::BranchListEntry,
    daemon_runtime::RuntimeHookEvent,
    file_tree::FileTreeEntry,
    knowledge_bridge::{KnowledgeDetailView, KnowledgeKind, KnowledgeListItem, KnowledgeListScope},
    launch_wizard::{LaunchWizardAction, LaunchWizardView},
    persistence::{
        CanvasViewport, PersistedWindowState, ProjectKind, WindowGeometry, WindowProcessStatus,
    },
    preset::WindowPreset,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArrangeMode {
    Tile,
    Stack,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FocusCycleDirection {
    Forward,
    Backward,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchEntriesPhase {
    Inventory,
    Hydrated,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FrontendEvent {
    FrontendReady,
    OpenProjectDialog,
    ReopenRecentProject {
        path: String,
    },
    SelectProjectTab {
        tab_id: String,
    },
    CloseProjectTab {
        tab_id: String,
    },
    CreateWindow {
        preset: WindowPreset,
        bounds: WindowGeometry,
    },
    FocusWindow {
        id: String,
        bounds: Option<WindowGeometry>,
    },
    CycleFocus {
        direction: FocusCycleDirection,
        bounds: WindowGeometry,
    },
    UpdateViewport {
        viewport: CanvasViewport,
    },
    ArrangeWindows {
        mode: ArrangeMode,
        bounds: WindowGeometry,
    },
    MaximizeWindow {
        id: String,
        bounds: WindowGeometry,
    },
    MinimizeWindow {
        id: String,
    },
    RestoreWindow {
        id: String,
    },
    ListWindows,
    UpdateWindowGeometry {
        id: String,
        geometry: WindowGeometry,
        cols: u16,
        rows: u16,
    },
    CloseWindow {
        id: String,
    },
    TerminalInput {
        id: String,
        data: String,
    },
    LoadFileTree {
        id: String,
        path: Option<String>,
    },
    LoadBranches {
        id: String,
    },
    LoadBoard {
        id: String,
    },
    LoadBoardHistory {
        id: String,
        before_entry_id: Option<String>,
        #[serde(default = "default_board_history_limit")]
        limit: usize,
    },
    LoadProfile {
        id: String,
    },
    LoadMemo {
        id: String,
    },
    LoadLogs {
        id: String,
    },
    LoadKnowledgeBridge {
        id: String,
        knowledge_kind: KnowledgeKind,
        #[serde(default)]
        request_id: Option<u64>,
        selected_number: Option<u64>,
        refresh: bool,
        list_scope: Option<KnowledgeListScope>,
    },
    SearchKnowledgeBridge {
        id: String,
        knowledge_kind: KnowledgeKind,
        query: String,
        request_id: u64,
        selected_number: Option<u64>,
        list_scope: Option<KnowledgeListScope>,
    },
    SelectKnowledgeBridgeEntry {
        id: String,
        knowledge_kind: KnowledgeKind,
        #[serde(default)]
        request_id: Option<u64>,
        number: u64,
        list_scope: Option<KnowledgeListScope>,
    },
    RunBranchCleanup {
        id: String,
        branches: Vec<String>,
        delete_remote: bool,
    },
    PostBoardEntry {
        id: String,
        entry_kind: BoardEntryKind,
        body: String,
        parent_id: Option<String>,
        topics: Vec<String>,
        owners: Vec<String>,
        #[serde(default)]
        targets: Vec<String>,
    },
    CreateMemoNote {
        id: String,
        title: String,
        body: String,
        pinned: bool,
    },
    UpdateMemoNote {
        id: String,
        note_id: String,
        title: String,
        body: String,
        pinned: bool,
    },
    DeleteMemoNote {
        id: String,
        note_id: String,
    },
    SelectProfile {
        id: String,
        profile_name: String,
    },
    CreateProfile {
        id: String,
        name: String,
    },
    SetActiveProfile {
        id: String,
        profile_name: String,
    },
    SaveProfile {
        id: String,
        current_name: String,
        name: String,
        description: String,
        env_vars: Vec<ProfileEnvEntryView>,
        disabled_env: Vec<String>,
    },
    DeleteProfile {
        id: String,
        profile_name: String,
    },
    OpenIssueLaunchWizard {
        id: String,
        issue_number: u64,
    },
    OpenStartWork,
    OpenLaunchWizard {
        id: String,
        branch_name: String,
        linked_issue_number: Option<u64>,
    },
    OpenActiveWorkLaunchWizard {
        branch_name: String,
        linked_issue_number: Option<u64>,
    },
    LaunchWizardAction {
        action: LaunchWizardAction,
        bounds: Option<WindowGeometry>,
    },
    ApplyUpdate,
    /// Settings > Custom Agents: list every stored custom agent. Response is
    /// [`BackendEvent::CustomAgentList`].
    ListCustomAgents,
    /// Settings > Custom Agents > Add from preset: enumerate built-in preset
    /// definitions for the picker. Response is
    /// [`BackendEvent::CustomAgentPresetList`].
    ListCustomAgentPresets,
    /// Settings > Custom Agents > Add > Claude Code (OpenAI-compat backend):
    /// persist a new custom agent seeded from the preset payload. Response
    /// is [`BackendEvent::CustomAgentSaved`] on success or
    /// [`BackendEvent::CustomAgentError`] on failure.
    AddCustomAgentFromPreset {
        input: ClaudeCodeOpenaiCompatInput,
    },
    /// Settings > Custom Agents > Edit: replace an existing custom agent in
    /// place. The agent id must match an existing entry.
    UpdateCustomAgent {
        agent: Box<CustomCodingAgent>,
    },
    /// Settings > Custom Agents > Delete: remove the custom agent with the
    /// given id.
    DeleteCustomAgent {
        agent_id: String,
    },
    /// Settings > Custom Agents > Test connection: probe
    /// `GET {base_url}/v1/models` with the provided api key. Response is
    /// [`BackendEvent::BackendConnectionResult`] on success or
    /// [`BackendEvent::CustomAgentError`] on failure.
    TestBackendConnection {
        base_url: String,
        api_key: String,
    },
    /// SPEC-1934 US-6: user accepted the Migration confirmation modal for
    /// `tab_id`. Backend runs `gwt::migration::execute_migration` and streams
    /// progress as [`BackendEvent::MigrationProgress`] / [`BackendEvent::MigrationDone`]
    /// / [`BackendEvent::MigrationError`].
    StartMigration {
        tab_id: String,
    },
    /// SPEC-1934 US-6.7: user dismissed the migration modal for `tab_id`.
    /// Tab opens with the original Normal Git layout; the modal will appear
    /// again on the next launch.
    SkipMigration {
        tab_id: String,
    },
    /// SPEC-1934 US-6.8: user chose Quit from the migration modal. The app
    /// terminates without touching the repository.
    QuitMigration {
        tab_id: String,
    },
}

fn default_board_history_limit() -> usize {
    50
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceView {
    pub viewport: CanvasViewport,
    pub windows: Vec<PersistedWindowState>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectTabView {
    pub id: String,
    pub title: String,
    pub project_root: String,
    pub kind: ProjectKind,
    pub workspace: WorkspaceView,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecentProjectView {
    pub path: String,
    pub title: String,
    pub kind: ProjectKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProfileEnvEntryView {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProfileEntryView {
    pub name: String,
    pub description: String,
    pub env_vars: Vec<ProfileEnvEntryView>,
    pub disabled_env: Vec<String>,
    pub is_default: bool,
    pub is_active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProfileSnapshotView {
    pub active_profile: String,
    pub selected_profile: String,
    pub profiles: Vec<ProfileEntryView>,
    pub merged_preview: Vec<ProfileEnvEntryView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AppStateView {
    pub app_version: String,
    pub tabs: Vec<ProjectTabView>,
    pub active_tab_id: Option<String>,
    pub recent_projects: Vec<RecentProjectView>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActiveWorkAgentView {
    pub session_id: String,
    pub window_id: Option<String>,
    pub agent_id: String,
    pub display_name: String,
    pub status_category: String,
    pub current_focus: Option<String>,
    pub branch: Option<String>,
    pub worktree_path: Option<String>,
    pub last_board_entry_id: Option<String>,
    pub last_board_entry_kind: Option<String>,
    pub coordination_scope: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActiveWorkProjectionView {
    pub id: String,
    pub title: String,
    pub status_category: String,
    pub status_text: String,
    pub owner: Option<String>,
    pub next_action: Option<String>,
    pub active_agents: usize,
    pub blocked_agents: usize,
    pub branch: Option<String>,
    pub worktree_path: Option<String>,
    pub pr_number: Option<u64>,
    pub board_refs: Vec<String>,
    pub agents: Vec<ActiveWorkAgentView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BackendEvent {
    WorkspaceState {
        workspace: AppStateView,
    },
    ActiveWorkProjection {
        projection: ActiveWorkProjectionView,
    },
    WindowList {
        windows: Vec<PersistedWindowState>,
    },
    TerminalOutput {
        id: String,
        data_base64: String,
    },
    TerminalSnapshot {
        id: String,
        data_base64: String,
    },
    TerminalStatus {
        id: String,
        status: WindowProcessStatus,
        detail: Option<String>,
    },
    WindowState {
        window_id: String,
        state: WindowProcessStatus,
    },
    FileTreeEntries {
        id: String,
        path: String,
        entries: Vec<FileTreeEntry>,
    },
    FileTreeError {
        id: String,
        path: String,
        message: String,
    },
    BranchEntries {
        id: String,
        phase: BranchEntriesPhase,
        entries: Vec<BranchListEntry>,
    },
    BoardEntries {
        id: String,
        entries: Vec<BoardEntry>,
        #[serde(default)]
        has_more_before: bool,
    },
    BoardHistoryPage {
        id: String,
        entries: Vec<BoardEntry>,
        has_more_before: bool,
    },
    ProfileSnapshot {
        id: String,
        snapshot: ProfileSnapshotView,
    },
    MemoNotes {
        id: String,
        notes: Vec<MemoNote>,
        selected_note_id: Option<String>,
    },
    LogEntries {
        id: String,
        entries: Vec<LogEvent>,
    },
    LogEntryAppended {
        entry: LogEvent,
    },
    KnowledgeEntries {
        id: String,
        knowledge_kind: KnowledgeKind,
        request_id: Option<u64>,
        list_scope: Option<KnowledgeListScope>,
        entries: Vec<KnowledgeListItem>,
        selected_number: Option<u64>,
        empty_message: Option<String>,
        refresh_enabled: bool,
    },
    KnowledgeSearchResults {
        id: String,
        knowledge_kind: KnowledgeKind,
        query: String,
        request_id: u64,
        list_scope: Option<KnowledgeListScope>,
        entries: Vec<KnowledgeListItem>,
        selected_number: Option<u64>,
        empty_message: Option<String>,
        refresh_enabled: bool,
    },
    KnowledgeDetail {
        id: String,
        knowledge_kind: KnowledgeKind,
        request_id: Option<u64>,
        list_scope: Option<KnowledgeListScope>,
        detail: KnowledgeDetailView,
    },
    BranchCleanupResult {
        id: String,
        results: Vec<BranchCleanupResultEntry>,
    },
    BranchError {
        id: String,
        message: String,
    },
    BoardError {
        id: String,
        message: String,
    },
    ProfileError {
        id: String,
        message: String,
    },
    MemoError {
        id: String,
        message: String,
    },
    LogError {
        id: String,
        message: String,
    },
    KnowledgeError {
        id: String,
        knowledge_kind: KnowledgeKind,
        request_id: Option<u64>,
        query: Option<String>,
        list_scope: Option<KnowledgeListScope>,
        message: String,
    },
    ProjectOpenError {
        message: String,
    },
    LaunchWizardState {
        wizard: Option<Box<LaunchWizardView>>,
    },
    LaunchProgress {
        id: String,
        message: String,
    },
    ProjectIndexStatus {
        project_root: String,
        status: crate::ProjectIndexStatusView,
    },
    RuntimeHookEvent {
        event: RuntimeHookEvent,
    },
    UpdateState(gwt_core::update::UpdateState),
    /// Response to [`FrontendEvent::ListCustomAgents`].
    CustomAgentList {
        agents: Vec<CustomCodingAgent>,
    },
    /// Response to [`FrontendEvent::ListCustomAgentPresets`].
    CustomAgentPresetList {
        presets: Vec<PresetDefinition>,
    },
    /// Response to [`FrontendEvent::AddCustomAgentFromPreset`] /
    /// [`FrontendEvent::UpdateCustomAgent`] (save success).
    CustomAgentSaved {
        agent: Box<CustomCodingAgent>,
    },
    /// Response to [`FrontendEvent::DeleteCustomAgent`].
    CustomAgentDeleted {
        agent_id: String,
    },
    /// Response to [`FrontendEvent::TestBackendConnection`] (success).
    BackendConnectionResult {
        models: Vec<String>,
    },
    /// Error reply for any custom-agent mutation or probe request.
    /// `code` is a stable machine-readable tag; `message` is human-readable.
    CustomAgentError {
        code: CustomAgentErrorCode,
        message: String,
    },
    /// SPEC-1934 US-6.1: a project tab was opened on a Normal Git layout.
    /// The frontend should present the migration confirmation modal.
    MigrationDetected {
        tab_id: String,
        project_root: String,
        branch: Option<String>,
        has_dirty: bool,
        has_locked: bool,
        has_submodules: bool,
    },
    /// SPEC-1934 FR-029: incremental progress while
    /// `gwt::migration::execute_migration` runs. `phase` is the snake_case key
    /// from `MigrationPhase::as_str()`.
    MigrationProgress {
        tab_id: String,
        phase: String,
        percent: u8,
    },
    /// SPEC-1934 US-6.9: migration completed successfully.
    /// `branch_worktree_path` is where the GUI should reload the project tab.
    MigrationDone {
        tab_id: String,
        branch_worktree_path: String,
    },
    /// SPEC-1934 US-6.6: migration failed; `recovery` is one of
    /// `untouched`, `rolled_back`, or `partial`. The frontend uses this to
    /// decide whether to offer Retry / Restore / Quit.
    MigrationError {
        tab_id: String,
        phase: String,
        message: String,
        recovery: String,
    },
}

/// Stable machine-readable error code on [`BackendEvent::CustomAgentError`].
/// Serializes as `snake_case` string so the frontend can compare against
/// literal values without string-matching the human-readable message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CustomAgentErrorCode {
    Storage,
    Duplicate,
    InvalidInput,
    NotFound,
    Probe,
}

#[cfg(test)]
mod tests {
    use gwt_core::{
        coordination::{AuthorKind, BoardEntry, BoardEntryKind},
        logging::{LogEvent, LogLevel},
        notes::MemoNote,
    };
    use serde_json::Value;

    use crate::{
        branch_list::{
            BranchCleanupAvailability, BranchCleanupInfo, BranchCleanupRisk, BranchListEntry,
            BranchScope,
        },
        persistence::WindowState,
    };

    use super::{
        BackendEvent, BranchEntriesPhase, FrontendEvent, ProfileEntryView, ProfileEnvEntryView,
        ProfileSnapshotView,
    };

    #[test]
    fn branch_entries_serializes_explicit_phase_contract() {
        let event = BackendEvent::BranchEntries {
            id: "branches-1".to_string(),
            phase: BranchEntriesPhase::Inventory,
            entries: Vec::new(),
        };

        let value = serde_json::to_value(&event).expect("serialize branch entries");
        assert_eq!(
            value.get("kind"),
            Some(&Value::String("branch_entries".to_string()))
        );
        assert_eq!(
            value.get("phase"),
            Some(&Value::String("inventory".to_string()))
        );
    }

    #[test]
    fn branch_entries_serializes_actual_merge_target_reference_contract() {
        let event = BackendEvent::BranchEntries {
            id: "branches-1".to_string(),
            phase: BranchEntriesPhase::Hydrated,
            entries: vec![BranchListEntry {
                name: "feature/demo".to_string(),
                scope: BranchScope::Local,
                is_head: false,
                upstream: Some("origin/feature/demo".to_string()),
                ahead: 0,
                behind: 0,
                last_commit_date: None,
                cleanup_ready: true,
                cleanup: BranchCleanupInfo {
                    availability: BranchCleanupAvailability::Safe,
                    execution_branch: Some("feature/demo".to_string()),
                    merge_target: Some(gwt_git::MergeTargetRef {
                        kind: gwt_git::MergeTarget::Develop,
                        reference: "origin/develop".to_string(),
                    }),
                    upstream: Some("origin/feature/demo".to_string()),
                    blocked_reason: None,
                    risks: vec![BranchCleanupRisk::RemoteTracking],
                },
            }],
        };

        let value = serde_json::to_value(&event).expect("serialize branch entries");
        let cleanup = &value["entries"][0]["cleanup"]["merge_target"];
        assert_eq!(
            cleanup["kind"],
            Value::String("develop".to_string()),
            "expected merge target kind to remain machine-readable",
        );
        assert_eq!(
            cleanup["reference"],
            Value::String("origin/develop".to_string()),
            "expected branch entries payload to expose the actual merge target ref",
        );
    }

    #[test]
    fn terminal_snapshot_serializes_explicit_kind_contract() {
        let event = BackendEvent::TerminalSnapshot {
            id: "tab-1::shell-1".to_string(),
            data_base64: "aGVsbG8=".to_string(),
        };

        let value = serde_json::to_value(&event).expect("serialize terminal snapshot");
        assert_eq!(
            value.get("kind"),
            Some(&Value::String("terminal_snapshot".to_string()))
        );
        assert_eq!(
            value.get("id"),
            Some(&Value::String("tab-1::shell-1".to_string()))
        );
        assert_eq!(
            value.get("data_base64"),
            Some(&Value::String("aGVsbG8=".to_string()))
        );
    }

    #[test]
    fn active_work_projection_uses_distinct_wire_event_from_canvas_workspace_state() {
        let event = BackendEvent::ActiveWorkProjection {
            projection: super::ActiveWorkProjectionView {
                id: "work-1".to_string(),
                title: "Implement Start Work".to_string(),
                status_category: "active".to_string(),
                status_text: "Launching from Project Bar".to_string(),
                owner: Some("SPEC-2359".to_string()),
                next_action: Some("Run launch tests".to_string()),
                active_agents: 1,
                blocked_agents: 0,
                branch: Some("work/20260504-1200".to_string()),
                worktree_path: Some("/tmp/repo/work/20260504-1200".to_string()),
                pr_number: None,
                board_refs: vec!["board-1".to_string()],
                agents: vec![super::ActiveWorkAgentView {
                    session_id: "session-1".to_string(),
                    window_id: Some("tab-1::agent-1".to_string()),
                    agent_id: "codex".to_string(),
                    display_name: "Codex".to_string(),
                    status_category: "active".to_string(),
                    current_focus: Some("Run launch tests".to_string()),
                    branch: Some("work/20260504-1200".to_string()),
                    worktree_path: Some("/tmp/repo/work/20260504-1200".to_string()),
                    last_board_entry_id: Some("board-1".to_string()),
                    last_board_entry_kind: Some("handoff".to_string()),
                    coordination_scope: Some("SPEC-2359 / start-work".to_string()),
                    updated_at: "2026-05-04T12:00:00Z".to_string(),
                }],
            },
        };

        let value = serde_json::to_value(&event).expect("serialize active work projection");

        assert_eq!(
            value.get("kind"),
            Some(&Value::String("active_work_projection".to_string())),
            "active work projection must not reuse canvas workspace_state"
        );
        assert_eq!(
            value
                .pointer("/projection/agents/0/display_name")
                .and_then(Value::as_str),
            Some("Codex"),
            "active work projection must expose per-agent summaries for Workspace UI"
        );
        assert_eq!(
            value
                .pointer("/projection/agents/0/last_board_entry_id")
                .and_then(Value::as_str),
            Some("board-1")
        );
        assert_eq!(
            value
                .pointer("/projection/agents/0/last_board_entry_kind")
                .and_then(Value::as_str),
            Some("handoff")
        );
        assert_eq!(
            value
                .pointer("/projection/agents/0/coordination_scope")
                .and_then(Value::as_str),
            Some("SPEC-2359 / start-work")
        );
    }

    #[test]
    fn frontend_event_accepts_global_open_start_work_command() {
        let event: FrontendEvent =
            serde_json::from_value(serde_json::json!({ "kind": "open_start_work" }))
                .expect("deserialize open_start_work");

        assert!(
            matches!(event, FrontendEvent::OpenStartWork),
            "Start Work must be a global command, not a Branches window event"
        );
    }

    #[test]
    fn frontend_event_accepts_workspace_add_agent_command() {
        let event: FrontendEvent = serde_json::from_value(serde_json::json!({
            "kind": "open_active_work_launch_wizard",
            "branch_name": "work/20260504-1200",
            "linked_issue_number": null
        }))
        .expect("deserialize workspace add-agent launch");

        assert!(
            matches!(
                event,
                FrontendEvent::OpenActiveWorkLaunchWizard {
                    branch_name,
                    linked_issue_number: None,
                } if branch_name == "work/20260504-1200"
            ),
            "Workspace Add Agent must not depend on a Branches window id"
        );
    }

    #[test]
    fn board_entries_serializes_snapshot_contract() {
        let event = BackendEvent::BoardEntries {
            id: "board-1".to_string(),
            entries: vec![BoardEntry::new(
                AuthorKind::Agent,
                "codex",
                BoardEntryKind::Status,
                "Waiting for next task",
                Some("ready".to_string()),
                None,
                vec!["coordination".to_string()],
                vec!["2018".to_string()],
            )],
            has_more_before: false,
        };

        let value = serde_json::to_value(&event).expect("serialize board entries");
        assert_eq!(
            value.get("kind"),
            Some(&Value::String("board_entries".to_string()))
        );
        assert_eq!(
            value["entries"][0]["kind"],
            Value::String("status".to_string()),
            "expected board entry kind to remain machine-readable on the wire",
        );
        assert_eq!(
            value["entries"][0]["related_topics"][0],
            Value::String("coordination".to_string()),
            "expected board snapshot payload to keep related topics on the wire",
        );
    }

    #[test]
    fn board_history_page_serializes_cursor_contract() {
        let frontend: FrontendEvent = serde_json::from_value(serde_json::json!({
            "kind": "load_board_history",
            "id": "board-1",
            "before_entry_id": "entry-3",
            "limit": 50
        }))
        .expect("deserialize board history request");
        assert!(matches!(
            frontend,
            FrontendEvent::LoadBoardHistory {
                id,
                before_entry_id: Some(before_entry_id),
                limit
            } if id == "board-1" && before_entry_id == "entry-3" && limit == 50
        ));

        let backend = BackendEvent::BoardHistoryPage {
            id: "board-1".to_string(),
            entries: vec![BoardEntry::new(
                AuthorKind::Agent,
                "codex",
                BoardEntryKind::Status,
                "Older update",
                None,
                None,
                vec![],
                vec![],
            )],
            has_more_before: true,
        };
        let value = serde_json::to_value(&backend).expect("serialize board history page");
        assert_eq!(
            value.get("kind"),
            Some(&Value::String("board_history_page".to_string()))
        );
        assert_eq!(value["has_more_before"], Value::Bool(true));
        assert_eq!(
            value["entries"][0]["body"],
            Value::String("Older update".into())
        );
    }

    #[test]
    fn profile_snapshot_serializes_explicit_kind_contract() {
        let event = BackendEvent::ProfileSnapshot {
            id: "profile-1".to_string(),
            snapshot: ProfileSnapshotView {
                active_profile: "default".to_string(),
                selected_profile: "default".to_string(),
                profiles: vec![ProfileEntryView {
                    name: "default".to_string(),
                    description: "Default profile".to_string(),
                    env_vars: vec![ProfileEnvEntryView {
                        key: "TERM".to_string(),
                        value: "xterm-256color".to_string(),
                    }],
                    disabled_env: vec!["SECRET".to_string()],
                    is_default: true,
                    is_active: true,
                }],
                merged_preview: vec![ProfileEnvEntryView {
                    key: "TERM".to_string(),
                    value: "xterm-256color".to_string(),
                }],
            },
        };

        let value = serde_json::to_value(&event).expect("serialize profile snapshot");
        assert_eq!(
            value.get("kind"),
            Some(&Value::String("profile_snapshot".to_string()))
        );
        assert_eq!(
            value["snapshot"]["selected_profile"],
            Value::String("default".to_string())
        );
        assert_eq!(
            value["snapshot"]["profiles"][0]["env_vars"][0]["key"],
            Value::String("TERM".to_string())
        );
    }

    #[test]
    fn memo_notes_serializes_snapshot_contract() {
        let event = BackendEvent::MemoNotes {
            id: "memo-1".to_string(),
            notes: vec![MemoNote {
                id: "note-1".to_string(),
                title: "Pinned note".to_string(),
                body: "Remember to verify the cache contract".to_string(),
                pinned: true,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            }],
            selected_note_id: Some("note-1".to_string()),
        };

        let value = serde_json::to_value(&event).expect("serialize memo notes");
        assert_eq!(
            value.get("kind"),
            Some(&Value::String("memo_notes".to_string()))
        );
        assert_eq!(value.get("id"), Some(&Value::String("memo-1".to_string())));
        assert_eq!(
            value["notes"][0]["pinned"],
            Value::Bool(true),
            "expected memo snapshot payload to keep pin ordering data on the wire",
        );
        assert_eq!(
            value["selected_note_id"],
            Value::String("note-1".to_string()),
            "expected memo snapshot payload to carry the preferred editor selection",
        );
    }

    #[test]
    fn window_state_serializes_explicit_contract() {
        let event = BackendEvent::WindowState {
            window_id: "window-1".to_string(),
            state: WindowState::Waiting,
        };

        let value = serde_json::to_value(&event).expect("serialize window state");
        assert_eq!(
            value.get("kind"),
            Some(&Value::String("window_state".to_string()))
        );
        assert_eq!(
            value.get("window_id"),
            Some(&Value::String("window-1".to_string()))
        );
        assert_eq!(
            value.get("state"),
            Some(&Value::String("waiting".to_string()))
        );
    }

    #[test]
    fn log_entries_serializes_snapshot_contract() {
        let event = BackendEvent::LogEntries {
            id: "logs-1".to_string(),
            entries: vec![
                LogEvent::new(LogLevel::Warn, "gwt", "watcher stalled").with_detail("tail retry")
            ],
        };

        let value = serde_json::to_value(&event).expect("serialize log entries");
        assert_eq!(
            value.get("kind"),
            Some(&Value::String("log_entries".to_string()))
        );
        assert_eq!(value.get("id"), Some(&Value::String("logs-1".to_string())));
        assert_eq!(
            value["entries"][0]["severity"],
            Value::String("Warn".to_string())
        );
        assert_eq!(
            value["entries"][0]["source"],
            Value::String("gwt".to_string())
        );
        assert_eq!(
            value["entries"][0]["message"],
            Value::String("watcher stalled".to_string())
        );
        assert_eq!(
            value["entries"][0]["detail"],
            Value::String("tail retry".to_string())
        );
    }

    #[test]
    fn protocol_source_layout_keeps_wire_schema_separate_from_transport_and_frontend_logic() {
        let source = include_str!("protocol.rs");
        let production_source = source
            .split("#[cfg(test)]")
            .next()
            .expect("protocol.rs should contain production source before tests");

        assert!(
            production_source.contains("pub enum FrontendEvent"),
            "expected protocol owner to define frontend wire events in protocol.rs",
        );
        assert!(
            production_source.contains("pub enum BackendEvent"),
            "expected protocol owner to define backend wire events in protocol.rs",
        );
        assert!(
            production_source.contains("#[serde(tag = \"kind\", rename_all = \"snake_case\")]"),
            "expected protocol owner to keep the stable tagged-union contract local to protocol.rs",
        );
        assert!(
            !production_source.contains("handle_frontend_message")
                && !production_source.contains("websocket_handler")
                && !production_source.contains("ClientHub"),
            "expected transport/runtime dispatch to stay out of protocol.rs",
        );
        assert!(
            !production_source.contains("document.addEventListener")
                && !production_source.contains("handleCanvasWheelEvent")
                && !production_source.contains("navigator.clipboard"),
            "expected frontend behavior details to stay out of protocol.rs",
        );
    }
}
