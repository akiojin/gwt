//! Runtime event handlers and daemon publish queue for SPEC-2077 Phase J1.
//!
//! This module owns the already daemon-aware runtime output/status/hook
//! publish path. The extraction is behavior-preserving: it keeps best-effort
//! daemon publish, same-process echo suppression through the existing payload
//! layer, and the local GUI state update path unchanged.

use base64::Engine as _;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

#[cfg(unix)]
use std::sync::mpsc as std_mpsc;
#[cfg(unix)]
use std::sync::Mutex;

use super::{
    close_window_from_workspace, should_auto_close_agent_window, AppRuntime, BackendEvent,
    OutboundEvent, WindowPreset, WindowProcessStatus,
};

/// Issue #3274: how many trailing non-empty screen lines survive into the
/// persistent window detail when an agent process errors out.
const AGENT_ERROR_TAIL_LINES: usize = 3;
const AGENT_ERROR_TAIL_MAX_CHARS: usize = 240;
/// Issue #3616: how many trailing screen lines are searched for a provider
/// quota notice. Wider than the error tail because a CLI can print a prompt or
/// blank frame after the notice, and the notice itself soft-wraps.
const QUOTA_NOTICE_TAIL_LINES: usize = 8;
/// Provider TUIs commonly clear and redraw the approval block after Enter.
/// This bounded settle window avoids a false Running frame between the clear
/// and the redraw without ever blocking the tao event loop.
const APPROVAL_SETTLE_DELAY: Duration = Duration::from_millis(100);
/// Claude Code's exact-resume failure line. When a resumed conversation no
/// longer exists in the agent's store, this is the only explanation the user
/// ever gets — promote it to an explicit diagnostic (SPEC-1921 exact session
/// restore amendment: stale provider ids keep a visible diagnostic).
const EXACT_RESUME_FAILURE_SIGNATURE: &str = "No conversation found with session ID";
const PROVIDER_ERROR_PREFIX: &str = "Error: ";
const RESUME_WRITER_CONFLICT_OUTER_PREFIX: &str = "Failed to resume session from ";
const RESUME_WRITER_CONFLICT_PREFIX: &str = "thread/resume failed during TUI bootstrap:";
const RESUME_WRITER_CONFLICT_SUFFIX: &str = "already has an active writer";
const RESUME_WRITER_CONFLICT_CODE: &str = "(code -32600)";

fn marker_is_inside_double_quotes(line: &str, marker_offset: usize) -> bool {
    let mut quoted = false;
    let mut escaped = false;
    for byte in line[..marker_offset].bytes() {
        if escaped {
            escaped = false;
            continue;
        }
        match byte {
            b'\\' => escaped = true,
            b'"' => quoted = !quoted,
            _ => {}
        }
    }
    quoted
}

fn resume_writer_conflict_outer_offset(line: &str) -> Option<usize> {
    let anchored_provider_output = |offset: usize| {
        let output = &line[offset..];
        if output.starts_with(RESUME_WRITER_CONFLICT_OUTER_PREFIX) {
            Some(offset)
        } else {
            output
                .strip_prefix(PROVIDER_ERROR_PREFIX)
                .filter(|detail| detail.starts_with(RESUME_WRITER_CONFLICT_OUTER_PREFIX))
                .map(|_| offset + PROVIDER_ERROR_PREFIX.len())
        }
    };

    let trimmed = line.trim_start();
    let leading_whitespace = line.len() - trimmed.len();
    anchored_provider_output(leading_whitespace).or_else(|| {
        let last_output = " — last output: ";
        let output_offset = line.find(last_output)? + last_output.len();
        anchored_provider_output(output_offset)
    })
}

/// Classify a typed failure out of an agent's exit detail.
///
/// Two causes are typed today, and they are checked in this order because they
/// answer different questions. A provider quota block is a property of the
/// account and applies to any session mode, so it is tested first; the
/// late-resume writer race can only happen while resuming.
pub(super) fn classify_issue_monitor_failure(
    detail: &str,
    session_mode: gwt_agent::SessionMode,
) -> Option<gwt::IssueMonitorFailure> {
    if let Some(failure) = classify_provider_usage_limit(detail) {
        return Some(failure);
    }
    classify_resume_writer_conflict(detail, session_mode)
}

/// Issue #3616: the provider stated it is out of quota.
///
/// Detection lives in `gwt_core::usage` because the message formats and their
/// reset clauses are provider domain knowledge, not runtime-event knowledge.
/// `Local::now()` is the reference zone on purpose: both providers print a
/// local wall clock with no offset, so the machine running the agent is the
/// only anchor available.
pub(super) fn classify_provider_usage_limit(detail: &str) -> Option<gwt::IssueMonitorFailure> {
    let notice = gwt_core::usage::detect_provider_limit_notice(detail, &chrono::Local::now())?;
    Some(provider_usage_limit_failure(&notice))
}

pub(super) fn provider_usage_limit_failure(
    notice: &gwt_core::usage::ProviderLimitNotice,
) -> gwt::IssueMonitorFailure {
    gwt::IssueMonitorFailure::ProviderUsageLimit {
        provider: match notice.provider {
            gwt_core::usage::UsageProvider::Codex => "codex".to_string(),
            gwt_core::usage::UsageProvider::ClaudeCode => "claude".to_string(),
        },
        resets_at: notice
            .resets_at
            .map(|at| at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
    }
}

/// Classify the provider's exact late-resume writer race without promoting
/// unrelated launch failures that happen to mention a writer.
fn classify_resume_writer_conflict(
    detail: &str,
    session_mode: gwt_agent::SessionMode,
) -> Option<gwt::IssueMonitorFailure> {
    if session_mode != gwt_agent::SessionMode::Resume {
        return None;
    }
    detail.lines().find_map(|line| {
        let outer_offset = resume_writer_conflict_outer_offset(line)?;
        let prefix_offset =
            outer_offset + line[outer_offset..].find(RESUME_WRITER_CONFLICT_PREFIX)?;
        if marker_is_inside_double_quotes(line, prefix_offset) {
            return None;
        }
        let after_prefix = prefix_offset + RESUME_WRITER_CONFLICT_PREFIX.len();
        let writer_offset =
            after_prefix + line[after_prefix..].find(RESUME_WRITER_CONFLICT_SUFFIX)?;
        if marker_is_inside_double_quotes(line, writer_offset) {
            return None;
        }
        let after_writer = writer_offset + RESUME_WRITER_CONFLICT_SUFFIX.len();
        let code_offset = after_writer + line[after_writer..].find(RESUME_WRITER_CONFLICT_CODE)?;
        if marker_is_inside_double_quotes(line, code_offset) {
            return None;
        }
        Some(gwt::IssueMonitorFailure::ResumeWriterConflict {
            holder_window_id: None,
        })
    })
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
        return Some(
            "Exact session restore failed. The agent no longer has this conversation; \
             use Continue work to start a new conversation with handoff context."
                .to_string(),
        );
    }
    match base {
        Some(base) if !base.is_empty() => Some(format!("{base} — last output: {tail}")),
        _ => Some(format!("Agent exited — last output: {tail}")),
    }
}

