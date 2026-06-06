use gwt_agent::{ClaudeCodeOpenaiCompatInput, CustomCodingAgent, PresetDefinition};
use gwt_core::{
    coordination::{BoardEntry, BoardEntryKind},
    logging::LogEvent,
};
use serde::{Deserialize, Serialize};

use crate::{
    branch_cleanup::{BranchCleanupProgressPhase, BranchCleanupResultEntry},
    branch_list::BranchListEntry,
    daemon_runtime::RuntimeHookEvent,
    file_content::{Encoding, Newline},
    file_tree::FileTreeEntry,
    knowledge_bridge::{KnowledgeDetailView, KnowledgeKind, KnowledgeListItem},
    launch_wizard::{LaunchWizardAction, LaunchWizardView},
    persistence::{
        CanvasViewport, PersistedWindowState, ProjectKind, WindowGeometry, WindowProcessStatus,
    },
    preset::WindowPreset,
    worktree_inventory::WorktreeEntry,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileContentMode {
    Text,
    Hex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileContentErrorKind {
    Denied,
    TooLarge,
    IoError,
    NotAFile,
    BinaryNotText,
    WindowNotFound,
    WindowMismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum IndexSearchScope {
    Issues,
    Specs,
    Memory,
    Discussions,
    Board,
    Files,
    #[serde(rename = "files-docs")]
    FilesDocs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum IndexSearchMatchMode {
    #[default]
    Semantic,
    AllTerms,
}

impl IndexSearchMatchMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Semantic => "semantic",
            Self::AllTerms => "all_terms",
        }
    }
}

impl IndexSearchScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Issues => "issues",
            Self::Specs => "specs",
            Self::Memory => "memory",
            Self::Discussions => "discussions",
            Self::Board => "board",
            Self::Files => "files",
            Self::FilesDocs => "files-docs",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IndexSearchTarget {
    Issue { number: u64 },
    Spec { spec_id: u64 },
    Memory { heading: String, date: String },
    Discussion { heading: String, date: String },
    Board { entry_id: String },
    File { path: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexSearchResult {
    pub scope: IndexSearchScope,
    pub title: String,
    pub subtitle: String,
    pub preview: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distance: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_mode: Option<IndexSearchMatchMode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub matched_terms: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_terms: Vec<String>,
    pub target: IndexSearchTarget,
}

/// SPEC-2006 Phase 2 amendment: structured error variants for the write
/// surface. Kept separate from read-side `FileContentErrorKind` so the GUI
/// can match exhaustively on save-only outcomes (conflict / read-only /
/// out-of-range / encoding fallback).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileContentSaveErrorKind {
    Denied,
    Conflict,
    ReadOnly,
    OutOfRange,
    TooLarge,
    IoError,
    NotAFile,
    WindowNotFound,
    WindowMismatch,
}

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

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum FileAttachment {
    NativePath {
        path: String,
    },
    Inline {
        filename: String,
        mime_type: Option<String>,
        size: u64,
        data_base64: String,
    },
    Uploaded {
        upload_id: String,
        filename: String,
        mime_type: Option<String>,
        size: u64,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FrontendEvent {
    FrontendReady,
    /// Toggle Claude account-usage collection (SPEC-2970 FR-009).
    SetClaudeAccountUsageEnabled {
        enabled: bool,
    },
    /// Request an immediate usage refresh (SPEC-2970 FR-022).
    RefreshUsage,
    StartupAutoResumeReady {
        bounds: WindowGeometry,
    },
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
    PasteImageUploaded {
        id: String,
        upload_id: String,
        mime_type: String,
        filename: Option<String>,
        size: u64,
    },
    AttachFiles {
        id: String,
        files: Vec<FileAttachment>,
    },
    LoadFileTree {
        id: String,
        path: Option<String>,
    },
    ListFileTreeWorktrees {
        id: String,
    },
    SelectFileTreeWorktree {
        id: String,
        worktree_id: String,
    },
    LoadFileContent {
        id: String,
        path: String,
        mode: FileContentMode,
        #[serde(default)]
        hex_offset: Option<u64>,
        #[serde(default)]
        hex_length: Option<u64>,
    },
    /// SPEC-2006 Phase 2 amendment: write the modified text or single hex
    /// byte back to disk. `expected_mtime` / `expected_size` are the values
    /// returned by the most recent read; mismatch raises Conflict.
    SaveFileContent {
        id: String,
        path: String,
        mode: FileContentMode,
        expected_mtime: u64,
        expected_size: u64,
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        encoding: Option<Encoding>,
        #[serde(default)]
        newline: Option<Newline>,
        #[serde(default)]
        has_bom: Option<bool>,
        #[serde(default)]
        hex_offset: Option<u64>,
        #[serde(default)]
        hex_byte: Option<u8>,
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
    /// SPEC-2809 Phase F2 — Console window mounts and asks the backend for
    /// the current `ProcessConsoleHub` ring buffer so historical lines
    /// (e.g. gh calls that happened before the window opened) are visible
    /// immediately. Reply is [`BackendEvent::ProcessConsoleSnapshot`].
    LoadProcessConsole {
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
    SearchProjectIndex {
        id: String,
        query: String,
        request_id: u64,
        scopes: Vec<IndexSearchScope>,
        #[serde(default)]
        worktree_hash: Option<String>,
        #[serde(default)]
        match_mode: IndexSearchMatchMode,
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
        #[serde(default)]
        force_filesystem_delete: bool,
    },
    RunWorkspaceCleanup {
        branch: String,
        delete_remote: bool,
        #[serde(default)]
        force_filesystem_delete: bool,
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
    /// Request the full Project Index health table. Startup only probes the
    /// current worktree; Settings.Index asks for the expensive all-worktree
    /// view on demand.
    RefreshIndexStatus {
        project_root: String,
    },
    PostBoardEntry {
        id: String,
        entry_kind: BoardEntryKind,
        body: String,
        /// SPEC-2963: optional post title/subject from the composer.
        #[serde(default)]
        title: Option<String>,
        parent_id: Option<String>,
        topics: Vec<String>,
        owners: Vec<String>,
        #[serde(default)]
        targets: Vec<String>,
        #[serde(default)]
        mentions: Vec<gwt_core::coordination::BoardMention>,
        /// SPEC-2959: composer "To:" selection. Pins the post to a specific
        /// Work lane (its workspace id). `None` uses the active-workspace default.
        #[serde(default)]
        target_workspace: Option<String>,
        /// SPEC-2959: when `true`, post to the General lane (empty audience)
        /// regardless of `target_workspace` or the active workspace.
        #[serde(default)]
        broadcast: bool,
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
    /// SPEC-2359 US-42: enumerate agents that can be resumed for the
    /// current Workspace. The Resume button on a Workspace card sends
    /// this instead of [`Self::ResumeWorkspace`] so the user can pick
    /// which previous agent to restart without going through the Launch
    /// Wizard.
    ListResumableAgents {
        #[serde(default)]
        workspace_id: Option<String>,
    },
    /// SPEC-2359 US-42: spawn a single previously-assigned agent in the
    /// current Workspace without opening the Launch Wizard. The
    /// `session_id` matches one of the entries returned by
    /// [`BackendEvent::WorkspaceResumableAgents`]. `bounds` carries the
    /// frontend's current viewport so the spawned agent window appears at
    /// a sensible position inside the visible canvas.
    ResumeWorkspaceAgent {
        session_id: String,
        bounds: WindowGeometry,
    },
    ResumeBranchLatestAgent {
        id: String,
        branch_name: String,
        bounds: WindowGeometry,
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
    /// SPEC-2785 US-1 / FR-C: user clicked the server URL cell in the status
    /// strip. Backend opens the URL in the OS default browser only when it
    /// matches the embedded server's bound URL (FR-E same-origin gate). The
    /// renderer-supplied `url` is treated as untrusted and validated against
    /// `AppRuntime::server_url` before any opener is spawned.
    OpenServerUrl {
        url: String,
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
    /// Settings > Agent Backends (SPEC-1921 2026-05-18 amendment / FR-099):
    /// list every saved Backend Override profile for the given built-in
    /// agent. Response is [`BackendEvent::AgentBackendList`].
    ListAgentBackends {
        agent: gwt_agent::BuiltinAgentId,
    },
    /// Settings > Agent Backends > Add: persist a new Backend Override
    /// profile under `[builtinAgents.<agent>.backends.<id>]`. Response is
    /// [`BackendEvent::AgentBackendSaved`] on success or
    /// [`BackendEvent::AgentBackendError`] on failure.
    AddAgentBackend {
        agent: gwt_agent::BuiltinAgentId,
        profile: Box<gwt_agent::AgentBackendProfile>,
    },
    /// Settings > Agent Backends > Edit: replace an existing profile in
    /// place. The patch id must match an existing entry.
    UpdateAgentBackend {
        agent: gwt_agent::BuiltinAgentId,
        id: String,
        profile: Box<gwt_agent::AgentBackendProfile>,
    },
    /// Settings > Agent Backends > Delete: remove the profile with the
    /// given id.
    DeleteAgentBackend {
        agent: gwt_agent::BuiltinAgentId,
        id: String,
    },
    /// Settings > Agent Backends > Test connection: same `/v1/models` probe
    /// as [`FrontendEvent::TestBackendConnection`], scoped to the chosen
    /// built-in agent for future per-agent probe variation.
    TestAgentBackendConnection {
        agent: gwt_agent::BuiltinAgentId,
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
    /// SPEC-2963: query remote Board provider sign-in state. Backend replies
    /// with [`BackendEvent::BoardAuthStatus`].
    GetBoardAuthStatus,
    /// SPEC-2963: begin OAuth sign-in for a remote Board provider
    /// (`slack` / `teams`). Backend opens the browser and replies with
    /// [`BackendEvent::BoardAuthStatus`] (message describes next steps).
    BoardProviderSignIn {
        provider: String,
    },
    /// SPEC-2963: clear stored credentials for a remote Board provider.
    BoardProviderSignOut {
        provider: String,
    },
    /// SPEC-2963: persist remote Board provider configuration from the settings
    /// UI. Non-secret fields (`client_id`, `default_channel`, `tenant_id`) are
    /// written to `config.toml`; `client_secret` is routed to the secure
    /// credential store, never to `config.toml` (FR-006). Each `Some("")`
    /// clears that field; `None` leaves it unchanged. Backend replies with
    /// [`BackendEvent::BoardAuthStatus`] carrying the refreshed config view.
    UpdateBoardProviderConfig {
        /// `slack` or `teams`.
        provider: String,
        #[serde(default)]
        client_id: Option<String>,
        #[serde(default)]
        default_channel: Option<String>,
        #[serde(default)]
        tenant_id: Option<String>,
        #[serde(default)]
        client_secret: Option<String>,
    },
    /// SPEC-2963 FR-005: persist the fixed loopback OAuth callback port from the
    /// settings UI. `0` resets to the default (8765). Backend replies with
    /// [`BackendEvent::BoardAuthStatus`] carrying the canonical port. Takes
    /// effect on the next launch (the callback listener binds at server boot).
    UpdateBoardOauthPort {
        port: u16,
    },
    /// SPEC-1933 US-4: Settings > System > Language select changed. Backend
    /// persists the value to `~/.gwt/config.toml` under `[ai].language` and
    /// replies with [`BackendEvent::SystemSettingsUpdated`] on success or
    /// [`BackendEvent::SystemSettingsError`] on failure.
    UpdateSystemSettings {
        language: String,
        #[serde(default)]
        codex_trust_managed_hooks: Option<bool>,
        /// SPEC-2959: Board provider selection (`local` / `slack` / `teams`).
        /// `None` leaves the persisted value unchanged.
        #[serde(default)]
        board_provider: Option<String>,
    },
    /// SPEC #2920 Phase 11: Settings > System opened. Backend replies with
    /// the current OS autostart registration state for this user.
    GetAutostartStatus,
    /// SPEC #2920 Phase 11: Settings > System > Launch GWT at login changed.
    /// Backend installs or uninstalls the per-user autostart registration and
    /// replies with the authoritative status on success.
    UpdateAutostart {
        enabled: bool,
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
    /// Diagnostics > Stop UI Trace: persist the browser-side metadata-only
    /// trace payload as a project-scoped JSONL artifact. Backend replies with
    /// [`BackendEvent::UiTraceSaved`] or [`BackendEvent::UiTraceError`].
    SaveUiTrace {
        trace: UiTracePayload,
    },
    /// SPEC #2780 Release Notes window. Request the bundled CHANGELOG entries
    /// and ask the frontend to open / focus the Release Notes window on
    /// `focus_version` when set (otherwise the newest entry). Backend replies
    /// with [`BackendEvent::ReleaseNotesPayload`] on success or
    /// [`BackendEvent::ReleaseNotesError`] when no entries could be produced.
    OpenReleaseNotes {
        id: String,
        #[serde(default)]
        focus_version: Option<String>,
    },
    /// SPEC #2780 v2 Amendment (FR-014): user clicked the Update / Downgrade
    /// action button in the Release Notes window. Backend resolves the
    /// release-by-tag, builds [`gwt_core::update::UpdateState::Available`]
    /// with the chosen version's platform-specific asset, then drives the
    /// existing prepare → apply pipeline. Progress and completion broadcast
    /// via [`BackendEvent::UpdateProgress`] / [`BackendEvent::UpdateReady`]
    /// / [`BackendEvent::UpdateApplyError`] (the same modal the standard
    /// update CTA uses).
    ApplyUpdateToVersion {
        version: String,
    },
    /// SPEC-2359 Phase W-12 Slice 4 (FR-352): the user closed a Work from the
    /// Work surface. `work_id` is the Work item id (`work-session-<session_id>`
    /// for agent-session Works); `close_kind` is `"done"` or `"discarded"`.
    /// Done records a terminal completion, Discarded a terminal discard; both
    /// remove the Work from the active surface. The backend blocks the close
    /// when the owning agent session is still live (the worktree is only
    /// removed for Paused Works that have no running agent).
    CloseWork {
        work_id: String,
        close_kind: String,
    },
}

/// Browser-side metadata-only UI trace payload sent by Diagnostics > Stop UI
/// Trace. Top-level fields are typed so backend validation is explicit, while
/// individual entries remain schema-flexible for low-friction diagnostics.
#[derive(Debug, Clone, Deserialize)]
pub struct UiTracePayload {
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    entries: Option<Vec<UiTraceEntry>>,
}

impl UiTracePayload {
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    pub fn entries(&self) -> Result<&[UiTraceEntry], &'static str> {
        self.entries
            .as_deref()
            .ok_or("trace payload missing entries array")
    }
}

/// One trace entry. Non-object entries are preserved as invalid entries so the
/// artifact can still be written and inspected instead of dropping the session.
#[derive(Debug, Clone)]
pub struct UiTraceEntry {
    fields: Option<serde_json::Map<String, serde_json::Value>>,
}

impl UiTraceEntry {
    pub fn field(&self, key: &str) -> Option<&serde_json::Value> {
        self.fields.as_ref()?.get(key)
    }

    pub fn fields(&self) -> Option<&serde_json::Map<String, serde_json::Value>> {
        self.fields.as_ref()
    }
}

impl<'de> Deserialize<'de> for UiTraceEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        let fields = match value {
            serde_json::Value::Object(fields) => Some(fields),
            _ => None,
        };
        Ok(Self { fields })
    }
}

fn default_board_history_limit() -> usize {
    50
}

#[allow(dead_code)]
fn default_newline() -> Newline {
    Newline::Lf
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceView {
    pub viewport: CanvasViewport,
    pub windows: Vec<PersistedWindowState>,
    // Compatibility field only. Workspace history is intentionally not carried
    // by frequently-broadcast workspace_state events; active_work_projection is
    // the owner for work item / history payloads.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub work_items: Vec<WorkspaceHistoryView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectTabView {
    pub id: String,
    pub title: String,
    pub project_root: String,
    pub kind: ProjectKind,
    pub workspace: WorkspaceView,
    #[serde(default)]
    pub running_agent_count: u32,
    #[serde(default)]
    pub running_agents: Vec<RunningAgentSummary>,
}

// SPEC-2013 FR-011: project tab close 確認 modal が表示する running agent の
// 最小情報。`display_name` は `dynamic_title` → `purpose_title` → `title` の
// 優先順で、`branch` は worktree から導出する。frontend はこのリストを
// 集計結果として消費し、tab close 経路の確認ダイアログに反映する。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunningAgentSummary {
    pub display_name: String,
    pub branch: Option<String>,
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
    #[serde(default)]
    pub os_env: Vec<ProfileEnvEntryView>,
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

/// SPEC-2359 Phase W-12 (FR-349): default wire value for
/// [`ActiveWorkItemView::lifecycle_state`] when deserializing payloads that
/// predate the field. Legacy active Work entries are always live (a group of
/// an assigned, running agent), so the back-compat default is `"active"`.
fn default_work_lifecycle_state() -> String {
    "active".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActiveWorkItemView {
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
    pub board_refs: Vec<String>,
    pub agents: Vec<ActiveWorkAgentView>,
    /// SPEC-2359 Phase W-12 (FR-349): agent-session Work lifecycle state
    /// (active / paused / done / discarded). Distinct from `status_category`
    /// which tracks runtime agent activity. Back-compat default is `"active"`.
    #[serde(default = "default_work_lifecycle_state")]
    pub lifecycle_state: String,
    /// SPEC-2359 Phase W-12 (FR-349): RFC3339 timestamp of the explicit user
    /// close (Done / Discarded). None while the Work is active / paused.
    #[serde(default)]
    pub closed_at: Option<String>,
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
    #[serde(default, alias = "workspaces", alias = "work_items")]
    pub works: Vec<WorkspaceHistoryView>,
    pub cleanup_candidate: Option<ActiveWorkCleanupCandidateView>,
    #[serde(default)]
    pub active_work_count: usize,
    #[serde(default)]
    pub active_works: Vec<ActiveWorkItemView>,
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
    /// Provider usage snapshot: account-level windows + per-session usage +
    /// daily/weekly consumption (SPEC-2970 FR-010). Reuses the gwt-core domain
    /// types directly.
    ProviderUsage {
        accounts: Vec<gwt_core::usage::ProviderUsage>,
        sessions: Vec<gwt_core::usage::SessionUsage>,
        consumption: Vec<gwt_core::usage::ProviderConsumption>,
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
    FileTreeWorktrees {
        id: String,
        entries: Vec<WorktreeEntry>,
    },
    FileTreeWorktreeSelected {
        id: String,
        worktree_id: String,
    },
    FileTreeWorktreeError {
        id: String,
        message: String,
    },
    FileContentText {
        id: String,
        path: String,
        encoding: Encoding,
        text: String,
        total_size: u64,
        // SPEC-2006 Phase 2 amendment: extra fields the GUI needs to support
        // dirty/save/conflict flows. Defaults keep older clients compiling.
        #[serde(default)]
        mtime: u64,
        #[serde(default)]
        has_bom: bool,
        #[serde(default = "default_newline")]
        newline: Newline,
        #[serde(default)]
        read_only: bool,
    },
    FileContentHex {
        id: String,
        path: String,
        offset: u64,
        bytes_b64: String,
        total_size: u64,
        #[serde(default)]
        mtime: u64,
        #[serde(default)]
        read_only: bool,
    },
    FileContentError {
        id: String,
        path: String,
        error_kind: FileContentErrorKind,
        message: String,
        #[serde(default)]
        size: Option<u64>,
        #[serde(default)]
        limit: Option<u64>,
    },
    /// SPEC-2006 Phase 2 amendment: successful write. `new_mtime` / `new_size`
    /// become the next `expected_*` baseline so subsequent saves keep their
    /// conflict checks aligned with what is actually on disk.
    FileContentSaved {
        id: String,
        path: String,
        mode: FileContentMode,
        new_mtime: u64,
        new_size: u64,
        #[serde(default)]
        encoding_fallback: u64,
    },
    FileContentSaveError {
        id: String,
        path: String,
        mode: FileContentMode,
        error_kind: FileContentSaveErrorKind,
        message: String,
        #[serde(default)]
        current_mtime: Option<u64>,
        #[serde(default)]
        current_size: Option<u64>,
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
    /// SPEC-2809 — A single redacted, ANSI-stripped stdout/stderr line
    /// from an external process (gh / git / docker / agent / runner)
    /// piped through `gwt_core::process_console::ProcessConsoleHub`.
    /// Console window and Logs window both consume this event.
    ProcessLine {
        line: gwt_core::process_console::ProcessLine,
    },
    /// SPEC-2809 Phase F2 — reply to [`FrontendEvent::LoadProcessConsole`]
    /// containing the current ring buffer for every kind, time-sorted.
    /// The Console window controller replays these into its per-kind
    /// buffers so historical lines are visible on first mount.
    ProcessConsoleSnapshot {
        id: String,
        lines: Vec<gwt_core::process_console::ProcessLine>,
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
    ProjectIndexSearchResults {
        id: String,
        query: String,
        request_id: u64,
        results: Vec<IndexSearchResult>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        suggestions: Vec<IndexSearchResult>,
    },
    ProjectIndexSearchError {
        id: String,
        query: String,
        request_id: u64,
        message: String,
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
    BranchCleanupProgress {
        id: String,
        branch: String,
        execution_branch: Option<String>,
        index: usize,
        total: usize,
        phase: BranchCleanupProgressPhase,
        message: String,
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
    /// SPEC-2359 US-42: response to [`FrontendEvent::ListResumableAgents`].
    /// `agents` is empty when the active Workspace has no resumable
    /// agents (no `is_assigned()` entry with a recoverable session id),
    /// so the picker modal can render an explicit "Nothing to resume"
    /// notice instead of silently leaving the user without feedback.
    WorkspaceResumableAgents {
        agents: Vec<crate::launch_wizard::ResumableAgentView>,
        #[serde(skip_serializing_if = "Option::is_none")]
        workspace_id: Option<String>,
    },
    /// SPEC-2359 US-42: spawn failure for [`FrontendEvent::ResumeWorkspaceAgent`].
    /// Client-scoped reply so the picker modal can keep itself open and
    /// re-enable the selected entry with the user-facing reason.
    WorkspaceResumeAgentError {
        session_id: String,
        message: String,
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
    /// Response to [`FrontendEvent::ListAgentBackends`].
    AgentBackendList {
        agent: gwt_agent::BuiltinAgentId,
        backends: Vec<gwt_agent::AgentBackendProfile>,
    },
    /// Response to [`FrontendEvent::AddAgentBackend`] /
    /// [`FrontendEvent::UpdateAgentBackend`] (save success).
    AgentBackendSaved {
        agent: gwt_agent::BuiltinAgentId,
        profile: Box<gwt_agent::AgentBackendProfile>,
    },
    /// Response to [`FrontendEvent::DeleteAgentBackend`].
    AgentBackendDeleted {
        agent: gwt_agent::BuiltinAgentId,
        id: String,
    },
    /// Error reply for any agent-backend mutation or probe request.
    AgentBackendError {
        agent: gwt_agent::BuiltinAgentId,
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
        /// SPEC-2959: current Board provider (`local` / `slack` / `teams`).
        #[serde(skip_serializing_if = "Option::is_none")]
        board_provider: Option<String>,
    },
    /// SPEC-2963: remote Board provider sign-in state, the editable provider
    /// configuration (non-secret), and an optional status message. The settings
    /// UI uses the `*_client_id` / `*_default_channel` / `*_tenant_id` fields to
    /// prefill its inputs and the `*_has_secret` flags to show "configured"
    /// without ever echoing the secret.
    BoardAuthStatus {
        slack: bool,
        teams: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        slack_client_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        slack_default_channel: Option<String>,
        #[serde(default)]
        slack_has_secret: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        teams_client_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        teams_tenant_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        teams_default_channel: Option<String>,
        /// Fixed loopback port for the OAuth callback. The settings UI shows the
        /// redirect URL `http://127.0.0.1:<port>/oauth/callback` to register.
        #[serde(default)]
        oauth_redirect_port: u16,
    },
    /// SPEC-1933 US-4: confirmation that
    /// [`FrontendEvent::UpdateSystemSettings`] persisted successfully.
    /// `language` echoes the saved value so the frontend can reconcile
    /// optimistic UI with the authoritative config state.
    SystemSettingsUpdated {
        language: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        codex_trust_managed_hooks: Option<bool>,
        /// SPEC-2959: persisted Board provider echoed back for reconciliation.
        #[serde(skip_serializing_if = "Option::is_none")]
        board_provider: Option<String>,
    },
    /// SPEC-1933 US-4: error reply for [`FrontendEvent::GetSystemSettings`]
    /// or [`FrontendEvent::UpdateSystemSettings`]. `message` is
    /// human-readable; the frontend surfaces it as an inline status row in
    /// the System tab.
    SystemSettingsError {
        message: String,
    },
    /// SPEC #2920 Phase 11: response to
    /// [`FrontendEvent::GetAutostartStatus`] or
    /// [`FrontendEvent::UpdateAutostart`]. Carries the authoritative
    /// per-user OS autostart registration state.
    AutostartStatus {
        enabled: bool,
        mechanism: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        install_path: Option<String>,
    },
    /// SPEC #2920 Phase 11: error reply for autostart status/update. The
    /// frontend surfaces this inline in Settings > System.
    AutostartError {
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
    /// Diagnostics trace artifact was written to the project log directory.
    UiTraceSaved {
        path: String,
        entries: usize,
    },
    /// Diagnostics trace artifact could not be written or the payload was
    /// malformed.
    UiTraceError {
        message: String,
    },
    /// SPEC #2780 Release Notes window payload. Carries the parsed entries
    /// from the bundled `CHANGELOG.md` so the frontend can render the
    /// sidebar + content pane without further round-trips.
    ///
    /// `current_version` (SPEC #2780 v2 Amendment / FR-013) lets the frontend
    /// label the Update / Downgrade / Current action button without a second
    /// round-trip. The value is the running binary's `CARGO_PKG_VERSION`.
    ReleaseNotesPayload {
        id: String,
        entries: Vec<gwt_core::release_notes::ReleaseEntry>,
        #[serde(skip_serializing_if = "Option::is_none")]
        focus_version: Option<String>,
        current_version: String,
    },
    /// SPEC #2780 Release Notes window error. Emitted only when the bundled
    /// changelog yielded no entries; the UI renders an error pane pointing
    /// to the canonical CHANGELOG location.
    ReleaseNotesError {
        id: String,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendEventDeliveryClass {
    Streamed,
    IdempotentLatest,
    Snapshot,
    EphemeralStatus,
    Error,
    BestEffortDaemon,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendEventBackpressurePolicy {
    PreserveOrder,
    LatestWins,
    ClientScopedSnapshot,
    BestEffort,
    FailOpenError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackendEventPolicy {
    pub kind: &'static str,
    pub delivery: BackendEventDeliveryClass,
    pub backpressure: BackendEventBackpressurePolicy,
}

impl BackendEventPolicy {
    const fn new(
        kind: &'static str,
        delivery: BackendEventDeliveryClass,
        backpressure: BackendEventBackpressurePolicy,
    ) -> Self {
        Self {
            kind,
            delivery,
            backpressure,
        }
    }

    pub fn coalesces_on_frontend(self) -> bool {
        matches!(self.delivery, BackendEventDeliveryClass::IdempotentLatest)
    }
}

pub const BACKEND_EVENT_POLICIES: &[BackendEventPolicy] = &[
    BackendEventPolicy::new(
        "workspace_state",
        BackendEventDeliveryClass::IdempotentLatest,
        BackendEventBackpressurePolicy::LatestWins,
    ),
    BackendEventPolicy::new(
        "active_work_projection",
        BackendEventDeliveryClass::IdempotentLatest,
        BackendEventBackpressurePolicy::LatestWins,
    ),
    BackendEventPolicy::new(
        "window_list",
        BackendEventDeliveryClass::IdempotentLatest,
        BackendEventBackpressurePolicy::LatestWins,
    ),
    BackendEventPolicy::new(
        "provider_usage",
        BackendEventDeliveryClass::IdempotentLatest,
        BackendEventBackpressurePolicy::LatestWins,
    ),
    BackendEventPolicy::new(
        "terminal_output",
        BackendEventDeliveryClass::Streamed,
        BackendEventBackpressurePolicy::PreserveOrder,
    ),
    BackendEventPolicy::new(
        "terminal_snapshot",
        BackendEventDeliveryClass::Snapshot,
        BackendEventBackpressurePolicy::ClientScopedSnapshot,
    ),
    BackendEventPolicy::new(
        "terminal_status",
        BackendEventDeliveryClass::EphemeralStatus,
        BackendEventBackpressurePolicy::BestEffort,
    ),
    BackendEventPolicy::new(
        "window_state",
        BackendEventDeliveryClass::EphemeralStatus,
        BackendEventBackpressurePolicy::BestEffort,
    ),
    BackendEventPolicy::new(
        "file_tree_entries",
        BackendEventDeliveryClass::Snapshot,
        BackendEventBackpressurePolicy::ClientScopedSnapshot,
    ),
    BackendEventPolicy::new(
        "file_tree_error",
        BackendEventDeliveryClass::Error,
        BackendEventBackpressurePolicy::FailOpenError,
    ),
    BackendEventPolicy::new(
        "file_tree_worktrees",
        BackendEventDeliveryClass::Snapshot,
        BackendEventBackpressurePolicy::ClientScopedSnapshot,
    ),
    BackendEventPolicy::new(
        "file_tree_worktree_selected",
        BackendEventDeliveryClass::EphemeralStatus,
        BackendEventBackpressurePolicy::BestEffort,
    ),
    BackendEventPolicy::new(
        "file_tree_worktree_error",
        BackendEventDeliveryClass::Error,
        BackendEventBackpressurePolicy::FailOpenError,
    ),
    BackendEventPolicy::new(
        "file_content_text",
        BackendEventDeliveryClass::Snapshot,
        BackendEventBackpressurePolicy::ClientScopedSnapshot,
    ),
    BackendEventPolicy::new(
        "file_content_hex",
        BackendEventDeliveryClass::Snapshot,
        BackendEventBackpressurePolicy::ClientScopedSnapshot,
    ),
    BackendEventPolicy::new(
        "file_content_error",
        BackendEventDeliveryClass::Error,
        BackendEventBackpressurePolicy::FailOpenError,
    ),
    BackendEventPolicy::new(
        "file_content_saved",
        BackendEventDeliveryClass::Snapshot,
        BackendEventBackpressurePolicy::ClientScopedSnapshot,
    ),
    BackendEventPolicy::new(
        "file_content_save_error",
        BackendEventDeliveryClass::Error,
        BackendEventBackpressurePolicy::FailOpenError,
    ),
    BackendEventPolicy::new(
        "branch_entries",
        BackendEventDeliveryClass::Snapshot,
        BackendEventBackpressurePolicy::ClientScopedSnapshot,
    ),
    BackendEventPolicy::new(
        "board_entries",
        BackendEventDeliveryClass::Snapshot,
        BackendEventBackpressurePolicy::ClientScopedSnapshot,
    ),
    BackendEventPolicy::new(
        "board_history_page",
        BackendEventDeliveryClass::Snapshot,
        BackendEventBackpressurePolicy::ClientScopedSnapshot,
    ),
    BackendEventPolicy::new(
        "profile_snapshot",
        BackendEventDeliveryClass::Snapshot,
        BackendEventBackpressurePolicy::ClientScopedSnapshot,
    ),
    BackendEventPolicy::new(
        "log_entries",
        BackendEventDeliveryClass::Snapshot,
        BackendEventBackpressurePolicy::ClientScopedSnapshot,
    ),
    BackendEventPolicy::new(
        "log_entry_appended",
        BackendEventDeliveryClass::Streamed,
        BackendEventBackpressurePolicy::PreserveOrder,
    ),
    BackendEventPolicy::new(
        "knowledge_entries",
        BackendEventDeliveryClass::Snapshot,
        BackendEventBackpressurePolicy::ClientScopedSnapshot,
    ),
    BackendEventPolicy::new(
        "knowledge_search_results",
        BackendEventDeliveryClass::Snapshot,
        BackendEventBackpressurePolicy::ClientScopedSnapshot,
    ),
    BackendEventPolicy::new(
        "project_index_search_results",
        BackendEventDeliveryClass::Snapshot,
        BackendEventBackpressurePolicy::ClientScopedSnapshot,
    ),
    BackendEventPolicy::new(
        "project_index_search_error",
        BackendEventDeliveryClass::Error,
        BackendEventBackpressurePolicy::FailOpenError,
    ),
    BackendEventPolicy::new(
        "knowledge_detail",
        BackendEventDeliveryClass::Snapshot,
        BackendEventBackpressurePolicy::ClientScopedSnapshot,
    ),
    BackendEventPolicy::new(
        "knowledge_bridge_phase_updated",
        BackendEventDeliveryClass::EphemeralStatus,
        BackendEventBackpressurePolicy::BestEffort,
    ),
    BackendEventPolicy::new(
        "branch_cleanup_result",
        BackendEventDeliveryClass::EphemeralStatus,
        BackendEventBackpressurePolicy::BestEffort,
    ),
    BackendEventPolicy::new(
        "branch_cleanup_progress",
        BackendEventDeliveryClass::Streamed,
        BackendEventBackpressurePolicy::PreserveOrder,
    ),
    BackendEventPolicy::new(
        "branch_error",
        BackendEventDeliveryClass::Error,
        BackendEventBackpressurePolicy::FailOpenError,
    ),
    BackendEventPolicy::new(
        "board_error",
        BackendEventDeliveryClass::Error,
        BackendEventBackpressurePolicy::FailOpenError,
    ),
    BackendEventPolicy::new(
        "profile_error",
        BackendEventDeliveryClass::Error,
        BackendEventBackpressurePolicy::FailOpenError,
    ),
    BackendEventPolicy::new(
        "log_error",
        BackendEventDeliveryClass::Error,
        BackendEventBackpressurePolicy::FailOpenError,
    ),
    BackendEventPolicy::new(
        "knowledge_error",
        BackendEventDeliveryClass::Error,
        BackendEventBackpressurePolicy::FailOpenError,
    ),
    BackendEventPolicy::new(
        "project_open_error",
        BackendEventDeliveryClass::Error,
        BackendEventBackpressurePolicy::FailOpenError,
    ),
    BackendEventPolicy::new(
        "clone_project_parent_selected",
        BackendEventDeliveryClass::EphemeralStatus,
        BackendEventBackpressurePolicy::BestEffort,
    ),
    BackendEventPolicy::new(
        "github_repository_search_results",
        BackendEventDeliveryClass::EphemeralStatus,
        BackendEventBackpressurePolicy::BestEffort,
    ),
    BackendEventPolicy::new(
        "github_repository_search_error",
        BackendEventDeliveryClass::Error,
        BackendEventBackpressurePolicy::FailOpenError,
    ),
    BackendEventPolicy::new(
        "clone_project_progress",
        BackendEventDeliveryClass::Streamed,
        BackendEventBackpressurePolicy::PreserveOrder,
    ),
    BackendEventPolicy::new(
        "clone_project_done",
        BackendEventDeliveryClass::EphemeralStatus,
        BackendEventBackpressurePolicy::BestEffort,
    ),
    BackendEventPolicy::new(
        "clone_project_error",
        BackendEventDeliveryClass::Error,
        BackendEventBackpressurePolicy::FailOpenError,
    ),
    BackendEventPolicy::new(
        "launch_wizard_open_error",
        BackendEventDeliveryClass::Error,
        BackendEventBackpressurePolicy::FailOpenError,
    ),
    BackendEventPolicy::new(
        "launch_wizard_state",
        BackendEventDeliveryClass::IdempotentLatest,
        BackendEventBackpressurePolicy::LatestWins,
    ),
    BackendEventPolicy::new(
        "workspace_resumable_agents",
        BackendEventDeliveryClass::Snapshot,
        BackendEventBackpressurePolicy::ClientScopedSnapshot,
    ),
    BackendEventPolicy::new(
        "workspace_resume_agent_error",
        BackendEventDeliveryClass::Error,
        BackendEventBackpressurePolicy::FailOpenError,
    ),
    BackendEventPolicy::new(
        "launch_progress",
        BackendEventDeliveryClass::Streamed,
        BackendEventBackpressurePolicy::PreserveOrder,
    ),
    BackendEventPolicy::new(
        "project_index_status",
        BackendEventDeliveryClass::IdempotentLatest,
        BackendEventBackpressurePolicy::LatestWins,
    ),
    BackendEventPolicy::new(
        "runtime_hook_event",
        BackendEventDeliveryClass::BestEffortDaemon,
        BackendEventBackpressurePolicy::BestEffort,
    ),
    BackendEventPolicy::new(
        "update_state",
        BackendEventDeliveryClass::IdempotentLatest,
        BackendEventBackpressurePolicy::LatestWins,
    ),
    BackendEventPolicy::new(
        "update_progress",
        BackendEventDeliveryClass::Streamed,
        BackendEventBackpressurePolicy::PreserveOrder,
    ),
    BackendEventPolicy::new(
        "update_ready",
        BackendEventDeliveryClass::EphemeralStatus,
        BackendEventBackpressurePolicy::BestEffort,
    ),
    BackendEventPolicy::new(
        "update_apply_pending_persisted",
        BackendEventDeliveryClass::EphemeralStatus,
        BackendEventBackpressurePolicy::BestEffort,
    ),
    BackendEventPolicy::new(
        "update_apply_error",
        BackendEventDeliveryClass::Error,
        BackendEventBackpressurePolicy::FailOpenError,
    ),
    BackendEventPolicy::new(
        "custom_agent_list",
        BackendEventDeliveryClass::Snapshot,
        BackendEventBackpressurePolicy::ClientScopedSnapshot,
    ),
    BackendEventPolicy::new(
        "custom_agent_preset_list",
        BackendEventDeliveryClass::Snapshot,
        BackendEventBackpressurePolicy::ClientScopedSnapshot,
    ),
    BackendEventPolicy::new(
        "custom_agent_saved",
        BackendEventDeliveryClass::EphemeralStatus,
        BackendEventBackpressurePolicy::BestEffort,
    ),
    BackendEventPolicy::new(
        "custom_agent_deleted",
        BackendEventDeliveryClass::EphemeralStatus,
        BackendEventBackpressurePolicy::BestEffort,
    ),
    BackendEventPolicy::new(
        "backend_connection_result",
        BackendEventDeliveryClass::Snapshot,
        BackendEventBackpressurePolicy::ClientScopedSnapshot,
    ),
    BackendEventPolicy::new(
        "custom_agent_error",
        BackendEventDeliveryClass::Error,
        BackendEventBackpressurePolicy::FailOpenError,
    ),
    BackendEventPolicy::new(
        "agent_backend_list",
        BackendEventDeliveryClass::Snapshot,
        BackendEventBackpressurePolicy::ClientScopedSnapshot,
    ),
    BackendEventPolicy::new(
        "agent_backend_saved",
        BackendEventDeliveryClass::EphemeralStatus,
        BackendEventBackpressurePolicy::BestEffort,
    ),
    BackendEventPolicy::new(
        "agent_backend_deleted",
        BackendEventDeliveryClass::EphemeralStatus,
        BackendEventBackpressurePolicy::BestEffort,
    ),
    BackendEventPolicy::new(
        "agent_backend_error",
        BackendEventDeliveryClass::Error,
        BackendEventBackpressurePolicy::FailOpenError,
    ),
    BackendEventPolicy::new(
        "migration_detected",
        BackendEventDeliveryClass::EphemeralStatus,
        BackendEventBackpressurePolicy::BestEffort,
    ),
    BackendEventPolicy::new(
        "migration_progress",
        BackendEventDeliveryClass::Streamed,
        BackendEventBackpressurePolicy::PreserveOrder,
    ),
    BackendEventPolicy::new(
        "migration_done",
        BackendEventDeliveryClass::EphemeralStatus,
        BackendEventBackpressurePolicy::BestEffort,
    ),
    BackendEventPolicy::new(
        "migration_error",
        BackendEventDeliveryClass::Error,
        BackendEventBackpressurePolicy::FailOpenError,
    ),
    BackendEventPolicy::new(
        "system_settings",
        BackendEventDeliveryClass::Snapshot,
        BackendEventBackpressurePolicy::ClientScopedSnapshot,
    ),
    BackendEventPolicy::new(
        "system_settings_updated",
        BackendEventDeliveryClass::EphemeralStatus,
        BackendEventBackpressurePolicy::BestEffort,
    ),
    BackendEventPolicy::new(
        "system_settings_error",
        BackendEventDeliveryClass::Error,
        BackendEventBackpressurePolicy::FailOpenError,
    ),
    BackendEventPolicy::new(
        "autostart_status",
        BackendEventDeliveryClass::Snapshot,
        BackendEventBackpressurePolicy::ClientScopedSnapshot,
    ),
    BackendEventPolicy::new(
        "autostart_error",
        BackendEventDeliveryClass::Error,
        BackendEventBackpressurePolicy::FailOpenError,
    ),
    BackendEventPolicy::new(
        "workspace_projection_prune_result",
        BackendEventDeliveryClass::EphemeralStatus,
        BackendEventBackpressurePolicy::BestEffort,
    ),
    BackendEventPolicy::new(
        "workspace_projection_prune_error",
        BackendEventDeliveryClass::Error,
        BackendEventBackpressurePolicy::FailOpenError,
    ),
    BackendEventPolicy::new(
        "ui_trace_saved",
        BackendEventDeliveryClass::EphemeralStatus,
        BackendEventBackpressurePolicy::BestEffort,
    ),
    BackendEventPolicy::new(
        "ui_trace_error",
        BackendEventDeliveryClass::Error,
        BackendEventBackpressurePolicy::FailOpenError,
    ),
    BackendEventPolicy::new(
        "release_notes_payload",
        BackendEventDeliveryClass::Snapshot,
        BackendEventBackpressurePolicy::ClientScopedSnapshot,
    ),
    BackendEventPolicy::new(
        "release_notes_error",
        BackendEventDeliveryClass::Error,
        BackendEventBackpressurePolicy::FailOpenError,
    ),
];

pub fn backend_event_policy(kind: &str) -> Option<BackendEventPolicy> {
    BACKEND_EVENT_POLICIES
        .iter()
        .find(|policy| policy.kind == kind)
        .copied()
}

impl BackendEvent {
    pub fn event_kind(&self) -> &'static str {
        match self {
            BackendEvent::WorkspaceState { .. } => "workspace_state",
            BackendEvent::ActiveWorkProjection { .. } => "active_work_projection",
            BackendEvent::WindowList { .. } => "window_list",
            BackendEvent::ProviderUsage { .. } => "provider_usage",
            BackendEvent::TerminalOutput { .. } => "terminal_output",
            BackendEvent::TerminalSnapshot { .. } => "terminal_snapshot",
            BackendEvent::TerminalStatus { .. } => "terminal_status",
            BackendEvent::WindowState { .. } => "window_state",
            BackendEvent::FileTreeEntries { .. } => "file_tree_entries",
            BackendEvent::FileTreeError { .. } => "file_tree_error",
            BackendEvent::FileTreeWorktrees { .. } => "file_tree_worktrees",
            BackendEvent::FileTreeWorktreeSelected { .. } => "file_tree_worktree_selected",
            BackendEvent::FileTreeWorktreeError { .. } => "file_tree_worktree_error",
            BackendEvent::FileContentText { .. } => "file_content_text",
            BackendEvent::FileContentHex { .. } => "file_content_hex",
            BackendEvent::FileContentError { .. } => "file_content_error",
            BackendEvent::FileContentSaved { .. } => "file_content_saved",
            BackendEvent::FileContentSaveError { .. } => "file_content_save_error",
            BackendEvent::BranchEntries { .. } => "branch_entries",
            BackendEvent::BoardEntries { .. } => "board_entries",
            BackendEvent::BoardHistoryPage { .. } => "board_history_page",
            BackendEvent::ProfileSnapshot { .. } => "profile_snapshot",
            BackendEvent::LogEntries { .. } => "log_entries",
            BackendEvent::LogEntryAppended { .. } => "log_entry_appended",
            BackendEvent::ProcessLine { .. } => "process_line",
            BackendEvent::ProcessConsoleSnapshot { .. } => "process_console_snapshot",
            BackendEvent::KnowledgeEntries { .. } => "knowledge_entries",
            BackendEvent::KnowledgeSearchResults { .. } => "knowledge_search_results",
            BackendEvent::ProjectIndexSearchResults { .. } => "project_index_search_results",
            BackendEvent::ProjectIndexSearchError { .. } => "project_index_search_error",
            BackendEvent::KnowledgeDetail { .. } => "knowledge_detail",
            BackendEvent::KnowledgeBridgePhaseUpdated { .. } => "knowledge_bridge_phase_updated",
            BackendEvent::BranchCleanupResult { .. } => "branch_cleanup_result",
            BackendEvent::BranchCleanupProgress { .. } => "branch_cleanup_progress",
            BackendEvent::BranchError { .. } => "branch_error",
            BackendEvent::BoardError { .. } => "board_error",
            BackendEvent::ProfileError { .. } => "profile_error",
            BackendEvent::LogError { .. } => "log_error",
            BackendEvent::KnowledgeError { .. } => "knowledge_error",
            BackendEvent::ProjectOpenError { .. } => "project_open_error",
            BackendEvent::CloneProjectParentSelected { .. } => "clone_project_parent_selected",
            BackendEvent::GithubRepositorySearchResults { .. } => {
                "github_repository_search_results"
            }
            BackendEvent::GithubRepositorySearchError { .. } => "github_repository_search_error",
            BackendEvent::CloneProjectProgress { .. } => "clone_project_progress",
            BackendEvent::CloneProjectDone { .. } => "clone_project_done",
            BackendEvent::CloneProjectError { .. } => "clone_project_error",
            BackendEvent::LaunchWizardOpenError { .. } => "launch_wizard_open_error",
            BackendEvent::LaunchWizardState { .. } => "launch_wizard_state",
            BackendEvent::WorkspaceResumableAgents { .. } => "workspace_resumable_agents",
            BackendEvent::WorkspaceResumeAgentError { .. } => "workspace_resume_agent_error",
            BackendEvent::LaunchProgress { .. } => "launch_progress",
            BackendEvent::ProjectIndexStatus { .. } => "project_index_status",
            BackendEvent::RuntimeHookEvent { .. } => "runtime_hook_event",
            BackendEvent::UpdateState(_) => "update_state",
            BackendEvent::UpdateProgress { .. } => "update_progress",
            BackendEvent::UpdateReady { .. } => "update_ready",
            BackendEvent::UpdateApplyPendingPersisted { .. } => "update_apply_pending_persisted",
            BackendEvent::UpdateApplyError { .. } => "update_apply_error",
            BackendEvent::CustomAgentList { .. } => "custom_agent_list",
            BackendEvent::CustomAgentPresetList { .. } => "custom_agent_preset_list",
            BackendEvent::CustomAgentSaved { .. } => "custom_agent_saved",
            BackendEvent::CustomAgentDeleted { .. } => "custom_agent_deleted",
            BackendEvent::BackendConnectionResult { .. } => "backend_connection_result",
            BackendEvent::CustomAgentError { .. } => "custom_agent_error",
            BackendEvent::AgentBackendList { .. } => "agent_backend_list",
            BackendEvent::AgentBackendSaved { .. } => "agent_backend_saved",
            BackendEvent::AgentBackendDeleted { .. } => "agent_backend_deleted",
            BackendEvent::AgentBackendError { .. } => "agent_backend_error",
            BackendEvent::MigrationDetected { .. } => "migration_detected",
            BackendEvent::MigrationProgress { .. } => "migration_progress",
            BackendEvent::MigrationDone { .. } => "migration_done",
            BackendEvent::MigrationError { .. } => "migration_error",
            BackendEvent::SystemSettings { .. } => "system_settings",
            BackendEvent::BoardAuthStatus { .. } => "board_auth_status",
            BackendEvent::SystemSettingsUpdated { .. } => "system_settings_updated",
            BackendEvent::SystemSettingsError { .. } => "system_settings_error",
            BackendEvent::AutostartStatus { .. } => "autostart_status",
            BackendEvent::AutostartError { .. } => "autostart_error",
            BackendEvent::WorkspaceProjectionPruneResult { .. } => {
                "workspace_projection_prune_result"
            }
            BackendEvent::WorkspaceProjectionPruneError { .. } => {
                "workspace_projection_prune_error"
            }
            BackendEvent::UiTraceSaved { .. } => "ui_trace_saved",
            BackendEvent::UiTraceError { .. } => "ui_trace_error",
            BackendEvent::ReleaseNotesPayload { .. } => "release_notes_payload",
            BackendEvent::ReleaseNotesError { .. } => "release_notes_error",
        }
    }

    pub fn delivery_policy(&self) -> BackendEventPolicy {
        backend_event_policy(self.event_kind())
            .expect("BackendEvent variant must be present in BACKEND_EVENT_POLICIES")
    }
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
            BranchResumeInfo, BranchScope,
        },
        persistence::{WindowGeometry, WindowState},
    };

    use super::{
        backend_event_policy, BackendEvent, BackendEventBackpressurePolicy,
        BackendEventDeliveryClass, BranchEntriesPhase, FrontendEvent, IndexSearchMatchMode,
        IndexSearchResult, IndexSearchScope, IndexSearchTarget, ProfileEntryView,
        ProfileEnvEntryView, ProfileSnapshotView, UiTracePayload, BACKEND_EVENT_POLICIES,
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
    fn frontend_event_startup_auto_resume_ready_deserializes_visible_bounds() {
        let event = serde_json::from_value::<FrontendEvent>(serde_json::json!({
            "kind": "startup_auto_resume_ready",
            "bounds": { "x": 8.0, "y": 16.0, "width": 1200.0, "height": 800.0 }
        }))
        .expect("deserialize startup auto-resume readiness");

        assert!(matches!(
            event,
            FrontendEvent::StartupAutoResumeReady {
                bounds: WindowGeometry {
                    x: 8.0,
                    y: 16.0,
                    width: 1200.0,
                    height: 800.0,
                }
            }
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
                resume: BranchResumeInfo {
                    available: true,
                    reason: None,
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
    fn branch_entries_resume_availability_serializes_and_defaults() {
        let legacy_entry = serde_json::from_value::<BranchListEntry>(serde_json::json!({
            "name": "feature/no-session",
            "scope": "local",
            "is_head": false,
            "upstream": null,
            "ahead": 0,
            "behind": 0,
            "last_commit_date": null,
            "cleanup_ready": false,
            "cleanup": {
                "availability": "blocked",
                "execution_branch": null,
                "merge_target": null,
                "upstream": null,
                "blocked_reason": "unknown",
                "risks": []
            }
        }))
        .expect("deserialize legacy branch entry without resume metadata");

        assert_eq!(
            legacy_entry.resume,
            BranchResumeInfo {
                available: false,
                reason: Some("No resumable session".to_string()),
            },
            "legacy branch entries should default to a disabled Resume action",
        );

        let event = BackendEvent::BranchEntries {
            id: "branches-1".to_string(),
            phase: BranchEntriesPhase::Hydrated,
            entries: vec![BranchListEntry {
                name: "feature/with-session".to_string(),
                scope: BranchScope::Local,
                is_head: false,
                upstream: None,
                ahead: 0,
                behind: 0,
                last_commit_date: None,
                cleanup_ready: false,
                cleanup: BranchCleanupInfo::default(),
                resume: BranchResumeInfo {
                    available: true,
                    reason: None,
                },
            }],
        };

        let value = serde_json::to_value(&event).expect("serialize branch entries");
        assert_eq!(
            value["entries"][0]["resume"]["available"],
            Value::Bool(true)
        );
        assert_eq!(value["entries"][0]["resume"]["reason"], Value::Null);
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
    fn backend_event_policy_classifies_high_risk_delivery_contract() {
        let terminal_output =
            backend_event_policy("terminal_output").expect("terminal_output policy");
        assert_eq!(terminal_output.kind, "terminal_output");
        assert_eq!(
            terminal_output.delivery,
            BackendEventDeliveryClass::Streamed
        );
        assert_eq!(
            terminal_output.backpressure,
            BackendEventBackpressurePolicy::PreserveOrder
        );

        let workspace_state =
            backend_event_policy("workspace_state").expect("workspace_state policy");
        assert_eq!(
            workspace_state.delivery,
            BackendEventDeliveryClass::IdempotentLatest
        );
        assert_eq!(
            workspace_state.backpressure,
            BackendEventBackpressurePolicy::LatestWins
        );
        assert!(workspace_state.coalesces_on_frontend());

        let active_work_projection =
            backend_event_policy("active_work_projection").expect("active_work_projection policy");
        assert_eq!(
            active_work_projection.delivery,
            BackendEventDeliveryClass::IdempotentLatest
        );
        assert_eq!(
            active_work_projection.backpressure,
            BackendEventBackpressurePolicy::LatestWins
        );

        let terminal_snapshot =
            backend_event_policy("terminal_snapshot").expect("terminal_snapshot policy");
        assert_eq!(
            terminal_snapshot.delivery,
            BackendEventDeliveryClass::Snapshot
        );
        assert_eq!(
            terminal_snapshot.backpressure,
            BackendEventBackpressurePolicy::ClientScopedSnapshot
        );
        assert!(!terminal_snapshot.coalesces_on_frontend());

        let runtime_hook_event =
            backend_event_policy("runtime_hook_event").expect("runtime_hook_event policy");
        assert_eq!(
            runtime_hook_event.delivery,
            BackendEventDeliveryClass::BestEffortDaemon
        );
        assert_eq!(
            runtime_hook_event.backpressure,
            BackendEventBackpressurePolicy::BestEffort
        );

        let file_content_saved =
            backend_event_policy("file_content_saved").expect("file_content_saved policy");
        assert_eq!(
            file_content_saved.delivery,
            BackendEventDeliveryClass::Snapshot
        );
        assert_eq!(
            file_content_saved.backpressure,
            BackendEventBackpressurePolicy::ClientScopedSnapshot
        );
    }

    #[test]
    fn frontend_coalescing_contract_matches_backend_latest_wins_policy() {
        let frontend_dispatcher = include_str!("../web/socket-receive-dispatcher.js");

        for policy in BACKEND_EVENT_POLICIES {
            assert_eq!(
                frontend_dispatcher.contains(&format!("\"{}\"", policy.kind)),
                policy.coalesces_on_frontend(),
                "{} backend policy disagrees with DEFAULT_COALESCE_KINDS",
                policy.kind
            );
        }

        let event = BackendEvent::TerminalOutput {
            id: "tab-1::shell-1".to_string(),
            data_base64: "ZWNobw==".to_string(),
        };
        assert_eq!(event.delivery_policy().kind, "terminal_output");
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
                works: Vec::new(),
                cleanup_candidate: Some(super::ActiveWorkCleanupCandidateView {
                    branch: "work/20260504-1200".to_string(),
                    worktree_path: Some("/tmp/repo/work/20260504-1200".to_string()),
                    reason: "workspace_done".to_string(),
                    default_delete_remote: false,
                    remote_delete_available: true,
                }),
                active_work_count: 1,
                active_works: vec![super::ActiveWorkItemView {
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
                    board_refs: vec!["board-1".to_string()],
                    agents: Vec::new(),
                    lifecycle_state: "active".to_string(),
                    closed_at: None,
                }],
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
            "active work projection must expose per-agent summaries for Work UI"
        );
        assert!(value.pointer("/projection/works").is_some());
        assert!(value.pointer("/projection/workspaces").is_none());
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
            "Work Overview should receive recent summary journal entries without replaying Board history"
        );
        assert_eq!(
            value
                .pointer("/projection/cleanup_candidate/default_delete_remote")
                .and_then(Value::as_bool),
            Some(false),
            "Work cleanup must default to local-only deletion"
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

    // SPEC-2785 US-1 AS-1 / FR-C: frontend → backend WS payload contract for
    // opening the server URL in the OS default browser.
    #[test]
    fn frontend_event_accepts_open_server_url() {
        let event: FrontendEvent = serde_json::from_value(serde_json::json!({
            "kind": "open_server_url",
            "url": "http://127.0.0.1:54321/"
        }))
        .expect("deserialize open_server_url");

        assert!(matches!(
            event,
            FrontendEvent::OpenServerUrl { ref url } if url == "http://127.0.0.1:54321/"
        ));
    }

    #[test]
    fn frontend_event_accepts_project_index_full_refresh_request() {
        let event: FrontendEvent = serde_json::from_value(serde_json::json!({
            "kind": "refresh_index_status",
            "project_root": "/repo/worktree"
        }))
        .expect("deserialize refresh_index_status");

        assert!(matches!(
            event,
            FrontendEvent::RefreshIndexStatus { project_root }
                if project_root == "/repo/worktree"
        ));
    }

    #[test]
    fn frontend_event_accepts_index_search_request() {
        let event: FrontendEvent = serde_json::from_value(serde_json::json!({
            "kind": "search_project_index",
            "id": "tab-1:index-1",
            "query": "Board semantic search",
            "request_id": 42,
            "scopes": ["issues", "specs", "board", "files", "files-docs", "memory", "discussions"],
            "worktree_hash": "wt-a",
            "match_mode": "all_terms"
        }))
        .expect("deserialize search_project_index");

        assert!(matches!(
            event,
            FrontendEvent::SearchProjectIndex {
                id,
                query,
                request_id: 42,
                scopes,
                worktree_hash,
                match_mode
            } if id == "tab-1:index-1"
                && query == "Board semantic search"
                && scopes.contains(&IndexSearchScope::Board)
                && scopes.contains(&IndexSearchScope::Discussions)
                && scopes.contains(&IndexSearchScope::FilesDocs)
                && worktree_hash.as_deref() == Some("wt-a")
                && match_mode == IndexSearchMatchMode::AllTerms
        ));
    }

    #[test]
    fn index_search_request_defaults_to_semantic_match_mode() {
        let event: FrontendEvent = serde_json::from_value(serde_json::json!({
            "kind": "search_project_index",
            "id": "tab-1:index-1",
            "query": "Board semantic search",
            "request_id": 42,
            "scopes": ["board"]
        }))
        .expect("deserialize search_project_index");

        assert!(matches!(
            event,
            FrontendEvent::SearchProjectIndex {
                match_mode: IndexSearchMatchMode::Semantic,
                ..
            }
        ));
    }

    #[test]
    fn backend_event_serializes_index_search_results_contract() {
        let event = BackendEvent::ProjectIndexSearchResults {
            id: "tab-1:index-1".to_string(),
            query: "Board semantic search".to_string(),
            request_id: 42,
            results: vec![IndexSearchResult {
                scope: IndexSearchScope::Board,
                title: "Board search".to_string(),
                subtitle: "status · Codex".to_string(),
                preview: "Board discussion history".to_string(),
                distance: Some(0.1234),
                match_mode: Some(IndexSearchMatchMode::AllTerms),
                matched_terms: vec!["Board".to_string(), "search".to_string()],
                missing_terms: Vec::new(),
                target: IndexSearchTarget::Board {
                    entry_id: "board-1".to_string(),
                },
            }],
            suggestions: vec![IndexSearchResult {
                scope: IndexSearchScope::Board,
                title: "Board suggestion".to_string(),
                subtitle: "status · Codex".to_string(),
                preview: "Board history".to_string(),
                distance: Some(0.2234),
                match_mode: Some(IndexSearchMatchMode::AllTerms),
                matched_terms: vec!["Board".to_string()],
                missing_terms: vec!["search".to_string()],
                target: IndexSearchTarget::Board {
                    entry_id: "board-2".to_string(),
                },
            }],
        };

        let value = serde_json::to_value(&event).expect("serialize event");
        assert_eq!(value["kind"], "project_index_search_results");
        assert_eq!(value["results"][0]["scope"], "board");
        assert_eq!(value["results"][0]["match_mode"], "all_terms");
        assert_eq!(value["results"][0]["matched_terms"][0], "Board");
        assert_eq!(value["results"][0]["target"]["kind"], "board");
        assert_eq!(value["results"][0]["target"]["entry_id"], "board-1");
        assert_eq!(value["suggestions"][0]["missing_terms"][0], "search");
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
    fn frontend_event_accepts_uploaded_terminal_image_paste_payload() {
        let event: FrontendEvent = serde_json::from_value(serde_json::json!({
            "kind": "paste_image_uploaded",
            "id": "tab-1::agent-1",
            "upload_id": "upload-1",
            "mime_type": "image/png",
            "filename": "screenshot.png",
            "size": 12
        }))
        .expect("deserialize uploaded image paste event");

        assert!(
            matches!(
                event,
                FrontendEvent::PasteImageUploaded {
                    id,
                    upload_id,
                    mime_type,
                    filename: Some(filename),
                    size,
                } if id == "tab-1::agent-1"
                    && upload_id == "upload-1"
                    && mime_type == "image/png"
                    && filename == "screenshot.png"
                    && size == 12
            ),
            "uploaded image paste must expose upload id and image metadata"
        );
    }

    #[test]
    fn frontend_event_accepts_terminal_file_attachment_paths() {
        let event: FrontendEvent = serde_json::from_value(serde_json::json!({
            "kind": "attach_files",
            "id": "tab-1::agent-1",
            "files": [
                {
                    "source": "native_path",
                    "path": "/Users/me/report.pdf"
                }
            ]
        }))
        .expect("deserialize file attachment event");

        assert!(
            matches!(
                event,
                FrontendEvent::AttachFiles { id, files }
                    if id == "tab-1::agent-1"
                        && matches!(
                            files.as_slice(),
                            [super::FileAttachment::NativePath { path }]
                                if path == "/Users/me/report.pdf"
                        )
            ),
            "native file drops must expose terminal id and host path"
        );
    }

    #[test]
    fn frontend_event_accepts_terminal_inline_file_attachments() {
        let event: FrontendEvent = serde_json::from_value(serde_json::json!({
            "kind": "attach_files",
            "id": "tab-1::agent-1",
            "files": [
                {
                    "source": "inline",
                    "filename": "notes.txt",
                    "mime_type": "text/plain",
                    "size": 11,
                    "data_base64": "aGVsbG8gd29ybGQ="
                }
            ]
        }))
        .expect("deserialize inline file attachment event");

        assert!(
            matches!(
                event,
                FrontendEvent::AttachFiles { id, files }
                    if id == "tab-1::agent-1"
                        && matches!(
                            files.as_slice(),
                            [super::FileAttachment::Inline {
                                filename,
                                mime_type: Some(mime_type),
                                size,
                                data_base64,
                            }] if filename == "notes.txt"
                                && mime_type == "text/plain"
                                && *size == 11
                                && data_base64 == "aGVsbG8gd29ybGQ="
                        )
            ),
            "browser file drops must expose inline file metadata and bytes"
        );
    }

    #[test]
    fn frontend_event_accepts_uploaded_terminal_file_attachments() {
        let event: FrontendEvent = serde_json::from_value(serde_json::json!({
            "kind": "attach_files",
            "id": "tab-1::agent-1",
            "files": [
                {
                    "source": "uploaded",
                    "upload_id": "upload-1",
                    "filename": "large.bin",
                    "mime_type": "application/octet-stream",
                    "size": 10485761
                }
            ]
        }))
        .expect("deserialize uploaded file attachment event");

        assert!(
            matches!(
                event,
                FrontendEvent::AttachFiles { id, files }
                    if id == "tab-1::agent-1"
                        && matches!(
                            files.as_slice(),
                            [super::FileAttachment::Uploaded {
                                upload_id,
                                filename,
                                mime_type: Some(mime_type),
                                size,
                            }] if upload_id == "upload-1"
                                && filename == "large.bin"
                                && mime_type == "application/octet-stream"
                                && *size == 10485761
                        )
            ),
            "browser streaming uploads must expose upload id and metadata"
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
                os_env: vec![ProfileEnvEntryView {
                    key: "PATH".to_string(),
                    value: "/usr/bin".to_string(),
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
        assert_eq!(
            value["snapshot"]["os_env"][0]["value"],
            Value::String("/usr/bin".to_string())
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

    #[test]
    fn frontend_event_update_board_provider_config_round_trips() {
        // SPEC-2963 FR-006: settings UI sends non-secret + secret fields here.
        let payload = r#"{"kind":"update_board_provider_config","provider":"slack","client_id":"C-id","default_channel":"CHAN","client_secret":"sek"}"#;
        let event: FrontendEvent =
            serde_json::from_str(payload).expect("deserialize UpdateBoardProviderConfig");
        match event {
            FrontendEvent::UpdateBoardProviderConfig {
                provider,
                client_id,
                default_channel,
                tenant_id,
                client_secret,
            } => {
                assert_eq!(provider, "slack");
                assert_eq!(client_id.as_deref(), Some("C-id"));
                assert_eq!(default_channel.as_deref(), Some("CHAN"));
                assert_eq!(tenant_id, None);
                assert_eq!(client_secret.as_deref(), Some("sek"));
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn frontend_event_update_board_oauth_port_round_trips() {
        let payload = r#"{"kind":"update_board_oauth_port","port":9123}"#;
        let event: FrontendEvent =
            serde_json::from_str(payload).expect("deserialize UpdateBoardOauthPort");
        match event {
            FrontendEvent::UpdateBoardOauthPort { port } => assert_eq!(port, 9123),
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn backend_event_board_auth_status_carries_config_view_without_secret() {
        let event = BackendEvent::BoardAuthStatus {
            slack: true,
            teams: false,
            message: Some("Saved slack configuration.".to_string()),
            slack_client_id: Some("C-id".to_string()),
            slack_default_channel: Some("CHAN".to_string()),
            slack_has_secret: true,
            teams_client_id: None,
            teams_tenant_id: None,
            teams_default_channel: None,
            oauth_redirect_port: 8765,
        };
        let value = serde_json::to_value(&event).expect("serialize");
        assert_eq!(value["kind"], "board_auth_status");
        assert_eq!(value["slack"], true);
        assert_eq!(value["slack_client_id"], "C-id");
        assert_eq!(value["slack_has_secret"], true);
        assert_eq!(value["oauth_redirect_port"], 8765);
        // The secret value itself is never part of the wire payload.
        assert!(value.get("slack_client_secret").is_none());
        assert!(value.get("client_secret").is_none());
    }

    #[test]
    fn frontend_event_save_ui_trace_deserializes_payload() {
        let event: FrontendEvent = serde_json::from_value(serde_json::json!({
            "kind": "save_ui_trace",
            "trace": {
                "session_id": "trace-1",
                "entries": [
                    { "kind": "trace_start", "ts": 1 }
                ]
            }
        }))
        .expect("deserialize save_ui_trace");
        match event {
            FrontendEvent::SaveUiTrace { trace } => {
                assert_eq!(trace.session_id(), Some("trace-1"));
                let entries = trace.entries().expect("entries");
                assert_eq!(
                    entries[0].field("kind").and_then(serde_json::Value::as_str),
                    Some("trace_start")
                );
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn ui_trace_payload_keeps_missing_entries_as_runtime_validation_error() {
        let trace: UiTracePayload = serde_json::from_value(serde_json::json!({
            "session_id": "trace-1"
        }))
        .expect("deserialize trace payload");

        assert_eq!(
            trace
                .entries()
                .expect_err("missing entries should be validated by runtime"),
            "trace payload missing entries array"
        );
    }

    #[test]
    fn backend_event_ui_trace_saved_serializes() {
        let event = BackendEvent::UiTraceSaved {
            path: "/tmp/ui-trace.jsonl".to_string(),
            entries: 2,
        };
        let value = serde_json::to_value(&event).expect("serialize");
        assert_eq!(value["kind"], "ui_trace_saved");
        assert_eq!(value["path"], "/tmp/ui-trace.jsonl");
        assert_eq!(value["entries"], 2);
    }

    #[test]
    fn backend_event_ui_trace_error_serializes() {
        let event = BackendEvent::UiTraceError {
            message: "trace payload missing entries".to_string(),
        };
        let value = serde_json::to_value(&event).expect("serialize");
        assert_eq!(value["kind"], "ui_trace_error");
        assert_eq!(value["message"], "trace payload missing entries");
    }
}
