//! Runtime event handlers and daemon publish queue for SPEC-2077 Phase J1.
//!
//! This module owns the already daemon-aware runtime output/status/hook
//! publish path. The extraction is behavior-preserving: it keeps best-effort
//! daemon publish, same-process echo suppression through the existing payload
//! layer, and the local GUI state update path unchanged.

use base64::Engine as _;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc as std_mpsc;
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use super::{AppRuntime, BackendEvent, OutboundEvent, WindowPreset, WindowProcessStatus};

/// Preserve indentation and interior blank rows; omit only unused screen rows.
pub(super) fn terminal_preview_text(contents: &str) -> String {
    let mut lines: Vec<&str> = contents.lines().collect();
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    lines[lines.len().saturating_sub(3)..].join("\n")
}

pub(super) const RUNTIME_OUTPUT_BATCH_INTERVAL: Duration = Duration::from_millis(150);

pub(crate) struct PendingRuntimeOutput {
    authoritative_owner: bool,
    incarnation: Option<u64>,
    deadline: Instant,
    pub(super) data: Vec<u8>,
    screen: Option<String>,
}

/// Issue #3616: how long a quota notice must sit on a *live* pane's screen,
/// with no agent activity at all, before it is treated as a block.
///
/// A blocked pane produces nothing for days. A working agent runs a tool —
/// and so fires a hook — far inside this window, and any hook abandons the
/// candidate. The cost of waiting is a held slot for two minutes; the cost of
/// not waiting is releasing a healthy launch because its own output quoted the
/// provider's sentence.
#[cfg(test)]
const PROVIDER_QUOTA_SETTLE_SECS: i64 = 120;

/// Provider TUIs commonly clear and redraw the approval block after Enter.
/// This bounded settle window avoids a false Running frame between the clear
/// and the redraw without ever blocking the tao event loop.
const APPROVAL_SETTLE_DELAY: Duration = Duration::from_millis(100);
fn runtime_hook_source_event_profile_label(source_event: Option<&str>) -> &'static str {
    match source_event {
        Some("SessionStart") => "session_start",
        Some("UserPromptSubmit") => "user_prompt_submit",
        Some("PreToolUse") => "pre_tool_use",
        Some("PostToolUse") => "post_tool_use",
        Some("Stop") => "stop",
        Some("SubagentStart") => "subagent_start",
        Some("SubagentStop") => "subagent_stop",
        Some("Notification") => "notification",
        Some("PermissionRequest") => "permission_request",
        Some(_) => "other",
        None => "none",
    }
}

/// Issue #4406 AC-4: the per-stage cost of one `RuntimeHook` dispatch.
///
/// The dispatch timer in `main.rs` reports that `RuntimeHook` held the GUI
/// thread — in production for as long as 257,075ms — but not which of the
/// dispatch's synchronous stages spent it. Every stage below can reach disk,
/// the Host bridge or GitHub, so naming the dominant one is what turns a stall
/// report into a place to look.
#[derive(Default)]
struct RuntimeHookStageTimings {
    stages: Vec<(&'static str, u64)>,
}

impl RuntimeHookStageTimings {
    fn record_millis(&mut self, stage: &'static str, elapsed_ms: u64) {
        self.stages.push((stage, elapsed_ms));
    }

    /// Time `run` and attribute it to `stage`. Returns whatever `run` returns
    /// so a measured call keeps the shape of the unmeasured one.
    fn measure<T>(&mut self, stage: &'static str, run: impl FnOnce() -> T) -> T {
        let started = std::time::Instant::now();
        let value = run();
        self.record_millis(stage, started.elapsed().as_millis() as u64);
        value
    }

    /// The stage that held the loop longest, or `None` when the dispatch
    /// returned before reaching any measured stage.
    ///
    /// Ties resolve to the stage that ran first so repeated specimens of one
    /// stall keep naming one stage instead of alternating between two.
    fn dominant(&self) -> Option<(&'static str, u64)> {
        self.stages.iter().copied().reduce(|dominant, candidate| {
            if candidate.1 > dominant.1 {
                candidate
            } else {
                dominant
            }
        })
    }
}

/// Issue #4406 AC-4: report one `RuntimeHook` dispatch against the same budget
/// the event-loop dispatch timer uses, naming the stage that dominated it.
///
/// `stage` is `unattributed` when the dispatch went over budget without any
/// measured stage doing so — the time is then spread across the unmeasured
/// body, and claiming a stage would send the next reader after the wrong code.
fn log_runtime_hook_stage_timing(
    elapsed_ms: u64,
    source_event: &'static str,
    stages: &RuntimeHookStageTimings,
) {
    let (stage, stage_elapsed_ms) = stages.dominant().unwrap_or(("unattributed", 0));
    if elapsed_ms >= crate::GUI_EVENT_LOOP_SLOW_DISPATCH_MS {
        tracing::warn!(
            target: "gwt.frontend.timing",
            stage,
            stage_elapsed_ms,
            elapsed_ms,
            source_event,
            "RuntimeHook dispatch exceeded the GUI event loop budget"
        );
    } else {
        tracing::debug!(
            target: "gwt.frontend.timing",
            stage,
            stage_elapsed_ms,
            elapsed_ms,
            source_event,
            "RuntimeHook dispatch handled"
        );
    }
}

#[cfg(test)]
mod runtime_hook_stage_timing_tests {
    use super::{log_runtime_hook_stage_timing, RuntimeHookStageTimings};