impl AppRuntime {
    /// Issue #3366 — whether any project tab's workspace still holds a
    /// window of the given preset. Docked windows stay in the workspace
    /// window list, so tab groups are covered. The check spans every tab
    /// (not just the active one) because surfaces on an inactive tab keep
    /// their accumulated client state and do not re-request a snapshot
    /// when their tab becomes active again.
    fn any_window_open(&self, preset: WindowPreset) -> bool {
        self.tabs.iter().any(|tab| {
            tab.workspace
                .persisted()
                .windows
                .iter()
                .any(|window| window.preset == preset)
        })
    }

    /// Issue #3366 — deliver one external-process line to the client hub
    /// only while a Console window exists. Raw `process_line` events are
    /// consumed exclusively by Console window controllers, and every
    /// Console mount replays the `ProcessConsoleHub` ring buffer through
    /// `LoadProcessConsole`, so nothing is lost while suppressed.
    /// Unconditional broadcast measured ≈956 msg/s under normal agent
    /// load and delayed a new client's first workspace paint by ~1 min.
    pub(crate) fn process_line_events(
        &self,
        line: gwt_core::process_console::ProcessLine,
    ) -> Vec<OutboundEvent> {
        if !self.any_window_open(WindowPreset::Console) {
            return Vec::new();
        }
        vec![OutboundEvent::broadcast(BackendEvent::ProcessLine { line })]
    }

    /// Issue #3366 — deliver one tracing log event to the client hub only
    /// while a Logs window exists. `log_entry_appended` is consumed
    /// exclusively by Logs window state, and `LoadLogs` re-reads the log
    /// directory on mount, so the live stream is pure overhead without an
    /// open Logs surface.
    pub(crate) fn log_entry_events(
        &self,
        entry: gwt_core::logging::LogEvent,
    ) -> Vec<OutboundEvent> {
        if !self.any_window_open(WindowPreset::Logs) {
            return Vec::new();
        }
        vec![OutboundEvent::broadcast(BackendEvent::LogEntryAppended {
            entry,
        })]
    }

    pub(crate) fn handle_runtime_output(
        &mut self,
        id: String,
        data: Vec<u8>,
    ) -> Vec<OutboundEvent> {
        self.handle_runtime_output_inner(id, data, true)
    }

    pub(crate) fn handle_runtime_output_event(
        &mut self,
        id: String,
        incarnation: u64,
        data: Vec<u8>,
    ) -> Vec<OutboundEvent> {
        if !self.runtime_incarnation_is_current(&id, incarnation) {
            return Vec::new();
        }
        self.handle_runtime_output(id, data)
    }

    pub(crate) fn handle_daemon_runtime_output(
        &mut self,
        id: String,
        data: Vec<u8>,
    ) -> Vec<OutboundEvent> {
        // Daemon publications describe another process and intentionally keep
        // their existing wire contract; a local PTY incarnation is neither
        // available nor authoritative for this path.
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
        // Issue #3475: progress evidence for the authenticated SessionStart
        // readiness deadline. Saturating because the counter only ever needs to
        // answer "did this pane emit anything since the last deadline".
        let observed = self.window_output_bytes.entry(id.clone()).or_insert(0);
        *observed = observed.saturating_add(data.len() as u64);
        if publish_to_daemon {
            if let Some(tab) = self.tab(&address.tab_id) {
                publish_runtime_output_change(&tab.project_root, &id, &data);
            }
        }
        let output_id = id.clone();
        let mut events = vec![OutboundEvent::broadcast(BackendEvent::TerminalOutput {
            id,
            data_base64: base64::engine::general_purpose::STANDARD.encode(data),
        })];
        if publish_to_daemon {
            let prompt = self.current_screen_approval_prompt(&output_id);
            events.extend(self.observe_runtime_approval_prompt(&output_id, prompt));
        }
        events
    }

    fn current_screen_approval_prompt(&self, id: &str) -> Option<u64> {
        let (provider, screen) = self.current_approval_screen(id)?;
        gwt::window_state::approval_prompt_fingerprint(provider, &screen)
    }

    fn current_approval_screen(
        &self,
        id: &str,
    ) -> Option<(gwt::window_state::ApprovalPromptProvider, String)> {
        let provider = self.approval_prompt_provider(id);
        if provider == gwt::window_state::ApprovalPromptProvider::Unsupported {
            return None;
        }
        let runtime = self.runtimes.get(id)?;
        let pane = runtime.pane.lock().ok()?;
        Some((provider, pane.screen().contents()))
    }

    fn approval_prompt_provider(
        &self,
        window_id: &str,
    ) -> gwt::window_state::ApprovalPromptProvider {
        use gwt::window_state::ApprovalPromptProvider;

        match self.window_preset(window_id) {
            Some(WindowPreset::Codex) => return ApprovalPromptProvider::Codex,
            Some(WindowPreset::Claude) => return ApprovalPromptProvider::ClaudeCode,
            Some(WindowPreset::Agent) => {}
            _ => return ApprovalPromptProvider::Unsupported,
        }
        let live_agent = self
            .active_agent_sessions
            .get(window_id)
            .map(|session| session.agent_id.as_str());
        let persisted_agent = self.window_lookup.get(window_id).and_then(|address| {
            self.tab(&address.tab_id)
                .and_then(|tab| tab.workspace.window(&address.raw_id))
                .and_then(|window| window.agent_id.as_deref())
        });
        match live_agent
            .or(persisted_agent)
            .and_then(gwt_agent::resolve_agent_id)
        {
            Some(gwt_agent::AgentId::Codex) => ApprovalPromptProvider::Codex,
            Some(gwt_agent::AgentId::ClaudeCode) => ApprovalPromptProvider::ClaudeCode,
            _ => ApprovalPromptProvider::Unsupported,
        }
    }

