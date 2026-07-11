pub mod agent_backend_dispatch;
pub(crate) mod agent_project_state;
pub mod backend_service;
pub mod board_audience;
pub mod board_provider;
pub mod board_remote;
pub mod branch_cleanup;
pub mod branch_list;
pub mod cli;
pub mod custom_agents_dispatch;
pub mod custom_agents_service;
#[cfg(unix)]
pub mod daemon_publisher;
pub mod daemon_runtime;
#[cfg(unix)]
pub mod daemon_subscriber;
mod discussion_resume;
pub mod file_content;
pub mod file_tree;
pub mod gui_single_instance;
pub mod handlers;
pub mod index_search;
pub mod index_worker;
pub mod issue_cache;
pub mod issue_monitor;
pub mod issue_monitor_authz;
pub mod issue_monitor_gate;
pub mod issue_monitor_review;
pub mod issue_monitor_worker;
pub mod knowledge_bridge;
pub mod launch_wizard;
pub mod managed_assets;
pub mod migration;
pub mod native_app;
pub(crate) mod path_filter;
pub mod persistence;
pub mod preset;
pub mod process;
pub mod profile_dispatch;
pub mod protocol;
pub mod runtime_daemon_events;
pub mod start_work;
pub mod system_settings;
pub mod web_protocol_enums;
pub mod window_canvas;
pub mod window_state;
pub mod work_notes;
pub mod worktree_inventory;

#[cfg(test)]
pub(crate) fn env_test_lock() -> &'static std::sync::Mutex<()> {
    gwt_core::test_support::env_lock()
}