    fn timings(stages: &[(&'static str, u64)]) -> RuntimeHookStageTimings {
        let mut recorded = RuntimeHookStageTimings::default();
        for (stage, elapsed_ms) in stages {
            recorded.record_millis(stage, *elapsed_ms);
        }
        recorded
    }

    /// Issue #4406 AC-4: "RuntimeHook blocked the GUI event loop for 257075ms"
    /// names the dispatch but not the stage inside it, so the production stall
    /// cannot be assigned to a cause. The over-budget warning must name the
    /// stage that held the loop and how long it held it.
    #[test]
    fn an_over_budget_runtime_hook_dispatch_names_its_dominant_stage() {
        let recorded = timings(&[
            ("issue_monitor_heartbeat", 3),
            ("fresh_execution_launch_finalization", 25_000),
            ("window_state_recompute", 4),
            ("persist", 11),
        ]);
        let output = crate::tests::capture_timing_warnings(|| {
            log_runtime_hook_stage_timing(28_160, "session_start", &recorded);
        });
        let logs: Vec<serde_json::Value> = output
            .lines()
            .map(|line| serde_json::from_str(line).expect("runtime hook stage timing JSON"))
            .collect();
        assert_eq!(
            logs.len(),
            1,
            "an over-budget dispatch must warn exactly once"
        );
        let fields = &logs[0]["fields"];
        assert_eq!(fields["stage"], "fresh_execution_launch_finalization");
        assert_eq!(fields["stage_elapsed_ms"], 25_000);
        assert_eq!(fields["elapsed_ms"], 28_160);
        assert_eq!(fields["source_event"], "session_start");
    }

    /// The threshold is the same one the dispatch timer uses, so an attributed
    /// warning never appears for a dispatch the event loop never noticed.
    #[test]
    fn a_within_budget_runtime_hook_dispatch_does_not_warn() {
        let recorded = timings(&[("persist", 12)]);
        let output = crate::tests::capture_timing_warnings(|| {
            log_runtime_hook_stage_timing(
                crate::GUI_EVENT_LOOP_SLOW_DISPATCH_MS - 1,
                "stop",
                &recorded,
            );
            log_runtime_hook_stage_timing(
                crate::GUI_EVENT_LOOP_SLOW_DISPATCH_MS,
                "stop",
                &recorded,
            );
        });
        assert_eq!(
            output.lines().count(),
            1,
            "only the dispatch at or over budget may warn"
        );
    }

    /// A dispatch can exceed the budget without any single measured stage doing
    /// so — the time is then spread across the unmeasured body. Reporting a
    /// stage anyway would send the next reader after the wrong code.
    #[test]
    fn a_dispatch_with_no_measured_stages_reports_no_stage() {
        let output = crate::tests::capture_timing_warnings(|| {
            log_runtime_hook_stage_timing(500, "pre_tool_use", &RuntimeHookStageTimings::default());
        });
        let logs: Vec<serde_json::Value> = output
            .lines()
            .map(|line| serde_json::from_str(line).expect("runtime hook stage timing JSON"))
            .collect();
        assert_eq!(logs.len(), 1);
        let fields = &logs[0]["fields"];
        assert_eq!(fields["stage"], "unattributed");
        assert_eq!(fields["stage_elapsed_ms"], 0);
        assert_eq!(fields["elapsed_ms"], 500);
    }

    /// Ties go to the stage that ran first, so repeated specimens of the same
    /// stall name the same stage instead of alternating between two.
    #[test]
    fn equal_stage_times_resolve_to_the_earlier_stage() {
        let recorded = timings(&[("issue_monitor_heartbeat", 900), ("persist", 900)]);
        let output = crate::tests::capture_timing_warnings(|| {
            log_runtime_hook_stage_timing(1_900, "notification", &recorded);
        });
        let logs: Vec<serde_json::Value> = output
            .lines()
            .map(|line| serde_json::from_str(line).expect("runtime hook stage timing JSON"))
            .collect();
        assert_eq!(logs[0]["fields"]["stage"], "issue_monitor_heartbeat");
    }
}

pub(super) use gwt::project_runtime::runtime_events::classify_issue_monitor_failure;
#[cfg(test)]
pub(super) use gwt::project_runtime::runtime_events::{
    provider_label, provider_usage_limit_failure,
};

impl AppRuntime {
    pub(super) fn relay_provider_usage_to_owners(
        &self,
        accounts: &[gwt_core::usage::ProviderUsage],
    ) {
        let mut owners = HashMap::new();
        for runtime in self.runtimes.values() {
            owners
                .entry(runtime.owner_identity.host_instance_id.clone())
                .or_insert_with(|| runtime.client.clone());
        }
        for client in owners.into_values() {
            let accounts = accounts.to_vec();
            if let Err(error) = self.blocking_tasks.try_spawn(move || {
                let result = tokio::runtime::Builder::new_current_thread().enable_all().build()
                    .map_err(|error| error.to_string()).and_then(|runtime| runtime.block_on(client.command(
                        gwt::project_runtime::events::OwnerCommand::ProviderUsageSnapshot { accounts })));
                if let Err(error) = result { tracing::warn!(%error, "provider usage projection was not accepted by project owner"); }
            }) { tracing::warn!(%error, "provider usage owner relay could not be scheduled"); }
        }
    }

    /// Deliver the shared diagnostic stream only to projects with a consumer.
    /// Docked surfaces retain their subscriptions, independently of UI selection.
    fn project_events_for_open_surface(
        &self,
        preset: WindowPreset,
        event: BackendEvent,
    ) -> Vec<OutboundEvent> {
        self.tabs
            .iter()
            .filter(|tab| {
                tab.workspace
                    .persisted()
                    .windows
                    .iter()
                    .any(|window| window.preset == preset)
            })
            .filter_map(|tab| {
                self.project_key_for_tab(&tab.id)
                    .cloned()
                    .map(|key| OutboundEvent::project(key, event.clone()))
            })
            .collect()
    }

    pub(crate) fn log_entry_events(
        &self,
        entry: gwt_core::logging::LogEvent,
    ) -> Vec<OutboundEvent> {
        let project_scope = entry.project_scope.clone();
        let mut events = self.project_events_for_open_surface(
            WindowPreset::Logs,
            BackendEvent::LogEntryAppended { entry },
        );
        if let Some(scope) = project_scope {
            events.retain(|event| {
                matches!(&event.target,
                super::DispatchTarget::Project(key) if key.as_str() == scope)
            });
        }
        events
    }

    /// Test-only entry that streams output and drains its derived batch;
    /// production output arrives through [`Self::handle_runtime_output_event`]
    /// with the reader thread's position (Issue #4095).
    #[cfg(test)]
    pub(crate) fn handle_runtime_output(
        &mut self,
        id: String,
        data: Vec<u8>,
    ) -> Vec<OutboundEvent> {
        let screen = self.runtimes.get(&id).map(|runtime| runtime.screen.clone());
        let mut events =
            self.handle_runtime_output_inner(id.clone(), data, true, false, None, screen);
        events.extend(self.flush_runtime_output_batch(&id));
        events
    }

    /// `seq` is the pane stream position the reader thread observed right
    /// after parsing `data` (Issue #4095); it lets a client queue drop this
    /// chunk when a snapshot taken at or past that position is queued first.
    #[cfg(test)]
    pub(crate) fn handle_runtime_output_event(
        &mut self,
        id: String,
        incarnation: u64,
        data: Vec<u8>,
        seq: u64,
        screen: Option<String>,
    ) -> Vec<OutboundEvent> {
        if !self.runtime_incarnation_is_current(&id, incarnation) {
            return Vec::new();
        }
        self.handle_runtime_output_inner(id, data, true, false, Some(seq), screen)
    }

    pub(crate) fn handle_daemon_runtime_output(
        &mut self,
        id: String,
        data: Vec<u8>,
        preview_text: Option<String>,
    ) -> Vec<OutboundEvent> {
        let preview = preview_text.and_then(|text| {
            let project_key = self.project_key_for_window(&id)?.clone();
            self.remote_terminal_previews
                .insert(id.clone(), text.clone());
            Some(OutboundEvent::project(
                project_key,
                BackendEvent::TerminalPreview {
                    id: id.clone(),
                    text,
                },
            ))
        });
        // Daemon publications describe another process; a local PTY
        // incarnation is neither
        // available nor authoritative for this path.
        let mut events = self.handle_runtime_output_inner(id, data, false, false, None, None);
        events.extend(preview);
        events
    }

    /// Authoritative bytes and screen supplied by the daemon, with the existing preview batching.
    pub(crate) fn handle_owner_runtime_output(
        &mut self,
        id: String,
        data: Vec<u8>,
        seq: u64,
        screen: String,
    ) -> Vec<OutboundEvent> {
        self.handle_runtime_output_inner(id, data, true, true, Some(seq), Some(screen))
    }

    fn handle_runtime_output_inner(
        &mut self,
        id: String,
        data: Vec<u8>,
        publish_to_daemon: bool,
        authoritative_owner: bool,
        stream_seq: Option<u64>,
        screen: Option<String>,
    ) -> Vec<OutboundEvent> {
        let _project_scope = self.enter_window_log_scope(&id);
        if !self.window_lookup.contains_key(&id) {
            return Vec::new();
        }
        // Issue #3475: progress evidence for the authenticated SessionStart
        // readiness deadline. Saturating because the counter only ever needs to
        // answer "did this pane emit anything since the last deadline".
        let observed = self.window_output_bytes.entry(id.clone()).or_insert(0);
        *observed = observed.saturating_add(data.len() as u64);
        // Issue #4608: any output — local or relayed — is the pane writing,
        // which is the Monitor's hook-independent liveness signal.
        if !data.is_empty() {
            self.window_last_output_at
                .insert(id.clone(), chrono::Utc::now());
        }
        // Raw bytes retain their original stream position and latency. Only
        // screen-derived work and the best-effort daemon relay are batched.
        let mut flush = false;
        if publish_to_daemon {
            let incarnation = self.runtimes.get(&id).map(|runtime| runtime.incarnation);
            let pending = self
                .pending_runtime_outputs
                .entry(id.clone())
                .or_insert_with(|| PendingRuntimeOutput {
                    authoritative_owner,
                    incarnation,
                    deadline: Instant::now() + RUNTIME_OUTPUT_BATCH_INTERVAL,
                    data: Vec::new(),
                    screen: None,
                });
            if pending.incarnation != incarnation
                || pending.authoritative_owner != authoritative_owner
            {
                *pending = PendingRuntimeOutput {
                    authoritative_owner,
                    incarnation,
                    deadline: Instant::now() + RUNTIME_OUTPUT_BATCH_INTERVAL,
                    data: Vec::new(),
                    screen: None,
                };
            }
            pending.data.extend_from_slice(&data);
            pending.screen = screen;
            flush = pending.data.len() >= gwt::pane_runtime::OUTPUT_BATCH_MAX_BYTES;
        }
        let Some(project_key) = self.project_key_for_window(&id).cloned() else {
            return Vec::new();
        };
        let mut events = vec![OutboundEvent::project(
            project_key,
            BackendEvent::TerminalOutput {
                id: id.clone(),
                data_base64: base64::engine::general_purpose::STANDARD.encode(data),
            },
        )
        .with_terminal_stream_seq(stream_seq)];
        if flush {
            events.extend(self.flush_runtime_output_batch(&id));
        }
        events
    }

    pub(crate) fn next_runtime_output_deadline(&self) -> Option<Instant> {
        self.pending_runtime_outputs
            .values()
            .map(|pending| pending.deadline)
            .min()
    }

    pub(crate) fn flush_runtime_output_batches(&mut self, now: Instant) -> Vec<OutboundEvent> {
        let due: Vec<String> = self
            .pending_runtime_outputs
            .iter()
            .filter(|(_, pending)| pending.deadline <= now)
            .map(|(id, _)| id.clone())
            .collect();
        let mut events = Vec::new();
        for id in due {
            events.extend(self.flush_runtime_output_batch(&id));
        }
        events
    }

    pub(super) fn flush_runtime_output_batch(&mut self, id: &str) -> Vec<OutboundEvent> {
        let Some(pending) = self.pending_runtime_outputs.remove(id) else {
            return Vec::new();
        };
        if self.runtimes.get(id).map(|runtime| runtime.incarnation) != pending.incarnation {
            return Vec::new();
        }
        let Some(address) = self.window_lookup.get(id).cloned() else {
            return Vec::new();
        };
        let Some(project_key) = self.project_key_for_window(id).cloned() else {
            return Vec::new();
        };
        let batch_started = Instant::now();
        let _project_scope = self.enter_window_log_scope(id);
        let scope_elapsed = batch_started.elapsed();
        // The parser worker captured this screen with the batch's stream
        // position. Never wait for the Pane mutex or render on the GUI thread.
        let screen = pending.screen;
        let preview = screen.as_deref().map(terminal_preview_text);
        let preview_elapsed = batch_started.elapsed();
        if pending.authoritative_owner {
            return preview
                .into_iter()
                .map(|text| {
                    OutboundEvent::project(
                        project_key.clone(),
                        BackendEvent::TerminalPreview {
                            id: id.to_string(),
                            text,
                        },
                    )
                })
                .collect();
        }
        if let Some(tab) = self.tab(&address.tab_id) {
            publish_runtime_output_change(&tab.project_root, id, &pending.data, preview.clone());
        }
        let relay_elapsed = batch_started.elapsed();
        let mut events = Vec::new();
        if let Some(text) = preview {
            events.push(OutboundEvent::project(
                project_key,
                BackendEvent::TerminalPreview {
                    id: id.to_string(),
                    text,
                },
            ));
        }
        tracing::debug!(target: "gwt.frontend.timing", marker = "issue_5379_runtime_output_batch",
            elapsed_ms = batch_started.elapsed().as_millis() as u64,
            scope_us = scope_elapsed.as_micros() as u64,
            preview_us = (preview_elapsed - scope_elapsed).as_micros() as u64,
            relay_us = (relay_elapsed - preview_elapsed).as_micros() as u64,
            "GUI runtime output batch projected");
        events
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
        Some((provider, runtime.screen.clone()))
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
            self.status_events(window_id.to_string(), composed, None)
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
        self.handle_runtime_status_inner(id, status, detail, true, false, false)
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
        self.handle_runtime_status_inner(id, status, detail, true, exit_confirmed, false)
    }

    #[cfg(test)]
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
        // Flush the final bytes/preview before status can tear the pane down.
        let mut events = self.flush_runtime_output_batch(&id);
        events.extend(self.handle_runtime_status_inner(
            id,
            status,
            detail,
            true,
            exit_confirmed,
            true,
        ));
        events
    }

    #[cfg(test)]
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
        self.handle_runtime_status_inner(id, status, detail, false, false, false)
    }