    pub(crate) fn observe_runtime_approval_prompt(
        &mut self,
        window_id: &str,
        observed: Option<u64>,
    ) -> Vec<OutboundEvent> {
        let current = self.window_approval_waiting.get(window_id).copied();
        match (current, observed) {
            (None, None) => Vec::new(),
            (None, Some(fingerprint)) => self.set_runtime_approval_latch(
                window_id,
                Some(super::ApprovalPromptLatch {
                    active_fingerprint: Some(fingerprint),
                    resolving_fingerprint: None,
                    resolution_started: false,
                    pending_settle_token: None,
                }),
                true,
                false,
            ),
            (Some(latch), None)
                if latch.resolution_started
                    && (latch.active_fingerprint.is_none()
                        || latch.resolving_fingerprint == latch.active_fingerprint) =>
            {
                if latch.pending_settle_token.is_none() {
                    self.schedule_runtime_approval_settle(window_id);
                }
                Vec::new()
            }
            (Some(_), None) => Vec::new(),
            (Some(mut latch), Some(fingerprint))
                if latch.active_fingerprint.is_none()
                    || latch.active_fingerprint == Some(fingerprint) =>
            {
                latch.active_fingerprint = Some(fingerprint);
                if latch.resolution_started {
                    latch.resolving_fingerprint = None;
                    latch.resolution_started = false;
                    latch.pending_settle_token = None;
                }
                self.window_approval_waiting
                    .insert(window_id.to_string(), latch);
                Vec::new()
            }
            (Some(_), Some(fingerprint)) => {
                let mut events = self.set_runtime_approval_latch(window_id, None, true, true);
                events.extend(self.set_runtime_approval_latch(
                    window_id,
                    Some(super::ApprovalPromptLatch {
                        active_fingerprint: Some(fingerprint),
                        resolving_fingerprint: None,
                        resolution_started: false,
                        pending_settle_token: None,
                    }),
                    true,
                    true,
                ));
                events
            }
        }
    }

    pub(crate) fn begin_runtime_approval_resolution(&mut self, window_id: &str) {
        let Some(latch) = self.window_approval_waiting.get_mut(window_id) else {
            return;
        };
        latch.resolving_fingerprint = latch.active_fingerprint;
        latch.resolution_started = true;
        latch.pending_settle_token = None;
    }

    pub(crate) fn cancel_runtime_approval_resolution(&mut self, window_id: &str) {
        let Some(latch) = self.window_approval_waiting.get_mut(window_id) else {
            return;
        };
        latch.resolving_fingerprint = None;
        latch.resolution_started = false;
        latch.pending_settle_token = None;
    }

    fn schedule_runtime_approval_settle(&mut self, window_id: &str) {
        self.approval_settle_epoch = self.approval_settle_epoch.wrapping_add(1).max(1);
        let token = self.approval_settle_epoch;
        let Some(latch) = self.window_approval_waiting.get_mut(window_id) else {
            return;
        };
        latch.pending_settle_token = Some(token);
        let proxy = self.proxy.clone();
        let id = window_id.to_string();
        thread::spawn(move || {
            thread::sleep(APPROVAL_SETTLE_DELAY);
            proxy.send(super::UserEvent::RuntimeApprovalSettle { id, token });
        });
    }

    pub(crate) fn handle_runtime_approval_settle(
        &mut self,
        window_id: &str,
        token: u64,
    ) -> Vec<OutboundEvent> {
        let Some(latch) = self.window_approval_waiting.get(window_id).copied() else {
            return Vec::new();
        };
        if latch.pending_settle_token != Some(token) || !latch.resolution_started {
            return Vec::new();
        }
        let Some((provider, screen)) = self.current_approval_screen(window_id) else {
            return Vec::new();
        };
        if let Some(fingerprint) = gwt::window_state::approval_prompt_fingerprint(provider, &screen)
        {
            return self.observe_runtime_approval_prompt(window_id, Some(fingerprint));
        }
        if gwt::window_state::has_approval_prompt_evidence(provider, &screen) {
            if let Some(latch) = self.window_approval_waiting.get_mut(window_id) {
                if latch.pending_settle_token == Some(token) {
                    latch.pending_settle_token = None;
                }
            }
            return Vec::new();
        }
        self.set_runtime_approval_latch(window_id, None, true, false)
    }

    #[cfg(test)]
    pub(crate) fn handle_runtime_approval_wait_state(
        &mut self,
        window_id: &str,
        waiting: bool,
    ) -> Vec<OutboundEvent> {
        self.handle_runtime_approval_wait_state_inner(window_id, waiting, true)
    }

    pub(crate) fn handle_daemon_runtime_approval_wait_state(
        &mut self,
        window_id: &str,
        waiting: bool,
    ) -> Vec<OutboundEvent> {
        self.handle_runtime_approval_wait_state_inner(window_id, waiting, false)
    }

    fn handle_runtime_approval_wait_state_inner(
        &mut self,
        window_id: &str,
        waiting: bool,
        publish_to_daemon: bool,
    ) -> Vec<OutboundEvent> {
        let latch = waiting.then_some(super::ApprovalPromptLatch::default());
        self.set_runtime_approval_latch(window_id, latch, publish_to_daemon, false)
    }

    fn set_runtime_approval_latch(
        &mut self,
        window_id: &str,
        latch: Option<super::ApprovalPromptLatch>,
        publish_to_daemon: bool,
        force_status: bool,
    ) -> Vec<OutboundEvent> {
        if !self.tracked_window_exists(window_id) {
            self.window_approval_waiting.remove(window_id);
            return Vec::new();
        }
        let before = self.window_status(window_id);
        let was_waiting = self.window_approval_waiting.contains_key(window_id);
        let waiting = latch.is_some();
        if let Some(latch) = latch {
            self.window_approval_waiting
                .insert(window_id.to_string(), latch);
        } else {
            self.window_approval_waiting.remove(window_id);
        }
        let overlay_changed = was_waiting != waiting;
        if !overlay_changed && !force_status {
            return Vec::new();
        }
        if overlay_changed && publish_to_daemon {
            if let Some(address) = self.window_lookup.get(window_id) {
                if let Some(tab) = self.tab(&address.tab_id) {
                    publish_runtime_approval_overlay_change(&tab.project_root, window_id, waiting);
                }
            }
        }
        let Some(composed) = self.recompute_window_state(window_id) else {
            return Vec::new();
        };
        if force_status || before != Some(composed) {
            Self::status_events(window_id.to_string(), composed, None)
        } else {
            Vec::new()
        }
    }

