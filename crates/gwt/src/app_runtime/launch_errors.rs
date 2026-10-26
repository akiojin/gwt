//! Launch error logging / error-event helpers split out of
//! `app_runtime/mod.rs` for SPEC-3064 Phase 1 (Pass 2).
//!
//! Owns:
//! - Launch Wizard action -> error-stage / label mapping
//!   ([`AppRuntime::launch_wizard_action_error_stage`],
//!   [`AppRuntime::launch_wizard_action_label`]) consumed by `wizard.rs`
//!   and `frontend_action_log.rs`
//! - Structured launch error logging
//!   ([`AppRuntime::log_launch_wizard_error`],
//!   `sanitize_launch_log_error`)
//! - The launch error -> frontend event bridge
//!   ([`AppRuntime::launch_error_terminal_output_event`],
//!   [`AppRuntime::launch_error_terminal_bytes`],
//!   [`AppRuntime::status_events`])

use base64::Engine as _;
use sha2::{Digest as _, Sha256};

use super::{AppRuntime, BackendEvent, LaunchWizardSession, OutboundEvent, WindowProcessStatus};

/// Agents whose missing-binary launch failure should be rewritten into an
/// actionable install hint instead of leaking the raw PTY/PATH error. Each
/// entry maps the spawn command token that appears in the raw error
/// (`Unable to spawn <command>`) to the user-facing guidance.
///
/// SPEC-3151 FR-003: a missing `opencode` binary surfaces install guidance rather than
/// `No viable candidates found in PATH`.
/// Recovery route appended to the owner-mismatch binding refusal.
///
/// Issue #3489: the refusal fails the launch before the PTY starts, so the pane
/// shows this one line and nothing else. The refusal itself names the Session
/// and both owners; this names what to do next so the window is not a dead end.
const EXECUTION_BINDING_OWNER_MISMATCH_RECOVERY: &str = concat!(
    ". Run the execution.status JSON operation for the exact recovery route, ",
    "then use Continue work on the owner's Work to create a successor, ",
    "or start a fresh launch for that owner."
);

const MISSING_BINARY_INSTALL_HINTS: &[(&str, &str)] = &[
    (
        "agy",
        concat!(
            "Antigravity CLI (`agy`) was not found in PATH. ",
            "Install it with: curl -fsSL https://antigravity.google/cli/install.sh | bash. ",
            "If it is already installed, ensure ~/.local/bin or the install directory is on PATH and ",
            "restart gwt."
        ),
    ),
    (
        "opencode",
        concat!(
            "OpenCode (`opencode`) was not found in PATH. ",
            "Install it with: npm i -g opencode-ai ",
            "(or: curl -fsSL https://opencode.ai/install | bash, ",
            "or: brew install anomalyco/tap/opencode). ",
            "If it is already installed, ensure the install directory is on PATH and restart gwt."
        ),
    ),
];

