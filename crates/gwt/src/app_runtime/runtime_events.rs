//! Runtime event handlers and daemon publish queue for SPEC-2077 Phase J1.
//!
//! This module owns the already daemon-aware runtime output/status/hook
//! publish path. The extraction is behavior-preserving: it keeps best-effort
//! daemon publish, same-process echo suppression through the existing payload
//! layer, and the local GUI state update path unchanged.

use base64::Engine as _;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::sync::mpsc as std_mpsc;
#[cfg(unix)]
use std::sync::Mutex;

use super::{
    close_window_from_workspace, mark_auto_resume_source_completed, should_auto_close_agent_window,
    AppRuntime, BackendEvent, OutboundEvent, PendingProviderRootClaim, WindowPreset,
    WindowProcessStatus,
};

/// Issue #3274: how many trailing non-empty screen lines survive into the
/// persistent window detail when an agent process errors out.
const AGENT_ERROR_TAIL_LINES: usize = 3;
const AGENT_ERROR_TAIL_MAX_CHARS: usize = 240;
/// Claude Code's exact-resume failure line. When a resumed conversation no
/// longer exists in the agent's store, this is the only explanation the user
/// ever gets — promote it to an explicit diagnostic (SPEC-1921 exact session
/// restore amendment: stale provider ids keep a visible diagnostic).
const EXACT_RESUME_FAILURE_SIGNATURE: &str = "No conversation found with session ID";

pub(super) fn mark_recovery_ready_for_session(
    sessions_dir: &Path,
    session_id: &str,
    project_dir_override: Option<&Path>,
    provider_root_claim: Option<&PendingProviderRootClaim>,
) -> std::io::Result<()> {
    let path = sessions_dir.join(format!("{session_id}.toml"));
    let session = gwt_agent::Session::load_and_migrate(&path)?;
    let mut durable_ready_committed = false;
    if let (Some(recovery_id), Some(project_root)) = (
        session.recovery_id.as_deref(),
        session.project_state_root.as_deref(),
    ) {
        let project_dir = project_dir_override
            .map(Path::to_path_buf)
            .unwrap_or_else(|| gwt_core::paths::gwt_project_dir_for_repo_path(project_root));
        let store = gwt_core::recovery::RecoveryStore::for_project_dir(project_dir);
        if store
            .load(recovery_id)
            .map_err(|error| std::io::Error::other(error.to_string()))?
            .is_some()
        {
            let exact_handoff = session.recovery_continuation.as_ref().filter(|handoff| {
                session.session_mode == gwt_agent::SessionMode::Resume
                    && !handoff.inherit_checkpoint
            });
            match (exact_handoff, provider_root_claim) {
                (Some(handoff), Some(claim))
                    if handoff.target_recovery_id == recovery_id
                        && handoff.source_recovery_id == claim.recovery_id =>
                {
                    store
                        .complete_claimed_provider_ready(
                            &claim.recovery_id,
                            recovery_id,
                            &claim.claim_token,
                            chrono::Utc::now(),
                            format!("provider-ready-claim:{session_id}"),
                        )
                        .map_err(|error| std::io::Error::other(error.to_string()))?;
                }
                (Some(_), Some(_)) => {
                    return Err(std::io::Error::other(
                        "exact recovery Ready handoff no longer matches its provider-root claim",
                    ));
                }
                (Some(_), None) => {
                    return Err(std::io::Error::other(
                        "exact recovery Ready requires its provider-root claim",
                    ));
                }
                (None, Some(_)) => {
                    return Err(std::io::Error::other(
                        "provider-root claim cannot complete Ready without an exact recovery handoff",
                    ));
                }
                (None, None) => {
                    store
                        .complete_provider_ready(
                            recovery_id,
                            chrono::Utc::now(),
                            format!("provider-ready:{session_id}"),
                        )
                        .map_err(|error| std::io::Error::other(error.to_string()))?;
                }
            }
            durable_ready_committed = true;
        }
    }
    let session_update = gwt_agent::update_session(sessions_dir, session_id, |session| {
        session.advance_recovery_launch_stage(gwt_agent::session::RecoveryLaunchStage::Ready)
    });
    match session_update {
        Ok(_) => Ok(()),
        Err(error) if durable_ready_committed => {
            tracing::warn!(
                session_id,
                error = %error,
                "Recovery Record crossed Ready but Session stage update failed"
            );
            Ok(())
        }
        Err(error) => Err(error),
    }
}

/// Compose the persistent window detail for an errored agent process from the
/// plain exit detail and the final screen tail. Pure so the classification is
/// unit-testable without a PTY.
fn compose_agent_error_detail(base: Option<String>, tail: Option<&str>) -> Option<String> {
    let tail = tail.map(str::trim).filter(|tail| !tail.is_empty());
    let Some(tail) = tail else {
        return base;
    };
    let tail: String = if tail.chars().count() > AGENT_ERROR_TAIL_MAX_CHARS {
        let mut truncated: String = tail.chars().take(AGENT_ERROR_TAIL_MAX_CHARS).collect();
        truncated.push('…');
        truncated
    } else {
        tail.to_string()
    };
    if tail.contains(EXACT_RESUME_FAILURE_SIGNATURE) {
        return Some(format!(
            "Exact session restore failed: {tail}. The agent no longer has this \
             conversation; launch a new agent session when you want to continue."
        ));
    }
    match base {
        Some(base) if !base.is_empty() => Some(format!("{base} — last output: {tail}")),
        _ => Some(format!("Agent exited — last output: {tail}")),
    }
}

