pub mod branch_list;
pub mod cli;
mod discussion_resume;
pub mod file_tree;
mod index_worker;
mod issue_cache;
pub mod launch_wizard;
pub mod managed_assets;
pub mod native_app;
pub mod persistence;
pub mod preset;
pub mod protocol;
pub mod workspace;

pub use branch_list::{list_branch_entries, BranchListEntry, BranchScope};
pub use file_tree::{list_directory_entries, FileTreeEntry, FileTreeEntryKind};
pub use launch_wizard::{
    build_builtin_agent_options, default_wizard_version_cache_path, AgentOption,
    DockerWizardContext, LaunchWizardAction, LaunchWizardCompletion, LaunchWizardContext,
    LaunchWizardLiveSessionView, LaunchWizardOptionView, LaunchWizardQuickStartView,
    LaunchWizardState, LaunchWizardStep, LaunchWizardSummaryView, LaunchWizardView,
    LiveSessionEntry, QuickStartEntry, QuickStartLaunchMode,
};
pub use managed_assets::refresh_managed_gwt_assets_for_worktree;
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
pub use protocol::{
    AppStateView, ArrangeMode, BackendEvent, FocusCycleDirection, FrontendEvent, ProjectTabView,
    RecentProjectView, WorkspaceView,
};
pub use workspace::WorkspaceState;