impl AppRuntime {
    pub(super) fn launch_wizard_action_error_stage(
        action: &gwt::LaunchWizardAction,
    ) -> &'static str {
        match action {
            gwt::LaunchWizardAction::Submit => "wizard_submit",
            gwt::LaunchWizardAction::ApplyQuickStart { .. } => "quick_start",
            gwt::LaunchWizardAction::SetLaunchPath { .. }
            | gwt::LaunchWizardAction::SelectQuickStart { .. }
            | gwt::LaunchWizardAction::SelectLiveSession { .. }
            | gwt::LaunchWizardAction::UseStartMethod { .. } => "launch_path_select",
            gwt::LaunchWizardAction::FocusExistingSession { .. }
            | gwt::LaunchWizardAction::MoveExistingPane { .. } => "focus_existing_session",
            gwt::LaunchWizardAction::StopAndStartSuccessor { .. } => "stop_and_start_successor",
            gwt::LaunchWizardAction::SetAgent { .. }
            | gwt::LaunchWizardAction::SetAgentMember { .. } => "agent_select",
            gwt::LaunchWizardAction::SetLaunchTarget { .. } => "launch_target_select",
            gwt::LaunchWizardAction::Select { .. } => "wizard_select",
            _ => "wizard_action",
        }
    }

    pub(super) fn launch_wizard_action_label(action: &gwt::LaunchWizardAction) -> &'static str {
        match action {
            gwt::LaunchWizardAction::Select { .. } => "select",
            gwt::LaunchWizardAction::Back => "back",
            gwt::LaunchWizardAction::Cancel => "cancel",
            gwt::LaunchWizardAction::SubmitText { .. } => "submit_text",
            gwt::LaunchWizardAction::ApplyQuickStart { .. } => "apply_quick_start",
            gwt::LaunchWizardAction::UseStartMethod { .. } => "use_start_method",
            gwt::LaunchWizardAction::SetLaunchPath { .. } => "set_launch_path",
            gwt::LaunchWizardAction::SelectQuickStart { .. } => "select_quick_start",
            gwt::LaunchWizardAction::SelectLiveSession { .. } => "select_live_session",
            gwt::LaunchWizardAction::FocusExistingSession { .. } => "focus_existing_session",
            gwt::LaunchWizardAction::StopAndStartSuccessor { .. } => "stop_and_start_successor",
            gwt::LaunchWizardAction::MoveExistingPane { .. } => "move_existing_pane",
            gwt::LaunchWizardAction::SetBranchMode { .. } => "set_branch_mode",
            gwt::LaunchWizardAction::SetBranchType { .. } => "set_branch_type",
            gwt::LaunchWizardAction::SetBranchName { .. } => "set_branch_name",
            gwt::LaunchWizardAction::SelectExistingBranch { .. } => "select_existing_branch",
            gwt::LaunchWizardAction::SetInitialPrompt { .. } => "set_initial_prompt",
            gwt::LaunchWizardAction::SetLaunchTarget { .. } => "set_launch_target",
            gwt::LaunchWizardAction::SetAgent { .. } => "set_agent",
            gwt::LaunchWizardAction::SetAgentMember { .. } => "set_agent_member",
            gwt::LaunchWizardAction::SetModel { .. } => "set_model",
            gwt::LaunchWizardAction::SetReasoning { .. } => "set_reasoning",
            gwt::LaunchWizardAction::SetRuntimeTarget { .. } => "set_runtime_target",
            gwt::LaunchWizardAction::SetWindowsShell { .. } => "set_windows_shell",
            gwt::LaunchWizardAction::SetDockerService { .. } => "set_docker_service",
            gwt::LaunchWizardAction::SetDockerLifecycle { .. } => "set_docker_lifecycle",
            gwt::LaunchWizardAction::SetExecutionMode { .. } => "set_execution_mode",
            gwt::LaunchWizardAction::SetLinkedIssue { .. } => "set_linked_issue",
            gwt::LaunchWizardAction::ClearLinkedIssue => "clear_linked_issue",
            gwt::LaunchWizardAction::SetSkipPermissions { .. } => "set_skip_permissions",
            gwt::LaunchWizardAction::SetFastMode { .. } => "set_fast_mode",
            gwt::LaunchWizardAction::SetCodexFastMode { .. } => "set_codex_fast_mode",
            gwt::LaunchWizardAction::SetHermesOption { .. } => "set_hermes_option",
            gwt::LaunchWizardAction::SetHermesSafeMode { .. } => "set_hermes_safe_mode",
            gwt::LaunchWizardAction::RunAgentSetup => "run_agent_setup",
            gwt::LaunchWizardAction::Submit => "submit",
            gwt::LaunchWizardAction::GotoStep { .. } => "goto_step",
            gwt::LaunchWizardAction::AddAgentSettingsSet => "add_agent_settings_set",
            gwt::LaunchWizardAction::RemoveAgentSettingsSet { .. } => "remove_agent_settings_set",
            gwt::LaunchWizardAction::MoveAgentSettingsSet { .. } => "move_agent_settings_set",
            gwt::LaunchWizardAction::SelectAgentSettingsSet { .. } => "select_agent_settings_set",
        }
    }

    pub(super) fn log_launch_wizard_error(
        session: &LaunchWizardSession,
        stage: &'static str,
        action: &'static str,
        requested_agent_id: Option<&str>,
        error: &str,
    ) {
        let view = session.wizard.view();
        let sanitized_error = Self::sanitize_launch_log_error(error);
        let linked_issue_number = view
            .linked_issue_number
            .map(|issue_number| issue_number.to_string())
            .unwrap_or_else(|| "none".to_string());
        let requested_agent_id = requested_agent_id.unwrap_or("none");
        let selected_docker_service = view.selected_docker_service.as_deref().unwrap_or("none");
        let holder_session_id = session
            .manual_holder_intent
            .as_ref()
            .map(|intent| intent.predecessor.session_id.as_str())
            .unwrap_or("none");
        let holder_window_id = session
            .manual_holder_intent
            .as_ref()
            .and_then(|intent| intent.local_window_id.as_deref())
            .unwrap_or("none");
        let holder_runtime_incarnation = session
            .manual_holder_intent
            .as_ref()
            .and_then(|intent| intent.local_runtime_incarnation)
            .map(|incarnation| incarnation.to_string())
            .unwrap_or_else(|| "none".to_string());
        let holder_fingerprint_digest = session
            .manual_holder_intent
            .as_ref()
            .map(|intent| {
                let digest = Sha256::digest(intent.fingerprint.as_bytes());
                hex::encode(digest)[..16].to_string()
            })
            .unwrap_or_else(|| "none".to_string());
        tracing::error!(
            target: "gwt::agent_launch",
            stage = %stage,
            action = %action,
            wizard_id = %session.wizard_id,
            tab_id = %session.tab_id,
            requested_agent_id = %requested_agent_id,
            selected_agent_id = %view.selected_agent_id,
            selected_launch_target = %view.selected_launch_target,
            selected_runtime_target = %view.selected_runtime_target,
            selected_docker_service = %selected_docker_service,
            linked_issue_number = %linked_issue_number,
            holder_session_id = %holder_session_id,
            holder_window_id = %holder_window_id,
            holder_runtime_incarnation = %holder_runtime_incarnation,
            holder_fingerprint_digest = %holder_fingerprint_digest,
            error = %sanitized_error,
            "launch wizard action failed"
        );
        gwt::error_report::report_error_and_publish(
            gwt_core::error_ledger::ErrorKind::LaunchFailure,
            sanitized_error,
            gwt_core::error_ledger::ErrorTarget {
                issue: view
                    .linked_issue_number
                    .or(session.issue_monitor_launch_issue_number),
                window_id: Some(session.wizard_id.clone()),
                session_id: session
                    .manual_holder_intent
                    .as_ref()
                    .map(|intent| intent.predecessor.session_id.clone()),
                project_root: Some(session.project_context.project_root.display().to_string()),
            },
        );
    }

    pub(super) fn sanitize_launch_log_error(error: &str) -> String {
        gwt::project_runtime::launch::sanitize_launch_log_error(error)
    }

    pub(super) fn user_facing_launch_error_detail(detail: &str) -> String {
        if let Some(hint) = Self::missing_binary_install_hint(detail) {
            return hint.to_string();
        }
        if detail.contains(gwt_agent::EXECUTION_BINDING_OWNER_MISMATCH) {
            return format!("{detail}{EXECUTION_BINDING_OWNER_MISMATCH_RECOVERY}");
        }
        detail.to_string()
    }

    fn missing_binary_install_hint(detail: &str) -> Option<&'static str> {
        MISSING_BINARY_INSTALL_HINTS
            .iter()
            .find(|(command, _)| {
                Self::is_missing_binary_error(detail, command)
                    || Self::is_unresolved_preflight_runner_error(detail, command)
            })
            .map(|(_, hint)| *hint)
    }

    /// A missing installed executable fails the preflight health check before
    /// PTY spawn and receives the same install guidance as a spawn failure.
    fn is_unresolved_preflight_runner_error(detail: &str, command: &str) -> bool {
        let Some(descriptor) = gwt_agent::builtin_agent_descriptor_for_command(command) else {
            return false;
        };
        detail.contains(&format!(
            "{} installed runner failed its health check",
            descriptor.display_name
        )) && detail.contains("installed executable not resolved")
    }

    fn is_missing_binary_error(detail: &str, command: &str) -> bool {
        detail.contains(&format!("Unable to spawn {command}"))
            && (detail.contains("No viable candidates found in PATH")
                || detail.contains("command not found")
                || detail.contains("No such file or directory"))
    }

    pub(super) fn launch_error_terminal_bytes(detail: &str) -> Vec<u8> {
        gwt::project_runtime::agent_dispatch::launch_error_terminal_bytes(detail)
    }

    pub(super) fn launch_error_terminal_output_event(
        &self,
        window_id: String,
        detail: &str,
    ) -> Option<OutboundEvent> {
        let key = self.project_key_for_window(&window_id)?.clone();
        Some(OutboundEvent::project(
            key,
            BackendEvent::TerminalOutput {
                id: window_id,
                data_base64: base64::engine::general_purpose::STANDARD
                    .encode(Self::launch_error_terminal_bytes(detail)),
            },
        ))
    }

    pub(super) fn status_events(
        &self,
        window_id: impl Into<String>,
        status: WindowProcessStatus,
        detail: Option<String>,
    ) -> Vec<OutboundEvent> {
        let window_id = window_id.into();
        let Some(key) = self.project_key_for_window(&window_id).cloned() else {
            return Vec::new();
        };
        vec![
            OutboundEvent::project(
                key.clone(),
                BackendEvent::WindowState {
                    window_id: window_id.clone(),
                    state: status,
                },
            ),
            OutboundEvent::project(
                key,
                BackendEvent::TerminalStatus {
                    id: window_id,
                    status,
                    detail,
                    error_code: None,
                    retryable: None,
                },
            ),
        ]
    }
}