pub(super) fn exact_resume_rejection_matches_root(tail: &str, expected_root_id: &str) -> bool {
    let expected_root_id = expected_root_id.trim();
    if expected_root_id.is_empty() {
        return false;
    }
    tail.split(EXACT_RESUME_FAILURE_SIGNATURE)
        .skip(1)
        .filter_map(|suffix| {
            suffix
                .trim_start_matches(|character: char| {
                    character.is_whitespace() || matches!(character, ':' | '=' | '`' | '"' | '\'')
                })
                .split_whitespace()
                .next()
        })
        .map(|reported| {
            reported.trim_matches(|character: char| {
                matches!(character, '.' | ',' | ';' | '`' | '"' | '\'')
            })
        })
        .any(|reported| reported == expected_root_id)
}

impl AppRuntime {
    pub(crate) fn handle_runtime_output(
        &mut self,
        id: String,
        data: Vec<u8>,
    ) -> Vec<OutboundEvent> {
        self.handle_runtime_output_inner(id, data, true)
    }

    pub(crate) fn handle_daemon_runtime_output(
        &mut self,
        id: String,
        data: Vec<u8>,
    ) -> Vec<OutboundEvent> {
        self.handle_runtime_output_inner(id, data, false)
    }

    fn handle_runtime_output_inner(
        &mut self,
        id: String,
        data: Vec<u8>,
        publish_to_daemon: bool,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(&id).cloned() else {
            return Vec::new();
        };
        if publish_to_daemon {
            if let Some(tab) = self.tab(&address.tab_id) {
                publish_runtime_output_change(&tab.project_root, &id, &data);
            }
        }
        vec![OutboundEvent::broadcast(BackendEvent::TerminalOutput {
            id,
            data_base64: base64::engine::general_purpose::STANDARD.encode(data),
        })]
    }

    pub(crate) fn handle_runtime_status(
        &mut self,
        id: String,
        status: WindowProcessStatus,
        detail: Option<String>,
    ) -> Vec<OutboundEvent> {
        self.handle_runtime_status_inner(id, status, detail, true)
    }

    pub(crate) fn handle_daemon_runtime_status(
        &mut self,
        id: String,
        status: WindowProcessStatus,
        detail: Option<String>,
    ) -> Vec<OutboundEvent> {
        self.handle_runtime_status_inner(id, status, detail, false)
    }