    fn handle_runtime_status_inner(
        &mut self,
        id: String,
        status: WindowProcessStatus,
        detail: Option<String>,
        _publish_to_daemon: bool,
        exit_confirmed: bool,
        _exact_runtime_incarnation: bool,
    ) -> Vec<OutboundEvent> {
        if let Some(runtime) = self.runtimes.get_mut(&id) {
            runtime.raw_status = status;
            runtime.status = status;
            runtime.detail = detail.clone();
            runtime.exit_confirmed |= exit_confirmed;
        }
        self.window_pty_statuses.insert(id.clone(), status);
        self.apply_owner_status_projection(&id, status, detail)
    }

    /// The trailing non-empty lines of a window's live vt100 screen, joined
    /// into one detail-sized string. `None` when the window has no runtime
    /// (already torn down) or the screen is blank (Issue #3274).
    /// Issue #3616: fold one live-screen observation into the quota state for
    /// `window_id`, returning whatever the resulting state change must publish.
    ///
    /// `screen` is the pane's currently rendered screen, or `None` when it has
    /// no runtime to read. Idempotent: a notice that is already a confirmed hold
    /// produces nothing, so this can be called on every output chunk.
    #[cfg(test)]
    pub(crate) fn observe_provider_quota_notice(
        &mut self,
        window_id: &str,
        screen: Option<&str>,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Vec<OutboundEvent> {
        let _project_scope = self.enter_window_log_scope(window_id);
        if self.provider_quota_holds.contains_key(window_id) {
            return Vec::new();
        }
        if !matches!(
            self.window_preset(window_id),
            Some(WindowPreset::Agent | WindowPreset::Claude | WindowPreset::Codex)
        ) {
            return Vec::new();
        }
        let notice = screen
            .and_then(|screen| {
                gwt_core::usage::limit_notice::detect_provider_limit_notice_with_clock(
                    screen,
                    || now.with_timezone(&chrono::Local),
                )
            })
            .filter(|notice| notice.provider == self.quota_provider_for_window(window_id));
        let Some(notice) = notice else {
            self.provider_quota_candidates.remove(window_id);
            self.released_provider_quota_notices.remove(window_id);
            return Vec::new();
        };
        if self.released_provider_quota_notices.get(window_id) == Some(&notice) {
            return Vec::new();
        }
        self.released_provider_quota_notices.remove(window_id);
        let agent_id = self.pane_agent_id(window_id);
        let first_seen = self
            .provider_quota_candidates
            .entry(window_id.to_string())
            .or_insert_with(|| super::ProviderQuotaCandidate { first_seen: now })
            .first_seen;
        // Issue #4908: telemetry cannot override a refusal. A live notice
        // still needs corroboration or the existing settle window below.
        let corroborated = agent_id.as_deref().is_some_and(|agent_id| {
            gwt::issue_monitor::provider_limit_reached_for_agent(
                agent_id,
                &self.provider_usage_accounts,
            )
        });
        let settled = now
            .signed_duration_since(first_seen)
            .num_seconds()
            .ge(&PROVIDER_QUOTA_SETTLE_SECS);
        if !corroborated && !settled {
            return Vec::new();
        }
        self.provider_quota_candidates.remove(window_id);
        // Millisecond precision so a hold formed right after an operator's
        // clear is ordered after that release instead of sharing its second.
        let evidence = gwt::IssueMonitorProviderQuotaHoldEvidence::screen_notice(
            &now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            window_id,
            screen.unwrap_or_default(),
        )
        .with_poller(agent_id.as_deref(), &self.provider_usage_accounts);
        self.commit_provider_quota_hold(window_id, &notice, agent_id.as_deref(), evidence)
    }

    /// Latch the block, project the pane as waiting, and tell the Monitor the
    /// slot is free.
    ///
    /// The slot is released even though the process may still be alive: the
    /// account is out for as long as the provider says, and holding a slot for
    /// days stops the whole queue at the default `max_active = 1`. The pane
    /// stays on screen with its session intact so the same conversation can be
    /// resumed after the reset.
    #[cfg(test)]
    fn commit_provider_quota_hold(
        &mut self,
        window_id: &str,
        notice: &gwt_core::usage::ProviderLimitNotice,
        agent_id: Option<&str>,
        evidence: gwt::IssueMonitorProviderQuotaHoldEvidence,
    ) -> Vec<OutboundEvent> {
        tracing::warn!(
            window_id = %window_id,
            agent_id = ?agent_id,
            screen_text = ?evidence.screen_text,
            screen_region = ?evidence.screen_region,
            matched_pattern = ?evidence.matched_pattern,
            poller_state = ?evidence.poller_state,
            poller_limit_reached = ?evidence.poller_limit_reached,
            poller_windows = ?evidence.poller_windows,
            "provider quota hold committed from the pane screen (Issue #3923)"
        );
        let failure = provider_usage_limit_failure(notice, agent_id, Some(evidence));
        let detail = gwt_core::usage::describe_provider_limit_notice(
            notice,
            Some(provider_label(notice, agent_id).as_str()),
        );
        self.provider_quota_holds
            .insert(window_id.to_string(), failure);
        self.window_details
            .insert(window_id.to_string(), detail.clone());
        let mut events = Vec::new();
        if let Some(composed) = self.recompute_window_state(window_id) {
            events.extend(self.status_events(window_id.to_string(), composed, Some(detail)));
        }
        events
    }

    /// Issue #4584: fold one live-screen observation into the API-error state
    /// for `window_id`.
    ///
    /// Both directions matter and both are immediate. An error that has just
    /// appeared stops the pane reading as `running` on this call, because the
    /// twenty minutes this Issue reports were spent waiting for some other
    /// mechanism to notice. An error that has left the screen releases the
    /// hold on this call, because output resuming is proof the agent is back
    /// — and that release is what keeps a momentary false reading from
    /// outliving the chunk that caused it.
    #[cfg(test)]
    pub(crate) fn observe_provider_api_error(
        &mut self,
        window_id: &str,
        screen: Option<&str>,
    ) -> Vec<OutboundEvent> {
        let _project_scope = self.enter_window_log_scope(window_id);
        if !matches!(
            self.window_preset(window_id),
            Some(WindowPreset::Agent | WindowPreset::Claude | WindowPreset::Codex)
        ) {
            return Vec::new();
        }
        let auth_agent = screen
            .and_then(gwt_core::usage::limit_notice::detect_provider_auth_notice)
            .filter(|provider| Some(*provider) == self.quota_provider_for_window(window_id))
            .map(|provider| match provider {
                gwt_core::usage::UsageProvider::Codex => "codex",
                gwt_core::usage::UsageProvider::ClaudeCode => "claude",
            });
        let mut events = Vec::new();
        if auth_agent.is_some() {
            self.provider_quota_holds.remove(window_id);
            self.provider_quota_candidates.remove(window_id);
        }
        // Issue #3616 owns this pane already, with a corroborated account and
        // a reset instant this reading does not have. Two holds for one pane
        // would only fight over its detail. An earlier API-error hold is
        // dropped rather than left behind: the quota hold now explains the
        // pane, and a stale entry would outlive it at teardown.
        if auth_agent.is_none() && self.provider_quota_holds.contains_key(window_id) {
            self.provider_api_error_holds.remove(window_id);
            return Vec::new();
        }
        let detected = auth_agent
            .map(|agent_id| gwt_core::usage::ProviderApiError {
                http_status: None,
                summary: gwt::issue_monitor::provider_reauthentication_required_message(agent_id),
                transient: false,
            })
            .or_else(|| screen.and_then(gwt_core::usage::detect_provider_api_error));
        match detected {
            Some(error) => {
                if self.provider_api_error_holds.get(window_id) == Some(&error) {
                    return events;
                }

                let detail = if auth_agent.is_some() {
                    error.summary.clone()
                } else {
                    gwt_core::usage::describe_provider_api_error(
                        &error,
                        self.pane_agent_id(window_id).as_deref(),
                    )
                };
                tracing::warn!(
                    window_id = %window_id,
                    http_status = ?error.http_status,
                    transient = error.transient,
                    summary = %error.summary,
                    "a provider API error ended this pane's turn; projecting it as waiting (Issue #4584)"
                );
                self.provider_api_error_holds
                    .insert(window_id.to_string(), error);
                self.window_details
                    .insert(window_id.to_string(), detail.clone());
                if let Some(composed) = self.recompute_window_state(window_id) {
                    events.extend(self.status_events(
                        window_id.to_string(),
                        composed,
                        Some(detail),
                    ));
                }
                events
            }
            None => {
                if self.provider_api_error_holds.remove(window_id).is_none() {
                    return Vec::new();
                }
                self.window_details.remove(window_id);
                let mut events = Vec::new();
                if let Some(composed) = self.recompute_window_state(window_id) {
                    events.extend(self.status_events(window_id.to_string(), composed, None));
                }
                events
            }
        }
    }

    #[cfg(test)]
    pub(super) fn quota_provider_for_window(
        &self,
        window_id: &str,
    ) -> Option<gwt_core::usage::UsageProvider> {
        use gwt::window_state::ApprovalPromptProvider;
        use gwt_core::usage::UsageProvider;
        match self.approval_prompt_provider(window_id) {
            ApprovalPromptProvider::Codex => Some(UsageProvider::Codex),
            ApprovalPromptProvider::ClaudeCode => Some(UsageProvider::ClaudeCode),
            ApprovalPromptProvider::Unsupported => None,
        }
    }

    /// Issue #3616: the agent this pane is running, live entry first and the
    /// persisted window second.
    ///
    /// PTY teardown removes the active session before the failure is published,
    /// so the persisted fallback is what keeps the account attributable at the
    /// moment it matters — the same ordering `approval_prompt_provider` uses.
    #[cfg(test)]
    pub(crate) fn pane_agent_id(&self, window_id: &str) -> Option<String> {
        self.active_agent_sessions
            .get(window_id)
            .map(|session| session.agent_id.clone())
            .or_else(|| {
                let address = self.window_lookup.get(window_id)?;
                self.tab(&address.tab_id)?
                    .workspace
                    .window(&address.raw_id)?
                    .agent_id
                    .clone()
            })
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

    /// Issue #4406 AC-4: time the dispatch and attribute it to a stage.
    ///
    /// The body keeps its own early returns, so the measurement is taken here
    /// rather than inside it — a dispatch that bails out early is exactly the
    /// one worth recording.
    fn handle_runtime_hook_event_inner(
        &mut self,
        event: gwt::RuntimeHookEvent,
        publish_to_daemon: bool,
    ) -> Vec<OutboundEvent> {
        let project_scope = self
            .active_window_for_runtime_event(&event)
            .as_deref()
            .and_then(|id| self.project_log_scope_for_window(id))
            .or_else(|| {
                event.project_root.as_deref().and_then(|root| {
                    self.tabs
                        .iter()
                        .find(|tab| tab.project_root == Path::new(root))
                        .and_then(|tab| self.project_log_scope_for_tab(&tab.id))
                })
            })
            .cloned();
        let _project_scope = project_scope
            .as_ref()
            .map(|scope| scope.enter())
            .unwrap_or_else(|| {
                tracing::trace_span!(target: "gwt_log_scope", parent: None, "machine_runtime_hook")
                    .entered()
            });
        let started = std::time::Instant::now();
        let source_event = runtime_hook_source_event_profile_label(event.source_event.as_deref());
        let mut stages = RuntimeHookStageTimings::default();
        let events = self.handle_runtime_hook_event_stages(event, publish_to_daemon, &mut stages);
        log_runtime_hook_stage_timing(started.elapsed().as_millis() as u64, source_event, &stages);
        events
    }

    fn handle_runtime_hook_event_stages(
        &mut self,
        event: gwt::RuntimeHookEvent,
        _publish_to_daemon: bool,
        stages: &mut RuntimeHookStageTimings,
    ) -> Vec<OutboundEvent> {
        let mut events = Vec::new();
        let window_id = self.active_window_for_runtime_event(&event);
        if Self::should_broadcast_runtime_hook_event_to_frontend(&event) {
            let key = window_id
                .as_deref()
                .and_then(|id| self.project_key_for_window(id))
                .cloned()
                .or_else(|| {
                    event.project_root.as_deref().and_then(|root| {
                        self.tabs
                            .iter()
                            .find(|tab| {
                                super::same_worktree_path(&tab.project_root, Path::new(root))
                            })
                            .and_then(|tab| self.project_key_for_tab(&tab.id).cloned())
                    })
                });
            if let Some(key) = key {
                let mut public = event.clone();
                public.continuation_readiness_nonce = None;
                events.push(OutboundEvent::project(
                    key,
                    BackendEvent::RuntimeHookEvent { event: public },
                ));
            }
        }
        let Some(window_id) = window_id else {
            return events;
        };
        if event.source_event.as_deref() == Some("UserPromptSubmit") {
            // An exact owner ACK can release retained input without a status edge.
            self.flush_pending_pm_wake(&window_id);
        }
        let Some(hook_state) = gwt::window_state::runtime_hook_window_state(&event) else {
            return events;
        };
        // This is a display cache. Authenticated readiness, Session and Monitor
        // writes have already been applied by the fixed project owner.
        self.window_hook_states
            .insert(window_id.clone(), hook_state);
        let status = self
            .runtimes
            .get(&window_id)
            .map(|runtime| runtime.status)
            .unwrap_or(hook_state);
        stages.measure("window_state_projection", || {
            events.extend(self.apply_owner_status_projection(&window_id, status, event.message));
        });
        events
    }

    fn should_broadcast_runtime_hook_event_to_frontend(event: &gwt::RuntimeHookEvent) -> bool {
        event.kind != gwt::RuntimeHookEventKind::RuntimeState
    }
}

const RUNTIME_DAEMON_PUBLISH_QUEUE_CAPACITY: usize = 4096;
enum RuntimeDaemonPublish {
    Output {
        project_root: PathBuf,
        id: String,
        data: Vec<u8>,
        preview_text: Option<String>,
    },
    #[cfg(test)]
    Status {
        project_root: PathBuf,
        id: String,
        status: WindowProcessStatus,
        detail: Option<String>,
    },
}
#[derive(Debug)]
struct RuntimeDaemonApprovalPublish {
    project_root: PathBuf,
    id: String,
    waiting: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeDaemonPublishEnqueueError {
    Full,
    Disconnected,
}
static RUNTIME_DAEMON_PUBLISH_QUEUE: std::sync::OnceLock<
    Mutex<Option<std_mpsc::SyncSender<RuntimeDaemonPublish>>>,
> = std::sync::OnceLock::new();
static RUNTIME_DAEMON_APPROVAL_PUBLISH_QUEUE: std::sync::OnceLock<
    Mutex<Option<std_mpsc::Sender<RuntimeDaemonApprovalPublish>>>,
> = std::sync::OnceLock::new();
fn runtime_daemon_publish_sender() -> Option<std_mpsc::SyncSender<RuntimeDaemonPublish>> {
    let queue = RUNTIME_DAEMON_PUBLISH_QUEUE.get_or_init(|| Mutex::new(None));
    runtime_daemon_publish_sender_from(queue, |receiver| {
        std::thread::Builder::new()
            .name("gwt-runtime-daemon-publish-worker".to_string())
            .spawn(move || run_runtime_daemon_publish_worker(receiver))
            .map(|_handle| ())
    })
}
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
fn run_runtime_daemon_publish_worker(receiver: std_mpsc::Receiver<RuntimeDaemonPublish>) {
    let Ok(mut publisher) = gwt::daemon_publisher::PersistentEventPublisher::new() else {
        tracing::warn!("runtime daemon publisher initialization failed");
        return;
    };
    for publish in receiver {
        publish_runtime_daemon_event(&mut publisher, publish);
    }
}
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
fn run_runtime_daemon_approval_publish_worker(
    receiver: std_mpsc::Receiver<RuntimeDaemonApprovalPublish>,
) {
    let Ok(mut publisher) = gwt::daemon_publisher::PersistentEventPublisher::new() else {
        tracing::warn!("runtime daemon approval publisher initialization failed");
        return;
    };
    for publish in receiver {
        publish_runtime_daemon_approval_event(&mut publisher, publish);
    }
}
fn try_enqueue_runtime_daemon_publish(
    sender: &std_mpsc::SyncSender<RuntimeDaemonPublish>,
    publish: RuntimeDaemonPublish,
) -> Result<(), RuntimeDaemonPublishEnqueueError> {
    sender.try_send(publish).map_err(|err| match err {
        std_mpsc::TrySendError::Full(_) => RuntimeDaemonPublishEnqueueError::Full,
        std_mpsc::TrySendError::Disconnected(_) => RuntimeDaemonPublishEnqueueError::Disconnected,
    })
}
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
fn publish_runtime_daemon_event(
    publisher: &mut gwt::daemon_publisher::PersistentEventPublisher,
    publish: RuntimeDaemonPublish,
) {
    match publish {
        RuntimeDaemonPublish::Output {
            project_root,
            id,
            data,
            preview_text,
        } => {
            let payload = gwt::runtime_daemon_events::runtime_output_payload(
                &id,
                &data,
                std::process::id(),
                preview_text.as_deref(),
            );
            let result = publisher.publish(
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
        #[cfg(test)]
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
            let result = publisher.publish(
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
    }
}
fn publish_runtime_daemon_approval_event(
    publisher: &mut gwt::daemon_publisher::PersistentEventPublisher,
    publish: RuntimeDaemonApprovalPublish,
) {
    let payload = gwt::runtime_daemon_events::runtime_approval_overlay_payload(
        &publish.id,
        publish.waiting,
        std::process::id(),
    );
    let result = publisher.publish(
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
fn publish_runtime_output_change(
    project_root: &Path,
    id: &str,
    data: &[u8],
    mut preview_text: Option<String>,
) {
    // Keep the original 4KiB-per-slot queue bound even during a large burst.
    // The frames are enqueued together at the batch deadline, in byte order;
    // only the last carries the shared screen preview.
    let mut chunks = data.chunks(4096).peekable();
    while let Some(chunk) = chunks.next() {
        enqueue_runtime_daemon_publish(RuntimeDaemonPublish::Output {
            project_root: project_root.to_path_buf(),
            id: id.to_string(),
            data: chunk.to_vec(),
            preview_text: if chunks.peek().is_none() {
                preview_text.take()
            } else {
                None
            },
        });
    }
}

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

#[cfg(test)]
mod tests {
    use super::{
        runtime_daemon_approval_publish_sender_from, runtime_daemon_publish_sender_from,
        try_enqueue_runtime_daemon_publish, RuntimeDaemonApprovalPublish, RuntimeDaemonPublish,
        RuntimeDaemonPublishEnqueueError,
    };
    use crate::WindowProcessStatus;
    use std::path::PathBuf;
    use std::sync::{mpsc, Mutex};

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
                preview_text: None,
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
                preview_text: None,
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
