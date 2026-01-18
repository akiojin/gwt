//! TUI Screens

#![allow(unused_imports)]

pub mod branch_list;
pub mod confirm;
pub mod environment;
pub mod error;
pub mod help;
pub mod logs;
pub mod pane_list;
pub mod profiles;
pub mod split_layout;
pub mod settings;
pub mod wizard;
pub mod worktree_create;

pub use branch_list::{render_branch_list, BranchItem, BranchListState, BranchType};
pub use confirm::{render_confirm, ConfirmState};
pub use environment::{collect_os_env, render_environment, EnvironmentState};
pub use error::{render_error, ErrorState};
pub use help::{render_help, HelpState};
pub use logs::{render_logs, LogsState};
pub use pane_list::{render_pane_list, PaneListState};
pub use profiles::{render_profiles, ProfilesState};
pub use split_layout::{calculate_split_layout, FocusPanel, SplitLayoutState};
pub use settings::{render_settings, SettingsState};
pub use wizard::{
    render_wizard, CodingAgent, ExecutionMode, QuickStartEntry, ReasoningLevel,
    WizardConfirmResult, WizardState,
};
pub use worktree_create::{render_worktree_create, WorktreeCreateState};