    fn handle_runtime_status_inner(
        &mut self,
        id: String,
        status: WindowProcessStatus,
        detail: Option<String>,
        publish_to_daemon: bool,
    ) -> Vec<OutboundEvent> {
        let Some(_address) = self.window_lookup.get(&id).cloned() else {
            self.remove_window_state_tracking(&id);
            self.mark_agent_session_stopped(&id);
            self.deregister_pty_writer(&id);
            self.runtimes.remove(&id);
            self.codex_bridge_routes.remove(&id);
            self.window_details.remove(&id);
            // SPEC-3214 FR-002: the status arrived after the window was torn
            // down, so the PTY is gone — safe point to destroy any pending
            // intake worktree.
            return self.take_ephemeral_worktree_cleanup_events();
        };
        let is_agent_window = self.window_preset(&id) == Some(WindowPreset::Agent);
        if publish_to_daemon {
            if let Some(address) = self.window_lookup.get(&id) {
                if let Some(tab) = self.tab(&address.tab_id) {
                    publish_runtime_status_change(&tab.project_root, &id, status, detail.clone());
                }
            }
        }

        // SPEC #3200 T-045/FR-025: a running agent on a monitored autonomous
        // issue is a liveness signal — refresh its stuck-detection window.
        if is_agent_window && matches!(status, WindowProcessStatus::Running) {
            self.issue_monitor_heartbeat(&id);
        }

        let keep_active_agent_session_for_recovery =
            self.should_keep_active_agent_session_for_recoverable_pty_error(&id, status);
        let pending_auto_recovery = self.pending_auto_resume_sources.contains_key(&id);
        // Issue #3274: an errored agent runtime is torn down below, dropping
        // its vt100 state — a client that reconnects later replays nothing and
        // an empty Error window gives no clue why. Capture the final screen
        // tail into the persistent detail before the state is gone; the raw
        // output stays available in logs.
        let final_screen_tail = if matches!(status, WindowProcessStatus::Error)
            && matches!(
                self.window_preset(&id),
                Some(WindowPreset::Agent | WindowPreset::Claude | WindowPreset::Codex)
            ) {
            self.final_screen_tail(&id)
        } else {
            None
        };
        let exact_resume_rejected = pending_auto_recovery
            && self
                .pending_auto_resume_exact_root(&id)
                .zip(final_screen_tail.as_deref())
                .is_some_and(|(expected_root, tail)| {
                    exact_resume_rejection_matches_root(tail, &expected_root)
                });
        let detail = if final_screen_tail.is_some() {
            compose_agent_error_detail(detail, final_screen_tail.as_deref())
        } else {
            detail
        };
        if matches!(status, WindowProcessStatus::Error) {
            self.window_hook_states.remove(&id);
        }
        self.window_pty_statuses.insert(id.clone(), status);
        let composed_status = self.recompute_window_state(&id).unwrap_or(status);
        let should_auto_close = !pending_auto_recovery
            && should_auto_close_agent_window(&self.active_agent_sessions, &id, &composed_status)
            && self.window_hook_states.get(&id).copied() == Some(WindowProcessStatus::Stopped);
        match detail.as_ref() {
            Some(detail) if !detail.is_empty() => {
                self.window_details.insert(id.clone(), detail.clone());
            }
            _ => {
                self.window_details.remove(&id);
            }
        }
        if exact_resume_rejected {
            // The failed provider attempt may be an ephemeral Intake. Replace
            // it before the normal terminal path can classify and delete its
            // clean worktree. No mapping means no active recovery owner, in
            // which case the historical diagnostic-only path remains intact.
            self.codex_bridge_routes.remove(&id);
            if let Some(mut fallback_events) = self.fallback_after_exact_resume_rejection(&id) {
                if fallback_events.is_empty() {
                    let _ = self.persist();
                    fallback_events.extend(Self::status_events(
                        id.clone(),
                        composed_status,
                        detail,
                    ));
                    fallback_events.extend(self.launch_next_startup_auto_resume_session());
                }
                return fallback_events;
            }
        }
        if should_auto_close {
            self.clear_agent_window_startup_restore(&id);
            self.stop_window_runtime(&id);
            self.remove_window_state_tracking(&id);
            // SPEC-3214 FR-002: `stop_window_runtime` above killed and joined
            // the PTY, so a pending intake worktree can be destroyed now.
            let cleanup_events = self.take_ephemeral_worktree_cleanup_events();
            if !close_window_from_workspace(
                &mut self.tabs,
                &mut self.window_lookup,
                &mut self.window_details,
                &id,
            ) {
                return cleanup_events;
            }
            let _ = self.persist();
            let mut events = cleanup_events;
            self.push_workspace_and_active_work_projection_broadcasts(&mut events);
            return events;
        }
        if keep_active_agent_session_for_recovery {
            self.recoverable_agent_error_windows.insert(id.clone());
        } else if status != WindowProcessStatus::Error {
            self.recoverable_agent_error_windows.remove(&id);
        }
        let mut advance_startup_queue = false;
        if matches!(
            status,
            WindowProcessStatus::Error | WindowProcessStatus::Stopped
        ) && (!keep_active_agent_session_for_recovery || pending_auto_recovery)
        {
            if pending_auto_recovery {
                self.mark_pending_auto_resume_attention(
                    &id,
                    Self::recovery_provider_stopped_attention_reason(),
                );
                self.codex_bridge_routes.remove(&id);
                self.preserve_failed_auto_resume_attempt(&id);
                advance_startup_queue = true;
            } else {
                self.runtimes.remove(&id);
                self.codex_bridge_routes.remove(&id);
                self.remove_window_state_tracking(&id);
                self.mark_agent_session_stopped(&id);
            }
        }
        let _ = self.persist();

        // SPEC-3214 FR-002: a Stopped/Error status means the PTY process has
        // exited — drain any intake worktree cleanup queued by the session
        // stop above (or by an earlier explicit stop of this window).
        let mut events = self.take_ephemeral_worktree_cleanup_events();
        if advance_startup_queue {
            events.extend(self.launch_next_startup_auto_resume_session());
        }
        if is_agent_window
            && composed_status == WindowProcessStatus::Error
            && !keep_active_agent_session_for_recovery
        {
            let message = detail
                .as_deref()
                .unwrap_or("Agent entered error state")
                .to_string();
            events.extend(self.issue_monitor_agent_failed_events(&id, &message));
        }
        if matches!(
            status,
            WindowProcessStatus::Error | WindowProcessStatus::Stopped
        ) {
            if let Some(event) = self.active_work_projection_broadcast_for_active_tab() {
                events.push(event);
            }
        }
        events.extend(Self::status_events(id, composed_status, detail));
        events
    }

    /// The trailing non-empty lines of a window's live vt100 screen, joined
    /// into one detail-sized string. `None` when the window has no runtime
    /// (already torn down) or the screen is blank (Issue #3274).
    fn final_screen_tail(&self, id: &str) -> Option<String> {
        let runtime = self.runtimes.get(id)?;
        let pane = runtime.pane.lock().ok()?;
        let contents = pane.screen().contents();
        let lines: Vec<&str> = contents
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect();
        let start = lines.len().saturating_sub(AGENT_ERROR_TAIL_LINES);
        let tail = lines[start..].join(" ");
        (!tail.is_empty()).then_some(tail)
    }

    fn pending_auto_resume_exact_root(&self, window_id: &str) -> Option<String> {
        let source_session_id = self.pending_auto_resume_sources.get(window_id)?;
        let source_path = self.sessions_dir.join(format!("{source_session_id}.toml"));
        let source = gwt_agent::Session::load_and_migrate(&source_path).ok()?;
        source.exact_resume_session_id().map(str::to_string)
    }

    pub(crate) fn handle_runtime_hook_event(
        &mut self,
        event: gwt::RuntimeHookEvent,
    ) -> Vec<OutboundEvent> {
        self.handle_runtime_hook_event_inner(event, true)
    }

    pub(crate) fn handle_daemon_runtime_hook_event(
        &mut self,
        event: gwt::RuntimeHookEvent,
    ) -> Vec<OutboundEvent> {
        self.handle_runtime_hook_event_inner(event, false)
    }

