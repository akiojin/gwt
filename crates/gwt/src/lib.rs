pub mod branch_cleanup;
pub mod branch_list;
pub mod cli;
pub mod custom_agents_dispatch;
pub mod custom_agents_service;
pub mod daemon_runtime;
mod discussion_resume;
pub mod file_tree;
pub mod index_worker;
mod issue_cache;
pub mod knowledge_bridge;
pub mod launch_wizard;
pub mod managed_assets;
pub mod native_app;
pub mod persistence;
pub mod preset;
pub mod profiles_dispatch;
pub mod profiles_service;
pub mod protocol;
pub mod workspace;

pub use branch_cleanup::{
    cleanup_selected_branches, BranchCleanupResultEntry, BranchCleanupResultStatus,
};
pub use branch_list::{
    hydrate_branch_entries_with_active_sessions, list_branch_entries_with_active_sessions,
    BranchCleanupAvailability, BranchCleanupBlockedReason, BranchCleanupInfo, BranchCleanupRisk,
};
pub use branch_list::{list_branch_entries, list_branch_inventory, BranchListEntry, BranchScope};
pub use custom_agents_service::{
    add_from_preset, delete_custom_agent, list_custom_agents, list_presets, probe_backend,
    update_custom_agent, CustomAgentsServiceError,
};
pub use daemon_runtime::{HookForwardTarget, RuntimeHookEvent, RuntimeHookEventKind};
pub use file_tree::{list_directory_entries, FileTreeEntry, FileTreeEntryKind};
pub use gwt_agent::{ClaudeCodeOpenaiCompatInput, PresetDefinition, PresetId};
pub use knowledge_bridge::{
    load_knowledge_bridge, KnowledgeBridgeView, KnowledgeDetailSection, KnowledgeDetailView,
    KnowledgeKind, KnowledgeListItem,
};
pub use launch_wizard::{
    build_builtin_agent_options, default_wizard_version_cache_path, AgentOption,
    DockerWizardContext, LaunchTargetKind, LaunchWizardAction, LaunchWizardCompletion,
    LaunchWizardContext, LaunchWizardHydration, LaunchWizardLaunchRequest,
    LaunchWizardLiveSessionView, LaunchWizardOptionView, LaunchWizardQuickStartView,
    LaunchWizardState, LaunchWizardStep, LaunchWizardSummaryView, LaunchWizardView,
    LiveSessionEntry, QuickStartEntry, QuickStartLaunchMode, ShellLaunchConfig,
};
pub use managed_assets::refresh_managed_gwt_assets_for_worktree;
#[cfg(target_os = "windows")]
pub use native_app::windows_app_icon;
#[cfg(target_os = "macos")]
pub use native_app::MacosNativeMenu;
pub use native_app::{
    macos_bundle_identifier, macos_native_menu_titles, native_menu_command_for_id,
    NativeMenuCommand, APP_NAME, MACOS_BUNDLE_IDENTIFIER, OPEN_PROJECT_MENU_ID, RELOAD_MENU_ID,
};
pub use persistence::{
    default_session_state, default_workspace_state, empty_workspace_state,
    legacy_workspace_state_path, load_restored_workspace_state, load_session_state,
    load_workspace_state, migrate_legacy_workspace_state, pause_process_windows_for_restore,
    project_title_from_path, save_session_state, save_workspace_state, workspace_state_path,
    CanvasViewport, PersistedSessionState, PersistedSessionTabState, PersistedWindowState,
    PersistedWorkspaceState, ProjectKind, RecentProjectEntry, WindowGeometry, WindowProcessStatus,
};
pub use preset::{
    detect_shell_program, resolve_launch_spec, LaunchSpec, PresetResolveError, ShellProgram,
    WindowPreset, WindowSurface,
};
pub use profiles_service::{
    add_disabled_env, add_profile, delete_disabled_env, delete_env_var, delete_profile,
    load_profile_snapshot, set_env_var, switch_profile, update_disabled_env, update_env_var,
    update_profile, ProfileEnvVarSource, ProfileEnvVarView, ProfileServiceError, ProfileSnapshot,
    ProfileView,
};
pub use protocol::{
    AppStateView, ArrangeMode, BackendEvent, BoardEntryView, BranchEntriesPhase,
    CustomAgentErrorCode, FocusCycleDirection, FrontendEvent, ProfileErrorCode, ProjectTabView,
    RecentProjectView, WorkspaceView,
};
pub use workspace::WorkspaceState;
