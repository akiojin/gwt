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
        selected_number: Option<u64>,
        refresh: bool,
        list_scope: Option<KnowledgeListScope>,
    },
    SelectKnowledgeBridgeEntry {
        id: String,
        knowledge_kind: KnowledgeKind,
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
    OpenLaunchWizard {
        id: String,
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

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BackendEvent {
    WorkspaceState {
        workspace: AppStateView,
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
        entries: Vec<KnowledgeListItem>,
        selected_number: Option<u64>,
        empty_message: Option<String>,
        refresh_enabled: bool,
    },
    KnowledgeDetail {
        id: String,
        knowledge_kind: KnowledgeKind,
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
        BackendEvent, BranchEntriesPhase, ProfileEntryView, ProfileEnvEntryView,
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