    fn handle_runtime_hook_event_inner(
        &mut self,
        event: gwt::RuntimeHookEvent,
        publish_to_daemon: bool,
    ) -> Vec<OutboundEvent> {
        if publish_to_daemon {
            if let Some(project_root) = event.project_root.as_deref().map(PathBuf::from) {
                publish_runtime_hook_change(&project_root, &event);
            }
        }
        let mut events = Vec::new();
        if Self::should_broadcast_runtime_hook_event_to_frontend(&event) {
            events.push(OutboundEvent::broadcast(BackendEvent::RuntimeHookEvent {
                event: event.clone(),
            }));
        }
        let Some(window_id) = self.active_window_for_runtime_event(&event) else {
            return events;
        };
        let is_agent_window = self.window_preset(&window_id) == Some(WindowPreset::Agent);
        let Some(hook_state) = gwt::window_state::runtime_hook_window_state(&event) else {
            return events;
        };
        let codex_bridge_waiting_for_root = self
            .codex_bridge_routes
            .get(&window_id)
            .is_some_and(|route| !route.root_forwarded());
        if gwt::window_state::is_live_agent_hook_state(hook_state) && !codex_bridge_waiting_for_root
        {
            let ready_session_id = self
                .active_agent_sessions
                .get(&window_id)
                .map(|session| session.session_id.clone());
            if let Some(ready_session_id) = ready_session_id {
                let provider_root_claim =
                    self.pending_provider_root_claims.get(&window_id).cloned();
                match mark_recovery_ready_for_session(
                    &self.sessions_dir,
                    &ready_session_id,
                    provider_root_claim
                        .as_ref()
                        .map(|claim| claim.project_dir.as_path()),
                    provider_root_claim.as_ref(),
                ) {
                    Ok(()) => {
                        self.pending_provider_root_claims.remove(&window_id);
                        if let Some(source_session_id) =
                            self.pending_auto_resume_sources.get(&window_id).cloned()
                        {
                            match mark_auto_resume_source_completed(
                                &self.sessions_dir,
                                &source_session_id,
                            ) {
                                Ok(()) => {
                                    self.pending_auto_resume_sources.remove(&window_id);
                                    events.extend(self.launch_next_startup_auto_resume_session());
                                }
                                Err(error) => {
                                    tracing::warn!(
                                        source_session_id,
                                        error = %error,
                                        "provider ready but source finalization failed; keeping recovery active"
                                    );
                                    events.extend(self.launch_next_startup_auto_resume_session());
                                }
                            }
                        }
                    }
                    Err(error) => {
                        tracing::warn!(
                            session_id = %ready_session_id,
                            error = %error,
                            "provider ready barrier failed; keeping recovery source active"
                        );
                        self.abort_pending_provider_root_claim_attempt(
                            &window_id,
                            "Provider Ready claim barrier rejected the recovery launch",
                        );
                        events.extend(self.launch_next_startup_auto_resume_session());
                        return events;
                    }
                }
            }
        }
        self.recoverable_agent_error_windows.remove(&window_id);
        if self.window_hook_states.get(&window_id).copied() == Some(hook_state) {
            return events;
        }
        self.window_hook_states
            .insert(window_id.clone(), hook_state);
        let Some(composed_state) = self.recompute_window_state(&window_id) else {
            return events;
        };
        let hook_detail = event
            .message
            .as_deref()
            .map(str::trim)
            .filter(|message| !message.is_empty())
            .map(str::to_string);
        let should_auto_close = should_auto_close_agent_window(
            &self.active_agent_sessions,
            &window_id,
            &composed_state,
        );
        if should_auto_close {
            self.clear_agent_window_startup_restore(&window_id);
            self.stop_window_runtime(&window_id);
            self.remove_window_state_tracking(&window_id);
            // SPEC-3214 FR-002: PTY killed and joined above — safe to destroy
            // a pending intake worktree.
            events.extend(self.take_ephemeral_worktree_cleanup_events());
            if close_window_from_workspace(
                &mut self.tabs,
                &mut self.window_lookup,
                &mut self.window_details,
                &window_id,
            ) {
                let _ = self.persist();
                self.push_workspace_and_active_work_projection_broadcasts(&mut events);
            }
            return events;
        }
        if gwt::window_state::is_live_agent_hook_state(hook_state) {
            self.window_details.remove(&window_id);
        } else if let Some(detail) = hook_detail.as_ref() {
            self.window_details
                .insert(window_id.clone(), detail.clone());
        }
        let detail = hook_detail.or_else(|| self.window_details.get(&window_id).cloned());
        let _ = self.persist();
        if is_agent_window && composed_state == WindowProcessStatus::Error {
            let message = detail
                .as_deref()
                .unwrap_or("Agent entered error state")
                .to_string();
            events.extend(self.issue_monitor_agent_failed_events(&window_id, &message));
        }
        if matches!(
            composed_state,
            WindowProcessStatus::Error | WindowProcessStatus::Stopped
        ) {
            if let Some(event) = self.active_work_projection_broadcast_for_active_tab() {
                events.push(event);
            }
        }
        events.extend(Self::status_events(window_id, composed_state, detail));
        events
    }

