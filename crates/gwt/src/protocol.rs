use gwt_agent::CustomCodingAgent;
use serde::{Deserialize, Serialize};

use crate::{
    branch_cleanup::BranchCleanupResultEntry,
    branch_list::BranchListEntry,
    custom_agents_service::{ClaudeCodeOpenaiCompatInput, PresetDefinition},
    daemon_runtime::RuntimeHookEvent,
    file_tree::FileTreeEntry,
    knowledge_bridge::{KnowledgeDetailView, KnowledgeKind, KnowledgeListItem},
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
    LoadKnowledgeBridge {
        id: String,
        knowledge_kind: KnowledgeKind,
        selected_number: Option<u64>,
        refresh: bool,
    },
    SelectKnowledgeBridgeEntry {
        id: String,
        knowledge_kind: KnowledgeKind,
        number: u64,
    },
    RunBranchCleanup {
        id: String,
        branches: Vec<String>,
        delete_remote: bool,
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
    use serde_json::Value;

    use crate::branch_list::{
        BranchCleanupAvailability, BranchCleanupInfo, BranchCleanupRisk, BranchListEntry,
        BranchScope,
    };

    use super::{BackendEvent, BranchEntriesPhase};

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
}
