use chrono::{DateTime, Utc};
use gwt_agent::{AgentColor, CustomCodingAgent, PresetDefinition, PresetId};
use gwt_core::coordination::{AuthorKind, BoardEntryKind};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    branch_cleanup::BranchCleanupResultEntry,
    branch_list::BranchListEntry,
    daemon_runtime::RuntimeHookEvent,
    file_tree::FileTreeEntry,
    knowledge_bridge::{KnowledgeDetailView, KnowledgeKind, KnowledgeListItem},
    launch_wizard::{LaunchWizardAction, LaunchWizardView},
    persistence::{
        CanvasViewport, PersistedWindowState, ProjectKind, WindowGeometry, WindowProcessStatus,
    },
    preset::WindowPreset,
    profiles_service::ProfileSnapshot,
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
    /// Settings > Custom Agents > Add from preset: persist a new custom agent
    /// seeded from the selected preset payload. Response is
    /// [`BackendEvent::CustomAgentSaved`] on success or
    /// [`BackendEvent::CustomAgentError`] on failure.
    AddCustomAgentFromPreset {
        preset_id: PresetId,
        payload: Value,
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
    ListProfiles {
        id: String,
        selected_profile: Option<String>,
    },
    SwitchProfile {
        id: String,
        profile_name: String,
    },
    AddProfile {
        id: String,
        name: String,
        description: String,
    },
    UpdateProfile {
        id: String,
        current_name: String,
        name: String,
        description: String,
    },
    DeleteProfile {
        id: String,
        profile_name: String,
    },
    SetProfileEnvVar {
        id: String,
        profile_name: String,
        key: String,
        value: String,
    },
    UpdateProfileEnvVar {
        id: String,
        profile_name: String,
        current_key: String,
        key: String,
        value: String,
    },
    DeleteProfileEnvVar {
        id: String,
        profile_name: String,
        key: String,
    },
    AddDisabledEnv {
        id: String,
        profile_name: String,
        key: String,
    },
    UpdateDisabledEnv {
        id: String,
        profile_name: String,
        current_key: String,
        key: String,
    },
    DeleteDisabledEnv {
        id: String,
        profile_name: String,
        key: String,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceView {
    pub viewport: CanvasViewport,
    pub windows: Vec<PersistedWindowState>,
}

/// Frontend-facing projection of a [`gwt_core::coordination::BoardEntry`].
///
/// 付加した `agent_color` は wire-only。`origin_agent_id` を既知の
/// [`gwt_agent::AgentId`] に正規化し、`default_color()` をここで
/// 計算してフロントに渡す (SPEC #2133 FR-006 / FR-012)。
#[derive(Debug, Clone, Serialize)]
pub struct BoardEntryView {
    pub id: String,
    pub author_kind: AuthorKind,
    pub author: String,
    pub kind: BoardEntryKind,
    pub body: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin_branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin_agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_color: Option<AgentColor>,
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
    BoardSnapshot {
        id: String,
        entries: Vec<BoardEntryView>,
    },
    BoardError {
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
    ProfileSnapshot {
        id: String,
        snapshot: ProfileSnapshot,
    },
    ProfileError {
        id: String,
        code: ProfileErrorCode,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileErrorCode {
    Storage,
    Duplicate,
    InvalidInput,
    NotFound,
    Protected,
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use crate::profiles_service::{
        ProfileEnvVarSource, ProfileEnvVarView, ProfileSnapshot, ProfileView,
    };

    use super::{BackendEvent, BranchEntriesPhase, FrontendEvent, PresetId};

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
    fn profile_snapshot_serializes_explicit_contract() {
        let event = BackendEvent::ProfileSnapshot {
            id: "profile-1".to_string(),
            snapshot: ProfileSnapshot {
                active: "default".to_string(),
                selected: "default".to_string(),
                profiles: vec![ProfileView {
                    name: "default".to_string(),
                    description: "Default profile".to_string(),
                    active: true,
                    env_vars: vec![ProfileEnvVarView {
                        key: "API_URL".to_string(),
                        value: "https://example.test".to_string(),
                        source: ProfileEnvVarSource::Profile,
                    }],
                    disabled_env: vec!["SECRET".to_string()],
                    merged_env: Vec::new(),
                }],
            },
        };

        let value = serde_json::to_value(&event).expect("serialize profile snapshot");
        assert_eq!(
            value.get("kind"),
            Some(&Value::String("profile_snapshot".to_string()))
        );
        assert_eq!(
            value.pointer("/snapshot/profiles/0/env_vars/0/source"),
            Some(&Value::String("profile".to_string()))
        );
        assert_eq!(
            value.pointer("/snapshot/profiles/0/disabled_env/0"),
            Some(&Value::String("SECRET".to_string()))
        );
    }

    #[test]
    fn add_custom_agent_from_preset_deserializes_preset_id_and_payload() {
        let event: FrontendEvent = serde_json::from_value(serde_json::json!({
            "kind": "add_custom_agent_from_preset",
            "preset_id": "claude_code_openai_compat",
            "payload": {
                "id": "claude-code-openai",
                "display_name": "Claude Code (OpenAI-compat)",
                "base_url": "https://proxy.example.com",
                "api_key": "sk-test-123",
                "default_model": "openai/gpt-oss-20b"
            }
        }))
        .expect("deserialize frontend event");

        match event {
            FrontendEvent::AddCustomAgentFromPreset { preset_id, payload } => {
                assert_eq!(preset_id, PresetId::ClaudeCodeOpenaiCompat);
                assert_eq!(
                    payload.get("default_model"),
                    Some(&Value::String("openai/gpt-oss-20b".to_string()))
                );
            }
            other => panic!("expected AddCustomAgentFromPreset, got {other:?}"),
        }
    }
}