    pub(crate) fn handle_codex_bridge_ready(
        &mut self,
        window_id: String,
        session_id: String,
    ) -> Vec<OutboundEvent> {
        let mut events = Vec::new();
        let route_is_ready = self
            .codex_bridge_routes
            .get(&window_id)
            .is_some_and(gwt::codex_bridge::CodexLaunchBridgeLease::root_forwarded);
        let session_matches = self
            .active_agent_sessions
            .get(&window_id)
            .is_some_and(|session| session.session_id == session_id);
        if !route_is_ready || !session_matches {
            return events;
        }

        let provider_root_claim = self.pending_provider_root_claims.get(&window_id).cloned();
        match mark_recovery_ready_for_session(
            &self.sessions_dir,
            &session_id,
            provider_root_claim
                .as_ref()
                .map(|claim| claim.project_dir.as_path()),
            provider_root_claim.as_ref(),
        ) {
            Ok(()) => {
                self.pending_provider_root_claims.remove(&window_id);
                if let Some(source_session_id) =
                    self.pending_auto_resume_sources.get(&window_id).cloned()
                {
                    match mark_auto_resume_source_completed(&self.sessions_dir, &source_session_id)
                    {
                        Ok(()) => {
                            self.pending_auto_resume_sources.remove(&window_id);
                            events.extend(self.launch_next_startup_auto_resume_session());
                        }
                        Err(error) => {
                            tracing::warn!(
                                source_session_id,
                                error = %error,
                                "Codex bridge ready but source finalization failed; keeping recovery active"
                            );
                            events.extend(self.launch_next_startup_auto_resume_session());
                        }
                    }
                }
            }
            Err(error) => {
                tracing::warn!(
                    session_id = %session_id,
                    error = %error,
                    "Codex bridge ready barrier failed; keeping recovery source active"
                );
                self.abort_pending_provider_root_claim_attempt(
                    &window_id,
                    "Provider Ready claim barrier rejected the recovery launch",
                );
                events.extend(self.launch_next_startup_auto_resume_session());
            }
        }
        events
    }

    pub(crate) fn handle_codex_bridge_failure(
        &mut self,
        window_id: String,
        session_id: String,
        failure: gwt::codex_bridge::CodexBridgeFailure,
    ) -> Vec<OutboundEvent> {
        let session_matches = self
            .active_agent_sessions
            .get(&window_id)
            .is_some_and(|session| session.session_id == session_id);
        if !session_matches || !self.codex_bridge_routes.contains_key(&window_id) {
            return Vec::new();
        }

        // Dropping the lease cancels the app-server before any replacement is
        // launched. The bearer capability and one-time route are retired with
        // it, so a fallback can never attach to the rejected provider.
        self.codex_bridge_routes.remove(&window_id);
        if failure.kind == gwt::codex_bridge::CodexBridgeFailureKind::DefinitiveThreadNotFound {
            if let Some(mut events) = self.fallback_after_exact_resume_rejection(&window_id) {
                if events.is_empty() {
                    let _ = self.persist();
                    events.extend(self.launch_next_startup_auto_resume_session());
                }
                return events;
            }
        }

        // Authentication, schema/protocol and transport failures are not
        // evidence that provider history is gone. Keep the source recovery and
        // require operator attention instead of silently creating a new root.
        if self.mark_pending_auto_resume_attention(&window_id, &failure.reason) {
            self.preserve_failed_auto_resume_attempt(&window_id);
            self.window_pty_statuses
                .insert(window_id.clone(), WindowProcessStatus::Error);
            self.window_details
                .insert(window_id.clone(), failure.reason.clone());
            let status = self
                .recompute_window_state(&window_id)
                .unwrap_or(WindowProcessStatus::Error);
            let _ = self.persist();
            let mut events = Self::status_events(window_id, status, Some(failure.reason));
            events.extend(self.launch_next_startup_auto_resume_session());
            return events;
        }

        self.window_details
            .insert(window_id.clone(), failure.reason.clone());
        Self::status_events(window_id, WindowProcessStatus::Error, Some(failure.reason))
    }

    fn should_keep_active_agent_session_for_recoverable_pty_error(
        &self,
        window_id: &str,
        status: WindowProcessStatus,
    ) -> bool {
        status == WindowProcessStatus::Error
            && self.active_agent_sessions.contains_key(window_id)
            && self
                .window_preset(window_id)
                .is_some_and(gwt::window_state::uses_agent_hook_state)
            && (self
                .window_hook_states
                .get(window_id)
                .is_some_and(|state| gwt::window_state::is_live_agent_hook_state(*state))
                || self.recoverable_agent_error_windows.contains(window_id))
    }

    fn should_broadcast_runtime_hook_event_to_frontend(event: &gwt::RuntimeHookEvent) -> bool {
        event.kind != gwt::RuntimeHookEventKind::RuntimeState
    }
}

#[cfg(unix)]
const RUNTIME_DAEMON_PUBLISH_QUEUE_CAPACITY: usize = 4096;

#[cfg(unix)]
enum RuntimeDaemonPublish {
    Output {
        project_root: PathBuf,
        id: String,
        data: Vec<u8>,
    },
    Status {
        project_root: PathBuf,
        id: String,
        status: WindowProcessStatus,
        detail: Option<String>,
    },
    Hook {
        project_root: PathBuf,
        event: gwt::RuntimeHookEvent,
    },
}

#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeDaemonPublishEnqueueError {
    Full,
    Disconnected,
}

#[cfg(unix)]
static RUNTIME_DAEMON_PUBLISH_QUEUE: std::sync::OnceLock<
    Mutex<Option<std_mpsc::SyncSender<RuntimeDaemonPublish>>>,
> = std::sync::OnceLock::new();

#[cfg(unix)]
fn runtime_daemon_publish_sender() -> Option<std_mpsc::SyncSender<RuntimeDaemonPublish>> {
    let queue = RUNTIME_DAEMON_PUBLISH_QUEUE.get_or_init(|| Mutex::new(None));
    runtime_daemon_publish_sender_from(queue, |receiver| {
        std::thread::Builder::new()
            .name("gwt-runtime-daemon-publish-worker".to_string())
            .spawn(move || run_runtime_daemon_publish_worker(receiver))
            .map(|_handle| ())
    })
}

