//! TUI Screens

pub mod agent_mode;
pub mod ai_wizard;
pub mod branch_list;
pub mod confirm;
pub mod environment;
pub mod error;
pub mod help;
pub mod logs;
pub mod pane_list;
pub mod profiles;
pub mod settings;
pub mod speckit_wizard;
pub mod split_layout;
pub mod wizard;
pub mod worktree_create;

pub use agent_mode::{
    render_agent_mode, render_session_selector, AgentMessage, AgentModeState, AgentRole,
};
pub use ai_wizard::{render_ai_wizard, AIWizardState};
pub use branch_list::{render_branch_list, BranchItem, BranchListState, BranchType};
pub use confirm::{render_confirm, ConfirmState};
pub use environment::{collect_os_env, render_environment, EnvironmentState};
pub use error::{render_error_with_queue, ErrorQueue, ErrorState};
pub use help::{render_help, HelpState};
pub use logs::{render_logs, LogsState};
pub use profiles::{render_profiles, ProfilesState};
pub use settings::{render_settings, SettingsState};
pub use speckit_wizard::{render_speckit_wizard, SpecKitWizardState};
pub use wizard::{
    render_wizard, CodingAgent, ExecutionMode, QuickStartEntry, ReasoningLevel,
    WizardConfirmResult, WizardState,
};
pub use worktree_create::{render_worktree_create, WorktreeCreateState};
