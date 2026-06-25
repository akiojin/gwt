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
//!   [`AppRuntime::log_window_launch_error`],
//!   `sanitize_launch_log_error`)
//! - The launch error -> frontend event bridge
//!   ([`AppRuntime::launch_error_events`],
//!   [`AppRuntime::launch_error_terminal_bytes`],
//!   [`AppRuntime::status_events`])

use base64::Engine as _;

use super::{
    AppRuntime, BackendEvent, LaunchFeedbackContext, LaunchWizardSession, OutboundEvent,
    WindowProcessStatus,
};

/// Agents whose missing-binary launch failure should be rewritten into an
/// actionable install hint instead of leaking the raw PTY/PATH error. Each
/// entry maps the spawn command token that appears in the raw error
/// (`Unable to spawn <command>`) to the user-facing guidance.
///
/// SPEC-3151 FR-003: OpenCode joined the table so a missing `opencode` binary
/// with no available package runner surfaces install guidance rather than
/// `No viable candidates found in PATH`.
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
            "OpenCode (`opencode`) was not found and no npm package runner (bunx/npx) is available. ",
            "Select a non-`Installed` version in the Launch wizard to run it via bunx/npx, ",
            "or install it with: npm i -g opencode-ai ",
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
            gwt::LaunchWizardAction::FocusExistingSession { .. } => "focus_existing_session",
            gwt::LaunchWizardAction::SetAgent { .. } => "agent_select",
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
            gwt::LaunchWizardAction::SetBranchMode { .. } => "set_branch_mode",
            gwt::LaunchWizardAction::SetBranchType { .. } => "set_branch_type",
            gwt::LaunchWizardAction::SetBranchName { .. } => "set_branch_name",
            gwt::LaunchWizardAction::SelectExistingBranch { .. } => "select_existing_branch",
            gwt::LaunchWizardAction::SetInitialPrompt { .. } => "set_initial_prompt",
            gwt::LaunchWizardAction::SetLaunchTarget { .. } => "set_launch_target",
            gwt::LaunchWizardAction::SetAgent { .. } => "set_agent",
            gwt::LaunchWizardAction::SetModel { .. } => "set_model",
            gwt::LaunchWizardAction::SetReasoning { .. } => "set_reasoning",
            gwt::LaunchWizardAction::SetRuntimeTarget { .. } => "set_runtime_target",
            gwt::LaunchWizardAction::SetWindowsShell { .. } => "set_windows_shell",
            gwt::LaunchWizardAction::SetDockerService { .. } => "set_docker_service",
            gwt::LaunchWizardAction::SetDockerLifecycle { .. } => "set_docker_lifecycle",
            gwt::LaunchWizardAction::SetVersion { .. } => "set_version",
            gwt::LaunchWizardAction::SetExecutionMode { .. } => "set_execution_mode",
            gwt::LaunchWizardAction::SetLinkedIssue { .. } => "set_linked_issue",
            gwt::LaunchWizardAction::ClearLinkedIssue => "clear_linked_issue",
            gwt::LaunchWizardAction::SetSkipPermissions { .. } => "set_skip_permissions",
            gwt::LaunchWizardAction::SetFastMode { .. } => "set_fast_mode",
            gwt::LaunchWizardAction::SetCodexFastMode { .. } => "set_codex_fast_mode",
            gwt::LaunchWizardAction::SetHermesOption { .. } => "set_hermes_option",
            gwt::LaunchWizardAction::SetHermesSafeMode { .. } => "set_hermes_safe_mode",
            gwt::LaunchWizardAction::RunOpenCodeSetup => "run_opencode_setup",
            gwt::LaunchWizardAction::Submit => "submit",
            gwt::LaunchWizardAction::GotoStep { .. } => "goto_step",
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
            selected_tool_version = %view.selected_version,
            selected_docker_service = %selected_docker_service,
            linked_issue_number = %linked_issue_number,
            error = %sanitized_error,
            "launch wizard action failed"
        );
    }

    fn log_window_launch_error(&self, stage: &'static str, window_id: &str, error: &str) {
        let (tab_id, raw_window_id) = self
            .window_lookup
            .get(window_id)
            .map(|address| (address.tab_id.as_str(), address.raw_id.as_str()))
            .unwrap_or(("unknown", "unknown"));
        let session = self.active_agent_sessions.get(window_id);
        let session_id = session
            .map(|session| session.session_id.as_str())
            .unwrap_or("unknown");
        let agent_id = session
            .map(|session| session.agent_id.as_str())
            .unwrap_or("unknown");
        let branch_name = session
            .map(|session| session.branch_name.as_str())
            .unwrap_or("unknown");
        let sanitized_error = Self::sanitize_launch_log_error(error);
        tracing::error!(
            target: "gwt::agent_launch",
            stage = %stage,
            window_id = %window_id,
            tab_id = %tab_id,
            raw_window_id = %raw_window_id,
            session_id = %session_id,
            agent_id = %agent_id,
            branch = %branch_name,
            error = %sanitized_error,
            "window launch failed"
        );
    }

    fn sanitize_launch_log_error(error: &str) -> String {
        let sensitive_env_keys = [
            "ANTHROPIC_API_KEY",
            "OPENAI_API_KEY",
            "GEMINI_API_KEY",
            "GOOGLE_API_KEY",
            "GITHUB_TOKEN",
            "GH_TOKEN",
            "GWT_HOOK_TOKEN",
            "HOOK_TOKEN",
        ];
        let sensitive_flags = [
            "--api-key",
            "--apikey",
            "--token",
            "--auth-token",
            "--hook-token",
        ];

        let mut tokens = Vec::new();
        let mut redact_next = false;
        for token in error.split_whitespace() {
            if redact_next {
                tokens.push("[REDACTED]".to_string());
                redact_next = false;
                continue;
            }

            let normalized = token
                .trim_matches(|ch: char| matches!(ch, '"' | '\'' | ',' | ';'))
                .to_ascii_lowercase();
            if sensitive_flags.iter().any(|flag| normalized == *flag) {
                tokens.push(token.to_string());
                redact_next = true;
                continue;
            }
            if let Some(flag) = sensitive_flags
                .iter()
                .find(|flag| normalized.starts_with(&format!("{flag}=")))
            {
                tokens.push(format!("{flag}=[REDACTED]"));
                continue;
            }
            if let Some((key, _value)) = token.split_once('=') {
                let normalized_key = key.trim_matches(|ch: char| matches!(ch, '"' | '\''));
                if sensitive_env_keys
                    .iter()
                    .any(|candidate| normalized_key.eq_ignore_ascii_case(candidate))
                {
                    tokens.push(format!("{normalized_key}=[REDACTED]"));
                    continue;
                }
            }

            tokens.push(token.to_string());
        }
        tokens.join(" ")
    }

    pub(super) fn launch_error_events(
        &mut self,
        window_id: String,
        detail: String,
        launch_feedback_context: Option<LaunchFeedbackContext>,
    ) -> Vec<OutboundEvent> {
        self.log_window_launch_error("launch_complete", &window_id, &detail);
        let user_detail = Self::user_facing_launch_error_detail(&detail);
        let issue_monitor_issue_number = launch_feedback_context
            .as_ref()
            .and_then(|context| context.issue_monitor_issue_number);
        let terminal_output =
            Self::launch_error_terminal_output_event(window_id.clone(), &user_detail);
        if self.tracked_window_exists(&window_id) {
            self.launch_error_terminal_details
                .insert(window_id.clone(), user_detail.clone());
            let mut events = self.handle_runtime_status(
                window_id,
                WindowProcessStatus::Error,
                Some(user_detail),
            );
            events.push(terminal_output);
            if let Some(issue_number) = issue_monitor_issue_number {
                events.extend(self.issue_monitor_launch_failed_events(issue_number, &detail));
            }
            return events;
        }
        let mut events = Self::status_events(
            window_id,
            WindowProcessStatus::Error,
            Some(user_detail.clone()),
        );
        events.push(terminal_output);
        if let Some(context) = launch_feedback_context {
            events.push(OutboundEvent::reply(
                context.client_id,
                BackendEvent::LaunchWizardOpenError {
                    title: context.title,
                    message: user_detail,
                },
            ));
        }
        if let Some(issue_number) = issue_monitor_issue_number {
            events.extend(self.issue_monitor_launch_failed_events(issue_number, &detail));
        }
        events
    }

    fn user_facing_launch_error_detail(detail: &str) -> String {
        if let Some(hint) = Self::missing_binary_install_hint(detail) {
            return hint.to_string();
        }
        detail.to_string()
    }

    fn missing_binary_install_hint(detail: &str) -> Option<&'static str> {
        MISSING_BINARY_INSTALL_HINTS
            .iter()
            .find(|(command, _)| Self::is_missing_binary_error(detail, command))
            .map(|(_, hint)| *hint)
    }

    fn is_missing_binary_error(detail: &str, command: &str) -> bool {
        detail.contains(&format!("Unable to spawn {command}"))
            && (detail.contains("No viable candidates found in PATH")
                || detail.contains("command not found")
                || detail.contains("No such file or directory"))
    }

    pub(super) fn launch_error_terminal_bytes(detail: &str) -> Vec<u8> {
        let mut message = String::from("\r\n[gwt] Launch failed before PTY started.\r\n");
        let detail = detail.trim();
        if !detail.is_empty() {
            message.push_str("[gwt] ");
            message.push_str(detail);
            message.push_str("\r\n");
        }
        message.into_bytes()
    }

    fn launch_error_terminal_output_event(window_id: String, detail: &str) -> OutboundEvent {
        OutboundEvent::broadcast(BackendEvent::TerminalOutput {
            id: window_id,
            data_base64: base64::engine::general_purpose::STANDARD
                .encode(Self::launch_error_terminal_bytes(detail)),
        })
    }

    pub(super) fn status_events(
        window_id: impl Into<String>,
        status: WindowProcessStatus,
        detail: Option<String>,
    ) -> Vec<OutboundEvent> {
        let window_id = window_id.into();
        vec![
            OutboundEvent::broadcast(BackendEvent::WindowState {
                window_id: window_id.clone(),
                state: status,
            }),
            OutboundEvent::broadcast(BackendEvent::TerminalStatus {
                id: window_id,
                status,
                detail,
            }),
        ]
    }
}