#[cfg(unix)]
fn runtime_daemon_publish_sender_from(
    queue: &Mutex<Option<std_mpsc::SyncSender<RuntimeDaemonPublish>>>,
    spawn_worker: impl FnOnce(std_mpsc::Receiver<RuntimeDaemonPublish>) -> std::io::Result<()>,
) -> Option<std_mpsc::SyncSender<RuntimeDaemonPublish>> {
    let Ok(mut queue) = queue.lock() else {
        tracing::debug!("runtime daemon publish queue lock poisoned");
        return None;
    };
    if let Some(sender) = queue.as_ref() {
        return Some(sender.clone());
    }

    let (sender, receiver) = std_mpsc::sync_channel(RUNTIME_DAEMON_PUBLISH_QUEUE_CAPACITY);
    match spawn_worker(receiver) {
        Ok(()) => {
            *queue = Some(sender.clone());
            Some(sender)
        }
        Err(err) => {
            tracing::debug!(error = %err, "runtime daemon publish worker spawn failed");
            None
        }
    }
}

#[cfg(unix)]
fn run_runtime_daemon_publish_worker(receiver: std_mpsc::Receiver<RuntimeDaemonPublish>) {
    for publish in receiver {
        publish_runtime_daemon_event(publish);
    }
}

#[cfg(unix)]
fn try_enqueue_runtime_daemon_publish(
    sender: &std_mpsc::SyncSender<RuntimeDaemonPublish>,
    publish: RuntimeDaemonPublish,
) -> Result<(), RuntimeDaemonPublishEnqueueError> {
    sender.try_send(publish).map_err(|err| match err {
        std_mpsc::TrySendError::Full(_) => RuntimeDaemonPublishEnqueueError::Full,
        std_mpsc::TrySendError::Disconnected(_) => RuntimeDaemonPublishEnqueueError::Disconnected,
    })
}

#[cfg(unix)]
fn enqueue_runtime_daemon_publish(publish: RuntimeDaemonPublish) {
    let Some(sender) = runtime_daemon_publish_sender() else {
        return;
    };
    if let Err(err) = try_enqueue_runtime_daemon_publish(&sender, publish) {
        tracing::debug!(
            ?err,
            "runtime daemon publish queue rejected event (non-fatal)"
        );
    }
}

#[cfg(unix)]
fn publish_runtime_daemon_event(publish: RuntimeDaemonPublish) {
    match publish {
        RuntimeDaemonPublish::Output {
            project_root,
            id,
            data,
        } => {
            let payload =
                gwt::runtime_daemon_events::runtime_output_payload(&id, &data, std::process::id());
            let result = gwt::daemon_publisher::publish_event(
                &project_root,
                gwt::runtime_daemon_events::RUNTIME_OUTPUT_CHANNEL,
                payload,
            );
            if let Err(err) = result {
                tracing::debug!(
                    error = %err,
                    project_root = %project_root.display(),
                    window_id = %id,
                    "runtime output daemon publish failed (non-fatal)"
                );
            }
        }
        RuntimeDaemonPublish::Status {
            project_root,
            id,
            status,
            detail,
        } => {
            let payload = gwt::runtime_daemon_events::runtime_status_payload(
                &id,
                status,
                detail,
                std::process::id(),
            );
            let result = gwt::daemon_publisher::publish_event(
                &project_root,
                gwt::runtime_daemon_events::RUNTIME_STATUS_CHANNEL,
                payload,
            );
            if let Err(err) = result {
                tracing::debug!(
                    error = %err,
                    project_root = %project_root.display(),
                    window_id = %id,
                    "runtime status daemon publish failed (non-fatal)"
                );
            }
        }
        RuntimeDaemonPublish::Hook {
            project_root,
            event,
        } => {
            let payload =
                gwt::runtime_daemon_events::runtime_hook_payload(&event, std::process::id());
            let result = gwt::daemon_publisher::publish_event(
                &project_root,
                gwt::runtime_daemon_events::RUNTIME_HOOK_CHANNEL,
                payload,
            );
            if let Err(err) = result {
                tracing::debug!(
                    error = %err,
                    project_root = %project_root.display(),
                    "runtime hook daemon publish failed (non-fatal)"
                );
            }
        }
    }
}

#[cfg(unix)]
fn publish_runtime_output_change(project_root: &Path, id: &str, data: &[u8]) {
    enqueue_runtime_daemon_publish(RuntimeDaemonPublish::Output {
        project_root: project_root.to_path_buf(),
        id: id.to_string(),
        data: data.to_vec(),
    });
}

#[cfg(not(unix))]
fn publish_runtime_output_change(_project_root: &Path, _id: &str, _data: &[u8]) {}

#[cfg(unix)]
fn publish_runtime_status_change(
    project_root: &Path,
    id: &str,
    status: WindowProcessStatus,
    detail: Option<String>,
) {
    enqueue_runtime_daemon_publish(RuntimeDaemonPublish::Status {
        project_root: project_root.to_path_buf(),
        id: id.to_string(),
        status,
        detail,
    });
}

#[cfg(not(unix))]
fn publish_runtime_status_change(
    _project_root: &Path,
    _id: &str,
    _status: WindowProcessStatus,
    _detail: Option<String>,
) {
}

#[cfg(unix)]
fn publish_runtime_hook_change(project_root: &Path, event: &gwt::RuntimeHookEvent) {
    enqueue_runtime_daemon_publish(RuntimeDaemonPublish::Hook {
        project_root: project_root.to_path_buf(),
        event: event.clone(),
    });
}