#[cfg(test)]
mod install_hint_tests {
    use super::AppRuntime;

    #[test]
    fn raw_spawn_failure_still_maps_to_install_hint() {
        let detail = "Unable to spawn agy: No viable candidates found in PATH";
        let user = AppRuntime::user_facing_launch_error_detail(detail);
        assert!(user.contains("antigravity.google/cli/install.sh"), "{user}");
    }

    /// SPEC-3864 FR-008 (AC-7): the preflight health check fails before any
    /// PTY spawn, so the install guidance must also fire on that shape.
    #[test]
    fn preflight_unresolved_runner_maps_to_install_hint() {
        let detail = "Antigravity CLI installed runner failed its health check. installed executable not resolved. Install it with `curl -fsSL https://antigravity.google/cli/install.sh | bash` and relaunch.";
        let user = AppRuntime::user_facing_launch_error_detail(detail);
        assert!(user.contains("antigravity.google/cli/install.sh"), "{user}");
        assert!(user.contains("not found in PATH"), "{user}");

        let opencode = "OpenCode installed runner failed its health check. installed executable not resolved. Install it with `npm i -g opencode-ai` and relaunch.";
        let user = AppRuntime::user_facing_launch_error_detail(opencode);
        assert!(user.contains("npm i -g opencode-ai"), "{user}");
    }

    #[test]
    fn preflight_failure_with_resolved_broken_runner_keeps_raw_detail() {
        // A resolvable-but-broken runner is a different failure; the raw
        // diagnostic must not be replaced by install guidance.
        let detail =
            "OpenCode installed runner failed its health check. exit status 1; runner broken";
        assert_eq!(AppRuntime::user_facing_launch_error_detail(detail), detail);
    }
}
