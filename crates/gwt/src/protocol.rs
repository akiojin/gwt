use gwt_agent::{ClaudeCodeOpenaiCompatInput, CustomCodingAgent, PresetDefinition};
use gwt_core::{
    coordination::{BoardEntry, BoardEntryKind},
    logging::LogEvent,
};
use serde::{Deserialize, Serialize};

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
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArrangeMode {
    Tile,
    Stack,
    Align,
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceResumeSource {
    Current,
    Journal,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FrontendEvent {
    FrontendReady,
    OpenProjectDialog,
    SelectCloneProjectParent,
    GithubRepositorySearch {
        query: String,
    },
    CloneProjectStart {
        url: String,
        parent_path: String,
    },
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
    DockWindowTab {
        id: String,
        target_id: String,
    },
    ActivateWindowTab {
        id: String,
    },
    DetachWindowTab {
        id: String,
        geometry: WindowGeometry,
    },
    ListWindows,
    UpdateWindowGeometry {
        id: String,
        geometry: WindowGeometry,
        cols: u16,
        rows: u16,
        #[serde(default)]
        base_geometry_revision: Option<u64>,
    },
    CloseWindow {
        id: String,
    },
    TerminalInput {
        id: String,
        data: String,
    },
    PasteImage {
        id: String,
        data_base64: String,
        mime_type: String,
        filename: Option<String>,
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
        #[serde(default)]
        all: bool,
    },
    LoadBoardHistory {
        id: String,
        before_entry_id: Option<String>,
        #[serde(default = "default_board_history_limit")]
        limit: usize,
        #[serde(default)]
        all: bool,
    },
    LoadProfile {
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
    },
    SearchKnowledgeBridge {
        id: String,
        knowledge_kind: KnowledgeKind,
        query: String,
        request_id: u64,
        selected_number: Option<u64>,
    },
    SelectKnowledgeBridgeEntry {
        id: String,
        knowledge_kind: KnowledgeKind,
        #[serde(default)]
        request_id: Option<u64>,
        number: u64,
    },
    /// SPEC-2017 US-8 — Kanban D&D writes a phase change back to the
    /// owning GitHub Issue. `target_phase=None` means Backlog (every
    /// `phase/*` label is removed); `Some("draft" | "planning" |
    /// "implementation" | "review" | "done")` means assign exactly that
    /// canonical phase. The frontend includes `request_id` so the
    /// matching [`BackendEvent::KnowledgeBridgePhaseUpdated`] response
    /// can clear the pending optimistic-UI entry.
    UpdateKnowledgeBridgePhase {
        id: String,
        request_id: u64,
        issue_number: u64,
        target_phase: Option<String>,
    },
    RunBranchCleanup {
        id: String,
        branches: Vec<String>,
        delete_remote: bool,
    },
    RunWorkspaceCleanup {
        branch: String,
        delete_remote: bool,
    },
    /// SPEC-1939 US-5: trigger a per-cell index rebuild for
    /// `(project_root, scope, worktree_hash?)`. The backend funnels this
    /// through the same orchestrator + `.lock` path as the auto-rebuild
    /// orchestrator and `gwt index rebuild` CLI so concurrent invocations
    /// dedup.
    RebuildIndexCell {
        project_root: String,
        scope: crate::IndexRebuildScope,
        #[serde(default)]
        worktree_hash: Option<String>,
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
        #[serde(default)]
        mentions: Vec<gwt_core::coordination::BoardMention>,
    },
    OpenBoardOriginAgent {
        id: String,
        origin_session_id: String,
        bounds: Option<WindowGeometry>,
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
    ResumeWorkspace {
        source: WorkspaceResumeSource,
        #[serde(default)]
        journal_id: Option<String>,
    },
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
    /// Legacy Phase 14 entry point. Frontend now sends
    /// [`FrontendEvent::ApplyUpdateStart`] / [`FrontendEvent::ApplyUpdateRestartNow`]
    /// instead. Kept so older clients and unit tests that still drive
    /// `apply_update` continue to work; routes to the same backend behavior as
    /// `ApplyUpdateRestartNow` (download → spawn helper → exit).
    ApplyUpdate,
    /// SPEC-2041 Phase 19 (FR-052..057): user clicked the update CTA. Backend
    /// downloads/prepares the asset and emits [`BackendEvent::UpdateProgress`]
    /// during the transfer plus [`BackendEvent::UpdateReady`] on completion,
    /// without exiting the parent process.
    ApplyUpdateStart,
    /// SPEC-2041 Phase 19 (FR-055): user pressed Cancel on the downloading
    /// modal. Backend aborts the in-flight download and removes any partial
    /// payload. Currently a best-effort no-op until async download lands.
    CancelUpdateDownload,
    /// SPEC-2041 Phase 19 (FR-059..061): user pressed `Later`. Binary stays
    /// preserved; backend emits [`BackendEvent::UpdateApplyPendingPersisted`]
    /// so the CTA morphs to ready state and same-session polling stops.
    ApplyUpdateLater,
    /// SPEC-2041 Phase 19 (FR-058): user pressed `Restart now`. Backend swaps
    /// the prepared binary via the helper subprocess and exits the parent.
    ApplyUpdateRestartNow,
    /// SPEC-2041 Phase 19 (FR-065): user pressed `Open log` on the failed
    /// modal. Backend opens the log file in the OS default application.
    OpenUpdateLog {
        log_path: Option<String>,
    },
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
    /// SPEC-1933 US-4: Settings > System tab opened. Backend replies with
    /// [`BackendEvent::SystemSettings`] containing the current global
    /// `[ai].language` value (`auto` / `en` / `ja`).
    GetSystemSettings,
    /// SPEC-1933 US-4: Settings > System > Language select changed. Backend
    /// persists the value to `~/.gwt/config.toml` under `[ai].language` and
    /// replies with [`BackendEvent::SystemSettingsUpdated`] on success or
    /// [`BackendEvent::SystemSettingsError`] on failure.
    UpdateSystemSettings {
        language: String,
        #[serde(default)]
        codex_trust_managed_hooks: Option<bool>,
    },
    /// SPEC-2359 US-41: classify Workspace projections under `~/.gwt/projects/`
    /// and either preview (`dry_run = true`) or apply (`dry_run = false`) the
    /// archive→delete transitions. `ids` limits the action to specific
    /// `WorkspaceProjection::id` values; an empty list means "every classified
    /// entry". Backend replies with [`BackendEvent::WorkspaceProjectionPruneResult`].
    WorkspaceProjectionPrune {
        #[serde(default)]
        dry_run: bool,
        #[serde(default)]
        ids: Vec<String>,
    },
}

fn default_board_history_limit() -> usize {
    50
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceView {
    pub viewport: CanvasViewport,
    pub windows: Vec<PersistedWindowState>,
    // SPEC-2359 US-37: Workspace Overview Completed カラムは
    // active_work_projection broadcast に依存していたが、その broadcast
    // は限定された trigger でしか走らないため起動直後に表示されない
    // 問題があった。workspace_state は frequently broadcast されるので、
    // 同 event に work_items を載せて broadcast invariant を 1 本化する。
    #[serde(default)]
    pub work_items: Vec<WorkspaceHistoryView>,
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
pub struct GitHubRepositorySearchResultView {
    pub full_name: String,
    pub description: Option<String>,
    pub url: String,
    pub default_branch: Option<String>,
    pub visibility: Option<String>,
    pub updated_at: Option<String>,
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
    pub affiliation_status: String,
    pub workspace_id: Option<String>,
    pub status_category: String,
    pub current_focus: Option<String>,
    pub title_summary: Option<String>,
    pub branch: Option<String>,
    pub worktree_path: Option<String>,
    pub last_board_entry_id: Option<String>,
    pub last_board_entry_kind: Option<String>,
    pub coordination_scope: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceJournalEntryView {
    pub id: String,
    pub updated_at: String,
    pub title: Option<String>,
    pub status_category: Option<String>,
    pub status_text: Option<String>,
    pub summary: Option<String>,
    pub owner: Option<String>,
    pub next_action: Option<String>,
    pub agent_session_id: Option<String>,
    pub agent_current_focus: Option<String>,
    pub agent_title_summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceHistoryAgentView {
    pub session_id: String,
    pub agent_id: Option<String>,
    pub display_name: Option<String>,
    pub updated_at: String,
}

pub type WorkspaceWorkAgentView = WorkspaceHistoryAgentView;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceExecutionContainerView {
    pub branch: Option<String>,
    pub worktree_path: Option<String>,
    pub pr_number: Option<u64>,
    pub pr_url: Option<String>,
    pub pr_state: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceHistoryEventView {
    pub id: String,
    pub workspace_id: String,
    pub kind: String,
    pub title: Option<String>,
    pub intent: Option<String>,
    pub summary: Option<String>,
    pub status_category: Option<String>,
    pub owner: Option<String>,
    pub next_action: Option<String>,
    pub agent_session_id: Option<String>,
    pub board_entry_id: Option<String>,
    pub related_workspace_id: Option<String>,
    pub updated_at: String,
}

pub type WorkspaceWorkEventView = WorkspaceHistoryEventView;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceHistoryView {
    pub id: String,
    pub title: String,
    pub intent: Option<String>,
    pub summary: Option<String>,
    pub status_category: String,
    pub owner: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
    pub agents: Vec<WorkspaceHistoryAgentView>,
    pub execution_containers: Vec<WorkspaceExecutionContainerView>,
    pub board_refs: Vec<String>,
    pub related_workspace_ids: Vec<String>,
    pub events: Vec<WorkspaceHistoryEventView>,
}

pub type WorkspaceWorkItemView = WorkspaceHistoryView;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActiveWorkCleanupCandidateView {
    pub branch: String,
    pub worktree_path: Option<String>,
    pub reason: String,
    pub default_delete_remote: bool,
    pub remote_delete_available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActiveWorkProjectionView {
    pub id: String,
    pub title: String,
    pub status_category: String,
    pub status_text: String,
    pub summary: Option<String>,
    pub owner: Option<String>,
    pub next_action: Option<String>,
    pub active_agents: usize,
    pub blocked_agents: usize,
    pub branch: Option<String>,
    pub worktree_path: Option<String>,
    pub pr_number: Option<u64>,
    pub pr_url: Option<String>,
    pub pr_state: Option<String>,
    pub pr_created_at: Option<String>,
    pub board_refs: Vec<String>,
    pub journal_entries: Vec<WorkspaceJournalEntryView>,
    #[serde(default, alias = "work_items")]
    pub workspaces: Vec<WorkspaceHistoryView>,
    pub cleanup_candidate: Option<ActiveWorkCleanupCandidateView>,
    pub agents: Vec<ActiveWorkAgentView>,
    #[serde(default)]
    pub unassigned_agents: Vec<ActiveWorkAgentView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BackendEvent {
    WorkspaceState {
        workspace: AppStateView,
    },
    ActiveWorkProjection {
        projection: Box<ActiveWorkProjectionView>,
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
        entries: Vec<KnowledgeListItem>,
        selected_number: Option<u64>,
        empty_message: Option<String>,
        refresh_enabled: bool,
    },
    KnowledgeDetail {
        id: String,
        knowledge_kind: KnowledgeKind,
        request_id: Option<u64>,
        detail: KnowledgeDetailView,
    },
    /// SPEC-2017 US-8 — Result of an
    /// [`FrontendEvent::UpdateKnowledgeBridgePhase`] request. On
    /// success the backend returns the freshly-rebuilt cache entry so
    /// the optimistic Kanban card can be replaced with authoritative
    /// labels and counters; on failure it returns a human-readable
    /// `message` so the frontend can roll back from `dndSnapshot` and
    /// surface a toast. `request_id` mirrors the originating frame so
    /// the `pendingPhaseUpdates` map can be reconciled even when
    /// multiple drops are in flight.
    KnowledgeBridgePhaseUpdated {
        id: String,
        request_id: u64,
        issue_number: u64,
        result: KnowledgePhaseUpdateResult,
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
    LogError {
        id: String,
        message: String,
    },
    KnowledgeError {
        id: String,
        knowledge_kind: KnowledgeKind,
        request_id: Option<u64>,
        query: Option<String>,
        message: String,
    },
    ProjectOpenError {
        message: String,
    },
    CloneProjectParentSelected {
        path: String,
    },
    GithubRepositorySearchResults {
        query: String,
        repositories: Vec<GitHubRepositorySearchResultView>,
    },
    GithubRepositorySearchError {
        query: String,
        message: String,
    },
    CloneProjectProgress {
        message: String,
    },
    CloneProjectDone {
        workspace_home: String,
    },
    CloneProjectError {
        message: String,
    },
    LaunchWizardOpenError {
        title: String,
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
    /// SPEC-2041 Phase 19 (FR-054): download progress for the current update.
    /// Emitted from `Backend` while a download is active; the `#update-modal`
    /// uses these to drive the progress bar and byte counter.
    UpdateProgress {
        /// Bytes already received.
        downloaded: u64,
        /// Expected total bytes (when the server advertises Content-Length).
        total: Option<u64>,
        /// Asset filename (e.g. `gwt-macos-arm64.tar.gz`).
        asset: Option<String>,
        /// Target version (without the `v` prefix).
        version: Option<String>,
    },
    /// SPEC-2041 Phase 19 (FR-056): download completed and the prepared payload
    /// lives on disk. Frontend transitions the modal to the `ready` state.
    UpdateReady {
        version: String,
        /// On-disk path to the prepared payload (extracted binary or installer).
        asset_path: String,
    },
    /// SPEC-2041 Phase 19 (FR-059): `Later` was confirmed. The downloaded
    /// binary is preserved (in-memory today; persistent across restarts once
    /// the bootstrap path lands in T-133). Frontend morphs the CTA to ready.
    UpdateApplyPendingPersisted {
        version: String,
    },
    UpdateApplyError {
        /// Phase 14 free-form message. Still emitted for backward compat.
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        /// Phase 19 (FR-063): structured failure stage
        /// (e.g. `"Download asset"`, `"Replace binary"`).
        #[serde(skip_serializing_if = "Option::is_none")]
        stage: Option<String>,
        /// Phase 19 (FR-063): human-readable reason.
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        /// Phase 19 (FR-065): path to the per-day update log so the modal can
        /// surface `[Open log]`.
        #[serde(skip_serializing_if = "Option::is_none")]
        log_path: Option<String>,
    },
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
    /// SPEC-1933 US-4: response to [`FrontendEvent::GetSystemSettings`].
    /// Carries the raw global `[ai].language` value from `~/.gwt/config.toml`
    /// (`auto` / `en` / `ja`). The frontend mirrors this value into the
    /// Settings > System > Language select.
    SystemSettings {
        language: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        codex_trust_managed_hooks: Option<bool>,
    },
    /// SPEC-1933 US-4: confirmation that
    /// [`FrontendEvent::UpdateSystemSettings`] persisted successfully.
    /// `language` echoes the saved value so the frontend can reconcile
    /// optimistic UI with the authoritative config state.
    SystemSettingsUpdated {
        language: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        codex_trust_managed_hooks: Option<bool>,
    },
    /// SPEC-1933 US-4: error reply for [`FrontendEvent::GetSystemSettings`]
    /// or [`FrontendEvent::UpdateSystemSettings`]. `message` is
    /// human-readable; the frontend surfaces it as an inline status row in
    /// the System tab.
    SystemSettingsError {
        message: String,
    },
    /// SPEC-2359 US-41: response to [`FrontendEvent::WorkspaceProjectionPrune`].
    /// `mode` is `"dry_run"` or `"applied"`; counts reflect the plan executed
    /// against `~/.gwt/projects/*/workspace/`.
    WorkspaceProjectionPruneResult {
        mode: String,
        archived: usize,
        deleted: usize,
        skipped: usize,
    },
    /// SPEC-2359 US-41: error reply for
    /// [`FrontendEvent::WorkspaceProjectionPrune`] when the backend cannot
    /// classify or apply the plan (e.g. unreadable projection file, IO error
    /// during delete).
    WorkspaceProjectionPruneError {
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

/// SPEC-2017 US-8 — Outcome of an
/// [`FrontendEvent::UpdateKnowledgeBridgePhase`] request, embedded in
/// [`BackendEvent::KnowledgeBridgePhaseUpdated`]. Tagged so the
/// frontend can branch on `result.kind === "ok" | "error"` without
/// pattern-matching on optional fields.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum KnowledgePhaseUpdateResult {
    /// Phase write-back succeeded. `fresh_entry` is the rebuilt cache
    /// entry (with the new labels / state / phase) so the optimistic
    /// Kanban card can be overwritten with authoritative data.
    Ok { fresh_entry: KnowledgeListItem },
    /// Phase write-back failed. `message` is human-readable so the
    /// toast / log can show it directly; the frontend rolls back the
    /// optimistic UI from `state.dndSnapshot`.
    Error { message: String },
}

#[cfg(test)]
mod tests {
    use gwt_core::{
        coordination::{
            AuthorKind, BoardEntry, BoardEntryKind, BoardMention, BoardMentionTargetKind,
        },
        logging::{LogEvent, LogLevel},
    };
    use serde_json::Value;

    use crate::{
        branch_list::{
            BranchCleanupAvailability, BranchCleanupInfo, BranchCleanupRisk, BranchListEntry,
            BranchScope,
        },
        persistence::{WindowGeometry, WindowState},
    };

    use super::{
        BackendEvent, BranchEntriesPhase, FrontendEvent, ProfileEntryView, ProfileEnvEntryView,
        ProfileSnapshotView,
    };

    #[test]
    fn update_window_geometry_deserializes_base_geometry_revision_contract() {
        let legacy = serde_json::from_value::<FrontendEvent>(serde_json::json!({
            "kind": "update_window_geometry",
            "id": "w-1",
            "geometry": { "x": 1.0, "y": 2.0, "width": 300.0, "height": 200.0 },
            "cols": 80,
            "rows": 24
        }))
        .expect("deserialize legacy update_window_geometry");

        assert!(matches!(
            legacy,
            FrontendEvent::UpdateWindowGeometry {
                id,
                geometry: WindowGeometry {
                    x: 1.0,
                    y: 2.0,
                    width: 300.0,
                    height: 200.0,
                },
                base_geometry_revision: None,
                ..
            } if id == "w-1"
        ));

        let modern = serde_json::from_value::<FrontendEvent>(serde_json::json!({
            "kind": "update_window_geometry",
            "id": "w-1",
            "geometry": { "x": 1.0, "y": 2.0, "width": 300.0, "height": 200.0 },
            "cols": 80,
            "rows": 24,
            "base_geometry_revision": 7
        }))
        .expect("deserialize modern update_window_geometry");

        assert!(matches!(
            modern,
            FrontendEvent::UpdateWindowGeometry {
                id,
                geometry: WindowGeometry {
                    x: 1.0,
                    y: 2.0,
                    width: 300.0,
                    height: 200.0,
                },
                base_geometry_revision: Some(7),
                ..
            } if id == "w-1"
        ));
    }

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
            projection: Box::new(super::ActiveWorkProjectionView {
                id: "work-1".to_string(),
                title: "Implement Start Work".to_string(),
                status_category: "active".to_string(),
                status_text: "Launching from Project Bar".to_string(),
                summary: Some("Launching from Project Bar".to_string()),
                owner: Some("SPEC-2359".to_string()),
                next_action: Some("Run launch tests".to_string()),
                active_agents: 1,
                blocked_agents: 0,
                branch: Some("work/20260504-1200".to_string()),
                worktree_path: Some("/tmp/repo/work/20260504-1200".to_string()),
                pr_number: Some(2538),
                pr_url: Some("https://github.com/akiojin/gwt/pull/2538".to_string()),
                pr_state: Some("OPEN".to_string()),
                pr_created_at: Some("2026-05-07T08:20:00+00:00".to_string()),
                board_refs: vec!["board-1".to_string()],
                journal_entries: vec![super::WorkspaceJournalEntryView {
                    id: "journal-1".to_string(),
                    updated_at: "2026-05-04T12:01:00Z".to_string(),
                    title: Some("Implement Start Work".to_string()),
                    status_category: Some("active".to_string()),
                    status_text: Some("Launching from Project Bar".to_string()),
                    summary: Some("Launching from Project Bar".to_string()),
                    owner: Some("SPEC-2359".to_string()),
                    next_action: Some("Run launch tests".to_string()),
                    agent_session_id: Some("session-1".to_string()),
                    agent_current_focus: Some("Run launch tests".to_string()),
                    agent_title_summary: Some("Launch tests".to_string()),
                }],
                workspaces: Vec::new(),
                cleanup_candidate: Some(super::ActiveWorkCleanupCandidateView {
                    branch: "work/20260504-1200".to_string(),
                    worktree_path: Some("/tmp/repo/work/20260504-1200".to_string()),
                    reason: "workspace_done".to_string(),
                    default_delete_remote: false,
                    remote_delete_available: true,
                }),
                agents: vec![super::ActiveWorkAgentView {
                    session_id: "session-1".to_string(),
                    window_id: Some("tab-1::agent-1".to_string()),
                    agent_id: "codex".to_string(),
                    display_name: "Codex".to_string(),
                    affiliation_status: "assigned".to_string(),
                    workspace_id: Some("work-1".to_string()),
                    status_category: "active".to_string(),
                    current_focus: Some("Run launch tests".to_string()),
                    title_summary: Some("Launch tests".to_string()),
                    branch: Some("work/20260504-1200".to_string()),
                    worktree_path: Some("/tmp/repo/work/20260504-1200".to_string()),
                    last_board_entry_id: Some("board-1".to_string()),
                    last_board_entry_kind: Some("handoff".to_string()),
                    coordination_scope: Some("SPEC-2359 / start-work".to_string()),
                    updated_at: "2026-05-04T12:00:00Z".to_string(),
                }],
                unassigned_agents: Vec::new(),
            }),
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
        assert_eq!(
            value.pointer("/projection/pr_url").and_then(Value::as_str),
            Some("https://github.com/akiojin/gwt/pull/2538")
        );
        assert_eq!(
            value
                .pointer("/projection/pr_state")
                .and_then(Value::as_str),
            Some("OPEN")
        );
        assert_eq!(
            value
                .pointer("/projection/journal_entries/0/summary")
                .and_then(Value::as_str),
            Some("Launching from Project Bar"),
            "Workspace Overview should receive recent summary journal entries without replaying Board history"
        );
        assert_eq!(
            value
                .pointer("/projection/cleanup_candidate/default_delete_remote")
                .and_then(Value::as_bool),
            Some(false),
            "Workspace cleanup must default to local-only deletion"
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
    fn frontend_event_accepts_github_project_clone_commands() {
        let parent: FrontendEvent = serde_json::from_value(serde_json::json!({
            "kind": "select_clone_project_parent"
        }))
        .expect("deserialize parent picker event");
        assert!(matches!(parent, FrontendEvent::SelectCloneProjectParent));

        let search: FrontendEvent = serde_json::from_value(serde_json::json!({
            "kind": "github_repository_search",
            "query": "akiojin/gwt"
        }))
        .expect("deserialize repository search event");
        assert!(matches!(
            search,
            FrontendEvent::GithubRepositorySearch { query } if query == "akiojin/gwt"
        ));

        let clone: FrontendEvent = serde_json::from_value(serde_json::json!({
            "kind": "clone_project_start",
            "url": "https://github.com/akiojin/gwt.git",
            "parent_path": "/tmp/projects"
        }))
        .expect("deserialize clone start event");
        assert!(matches!(
            clone,
            FrontendEvent::CloneProjectStart { url, parent_path }
                if url == "https://github.com/akiojin/gwt.git"
                    && parent_path == "/tmp/projects"
        ));
    }

    #[test]
    fn clone_project_backend_events_use_distinct_wire_contract() {
        let results = BackendEvent::GithubRepositorySearchResults {
            query: "gwt".to_string(),
            repositories: vec![super::GitHubRepositorySearchResultView {
                full_name: "akiojin/gwt".to_string(),
                description: Some("Git Worktree Manager".to_string()),
                url: "https://github.com/akiojin/gwt".to_string(),
                default_branch: Some("develop".to_string()),
                visibility: Some("public".to_string()),
                updated_at: Some("2026-05-13T00:00:00Z".to_string()),
            }],
        };
        let value = serde_json::to_value(results).expect("serialize search results");
        assert_eq!(
            value.get("kind"),
            Some(&Value::String(
                "github_repository_search_results".to_string()
            ))
        );
        assert_eq!(
            value
                .pointer("/repositories/0/full_name")
                .and_then(Value::as_str),
            Some("akiojin/gwt")
        );

        let error = BackendEvent::CloneProjectError {
            message: "target already exists".to_string(),
        };
        let value = serde_json::to_value(error).expect("serialize clone error");
        assert_eq!(
            value.get("kind"),
            Some(&Value::String("clone_project_error".to_string()))
        );
        assert_eq!(
            value.get("message").and_then(Value::as_str),
            Some("target already exists")
        );
    }

    #[test]
    fn launch_wizard_open_error_serializes_modal_error_contract() {
        let event = BackendEvent::LaunchWizardOpenError {
            title: "Start Work".to_string(),
            message: "Default base branch not found".to_string(),
        };

        let value = serde_json::to_value(&event).expect("serialize launch wizard open error");

        assert_eq!(
            value.get("kind"),
            Some(&Value::String("launch_wizard_open_error".to_string()))
        );
        assert_eq!(
            value.get("title"),
            Some(&Value::String("Start Work".to_string()))
        );
        assert_eq!(
            value.get("message"),
            Some(&Value::String("Default base branch not found".to_string()))
        );
    }

    #[test]
    fn frontend_event_accepts_workspace_resume_sources() {
        let current: FrontendEvent = serde_json::from_value(serde_json::json!({
            "kind": "resume_workspace",
            "source": "current"
        }))
        .expect("deserialize current workspace resume");

        assert!(matches!(
            current,
            FrontendEvent::ResumeWorkspace {
                source: super::WorkspaceResumeSource::Current,
                journal_id: None,
            }
        ));

        let journal: FrontendEvent = serde_json::from_value(serde_json::json!({
            "kind": "resume_workspace",
            "source": "journal",
            "journal_id": "journal-1"
        }))
        .expect("deserialize journal workspace resume");

        assert!(matches!(
            journal,
            FrontendEvent::ResumeWorkspace {
                source: super::WorkspaceResumeSource::Journal,
                journal_id: Some(id),
            } if id == "journal-1"
        ));
    }

    #[test]
    fn frontend_event_accepts_terminal_image_paste_payload() {
        let event: FrontendEvent = serde_json::from_value(serde_json::json!({
            "kind": "paste_image",
            "id": "tab-1::agent-1",
            "data_base64": "cG5nLWJ5dGVz",
            "mime_type": "image/png",
            "filename": "screenshot.png"
        }))
        .expect("deserialize image paste event");

        assert!(
            matches!(
                event,
                FrontendEvent::PasteImage {
                    id,
                    data_base64,
                    mime_type,
                    filename: Some(filename),
                } if id == "tab-1::agent-1"
                    && data_base64 == "cG5nLWJ5dGVz"
                    && mime_type == "image/png"
                    && filename == "screenshot.png"
            ),
            "image paste must expose window id, payload, MIME type, and optional filename"
        );
    }

    #[test]
    fn frontend_event_accepts_terminal_image_paste_without_filename() {
        let event: FrontendEvent = serde_json::from_value(serde_json::json!({
            "kind": "paste_image",
            "id": "tab-1::agent-1",
            "data_base64": "d2VicC1ieXRlcw==",
            "mime_type": "image/webp"
        }))
        .expect("deserialize image paste event without filename");

        assert!(
            matches!(event, FrontendEvent::PasteImage { filename: None, .. }),
            "clipboard images may not have a source filename"
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
    fn board_entries_serializes_typed_mentions_contract() {
        let entry = BoardEntry::new(
            AuthorKind::Agent,
            "codex",
            BoardEntryKind::Question,
            "Can you verify this?",
            None,
            None,
            vec![],
            vec![],
        )
        .with_mention(
            BoardMention::new(BoardMentionTargetKind::User, "akiojin").with_label("Akio"),
        );
        let event = BackendEvent::BoardEntries {
            id: "board-1".to_string(),
            entries: vec![entry],
            has_more_before: false,
        };

        let value = serde_json::to_value(&event).expect("serialize board entries");

        assert_eq!(value["entries"][0]["mentions"][0]["target_kind"], "user");
        assert_eq!(value["entries"][0]["mentions"][0]["target"], "akiojin");
        assert_eq!(value["entries"][0]["mentions"][0]["label"], "Akio");
    }

    #[test]
    fn open_board_origin_agent_deserializes_frontend_event_contract() {
        let event: FrontendEvent = serde_json::from_value(serde_json::json!({
            "kind": "open_board_origin_agent",
            "id": "tab-1::board-1",
            "origin_session_id": "session-origin",
            "bounds": {
                "x": 0.0,
                "y": 0.0,
                "width": 1200.0,
                "height": 800.0
            }
        }))
        .expect("deserialize open board origin agent event");

        assert!(matches!(
            event,
            FrontendEvent::OpenBoardOriginAgent {
                id,
                origin_session_id,
                bounds,
            } if id == "tab-1::board-1"
                && origin_session_id == "session-origin"
                && bounds.as_ref().is_some_and(|bounds| bounds.width == 1200.0)
        ));
    }

    #[test]
    fn post_board_entry_deserializes_typed_mentions() {
        let frontend: FrontendEvent = serde_json::from_value(serde_json::json!({
            "kind": "post_board_entry",
            "id": "board-1",
            "entry_kind": "question",
            "body": "Can you verify this?",
            "parent_id": null,
            "topics": [],
            "owners": [],
            "mentions": [
                {"target_kind": "user", "target": "akiojin", "label": "Akio"},
                {"target_kind": "agent", "target": "codex"}
            ]
        }))
        .expect("deserialize post board entry");

        assert!(matches!(
            frontend,
            FrontendEvent::PostBoardEntry { mentions, .. }
                if mentions.len() == 2
                    && mentions[0].target_kind == BoardMentionTargetKind::User
                    && mentions[0].target == "akiojin"
                    && mentions[1].target_kind == BoardMentionTargetKind::Agent
                    && mentions[1].target == "codex"
        ));
    }

    #[test]
    fn load_board_deserializes_all_view_opt_in() {
        let frontend: FrontendEvent = serde_json::from_value(serde_json::json!({
            "kind": "load_board",
            "id": "board-1",
            "all": true
        }))
        .expect("deserialize load board all");

        assert!(matches!(
            frontend,
            FrontendEvent::LoadBoard { id, all } if id == "board-1" && all
        ));
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
                limit,
                all
            } if id == "board-1" && before_entry_id == "entry-3" && limit == 50 && !all
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
    fn removed_memo_frontend_events_are_not_part_of_the_wire_contract() {
        for kind in [
            "load_memo",
            "create_memo_note",
            "update_memo_note",
            "delete_memo_note",
        ] {
            let event = serde_json::from_value::<FrontendEvent>(serde_json::json!({
                "kind": kind,
                "id": "memo-1",
                "note_id": "note-1",
                "title": "Note",
                "body": "Body",
                "pinned": false
            }));
            assert!(
                event.is_err(),
                "removed Memo frontend event `{kind}` must not deserialize"
            );
        }
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

    #[test]
    fn frontend_event_workspace_projection_prune_round_trips() {
        let payload = r#"{"kind":"workspace_projection_prune","dry_run":true,"ids":["w1","w2"]}"#;
        let event: FrontendEvent =
            serde_json::from_str(payload).expect("deserialize WorkspaceProjectionPrune");
        match event {
            FrontendEvent::WorkspaceProjectionPrune { dry_run, ids } => {
                assert!(dry_run);
                assert_eq!(ids, vec!["w1".to_string(), "w2".to_string()]);
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn frontend_event_workspace_projection_prune_defaults() {
        let payload = r#"{"kind":"workspace_projection_prune"}"#;
        let event: FrontendEvent = serde_json::from_str(payload).expect("deserialize defaults");
        match event {
            FrontendEvent::WorkspaceProjectionPrune { dry_run, ids } => {
                assert!(!dry_run);
                assert!(ids.is_empty());
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn backend_event_workspace_projection_prune_result_serializes() {
        let event = BackendEvent::WorkspaceProjectionPruneResult {
            mode: "dry_run".to_string(),
            archived: 3,
            deleted: 1,
            skipped: 5,
        };
        let value = serde_json::to_value(&event).expect("serialize");
        assert_eq!(value["kind"], "workspace_projection_prune_result");
        assert_eq!(value["mode"], "dry_run");
        assert_eq!(value["archived"], 3);
        assert_eq!(value["deleted"], 1);
        assert_eq!(value["skipped"], 5);
    }

    #[test]
    fn backend_event_workspace_projection_prune_error_serializes() {
        let event = BackendEvent::WorkspaceProjectionPruneError {
            message: "scan failed: permission denied".to_string(),
        };
        let value = serde_json::to_value(&event).expect("serialize");
        assert_eq!(value["kind"], "workspace_projection_prune_error");
        assert_eq!(value["message"], "scan failed: permission denied");
    }
}