#[cfg(not(unix))]
fn publish_runtime_hook_change(_project_root: &Path, _event: &gwt::RuntimeHookEvent) {}

#[cfg(test)]
mod recovery_ready_tests {
    use super::{mark_recovery_ready_for_session, PendingProviderRootClaim};

    #[test]
    fn provider_ready_barrier_updates_recovery_before_source_retirement() {
        let temp = tempfile::tempdir().expect("tempdir");
        let sessions_dir = temp.path().join("sessions");
        let project_dir = temp.path().join("project-store");
        let worktree = temp.path().join("intake");
        std::fs::create_dir_all(&worktree).expect("worktree");
        let mut session = gwt_agent::Session::new(&worktree, "work", gwt_agent::AgentId::Codex);
        session.project_state_root = Some(worktree.clone());
        let session_id = session.id.clone();
        let recovery_id = session.recovery_id.clone().expect("recovery id");
        session.session_mode = gwt_agent::SessionMode::Resume;
        session.recovery_continuation = Some(gwt_agent::RecoveryContinuationHandoff {
            source_session_id: session_id.clone(),
            source_recovery_id: recovery_id.clone(),
            target_recovery_id: recovery_id.clone(),
            source_checkpoint_revision: 0,
            reason: "Recovery Center requested exact provider resume".to_string(),
            inherit_checkpoint: false,
        });
        session.save(&sessions_dir).expect("save session");
        let store = gwt_core::recovery::RecoveryStore::for_project_dir(&project_dir);
        store
            .create(
                gwt_core::recovery::CreateRecovery {
                    recovery_id: recovery_id.clone(),
                    session_id: session_id.clone(),
                    repo_id: "repo".to_string(),
                    session_kind: gwt_core::recovery::RecoverySessionKind::Intake,
                    worktree_path: worktree,
                    launch_base_ref: None,
                    launch_base_oid: "1111111111111111111111111111111111111111".to_string(),
                    launch_head_oid: "1111111111111111111111111111111111111111".to_string(),
                    provider: "codex".to_string(),
                    model: None,
                    runtime: "host".to_string(),
                    initial_prompt: "Investigate".to_string(),
                    created_at: session.created_at,
                },
                "create",
            )
            .expect("create recovery");

        store
            .bind_root(
                &recovery_id,
                gwt_core::recovery::ProviderRootBinding {
                    root_id: "ready-provider-root".to_string(),
                    session_tree_id: None,
                    quality: gwt_core::recovery::BindingQuality::Verified,
                    bound_at: chrono::Utc::now(),
                },
                "bind-ready-root",
            )
            .expect("bind ready root");
        let before_claim = store.load(&recovery_id).unwrap().unwrap();
        let acquired_at = chrono::Utc::now();
        store
            .claim_recovery_with_provider_root(
                &recovery_id,
                before_claim.generation,
                "ready-provider-root",
                false,
                gwt_core::recovery::RecoveryLease {
                    lease_id: "ready-claim-token".to_string(),
                    holder_id: "runtime-ready-test".to_string(),
                    acquired_at,
                    expires_at: acquired_at + chrono::Duration::minutes(5),
                },
                "pending-ready-window",
                "ready integration test",
                "claim-ready-root",
            )
            .expect("claim ready root");

        mark_recovery_ready_for_session(
            &sessions_dir,
            &session_id,
            Some(&project_dir),
            Some(&PendingProviderRootClaim {
                recovery_id: recovery_id.clone(),
                claim_token: "ready-claim-token".to_string(),
                project_dir: project_dir.clone(),
                claim_ttl: chrono::Duration::minutes(5),
            }),
        )
        .expect("ready barrier");

        assert_eq!(
            store.load(&recovery_id).unwrap().unwrap().lifecycle,
            gwt_core::recovery::RecoveryLifecycle::Running
        );
        assert!(store
            .active_provider_root_claim("codex", "ready-provider-root", chrono::Utc::now())
            .unwrap()
            .is_none());
        let loaded =
            gwt_agent::Session::load(&sessions_dir.join(format!("{session_id}.toml"))).unwrap();
        assert_eq!(
            loaded.recovery_launch_stage,
            Some(gwt_agent::session::RecoveryLaunchStage::Ready)
        );
    }