    pub(crate) fn clear_runtime_approval_latch_without_status(
        &mut self,
        window_id: &str,
        publish_to_daemon: bool,
    ) {
        if self.window_approval_waiting.remove(window_id).is_none() || !publish_to_daemon {
            return;
        }
        if let Some(address) = self.window_lookup.get(window_id) {
            if let Some(tab) = self.tab(&address.tab_id) {
                publish_runtime_approval_overlay_change(&tab.project_root, window_id, false);
            }
        }
    }

    pub(crate) fn handle_runtime_status(
        &mut self,
        id: String,
        status: WindowProcessStatus,
        detail: Option<String>,
    ) -> Vec<OutboundEvent> {
        // Display and transport errors are not child-exit receipts. Callers
        // that observed `try_wait == Some` use `handle_runtime_status_event`
        // with the exact local incarnation and `exit_confirmed = true`.
        self.handle_runtime_status_inner(id, status, detail, true, false)
    }

    #[cfg(test)]
    pub(crate) fn handle_runtime_status_with_exit_confirmation(
        &mut self,
        id: String,
        status: WindowProcessStatus,
        detail: Option<String>,
        exit_confirmed: bool,
    ) -> Vec<OutboundEvent> {
        // Unit fixtures that exercise downstream terminal cleanup do not own a
        // live PTY incarnation. Production callers must use
        // `handle_runtime_status_event`, which also applies the incarnation
        // fence before accepting an exit receipt.
        self.handle_runtime_status_inner(id, status, detail, true, exit_confirmed)
    }

    pub(crate) fn handle_runtime_status_event(
        &mut self,
        id: String,
        incarnation: u64,
        status: WindowProcessStatus,
        detail: Option<String>,
        exit_confirmed: bool,
    ) -> Vec<OutboundEvent> {
        if !self.runtime_incarnation_is_current(&id, incarnation) {
            return Vec::new();
        }
        self.handle_runtime_status_inner(id, status, detail, true, exit_confirmed)
    }

    fn runtime_incarnation_is_current(&self, id: &str, incarnation: u64) -> bool {
        self.runtimes
            .get(id)
            .is_some_and(|runtime| runtime.incarnation == incarnation)
    }

    pub(crate) fn handle_daemon_runtime_status(
        &mut self,
        id: String,
        status: WindowProcessStatus,
        detail: Option<String>,
    ) -> Vec<OutboundEvent> {
        // The daemon wire format has no exact local PTY incarnation or child
        // exit receipt. It may update diagnostics, but never terminalize a
        // producing Session in this process.
        self.handle_runtime_status_inner(id, status, detail, false, false)
    }

    fn handle_runtime_status_inner(
        &mut self,
        id: String,
        status: WindowProcessStatus,
        detail: Option<String>,
        publish_to_daemon: bool,
        exit_confirmed: bool,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(&id).cloned() else {
            if !exit_confirmed {
                return Vec::new();
            }
            self.remove_window_state_tracking(&id);
            self.mark_agent_session_stopped(&id);
            self.deregister_pty_writer(&id);
            self.runtimes.remove(&id);
            self.window_details.remove(&id);
            // SPEC-3214 FR-002: the status arrived after the window was torn
            // down, so the PTY is gone — safe point to destroy any pending
            // intake worktree.
            return self.take_ephemeral_worktree_cleanup_events();
        };
        let issue_monitor_project_root = self
            .tab(&address.tab_id)
            .map(|tab| tab.project_root.clone());
        let is_agent_window = self.window_preset(&id) == Some(WindowPreset::Agent);
        // SPEC-3431 FR-003: capture the exiting window's session before the
        // teardown below removes the active entry — the PM crash handler
        // matches it against the durable registration.
        let pm_crash_candidate = self
            .active_agent_sessions
            .get(&id)
            .map(|session| session.session_id.clone());
        let approval_was_active = self.window_approval_waiting.contains_key(&id)
            || (status == WindowProcessStatus::Error
                && self.current_screen_approval_prompt(&id).is_some());
        let detail = if approval_was_active && status == WindowProcessStatus::Error {
            Some("Agent approval prompt ended unexpectedly".to_string())
        } else {
            detail
        };
        // A terminal PTY status ends the input generation even when the Pane
        // stays on screen for recovery diagnostics. Removing the registry
        // pointer before invalidation lets an in-flight authorized write
        // finish, while every worker still waiting to commit observes the
        // missing/stale generation and fails closed.
        if matches!(
            status,
            WindowProcessStatus::Error | WindowProcessStatus::Stopped
        ) {
            self.deregister_pty_writer(&id);
        }
        let issue_monitor_session_mode = self.issue_monitor_session_mode_for_window(&id);
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
            if let Some(project_root) = issue_monitor_project_root.as_deref() {
                self.issue_monitor_heartbeat(project_root, &id);
            }
        }

