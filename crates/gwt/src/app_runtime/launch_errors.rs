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
use sha2::{Digest as _, Sha256};

use super::{
    AppRuntime, BackendEvent, LaunchFeedbackContext, LaunchWizardSession, OutboundEvent,
    WindowProcessStatus,
};

pub(super) struct LaunchErrorInput {
    window_id: String,
    tab_id: String,
    raw_window_id: String,
    pub(super) active: Option<super::ActiveAgentSession>,
    project_root: Option<std::path::PathBuf>,
    sessions_dir: std::path::PathBuf,
    session_cache: super::launch::LaunchWizardMemoryCache,
    restored_launch: Option<Option<String>>,
    pub(super) feedback: Option<LaunchFeedbackContext>,
    update_observations: (
        Vec<gwt::update_drain::PaneObservation>,
        Vec<std::path::PathBuf>,
    ),
    materializer_id: String,
    fallback_timeout: std::time::Duration,
}

pub(super) struct PreparedLaunchError {
    issue_number: Option<u64>,
    project_root: Option<std::path::PathBuf>,
    monitor: Option<super::PreparedIssueMonitorLaunchFailure>,
    handoff_note: Option<(String, bool)>,
    monitor_message: String,
}

pub(super) fn prepare_launch_error(
    input: LaunchErrorInput,
    detail: &str,
    current: &std::sync::atomic::AtomicBool,
) -> Option<PreparedLaunchError> {
    let sanitized = AppRuntime::sanitize_launch_log_error(detail);
    let active = input.active.as_ref();
    {
        #[cfg(test)]
        let _test_log_lock = if LAUNCH_WIZARD_ERROR_CAPTURE_OWNS_LOCK.with(std::cell::Cell::get) {
            None
        } else {
            Some(
                launch_wizard_error_log_lock()
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            )
        };
        tracing::error!(target: "gwt::agent_launch", stage = "launch_complete",
            window_id = %input.window_id, tab_id = %input.tab_id,
            raw_window_id = %input.raw_window_id,
            session_id = active.map(|a| a.session_id.as_str()).unwrap_or("unknown"),
            agent_id = active.map(|a| a.agent_id.as_str()).unwrap_or("unknown"),
            branch = active.map(|a| a.branch_name.as_str()).unwrap_or("unknown"),
            error = %sanitized, "window launch failed");
    }
    gwt::error_report::report_error_and_publish(
        gwt_core::error_ledger::ErrorKind::LaunchFailure,
        sanitized,
        gwt_core::error_ledger::ErrorTarget {
            window_id: Some(input.window_id.clone()),
            session_id: active.map(|a| a.session_id.clone()),
            project_root: input.project_root.as_ref().map(|p| p.display().to_string()),
            issue: None,
        },
    );
    let feedback = input.feedback.as_ref();
    let project_root = feedback
        .and_then(|f| f.issue_monitor_project_root.clone())
        .or(input.project_root);
    let issue_number = feedback
        .and_then(|f| f.issue_monitor_issue_number)
        .or_else(|| {
            let active = active?;
            let linked = input
                .session_cache
                .session_by_id(&active.session_id)
                .and_then(|s| s.linked_issue_number)
                .or_else(|| {
                    gwt_agent::Session::load_and_migrate(
                        &input
                            .sessions_dir
                            .join(format!("{}.toml", active.session_id)),
                    )
                    .ok()
                    .and_then(|s| s.linked_issue_number)
                })?;
            let prefs = gwt::load_issue_monitor_prefs(
                &gwt::issue_monitor_prefs_path_for_repo_path(project_root.as_deref()?),
            )
            .ok()?;
            let monitor =
                gwt::IssueMonitorState::with_prefs(gwt::IssueMonitorConfig::default(), prefs);
            (monitor.launched_window_issue(&input.window_id) == Some(linked)).then_some(linked)
        });
    // Candidate cleanup is exact and remains valid after cancellation. Monitor
    // and restore mutations require the GUI's captured launch to still be current.
    if !current.load(std::sync::atomic::Ordering::Acquire) {
        return None;
    }
    let mut monitor = None;
    let mut handoff_note = None;
    if let Some(issue_number) = issue_number {
        if let Some(handoff) = feedback.and_then(|f| f.issue_monitor_autonomous_handoff.as_ref()) {
            handoff_note = Some(AppRuntime::prepare_answered_handoff_failure_note(
                project_root.as_deref(),
                handoff,
                feedback.is_some_and(|f| f.issue_monitor_autonomous_submit_started),
                detail,
            ));
        } else {
            let message = if gwt::issue_monitor::is_git_https_auth_error(detail) {
                gwt::issue_monitor::git_https_auth_setup_message(detail)
            } else {
                detail.to_string()
            };
            let delivery_id = feedback.and_then(|f| f.issue_monitor_delivery_id.as_deref());
            let mode = feedback
                .and_then(|f| f.issue_monitor_session_mode)
                .unwrap_or(gwt_agent::SessionMode::Normal);
            let payload = AppRuntime::issue_monitor_launch_failed_payload(
                issue_number,
                &message,
                delivery_id,
                delivery_id.map(|_| input.materializer_id.as_str()),
                mode,
            );
            let publication = match project_root.as_deref() {
                Some(root) => AppRuntime::publish_issue_monitor_control_owned(root, payload),
                None => Err(
                    gwt::runtime_daemon_events::IssueMonitorControlPublishError::TransportUnavailable(
                        "no owning project is available for launch failure".into(),
                    ),
                ),
            };
            let mut prepared = AppRuntime::prepare_issue_monitor_launch_failure(
                project_root.as_deref(),
                issue_number,
                &message,
                delivery_id,
                mode,
                &input.materializer_id,
                input.fallback_timeout,
                publication,
            );
            if let Some(monitor) = prepared.monitor.as_ref() {
                let mut status = monitor.status_view();
                AppRuntime::apply_issue_monitor_launch_profile_status_from_cache(
                    &mut status,
                    project_root.as_deref(),
                    &input.session_cache,
                );
                if let Some(drain) = status.update_drain.as_mut() {
                    let (panes, worktrees) = input.update_observations;
                    let snapshot =
                        AppRuntime::read_update_quiescence_snapshot(panes, worktrees, monitor);
                    drain.blocking = gwt::update_drain::update_quiescence(&snapshot)
                        .err()
                        .unwrap_or_default();
                }
                prepared.status = Some(Box::new(status));
            }
            prepared.defer_wake = true;
            monitor = Some(prepared);
        }
    } else if let Some(Some(source)) = input.restored_launch {
        super::startup::mark_auto_resume_source_completed(&input.sessions_dir, &source);
    }
    let monitor_message = if gwt::issue_monitor::is_git_https_auth_error(detail) {
        gwt::issue_monitor::git_https_auth_setup_message(detail)
    } else {
        detail.to_string()
    };
    Some(PreparedLaunchError {
        issue_number,
        project_root,
        monitor,
        handoff_note,
        monitor_message,
    })
}