    #[test]
    fn exact_ready_without_in_memory_claim_mapping_fails_closed() {
        let temp = tempfile::tempdir().expect("tempdir");
        let sessions_dir = temp.path().join("sessions");
        let project_dir = temp.path().join("project-store");
        let worktree = temp.path().join("intake");
        std::fs::create_dir_all(&worktree).expect("worktree");
        let mut session = gwt_agent::Session::new(&worktree, "work", gwt_agent::AgentId::Codex);
        session.project_state_root = Some(worktree.clone());
        session.session_mode = gwt_agent::SessionMode::Resume;
        let session_id = session.id.clone();
        let recovery_id = session.recovery_id.clone().expect("recovery id");
        session.recovery_continuation = Some(gwt_agent::RecoveryContinuationHandoff {
            source_session_id: "source-session".to_string(),
            source_recovery_id: "source-recovery".to_string(),
            target_recovery_id: recovery_id.clone(),
            source_checkpoint_revision: 0,
            reason: "Startup requested exact provider resume".to_string(),
            inherit_checkpoint: false,
        });
        session.save(&sessions_dir).expect("save exact session");
        let store = gwt_core::recovery::RecoveryStore::for_project_dir(&project_dir);
        store
            .create(
                gwt_core::recovery::CreateRecovery {
                    recovery_id: recovery_id.clone(),
                    session_id: session_id.clone(),
                    repo_id: "repo".to_string(),
                    session_kind: gwt_core::recovery::RecoverySessionKind::Intake,
                    worktree_path: worktree,
                    launch_base_ref: None,
                    launch_base_oid: "1".repeat(40),
                    launch_head_oid: "1".repeat(40),
                    provider: "codex".to_string(),
                    model: None,
                    runtime: "host".to_string(),
                    initial_prompt: "Investigate".to_string(),
                    created_at: session.created_at,
                },
                "create-missing-ready-claim",
            )
            .expect("create recovery");

        let error =
            mark_recovery_ready_for_session(&sessions_dir, &session_id, Some(&project_dir), None)
                .expect_err("exact Ready must not bypass a lost in-memory claim mapping");

        assert!(error.to_string().contains("provider-root claim"));
        let recovery = store.load(&recovery_id).unwrap().unwrap();
        assert!(recovery.launch_stage < gwt_core::recovery::RecoveryLaunchStage::Ready);
        assert_ne!(
            recovery.lifecycle,
            gwt_core::recovery::RecoveryLifecycle::Running
        );
        let session =
            gwt_agent::Session::load(&sessions_dir.join(format!("{session_id}.toml"))).unwrap();
        assert_ne!(
            session.recovery_launch_stage,
            Some(gwt_agent::session::RecoveryLaunchStage::Ready)
        );
    }

    #[test]
    fn fresh_ready_without_provider_root_claim_uses_the_normal_barrier() {
        let temp = tempfile::tempdir().expect("tempdir");
        let sessions_dir = temp.path().join("sessions");
        let project_dir = temp.path().join("project-store");
        let worktree = temp.path().join("intake");
        std::fs::create_dir_all(&worktree).expect("worktree");
        let mut session = gwt_agent::Session::new(&worktree, "work", gwt_agent::AgentId::Codex);
        session.project_state_root = Some(worktree.clone());
        let session_id = session.id.clone();
        let recovery_id = session.recovery_id.clone().expect("recovery id");
        session.save(&sessions_dir).expect("save fresh session");
        let store = gwt_core::recovery::RecoveryStore::for_project_dir(&project_dir);
        store
            .create(
                gwt_core::recovery::CreateRecovery {
                    recovery_id: recovery_id.clone(),
                    session_id: session_id.clone(),
                    repo_id: "repo".to_string(),
                    session_kind: gwt_core::recovery::RecoverySessionKind::Intake,
                    worktree_path: worktree,
                    launch_base_ref: None,
                    launch_base_oid: "1".repeat(40),
                    launch_head_oid: "1".repeat(40),
                    provider: "codex".to_string(),
                    model: None,
                    runtime: "host".to_string(),
                    initial_prompt: "Investigate".to_string(),
                    created_at: session.created_at,
                },
                "create-fresh-ready",
            )
            .expect("create recovery");

        mark_recovery_ready_for_session(&sessions_dir, &session_id, Some(&project_dir), None)
            .expect("fresh Ready");

        let recovery = store.load(&recovery_id).unwrap().unwrap();
        assert_eq!(
            recovery.lifecycle,
            gwt_core::recovery::RecoveryLifecycle::Running
        );
        assert_eq!(
            recovery.launch_stage,
            gwt_core::recovery::RecoveryLaunchStage::Ready
        );
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::path::PathBuf;

    #[cfg(unix)]
    use std::sync::{mpsc, Mutex};

    #[cfg(unix)]
    use super::{
        runtime_daemon_publish_sender_from, try_enqueue_runtime_daemon_publish,
        RuntimeDaemonPublish, RuntimeDaemonPublishEnqueueError,
    };
    #[cfg(unix)]
    use crate::WindowProcessStatus;

    #[cfg(unix)]
    #[test]
    fn runtime_daemon_publish_enqueue_is_bounded_and_nonblocking() {
        let (sender, _receiver) = mpsc::sync_channel(1);
        let project_root = PathBuf::from("/tmp/gwt-project");

        assert!(try_enqueue_runtime_daemon_publish(
            &sender,
            RuntimeDaemonPublish::Output {
                project_root: project_root.clone(),
                id: "tab-1::shell-1".to_string(),
                data: b"first".to_vec(),
            },
        )
        .is_ok());
        assert!(matches!(
            try_enqueue_runtime_daemon_publish(
                &sender,
                RuntimeDaemonPublish::Status {
                    project_root,
                    id: "tab-1::shell-1".to_string(),
                    status: WindowProcessStatus::Running,
                    detail: None,
                },
            ),
            Err(RuntimeDaemonPublishEnqueueError::Full)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn runtime_daemon_publish_sender_retries_after_spawn_failure() {
        let queue = Mutex::new(None);

        assert!(runtime_daemon_publish_sender_from(&queue, |_receiver| {
            Err(std::io::Error::other("spawn failed"))
        })
        .is_none());
        assert!(queue.lock().expect("queue").is_none());

        assert!(runtime_daemon_publish_sender_from(&queue, |_receiver| Ok(())).is_some());
        assert!(queue.lock().expect("queue").is_some());

        assert!(runtime_daemon_publish_sender_from(&queue, |_receiver| {
            panic!("sender should be reused without spawning a second worker")
        })
        .is_some());
    }
}