pub use branch_cleanup::{
    cleanup_selected_branches, cleanup_selected_branches_with_options,
    cleanup_selected_branches_with_progress, BranchCleanupOptions, BranchCleanupProgressEntry,
    BranchCleanupProgressPhase, BranchCleanupResultEntry, BranchCleanupResultStatus,
};
pub use branch_list::{
    hydrate_branch_entries_with_active_sessions, list_branch_entries_with_active_sessions,
    BranchCleanupAvailability, BranchCleanupBlockedReason, BranchCleanupInfo, BranchCleanupRisk,
};
pub use branch_list::{
    list_branch_entries, list_branch_inventory, next_branch_load_id, BranchListEntry,
    BranchResumeInfo, BranchScope,
};
pub use custom_agents_service::{
    add_from_preset, delete_custom_agent, list_custom_agents, list_presets, probe_backend,
    update_custom_agent, CustomAgentsServiceError,
};
pub use daemon_runtime::{HookForwardTarget, RuntimeHookEvent, RuntimeHookEventKind};
pub use file_content::{
    file_kind, read_binary_chunk, read_text_file, write_binary_byte, write_text_file, BinaryChunk,
    ContentLimits, Encoding, ExpectedMetadata, FileContentError, FileKind, Newline, TextResult,
    WriteOutcome,
};
pub use file_tree::{list_directory_entries, FileTreeEntry, FileTreeEntryKind};
pub use gwt_agent::{ClaudeCodeOpenaiCompatInput, PresetDefinition, PresetId};
pub use index_search::{search_project_index, work_advisory};
pub use index_worker::{
    aggregate_current_worktree_index_status_for_path, aggregate_project_index_status_for_path,
    auto_repair_unhealthy_scopes, auto_repair_unhealthy_targets, build_aggregated_status_view,
    collect_unhealthy_rebuild_targets, collect_unhealthy_rebuild_targets_for_project_root,
    default_rebuild_runner, global_aggregated_status_cache, list_worktree_probe_inputs,
    parse_scope_health, AggregatedStatusCache, IndexRebuildRunnerFn, IndexRebuildScope,
    IndexRebuildSpawner, ProjectIndexScopes, ProjectIndexStatusState, ProjectIndexStatusView,
    RebuildProgress, RebuildTarget, ScopeHealthView, WorktreeMeta, WorktreeProbeInput,
    WorktreeProbeOutcome,
};
pub use issue_monitor::{
    is_auto_improve_candidate, issue_monitor_launch_plan, issue_monitor_launch_profile_summary,
    issue_monitor_launch_prompt, issue_monitor_prefs_path_for_repo_path, load_issue_monitor_prefs,
    save_issue_monitor_prefs, scan_issue_monitor_candidates, AutonomousIssueRecord,
    AutonomousPhase, AutonomousReviewDispatch, EligibilityDecision, FailureClass,
    IssueMonitorConfig, IssueMonitorFailedIssue, IssueMonitorInboxItem, IssueMonitorIssue,
    IssueMonitorIssueState, IssueMonitorLaunchPlan, IssueMonitorLaunchProfile,
    IssueMonitorLaunchProfileSource, IssueMonitorLaunchRequest, IssueMonitorLaunchedIssue,
    IssueMonitorLaunchingIssue, IssueMonitorPrefs, IssueMonitorScanSummary, IssueMonitorState,
    IssueMonitorStatusView, MonitorInboxState,
};
pub use knowledge_bridge::{
    load_knowledge_bridge, refresh_knowledge_bridge_cache, search_knowledge_bridge,
    update_knowledge_phase, KnowledgeBridgeView, KnowledgeDetailSection, KnowledgeDetailView,
    KnowledgeKind, KnowledgeListItem, KnowledgeRelatedAgentView, KnowledgeRelatedSessionView,
    KnowledgeRelatedWorkView,
};
pub use launch_wizard::{
    build_agent_options, build_builtin_agent_options, default_wizard_version_cache_path,
    has_gwt_spec_label, knowledge_launch_target_branch_name, load_agent_options, AgentOption,
    DockerWizardContext, LaunchTargetKind, LaunchWizardAction, LaunchWizardCompletion,
    LaunchWizardContext, LaunchWizardHydration, LaunchWizardLaunchPath, LaunchWizardLaunchRequest,
    LaunchWizardLiveSessionView, LaunchWizardMode, LaunchWizardOptionView,
    LaunchWizardPreviousProfile, LaunchWizardPreviousProfiles, LaunchWizardProgressStepView,
    LaunchWizardQuickStartView, LaunchWizardStartMethodKind, LaunchWizardStartMethodView,
    LaunchWizardState, LaunchWizardStep, LaunchWizardSummaryView, LaunchWizardView,
    LinkedIssueKind, LiveSessionEntry, QuickStartEntry, QuickStartLaunchMode,
    ResumableAgentLifecycleStatus, ResumableAgentResumeKind, ResumableAgentView, ShellLaunchConfig,
};
pub use managed_assets::{
    refresh_existing_managed_gwt_assets_for_worktree, refresh_managed_gwt_assets_for_agent,
    refresh_managed_gwt_assets_for_agent_with_codex_hook_discovery_mode,
    refresh_managed_gwt_assets_for_worktree,
};
pub use native_app::{
    macos_bundle_identifier, APP_NAME, GUI_FRONT_DOOR_BINARY_NAME, INTERNAL_DAEMON_BINARY_NAME,
    MACOS_APP_BUNDLE_NAME, MACOS_BUNDLE_IDENTIFIER,
};
pub use persistence::{
    default_session_state, default_workspace_state, empty_workspace_state,
    legacy_workspace_state_path, load_restored_workspace_state, load_session_state,
    load_workspace_state, migrate_legacy_workspace_state, pause_process_windows_for_restore,
    project_title_from_path, save_session_state, save_workspace_state, workspace_state_path,
    AgentKanbanLane, CanvasViewport, PersistedSessionState, PersistedSessionTabState,
    PersistedWindowCanvasState, PersistedWindowState, ProjectKind, RecentProjectEntry,
    WindowGeometry, WindowLaneKind, WindowPlacement, WindowProcessStatus, WindowState,
};
pub use preset::{
    detect_shell_program, resolve_launch_spec, LaunchSpec, PresetResolveError, ShellProgram,
    WindowPreset, WindowSurface,
};
pub use protocol::{
    ActiveWorkAgentView, ActiveWorkCleanupCandidateView, ActiveWorkItemView,
    ActiveWorkProjectionView, AppStateView, ArrangeMode, AttachmentProgressPhase, BackendEvent,
    BranchEntriesPhase, CustomAgentErrorCode, FileAttachment, FileContentErrorKind,
    FileContentMode, FileContentSaveErrorKind, FocusCycleDirection, FrontendEvent,
    GitHubRepositorySearchResultView, IndexSearchMatchMode, IndexSearchResult, IndexSearchScope,
    IndexSearchTarget, ManagedHookHealthView, ManagedHookPendingDiscussionView,
    ManagedHookPendingGoalView, ManagedHookSlowHandlerView, ProfileEntryView, ProfileEnvEntryView,
    ProfileSnapshotView, ProjectTabView, RecentProjectView, RunningAgentSummary, UiTraceEntry,
    UiTracePayload, WorkAgentView, WorkEventView, WorkItemView, WorkspaceExecutionContainerView,
    WorkspaceHistoryAgentView, WorkspaceHistoryEventView, WorkspaceHistorySessionView,
    WorkspaceHistoryView, WorkspaceJournalEntryView, WorkspaceResumeSource, WorkspaceView,
};
pub use window_canvas::WindowCanvasState;