        // Issue #3616: read the provider's own explanation before anything is
        // torn down. A clean exit discards the screen entirely (the branch
        // below only composes a detail for `Error`), which is exactly why a
        // quota-dead Codex pane looked like finished work.
        let quota_notice =
            self.provider_quota_notice_for_exit(&id, status, exit_confirmed, &detail);
        match quota_notice.as_ref() {
            Some(notice) => {
                self.provider_quota_holds
                    .insert(id.clone(), provider_usage_limit_failure(notice));
            }
            None => {
                self.provider_quota_holds.remove(&id);
            }
        }
        let keep_active_agent_session_for_recovery = !exit_confirmed
            || quota_notice.is_some()
            || self.should_keep_active_agent_session_for_recoverable_pty_error(&id, status);
        // Issue #3274: an errored agent runtime is torn down below, dropping
        // its vt100 state — a client that reconnects later replays nothing and
        // an empty Error window gives no clue why. Capture the final screen
        // tail into the persistent detail before the state is gone; the raw
        // output stays available in logs.
        let detail = if let Some(notice) = quota_notice.as_ref() {
            Some(gwt_core::usage::describe_provider_limit_notice(notice))
        } else if matches!(status, WindowProcessStatus::Error)
            && !approval_was_active
            && matches!(
                self.window_preset(&id),
                Some(WindowPreset::Agent | WindowPreset::Claude | WindowPreset::Codex)
            )
        {
            compose_agent_error_detail(detail, self.final_screen_tail(&id).as_deref())
        } else {
            detail
        };
        // The hook state must still be dropped for a quota block. Keeping a
        // live `Idle` would make `compose_window_state_with_active_session`
        // resurrect the pane as Idle — a healthy-looking agent — and the quota
        // overlay below only rewrites the two terminal states.
        if matches!(status, WindowProcessStatus::Error) {
            self.window_hook_states.remove(&id);
        }
        self.window_pty_statuses.insert(id.clone(), status);
        let composed_status = self.recompute_window_state(&id).unwrap_or(status);
        if matches!(
            status,
            WindowProcessStatus::Stopped | WindowProcessStatus::Error
        ) {
            self.clear_runtime_approval_latch_without_status(&id, publish_to_daemon);
        }
        // The `window_hook_states == Some(Stopped)` condition is unreachable
        // (`window_state_for_hook_event` only returns `Idle` / `Running`), so
        // in practice an exiting agent window is never auto-closed. That is
        // deliberate for the pane itself — a stopped agent window stays on the
        // canvas so its final output remains readable
        // (`app_runtime_runtime_status_stopped_keeps_active_agent_window_for_diagnostics`,
        // #3274). SPEC-3431 FR-067 keeps that behaviour and fixes the separate
        // bug it was masking: the Issue Monitor was never told either, so the
        // launch's slot stayed held. Visibility and accounting are decided
        // independently below.
        let should_auto_close = exit_confirmed
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
            // SPEC-3431 FR-067: auto-close reaps the window itself instead of
            // going through `close_window_events`, so it also owes the Issue
            // Monitor the notification that path would have sent. Without it
            // the launch's slot stays held by a window that no longer exists.
            if is_agent_window {
                if let Some(project_root) = issue_monitor_project_root.as_deref() {
                    let message = detail
                        .as_deref()
                        .unwrap_or("Agent exited without completing the work")
                        .to_string();
                    events.extend(self.issue_monitor_agent_failed_events_with_mode(
                        project_root,
                        &id,
                        &message,
                        issue_monitor_session_mode,
                    ));
                }
            }
            self.push_workspace_and_active_work_projection_broadcasts(&mut events);
            return events;
        }
        if keep_active_agent_session_for_recovery {
            self.recoverable_agent_error_windows.insert(id.clone());
        } else if status != WindowProcessStatus::Error {
            self.recoverable_agent_error_windows.remove(&id);
        }
        if matches!(
            status,
            WindowProcessStatus::Error | WindowProcessStatus::Stopped
        ) && !keep_active_agent_session_for_recovery
        {
            self.runtimes.remove(&id);
            self.remove_window_state_tracking(&id);
            if !self.stop_pending_continue_work_session_without_projection(&id) {
                self.mark_agent_session_stopped(&id);
            }
        }
        let _ = self.persist();

        // SPEC-3214 FR-002: a Stopped/Error status means the PTY process has
        // exited — drain any intake worktree cleanup queued by the session
        // stop above (or by an earlier explicit stop of this window).
        let mut events = if exit_confirmed {
            self.take_ephemeral_worktree_cleanup_events()
        } else {
            Vec::new()
        };
        // SPEC-3431 FR-065: notify the Issue Monitor whenever an agent window
        // reaches Error, including when the pane is kept on screen.
        //
        // `keep_active_agent_session_for_recovery` used to gate this too, but
        // the two concerns are different. That flag is about **display**
        // (#3274: hold the pane so the user can read the final screen instead
        // of an empty Error window); whether the slot is still occupied is
        // about **accounting**. `WindowProcessStatus::Error` on an agent comes
        // from `try_wait` (gwt-terminal `Pane::check_status`), so the process
        // is gone either way and the slot is free either way.
        //
        // Conflating them leaked slots permanently: an agent whose last hook
        // state was `Idle` — which is every agent that ran its Stop hook —
        // satisfied the guard, so `agent_failed` was never published and the
        // row stayed `launched` forever. `recoverable_agent_error_windows` is
        // only cleared by a later hook event, and a dead process sends none.
        // With the default `max_active = 1` that stops the whole queue.
        //
        // SPEC-3431 FR-067: `Stopped` (a clean `exit 0`) leaks the same way.
        // It never reached `agent_failed`, and the auto-close gate below
        // additionally required `window_hook_states == Some(Stopped)` — a
        // value `window_state_for_hook_event` cannot produce — so the window
        // was never closed either and no `WindowClosed` control was published.
        // Nothing told the Monitor, so the row stayed `launched` holding the
        // slot. The process is gone in both cases, so both release the slot.
        //
        // This reports a clean exit through the failure channel on purpose:
        // the Monitor decides completion from the PR, not from an exit code,
        // so the only claim being made here is "this launch is over". Naming
        // it a success would be the lie, not naming it a failure.
        //
        // Issue #3616: a quota block composes to `Waiting` (the pane is not
        // done), so it is named here explicitly. The slot still has to be
        // released — the process is gone — but the Monitor receives it as a
        // typed hold rather than an attempt-consuming failure.
        if exit_confirmed
            && is_agent_window
            && (quota_notice.is_some()
                || matches!(
                    composed_status,
                    WindowProcessStatus::Error | WindowProcessStatus::Stopped
                ))
        {
            let default_message = if composed_status == WindowProcessStatus::Error {
                "Agent entered error state"
            } else {
                "Agent exited without completing the work"
            };
            let message = detail.as_deref().unwrap_or(default_message).to_string();
            if let Some(project_root) = issue_monitor_project_root.as_deref() {
                events.extend(self.issue_monitor_agent_failed_events_with_mode(
                    project_root,
                    &id,
                    &message,
                    issue_monitor_session_mode,
                ));
            }
        }
        // SPEC-3431 FR-003: an unexpected exit of the registered PM records a
        // crash and respawns when the backoff ladder allows. Clean self-exits
        // take the auto-close branch above (resident turnover: the ensure
        // gate revives the PM on the next project open); explicit closes run
        // `close_window_events`, which deregisters instead.
        if exit_confirmed
            && matches!(
                status,
                WindowProcessStatus::Error | WindowProcessStatus::Stopped
            )
            && !keep_active_agent_session_for_recovery
        {
            if let (Some(session_id), Some(project_root)) = (
                pm_crash_candidate.as_deref(),
                issue_monitor_project_root.as_ref(),
            ) {
                let tab_id = address.tab_id.clone();
                events.extend(self.handle_pm_crash(&tab_id, project_root, session_id));
            }
        }
        if exit_confirmed
            && matches!(
                status,
                WindowProcessStatus::Error | WindowProcessStatus::Stopped
            )
        {
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
        self.screen_tail(id, AGENT_ERROR_TAIL_LINES, " ")
    }

    fn screen_tail(&self, id: &str, lines: usize, separator: &str) -> Option<String> {
        let runtime = self.runtimes.get(id)?;
        let pane = runtime.pane.lock().ok()?;
        let contents = pane.screen().contents();
        let screen_lines: Vec<&str> = contents
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect();
        let start = screen_lines.len().saturating_sub(lines);
        let tail = screen_lines[start..].join(separator);
        (!tail.is_empty()).then_some(tail)
    }

    /// Issue #3616: whether this exit is the provider saying it is out of
    /// quota.
    ///
    /// Both the exit detail and the final screen are searched. The detail alone
    /// is not enough (a clean exit carries only "Process exited"), and the
    /// screen alone is not enough either (a launch-path failure reports through
    /// the detail before a pane exists). Only a confirmed exit of an agent pane
    /// qualifies — a live agent that merely *rendered* the sentence is still
    /// working.
    fn provider_quota_notice_for_exit(
        &self,
        id: &str,
        status: WindowProcessStatus,
        exit_confirmed: bool,
        detail: &Option<String>,
    ) -> Option<gwt_core::usage::ProviderLimitNotice> {
        if !exit_confirmed
            || !matches!(
                status,
                WindowProcessStatus::Error | WindowProcessStatus::Stopped
            )
            || !matches!(
                self.window_preset(id),
                Some(WindowPreset::Agent | WindowPreset::Claude | WindowPreset::Codex)
            )
        {
            return None;
        }
        let mut haystack = String::new();
        if let Some(detail) = detail.as_deref() {
            haystack.push_str(detail);
            haystack.push('\n');
        }
        if let Some(tail) = self.screen_tail(id, QUOTA_NOTICE_TAIL_LINES, "\n") {
            haystack.push_str(&tail);
        }
        gwt_core::usage::detect_provider_limit_notice(&haystack, &chrono::Local::now())
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
            let mut public_event = event.clone();
            public_event.continuation_readiness_nonce = None;
            events.push(OutboundEvent::broadcast(BackendEvent::RuntimeHookEvent {
                event: public_event,
            }));
        }
        let Some(window_id) = self.active_window_for_runtime_event(&event) else {
            return events;
        };
        let issue_monitor_session_mode = self.issue_monitor_session_mode_for_window(&window_id);
        let effective_before = self.window_status(&window_id);
        let approval_wait_cleared = self.window_approval_waiting.contains_key(&window_id);
        if approval_wait_cleared {
            self.clear_runtime_approval_latch_without_status(&window_id, publish_to_daemon);
        }
        let issue_monitor_project_root = self.issue_monitor_project_root_for_window(&window_id);
        // SPEC-3431 FR-068: a hook arrival is the one signal that an agent is
        // actually making progress. The PTY-status heartbeat below never fires
        // for a working agent (the watcher thread stays silent until the
        // process exits), so without this the activity clock froze at launch
        // and a rate-limited or hung agent was indistinguishable from a busy
        // one. Throttled inside `issue_monitor_heartbeat`.
        if let Some(project_root) = issue_monitor_project_root.clone() {
            self.issue_monitor_heartbeat(&project_root, &window_id);
        }
        if event.source_event.as_deref() == Some("SessionStart") {
            if let Err(error) = self.finalize_tool_runtime_migration_session_start(&window_id) {
                self.pending_tool_runtime_migrations.remove(&window_id);
                self.stop_window_runtime_without_session_projection(&window_id);
                if let Some(active) = self.active_agent_sessions.remove(&window_id) {
                    let _ = gwt_agent::persist_session_status(
                        &self.sessions_dir,
                        &active.session_id,
                        gwt_agent::AgentStatus::Interrupted,
                    );
                }
                self.revoke_agent_capability_for_window(&window_id);
                events.extend(self.launch_error_events_with_continue_work(
                    window_id,
                    format!(
                        "authenticated SessionStart could not commit tool runtime provenance migration: {error}"
                    ),
                    None,
                ));
                return events;
            }
            events.extend(self.finalize_fresh_execution_launch_session_start(
                &window_id,
                event.continuation_readiness_nonce.as_deref(),
            ));
            events.extend(self.finalize_continue_work_session_start(
                &window_id,
                event.continuation_readiness_nonce.as_deref(),
            ));
        }
        let is_agent_window = self.window_preset(&window_id) == Some(WindowPreset::Agent);
        let Some(hook_state) = gwt::window_state::runtime_hook_window_state(&event) else {
            if approval_wait_cleared {
                if let Some(composed) = self.recompute_window_state(&window_id) {
                    if effective_before != Some(composed) {
                        events.extend(Self::status_events(window_id, composed, None));
                    }
                }
            }
            return events;
        };
        self.recoverable_agent_error_windows.remove(&window_id);
        // Issue #3616: a hook arrival means the agent is running again, so any
        // quota hold recorded for this pane is stale.
        self.provider_quota_holds.remove(&window_id);
        let hook_state_changed =
            self.window_hook_states.get(&window_id).copied() != Some(hook_state);
        if !hook_state_changed && !approval_wait_cleared {
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
            if let Some(project_root) = issue_monitor_project_root.as_deref() {
                events.extend(self.issue_monitor_agent_failed_events_with_mode(
                    project_root,
                    &window_id,
                    &message,
                    issue_monitor_session_mode,
                ));
            }
        }
        if matches!(
            composed_state,
            WindowProcessStatus::Error | WindowProcessStatus::Stopped
        ) {
            if let Some(event) = self.active_work_projection_broadcast_for_active_tab() {
                events.push(event);
            }
        }
        if hook_state_changed || effective_before != Some(composed_state) {
            events.extend(Self::status_events(window_id, composed_state, detail));
        }
        events
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
#[derive(Debug)]
struct RuntimeDaemonApprovalPublish {
    project_root: PathBuf,
    id: String,
    waiting: bool,
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
static RUNTIME_DAEMON_APPROVAL_PUBLISH_QUEUE: std::sync::OnceLock<
    Mutex<Option<std_mpsc::Sender<RuntimeDaemonApprovalPublish>>>,
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
fn runtime_daemon_approval_publish_sender() -> Option<std_mpsc::Sender<RuntimeDaemonApprovalPublish>>
{
    let queue = RUNTIME_DAEMON_APPROVAL_PUBLISH_QUEUE.get_or_init(|| Mutex::new(None));
    runtime_daemon_approval_publish_sender_from(queue, |receiver| {
        std::thread::Builder::new()
            .name("gwt-runtime-daemon-approval-publish-worker".to_string())
            .spawn(move || run_runtime_daemon_approval_publish_worker(receiver))
            .map(|_handle| ())
    })
}

#[cfg(unix)]
fn runtime_daemon_approval_publish_sender_from(
    queue: &Mutex<Option<std_mpsc::Sender<RuntimeDaemonApprovalPublish>>>,
    spawn_worker: impl FnOnce(std_mpsc::Receiver<RuntimeDaemonApprovalPublish>) -> std::io::Result<()>,
) -> Option<std_mpsc::Sender<RuntimeDaemonApprovalPublish>> {
    let Ok(mut queue) = queue.lock() else {
        tracing::debug!("runtime daemon approval publish queue lock poisoned");
        return None;
    };
    if let Some(sender) = queue.as_ref() {
        return Some(sender.clone());
    }
    let (sender, receiver) = std_mpsc::channel();
    match spawn_worker(receiver) {
        Ok(()) => {
            *queue = Some(sender.clone());
            Some(sender)
        }
        Err(err) => {
            tracing::debug!(error = %err, "runtime daemon approval publish worker spawn failed");
            None
        }
    }
}

#[cfg(unix)]
fn run_runtime_daemon_approval_publish_worker(
    receiver: std_mpsc::Receiver<RuntimeDaemonApprovalPublish>,
) {
    for publish in receiver {
        publish_runtime_daemon_approval_event(publish);
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
fn publish_runtime_daemon_approval_event(publish: RuntimeDaemonApprovalPublish) {
    let payload = gwt::runtime_daemon_events::runtime_approval_overlay_payload(
        &publish.id,
        publish.waiting,
        std::process::id(),
    );
    let result = gwt::daemon_publisher::publish_event(
        &publish.project_root,
        gwt::runtime_daemon_events::RUNTIME_APPROVAL_OVERLAY_CHANNEL,
        payload,
    );
    if let Err(err) = result {
        tracing::debug!(
            error = %err,
            project_root = %publish.project_root.display(),
            window_id = %publish.id,
            waiting = publish.waiting,
            "runtime approval overlay daemon publish failed (non-fatal)"
        );
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

#[cfg(unix)]
fn publish_runtime_approval_overlay_change(project_root: &Path, id: &str, waiting: bool) {
    let Some(sender) = runtime_daemon_approval_publish_sender() else {
        return;
    };
    if sender
        .send(RuntimeDaemonApprovalPublish {
            project_root: project_root.to_path_buf(),
            id: id.to_string(),
            waiting,
        })
        .is_err()
    {
        tracing::debug!("runtime daemon approval publish queue disconnected");
    }
}

#[cfg(not(unix))]
fn publish_runtime_approval_overlay_change(_project_root: &Path, _id: &str, _waiting: bool) {}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::path::PathBuf;

    #[cfg(unix)]
    use std::sync::{mpsc, Mutex};

    #[cfg(unix)]
    use super::{
        runtime_daemon_approval_publish_sender_from, runtime_daemon_publish_sender_from,
        try_enqueue_runtime_daemon_publish, RuntimeDaemonApprovalPublish, RuntimeDaemonPublish,
        RuntimeDaemonPublishEnqueueError,
    };
    #[cfg(unix)]
    use crate::WindowProcessStatus;

    #[test]
    fn late_provider_active_writer_error_is_classified_as_resume_writer_conflict() {
        let detail = "Failed to resume session from ~/.codex/sessions/rollout.jsonl: \
            thread/resume failed during TUI bootstrap: thread 019 already has an active writer \
            (code -32600)";

        assert_eq!(
            super::classify_issue_monitor_failure(detail, gwt_agent::SessionMode::Resume),
            Some(gwt::IssueMonitorFailure::ResumeWriterConflict {
                holder_window_id: None,
            }),
            "a late provider race is typed immediately even when the external holder is unknown"
        );
        for prefixed_detail in [
            "Error: Failed to resume session from ~/.codex/sessions/rollout.jsonl: thread/resume failed during TUI bootstrap: thread 019 already has an active writer (code -32600)",
            "Agent exited — last output: Error: Failed to resume session from ~/.codex/sessions/rollout.jsonl: thread/resume failed during TUI bootstrap: thread 019 already has an active writer (code -32600)",
        ] {
            assert_eq!(
                super::classify_issue_monitor_failure(
                    prefixed_detail,
                    gwt_agent::SessionMode::Resume,
                ),
                Some(gwt::IssueMonitorFailure::ResumeWriterConflict {
                    holder_window_id: None,
                }),
                "the provider's observed Error: prefix must preserve typed recovery"
            );
        }
        assert_eq!(
            super::classify_issue_monitor_failure(
                "thread/resume failed during TUI bootstrap: provider temporarily unavailable",
                gwt_agent::SessionMode::Resume,
            ),
            None,
            "generic provider failures must stay on the existing failure path"
        );

        for (case, unrelated_detail) in [
            (
                "reversed markers",
                "thread/resume failed during TUI bootstrap: (code -32600) thread 019 already has an active writer",
            ),
            (
                "markers split across lines",
                "thread/resume failed during TUI bootstrap:\nthread 019 already has an active writer (code -32600)",
            ),
            (
                "different provider code",
                "thread/resume failed during TUI bootstrap: thread 019 already has an active writer (code -32601)",
            ),
            (
                "quoted compound diagnostic",
                "provider wrapper repeated \"thread/resume failed during TUI bootstrap: thread 019 already has an active writer (code -32600)\"",
            ),
            (
                "unanchored Error prefix",
                "provider wrapper: Error: Failed to resume session from ~/.codex/sessions/rollout.jsonl: thread/resume failed during TUI bootstrap: thread 019 already has an active writer (code -32600)",
            ),
            (
                "nested last-output prefix",
                "Agent exited — last output: Warning: Error: Failed to resume session from ~/.codex/sessions/rollout.jsonl: thread/resume failed during TUI bootstrap: thread 019 already has an active writer (code -32600)",
            ),
        ] {
            assert_eq!(
                super::classify_issue_monitor_failure(
                    unrelated_detail,
                    gwt_agent::SessionMode::Resume,
                ),
                None,
                "{case} must not be promoted to a typed writer conflict"
            );
        }
    }

    #[test]
    fn agent_failed_payload_carries_classified_resume_writer_conflict_to_daemon() {
        let detail = "Failed to resume session from ~/.codex/sessions/rollout.jsonl: \
            thread/resume failed during TUI bootstrap: thread 019 already has an active writer \
            (code -32600)";

        assert_eq!(
            super::super::AppRuntime::issue_monitor_agent_failed_payload(
                "tab-1::agent-42",
                detail,
                Some(42),
                gwt_agent::SessionMode::Resume,
            ),
            serde_json::json!({
                "agent_failed": {
                    "issue_number": 42,
                    "window_id": "tab-1::agent-42",
                    "message": detail,
                    "failure": {
                        "kind": "resume_writer_conflict",
                    },
                }
            }),
            "the payload helper used by issue_monitor_agent_failed_events must bridge the classifier into the daemon envelope"
        );
        assert_eq!(
            super::super::AppRuntime::issue_monitor_agent_failed_payload(
                "tab-1::agent-42",
                "provider temporarily unavailable",
                Some(42),
                gwt_agent::SessionMode::Resume,
            ),
            serde_json::json!({
                "agent_failed": {
                    "issue_number": 42,
                    "window_id": "tab-1::agent-42",
                    "message": "provider temporarily unavailable",
                }
            }),
            "generic failures must keep the legacy message-only envelope"
        );
    }

    #[test]
    fn launch_failed_payload_carries_classified_resume_writer_conflict_and_source_identity() {
        let detail = "Failed to resume session from ~/.codex/sessions/rollout.jsonl: \
            thread/resume failed during TUI bootstrap: thread 019 already has an active writer \
            (code -32600)";

        assert_eq!(
            super::super::AppRuntime::issue_monitor_launch_failed_payload(
                42,
                detail,
                Some("launch:effect-42"),
                Some("gui-42"),
                gwt_agent::SessionMode::Resume,
            ),
            serde_json::json!({
                "launch_failed": {
                    "issue_number": 42,
                    "message": detail,
                    "delivery_id": "launch:effect-42",
                    "materializer_id": "gui-42",
                    "failure": {
                        "kind": "resume_writer_conflict",
                    },
                }
            })
        );
    }

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
    fn runtime_approval_overlay_lane_survives_output_queue_saturation_in_order() {
        let (output_sender, _output_receiver) = mpsc::sync_channel(1);
        let project_root = PathBuf::from("/tmp/gwt-project");
        try_enqueue_runtime_daemon_publish(
            &output_sender,
            RuntimeDaemonPublish::Output {
                project_root: project_root.clone(),
                id: "tab-1::agent-1".to_string(),
                data: b"flood".to_vec(),
            },
        )
        .expect("fill output lane");

        let queue = Mutex::new(None);
        let (captured_tx, captured_rx) = mpsc::channel();
        let approval_sender =
            runtime_daemon_approval_publish_sender_from(&queue, move |receiver| {
                std::thread::spawn(move || {
                    for event in receiver {
                        captured_tx.send(event).expect("capture overlay event");
                    }
                });
                Ok(())
            })
            .expect("approval lane");
        approval_sender
            .send(RuntimeDaemonApprovalPublish {
                project_root: project_root.clone(),
                id: "tab-1::agent-1".to_string(),
                waiting: true,
            })
            .expect("waiting true");
        approval_sender
            .send(RuntimeDaemonApprovalPublish {
                project_root,
                id: "tab-1::agent-1".to_string(),
                waiting: false,
            })
            .expect("waiting false");

        assert!(captured_rx.recv().expect("true").waiting);
        assert!(!captured_rx.recv().expect("false").waiting);
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

    #[test]
    fn resume_writer_conflict_classification_and_payload_require_resume_session_origin() {
        let detail = "Failed to resume session from ~/.codex/sessions/rollout.jsonl: \
            thread/resume failed during TUI bootstrap: thread 019 already has an active writer \
            (code -32600)";
        let classify: fn(&str, gwt_agent::SessionMode) -> Option<gwt::IssueMonitorFailure> =
            super::classify_issue_monitor_failure;
        let agent_payload: fn(
            &str,
            &str,
            Option<u64>,
            gwt_agent::SessionMode,
        ) -> serde_json::Value = super::super::AppRuntime::issue_monitor_agent_failed_payload;
        let launch_payload: fn(
            u64,
            &str,
            Option<&str>,
            Option<&str>,
            gwt_agent::SessionMode,
        ) -> serde_json::Value = super::super::AppRuntime::issue_monitor_launch_failed_payload;

        assert_eq!(
            classify(detail, gwt_agent::SessionMode::Resume),
            Some(gwt::IssueMonitorFailure::ResumeWriterConflict {
                holder_window_id: None,
            }),
            "the exact provider diagnostic is typed only for an actual Resume launch"
        );
        let composed = format!("Process exited with status 1 — last output: {detail}");
        assert_eq!(
            classify(&composed, gwt_agent::SessionMode::Resume),
            Some(gwt::IssueMonitorFailure::ResumeWriterConflict {
                holder_window_id: None,
            }),
            "the runtime-composed error detail retains the anchored provider diagnostic",
        );
        for (case, echoed_detail) in [
            ("raw normal output", detail),
            (
                "single-quoted fresh output echo",
                "launcher echoed 'thread/resume failed during TUI bootstrap: thread 019 already has an active writer (code -32600)'",
            ),
        ] {
            assert_eq!(
                classify(echoed_detail, gwt_agent::SessionMode::Normal),
                None,
                "{case} is generic because a Normal/fresh launch cannot encounter a real Resume writer race"
            );
        }

        let resume_agent = agent_payload(
            "tab-1::agent-42",
            detail,
            Some(42),
            gwt_agent::SessionMode::Resume,
        );
        assert_eq!(
            resume_agent
                .pointer("/agent_failed/failure/kind")
                .and_then(serde_json::Value::as_str),
            Some("resume_writer_conflict")
        );
        let fresh_agent = agent_payload(
            "tab-1::agent-42",
            detail,
            Some(42),
            gwt_agent::SessionMode::Normal,
        );
        assert!(
            fresh_agent.pointer("/agent_failed/failure").is_none(),
            "a fresh AgentFailed payload keeps the marker as a generic message"
        );

        let resume_launch = launch_payload(
            42,
            detail,
            Some("launch:effect-42"),
            Some("gui-42"),
            gwt_agent::SessionMode::Resume,
        );
        assert_eq!(
            resume_launch
                .pointer("/launch_failed/failure/kind")
                .and_then(serde_json::Value::as_str),
            Some("resume_writer_conflict")
        );
        let fresh_launch = launch_payload(
            42,
            detail,
            Some("launch:effect-42"),
            Some("gui-42"),
            gwt_agent::SessionMode::Normal,
        );
        assert!(
            fresh_launch.pointer("/launch_failed/failure").is_none(),
            "a FreshRequired/Normal LaunchFailed payload cannot synthesize a Resume conflict"
        );
    }
}
