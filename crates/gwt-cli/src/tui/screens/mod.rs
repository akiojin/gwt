//! TUI Screens

pub mod agent_mode;
pub mod agent_pane;
pub mod ai_wizard;
pub mod branch_list;
pub mod clone_wizard;
pub mod confirm;
pub mod docker_progress;
pub mod environment;
pub mod error;
pub mod git_view;
pub mod help;
pub mod logs;
pub mod migration_dialog;
pub mod profiles;
pub mod service_select;
pub mod settings;
pub mod split_layout;
pub mod wizard;
pub mod worktree_create;

pub use agent_mode::{render_agent_mode, AgentMessage, AgentModeState, AgentRole};
pub use ai_wizard::{render_ai_wizard, AIWizardState};
pub use branch_list::{render_branch_list, BranchItem, BranchListState, BranchType};
pub use clone_wizard::{render_clone_wizard, CloneWizardState, CloneWizardStep};
pub use confirm::{render_confirm, ConfirmState};
pub use environment::{collect_os_env, render_environment, EnvironmentState};
pub use error::{render_error_with_queue, ErrorQueue, ErrorState};
pub use git_view::{render_git_view, GitViewCache, GitViewState};
pub use help::{render_help, HelpState};
pub use logs::{render_logs, LogsState};
pub use migration_dialog::{render_migration_dialog, MigrationDialogPhase, MigrationDialogState};
pub use profiles::{render_profiles, ProfilesState};
pub use settings::{render_settings, SettingsState};
pub use wizard::{
    render_wizard, CodingAgent, ExecutionMode, QuickStartDockerSettings, QuickStartEntry,
    ReasoningLevel, WizardConfirmResult, WizardState, WizardStep,
};
pub use worktree_create::{render_worktree_create, WorktreeCreateState};

// Agent pane rendering (SPEC-1d6dd9fc FR-047)
#[allow(unused_imports)]
pub use agent_pane::{render_agent_pane, AgentPaneView};

// Docker integration screens (SPEC-f5f5657e)
pub use service_select::{render_service_select, ServiceSelectState};