#[cfg(test)]
thread_local! {
    static LAUNCH_WIZARD_ERROR_CAPTURE_OWNS_LOCK: std::cell::Cell<bool> = const {
        std::cell::Cell::new(false)
    };
}

#[cfg(test)]
fn launch_wizard_error_log_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

#[cfg(test)]
struct LaunchWizardErrorCaptureScope;

#[cfg(test)]
impl Drop for LaunchWizardErrorCaptureScope {
    fn drop(&mut self) {
        LAUNCH_WIZARD_ERROR_CAPTURE_OWNS_LOCK.with(|owns_lock| owns_lock.set(false));
    }
}

#[cfg(test)]
pub(super) fn with_launch_wizard_error_log_capture<T>(operation: impl FnOnce() -> T) -> T {
    let _lock = launch_wizard_error_log_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    LAUNCH_WIZARD_ERROR_CAPTURE_OWNS_LOCK.with(|owns_lock| {
        assert!(!owns_lock.replace(true), "launch error capture cannot nest");
    });
    let _scope = LaunchWizardErrorCaptureScope;
    operation()
}

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
            gwt::LaunchWizardAction::StopAndStartSuccessor { .. } => "stop_and_start_successor",
            gwt::LaunchWizardAction::MoveExistingPane { .. } => "move_existing_pane",
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
        #[cfg(test)]
        let _test_log_lock = if LAUNCH_WIZARD_ERROR_CAPTURE_OWNS_LOCK.with(std::cell::Cell::get) {
            None
        } else {
            Some(
                launch_wizard_error_log_lock()
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            )
        };
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
        let project_root = self.window_lookup.get(window_id).and_then(|address| {
            self.tabs
                .iter()
                .find(|tab| tab.id == address.tab_id)
                .map(|tab| tab.project_root.display().to_string())
        });
        gwt::error_report::report_error_and_publish(
            gwt_core::error_ledger::ErrorKind::LaunchFailure,
            sanitized_error,
            gwt_core::error_ledger::ErrorTarget {
                window_id: Some(window_id.to_string()),
                session_id: session.map(|session| session.session_id.clone()),
                project_root,
                issue: None,
            },
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

    pub(super) fn capture_launch_error_input(
        &self,
        window_id: &str,
        feedback: Option<LaunchFeedbackContext>,
    ) -> LaunchErrorInput {
        let address = self.window_lookup.get(window_id);
        LaunchErrorInput {
            window_id: window_id.to_string(),
            tab_id: address
                .map(|a| a.tab_id.clone())
                .unwrap_or_else(|| "unknown".into()),
            raw_window_id: address
                .map(|a| a.raw_id.clone())
                .unwrap_or_else(|| "unknown".into()),
            active: self.active_agent_sessions.get(window_id).cloned(),
            project_root: self.issue_monitor_project_root_for_window(window_id),
            sessions_dir: self.sessions_dir.clone(),
            session_cache: self.launch_wizard_cache.clone(),
            restored_launch: self.restore_launch_windows.get(window_id).cloned(),
            feedback,
            update_observations: self.capture_update_quiescence_inputs(),
            materializer_id: self.issue_monitor_materializer_id.clone(),
            fallback_timeout: self.issue_monitor_fallback_commit_timeout,
        }
    }

    pub(super) fn apply_launch_error_status(
        &mut self,
        window_id: &str,
        detail: &str,
    ) -> Vec<OutboundEvent> {
        self.window_hook_states.remove(window_id);
        self.window_pty_statuses
            .insert(window_id.to_string(), WindowProcessStatus::Error);
        self.clear_runtime_approval_latch_without_status(window_id, true);
        let status = self
            .recompute_window_state(window_id)
            .unwrap_or(WindowProcessStatus::Error);
        self.window_details
            .insert(window_id.to_string(), detail.to_string());
        self.deregister_pty_writer(window_id);
        if let Some(root) = self.issue_monitor_project_root_for_window(window_id) {
            super::runtime_events::publish_runtime_status_change(
                &root,
                window_id,
                WindowProcessStatus::Error,
                Some(detail.to_string()),
            );
        }
        let _ = self.persist();
        self.status_events(window_id, status, Some(detail.to_string()))
    }

    pub(super) fn launch_error_events(
        &mut self,
        window_id: String,
        detail: String,
        launch_feedback_context: Option<LaunchFeedbackContext>,
    ) -> Vec<OutboundEvent> {
        self.launch_error_events_prepared(window_id, detail, launch_feedback_context, None)
    }

    pub(super) fn launch_error_events_prepared(
        &mut self,
        window_id: String,
        detail: String,
        launch_feedback_context: Option<LaunchFeedbackContext>,
        mut prepared: Option<PreparedLaunchError>,
    ) -> Vec<OutboundEvent> {
        if prepared.is_none() {
            self.log_window_launch_error("launch_complete", &window_id, &detail);
        }
        // Issue #4143 (AC-3): read the automatic-restore guard before anything
        // below can publish an Error status for this window. The launch is over
        // either way, so the marker is consumed here.
        self.record_restore_window_outcome(
            &window_id,
            Err(super::startup::RestoreRefusal::LaunchNotStarted),
        );
        let restored_launch = self.restore_launch_windows.remove(&window_id);
        let user_detail = Self::user_facing_launch_error_detail(&detail);
        let issue_monitor_issue_number =
            prepared.as_ref().and_then(|p| p.issue_number).or_else(|| {
                launch_feedback_context
                    .as_ref()
                    .and_then(|context| context.issue_monitor_issue_number)
            });
        let issue_monitor_delivery_id = launch_feedback_context
            .as_ref()
            .and_then(|context| context.issue_monitor_delivery_id.clone());
        let issue_monitor_project_root = launch_feedback_context
            .as_ref()
            .and_then(|context| context.issue_monitor_project_root.clone())
            .or_else(|| prepared.as_ref().and_then(|p| p.project_root.clone()))
            .or_else(|| self.issue_monitor_project_root_for_window(&window_id));
        let issue_monitor_session_mode = launch_feedback_context
            .as_ref()
            .and_then(|context| context.issue_monitor_session_mode)
            .unwrap_or(gwt_agent::SessionMode::Normal);
        let issue_monitor_autonomous_handoff = launch_feedback_context
            .as_ref()
            .and_then(|context| context.issue_monitor_autonomous_handoff.clone());
        let issue_monitor_autonomous_submit_started = launch_feedback_context
            .as_ref()
            .is_some_and(|context| context.issue_monitor_autonomous_submit_started);
        let terminal_output =
            self.launch_error_terminal_output_event(window_id.clone(), &user_detail);
        if self.tracked_window_exists(&window_id) {
            self.launch_error_terminal_details
                .insert(window_id.clone(), user_detail.clone());
            let mut events = if prepared.is_some() {
                self.apply_launch_error_status(&window_id, &user_detail)
            } else {
                self.handle_runtime_status(
                    window_id.clone(),
                    WindowProcessStatus::Error,
                    Some(user_detail),
                )
            };
            events.extend(terminal_output);
            // Issue #3927 (SPEC #3340 AS-44 / FR-048): a restore carries no
            // launch context, so a Monitor-owned restored window is
            // recognised through its Session's Issue link.
            let monitor_owned_issue = issue_monitor_issue_number.or_else(|| {
                if prepared.is_some() {
                    return None;
                }
                self.issue_monitor_owned_restore_issue(
                    &window_id,
                    issue_monitor_project_root.as_deref(),
                )
            });
            if let Some(issue_number) = monitor_owned_issue {
                if let Some(handoff) = issue_monitor_autonomous_handoff.as_ref() {
                    let (failure_events, committed) = self
                        .answered_handoff_launch_failure_events_prepared(
                            issue_monitor_project_root.as_deref(),
                            issue_number,
                            issue_monitor_delivery_id.as_deref(),
                            handoff,
                            issue_monitor_autonomous_submit_started,
                            &detail,
                            prepared.as_mut().and_then(|p| p.handoff_note.take()),
                        );
                    events.extend(failure_events);
                    // The durable retry owns this definitely pre-submit failure.
                    // Submitted ambiguity and any surviving runtime retain their
                    // window as evidence for exact reconciliation.
                    if committed && !self.runtimes.contains_key(&window_id) {
                        events.extend(
                            self.close_window_after_issue_monitor_finalize_events(&window_id),
                        );
                    }
                } else {
                    let (failure_events, committed) =
                        if let Some(receipt) = prepared.as_mut().and_then(|p| p.monitor.take()) {
                            self.apply_issue_monitor_launch_failure(
                                issue_monitor_project_root.as_deref(),
                                issue_number,
                                prepared
                                    .as_ref()
                                    .expect("prepared failure")
                                    .monitor_message
                                    .as_str(),
                                issue_monitor_delivery_id.as_deref(),
                                receipt,
                            )
                        } else {
                            self.issue_monitor_launch_failed_delivery_committed_events_with_mode(
                                issue_monitor_project_root.as_deref(),
                                issue_number,
                                &detail,
                                issue_monitor_delivery_id.as_deref(),
                                issue_monitor_session_mode,
                            )
                        };
                    events.extend(failure_events);
                    // FR-048: the concrete reason is already in gwt.log
                    // (`log_window_launch_error` above) and now durably in
                    // the Monitor's `error_message`; only then is the
                    // pre-PTY pane closed. A failed commit retains it.
                    if committed {
                        tracing::info!(
                            target: "gwt::agent_launch",
                            window_id = %window_id,
                            issue_number,
                            "closing the Issue Monitor-owned window that failed before PTY start"
                        );
                        events.extend(
                            self.close_window_after_issue_monitor_finalize_events(&window_id),
                        );
                    }
                }
                return events;
            }
            // Issue #4143 (AC-3): an automatic restore that failed before its
            // PTY started must not survive as a persistent Error window.
            // Keeping it made the failures self-propagating: the window
            // persisted, the next start restored it, and the next failure added
            // another one — the mechanism that grew 135 `Launch failed before
            // PTY started.` panes across restarts. The concrete reason is
            // already in gwt.log and in the host error ledger that backs
            // `errors.list` (`log_window_launch_error` above), so the
            // diagnostic outlives the pane.
            if let Some(source_session_id) = restored_launch {
                if self.window_lookup.contains_key(&window_id) {
                    tracing::warn!(
                        target: "gwt::agent_launch",
                        window_id = %window_id,
                        session_id = source_session_id.as_deref().unwrap_or("-"),
                        "closing the automatically restored window that failed before PTY start"
                    );
                    events
                        .extend(self.close_window_after_issue_monitor_finalize_events(&window_id));
                }
                // The window is gone, so nothing keeps the Session out of the
                // next start's restore set except the Session itself.
                if let Some(session_id) =
                    source_session_id.as_deref().filter(|_| prepared.is_none())
                {
                    super::startup::mark_auto_resume_source_completed(
                        &self.sessions_dir,
                        session_id,
                    );
                }
            }
            return events;
        }
        let mut events = self.status_events(
            window_id,
            WindowProcessStatus::Error,
            Some(user_detail.clone()),
        );
        events.extend(terminal_output);
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
            if let Some(handoff) = issue_monitor_autonomous_handoff.as_ref() {
                let (failure_events, _) = self.answered_handoff_launch_failure_events_prepared(
                    issue_monitor_project_root.as_deref(),
                    issue_number,
                    issue_monitor_delivery_id.as_deref(),
                    handoff,
                    issue_monitor_autonomous_submit_started,
                    &detail,
                    prepared.as_mut().and_then(|p| p.handoff_note.take()),
                );
                events.extend(failure_events);
            } else {
                let failure_events =
                    if let Some(receipt) = prepared.as_mut().and_then(|p| p.monitor.take()) {
                        self.apply_issue_monitor_launch_failure(
                            issue_monitor_project_root.as_deref(),
                            issue_number,
                            prepared
                                .as_ref()
                                .expect("prepared failure")
                                .monitor_message
                                .as_str(),
                            issue_monitor_delivery_id.as_deref(),
                            receipt,
                        )
                        .0
                    } else {
                        self.issue_monitor_launch_failed_delivery_events_with_mode(
                            issue_monitor_project_root.as_deref(),
                            issue_number,
                            &detail,
                            issue_monitor_delivery_id.as_deref(),
                            issue_monitor_session_mode,
                        )
                    };
                events.extend(failure_events);
            }
        }
        events
    }

    #[allow(clippy::too_many_arguments)]
    fn answered_handoff_launch_failure_events_prepared(
        &mut self,
        project_root: Option<&std::path::Path>,
        issue_number: u64,
        delivery_id: Option<&str>,
        handoff: &gwt::AutonomousHandoffDeliveryAttempt,
        submit_started: bool,
        detail: &str,
        prepared_note: Option<(String, bool)>,
    ) -> (Vec<OutboundEvent>, bool) {
        let local_delivery_key = delivery_id
            .map(str::to_string)
            .unwrap_or_else(|| format!("handoff:{}", handoff.handoff_id));
        self.issue_monitor_launch_deliveries
            .remove(&local_delivery_key);
        let (durable_note, failure_committed) = prepared_note.unwrap_or_else(|| {
            Self::prepare_answered_handoff_failure_note(
                project_root,
                handoff,
                submit_started,
                detail,
            )
        });
        let Some(context) = project_root.and_then(|root| self.project_context_for_root(root))
        else {
            return (Vec::new(), failure_committed);
        };
        (vec![OutboundEvent::project(context.project_key, BackendEvent::IssueMonitorToast {
            notification_transition: None,
            level: "error".to_string(),
            message: format!(
                "Issue Monitor could not confirm the exact answered-session submit{durable_note}: {detail}"
            ),
            issue_number: Some(issue_number),
        }).with_error_project_root(&context.project_root)], failure_committed)
    }

    fn prepare_answered_handoff_failure_note(
        project_root: Option<&std::path::Path>,
        handoff: &gwt::AutonomousHandoffDeliveryAttempt,
        submit_started: bool,
        detail: &str,
    ) -> (String, bool) {
        let mut failure_committed = false;
        let note = project_root.map_or_else(
            || "; the owning Project State is unavailable".to_string(),
            |project_root| {
                let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(project_root);
                let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
                if submit_started {
                    match gwt::mark_autonomous_handoff_delivery_ambiguous_from_prefs(
                        &prefs_path,
                        &handoff.handoff_id,
                        &handoff.session_id,
                        handoff.attempt,
                        detail,
                        &now,
                    ) {
                        Ok(true) => {
                            "; the ambiguous attempt was parked for human review".to_string()
                        }
                        Ok(false) => "; the durable attempt no longer matched".to_string(),
                        Err(error) => {
                            format!("; durable ambiguity could not be recorded: {error}")
                        }
                    }
                } else {
                    match gwt::record_autonomous_handoff_delivery_failure_from_prefs(
                        &prefs_path,
                        &handoff.handoff_id,
                        &handoff.session_id,
                        handoff.attempt,
                        detail,
                        &now,
                    ) {
                        Ok(gwt::AutonomousHandoffDeliveryFailureOutcome::Retry {
                            retry_not_before,
                            ..
                        }) => {
                            failure_committed = true;
                            format!(
                                "; the definitely pre-submit attempt will retry after {retry_not_before}"
                            )
                        }
                        Ok(gwt::AutonomousHandoffDeliveryFailureOutcome::Escalated {
                            ..
                        }) => {
                            failure_committed = true;
                            "; the bounded retry ladder was exhausted".to_string()
                        }
                        Ok(gwt::AutonomousHandoffDeliveryFailureOutcome::Rejected) => {
                            "; the durable attempt no longer matched".to_string()
                        }
                        Err(error) => {
                            format!("; durable pre-submit failure could not be recorded: {error}")
                        }
                    }
                }
            },
        );
        (note, failure_committed)
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
        let mut message = String::from("\r\n[gwt] Launch failed before PTY started.\r\n");
        let detail = detail.trim();
        if !detail.is_empty() {
            message.push_str("[gwt] ");
            message.push_str(detail);
            message.push_str("\r\n");
        }
        message.into_bytes()
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
