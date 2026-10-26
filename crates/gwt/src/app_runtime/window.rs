//! Window lifecycle and focus on the active project.
//!
//! Rail, Agents tabs and split layout own presentation. The registry holds
//! identity, metadata and process state; xterm's UpdateTerminalGrid is the
//! only source of live PTY dimensions.

use gwt::{AgentKanbanLane, FocusCycleDirection, WindowSurfaceOwner};

use super::{
    close_window_from_workspace, AppRuntime, BackendEvent, OutboundEvent, WindowPreset,
    WindowProcessStatus,
};

/// One uniform view over the window-keyed containers listed in
/// [`window_scoped_state`], so the release path and the residue probe can be
/// generated from the same list regardless of map or set.
trait WindowScopedEntries {
    fn forget(&mut self, id: &str);
    #[cfg(test)]
    fn holds(&self, id: &str) -> bool;
    #[cfg(test)]
    fn entry_count(&self) -> usize;
}

impl<V> WindowScopedEntries for std::collections::HashMap<String, V> {
    fn forget(&mut self, id: &str) {
        self.remove(id);
    }

    #[cfg(test)]
    fn holds(&self, id: &str) -> bool {
        self.contains_key(id)
    }

    #[cfg(test)]
    fn entry_count(&self) -> usize {
        self.len()
    }
}

impl WindowScopedEntries for std::collections::HashSet<String> {
    fn forget(&mut self, id: &str) {
        self.remove(id);
    }

    #[cfg(test)]
    fn holds(&self, id: &str) -> bool {
        self.contains(id)
    }

    #[cfg(test)]
    fn entry_count(&self) -> usize {
        self.len()
    }
}

fn shares_work_surface_singleton(preset: WindowPreset) -> bool {
    matches!(preset, WindowPreset::Work | WindowPreset::Branches)
}

/// Result of a close request (Issue #3629 AC-10): `closed` reports whether the
/// workspace record was actually removed, so callers can answer the requester
/// instead of treating an empty event list as success.
pub(crate) struct CloseWindowOutcome {
    pub(crate) closed: bool,
    pub(crate) events: Vec<OutboundEvent>,
}

pub(crate) struct PeerPaneCloseReply {
    pub(super) client_id: String,
}

/// Issue #4234 AC-3: every `AppRuntime` map that is keyed by a combined window
/// id, named once so the release path and the residue probe below can never
/// drift apart.
///
/// Adding a window-keyed map means adding one arm here. Both
/// [`AppRuntime::forget_window_scoped_state`] and
/// [`AppRuntime::window_scoped_state_residue`] are generated from this list, so
/// a map that is registered is both released on close and measured by the
/// regression test.
macro_rules! window_scoped_state {
    ($runtime:ident, $id:ident, $visit:ident) => {
        // Removed by the close path itself; listed so the probe proves it.
        $visit!($runtime, $id, runtimes);
        $visit!($runtime, $id, active_agent_sessions);
        $visit!($runtime, $id, agent_capability_tokens);
        $visit!($runtime, $id, window_details);
        $visit!($runtime, $id, window_lookup);
        $visit!($runtime, $id, profile_selections);
        $visit!($runtime, $id, restore_launch_windows);
        $visit!($runtime, $id, issue_monitor_review_dispatch_windows);
        // Released only on specific relaunch / exit routes before Issue #4234,
        // so an ordinary close left one entry per window behind for the life of
        // the process.
        $visit!($runtime, $id, launch_error_terminal_details);
        $visit!($runtime, $id, board_all_view_windows);
        $visit!($runtime, $id, pending_workspace_resume_contexts);
        $visit!($runtime, $id, pending_launch_feedback_contexts);
        $visit!($runtime, $id, runtime_hook_agent_failures_in_flight);
        $visit!($runtime, $id, initial_launch_workers);
        $visit!($runtime, $id, pending_launch_completions);
        $visit!($runtime, $id, pending_launch_delivery_acks);
        $visit!($runtime, $id, pending_continue_work);
        $visit!($runtime, $id, pending_fresh_execution_launches);
        $visit!($runtime, $id, pending_auto_resume_sources);
        $visit!($runtime, $id, terminal_close_candidates);
        $visit!($runtime, $id, window_pty_statuses);
        $visit!($runtime, $id, window_output_bytes);
        $visit!($runtime, $id, pending_runtime_outputs);
        $visit!($runtime, $id, window_last_output_at);
        $visit!($runtime, $id, window_hook_states);
        $visit!($runtime, $id, window_approval_waiting);
        $visit!($runtime, $id, recoverable_agent_error_windows);
        $visit!($runtime, $id, provider_quota_holds);
        $visit!($runtime, $id, provider_quota_candidates);
        $visit!($runtime, $id, released_provider_quota_notices);
        $visit!($runtime, $id, provider_api_error_holds);
        $visit!($runtime, $id, last_agent_activity);
        $visit!($runtime, $id, last_issue_monitor_heartbeat);
    };
}

impl AppRuntime {
    pub(crate) fn create_window_events(
        &mut self,
        context: &super::ProjectContext,
        preset: WindowPreset,
        surface_owner: Option<WindowSurfaceOwner>,
    ) -> Vec<OutboundEvent> {
        if preset.is_removed_legacy() {
            return Vec::new();
        }
        if !self.project_context_is_current(context) {
            return Vec::new();
        }
        let tab_id = context.tab_id.clone();
        let canonical_preset = preset.canonical_surface_preset();
        let surface_owner = surface_owner
            .filter(|owner| *owner != WindowSurfaceOwner::Pm || !preset.is_agent_terminal());
        if shares_work_surface_singleton(preset)
            || (canonical_preset == WindowPreset::Settings
                && surface_owner == Some(WindowSurfaceOwner::Pm))
        {
            let existing_id = {
                let Some(tab) = self.tab_mut(&tab_id) else {
                    return Vec::new();
                };
                let existing_id = tab
                    .workspace
                    .persisted()
                    .windows
                    .iter()
                    .filter(|window| {
                        window.surface_owner == surface_owner
                            && window.preset.canonical_surface_preset() == canonical_preset
                    })
                    .max_by_key(|window| window.z_index)
                    .map(|window| window.id.clone());
                if let Some(existing_id) = existing_id.as_deref() {
                    let _ = tab.workspace.activate_window_tab(existing_id);
                    let _ = tab.workspace.focus_window(existing_id);
                }
                existing_id
            };
            if existing_id.is_some() {
                return self.activate_tab_for_window_events(tab_id);
            }
        }
        let preset = canonical_preset;
        let window = {
            let Some(tab) = self.tab_mut(&tab_id) else {
                return Vec::new();
            };
            let window = tab.workspace.add_window(preset);
            tab.workspace.set_surface_owner(&window.id, surface_owner);
            window
        };
        self.register_window(&tab_id, &window.id);
        let runtime_events = self.start_window(&tab_id, &window.id, window.preset);
        let _ = self.persist();
        let mut events = vec![self.workspace_state_broadcast(context)];
        events.extend(runtime_events);
        events
    }

    pub(crate) fn focus_window_events(&mut self, id: &str) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id).cloned() else {
            return Vec::new();
        };
        let Some(tab) = self.tab_mut(&address.tab_id) else {
            return Vec::new();
        };
        if !tab.workspace.focus_window(&address.raw_id) {
            return Vec::new();
        }
        self.activate_tab_for_window_events(address.tab_id)
    }

    fn activate_tab_for_window_events(&mut self, tab_id: String) -> Vec<OutboundEvent> {
        let Some(context) = self.project_context(&tab_id) else {
            return Vec::new();
        };
        let _ = self.persist();
        vec![self.workspace_state_broadcast(&context)]
    }

    pub(crate) fn cycle_focus_events(
        &mut self,
        context: &super::ProjectContext,
        direction: FocusCycleDirection,
    ) -> Vec<OutboundEvent> {
        if !self.project_context_is_current(context) {
            return Vec::new();
        }
        let tab_id = context.tab_id.clone();
        let focused = {
            let Some(tab) = self.tab_mut(&tab_id) else {
                return Vec::new();
            };
            tab.workspace.cycle_focus(direction)
        };
        if focused.is_none() {
            return Vec::new();
        }
        // Focus never changes the frontend-owned terminal grid.
        let _ = self.persist();
        vec![self.workspace_state_broadcast(context)]
    }

    pub(crate) fn activate_window_tab_events(&mut self, id: &str) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id).cloned() else {
            return Vec::new();
        };
        let updated = {
            let Some(tab) = self.tab_mut(&address.tab_id) else {
                return Vec::new();
            };
            tab.workspace.activate_window_tab(&address.raw_id)
        };
        if !updated {
            return Vec::new();
        }
        // Activation records recency without changing the terminal grid.
        self.activate_tab_for_window_events(address.tab_id)
    }

    pub(crate) fn place_agent_window_in_kanban_events(
        &mut self,
        id: &str,
        board_id: &str,
        lane_id: AgentKanbanLane,
        order: Option<u32>,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id).cloned() else {
            return Vec::new();
        };
        let Some(board_address) = self.window_lookup.get(board_id).cloned() else {
            return Vec::new();
        };
        if address.tab_id != board_address.tab_id {
            return Vec::new();
        }
        let updated = {
            let Some(tab) = self.tab_mut(&address.tab_id) else {
                return Vec::new();
            };
            tab.workspace.place_agent_window_in_kanban(
                &address.raw_id,
                &board_address.raw_id,
                lane_id,
                order,
            )
        };
        if !updated {
            return Vec::new();
        }
        self.activate_tab_for_window_events(address.tab_id)
    }

    pub(crate) fn move_agent_kanban_card_events(
        &mut self,
        id: &str,
        board_id: &str,
        lane_id: AgentKanbanLane,
        order: u32,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id).cloned() else {
            return Vec::new();
        };
        let Some(board_address) = self.window_lookup.get(board_id).cloned() else {
            return Vec::new();
        };
        if address.tab_id != board_address.tab_id {
            return Vec::new();
        }
        let updated = {
            let Some(tab) = self.tab_mut(&address.tab_id) else {
                return Vec::new();
            };
            tab.workspace.move_agent_kanban_card(
                &address.raw_id,
                &board_address.raw_id,
                lane_id,
                order,
            )
        };
        if !updated {
            return Vec::new();
        }
        self.activate_tab_for_window_events(address.tab_id)
    }

    pub(crate) fn undock_agent_window_events(&mut self, id: &str) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id).cloned() else {
            return Vec::new();
        };
        let updated = {
            let Some(tab) = self.tab_mut(&address.tab_id) else {
                return Vec::new();
            };
            tab.workspace.undock_agent_window(&address.raw_id)
        };
        if !updated {
            return Vec::new();
        }
        self.activate_tab_for_window_events(address.tab_id)
    }

    /// SPEC-3885 FR-012: return a Windowized Issue window to the Issue list. The
    /// registry resolves both the Issue and its host window, so a stale frontend cannot
    /// send the window somewhere it does not belong.
    pub(crate) fn dock_agent_window_to_issue_events(&mut self, id: &str) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id).cloned() else {
            return Vec::new();
        };
        let Some(project_key) = self.project_key_for_window(id).cloned() else {
            return Vec::new();
        };
        let updated = {
            let Some(tab) = self.tab_mut(&address.tab_id) else {
                return Vec::new();
            };
            let issue_number = tab
                .workspace
                .window(&address.raw_id)
                .and_then(|window| window.linked_issue_number);
            tab.workspace
                .dock_agent_window_to_issue(&address.raw_id)
                .map_err(|reason| (reason, issue_number))
        };
        if let Err((reason, issue_number)) = updated {
            return vec![OutboundEvent::project(
                project_key,
                BackendEvent::IssueMonitorToast {
                    notification_transition: None,
                    notification_transition_id: None,
                    level: "warn".to_string(),
                    message: format!("Cannot return to list: {reason}."),
                    issue_number,
                },
            )];
        }
        self.activate_tab_for_window_events(address.tab_id)
    }

    pub(crate) fn set_agent_kanban_card_collapsed_events(
        &mut self,
        id: &str,
        collapsed: bool,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id).cloned() else {
            return Vec::new();
        };
        let updated = {
            let Some(tab) = self.tab_mut(&address.tab_id) else {
                return Vec::new();
            };
            tab.workspace
                .set_agent_kanban_card_collapsed(&address.raw_id, collapsed)
        };
        if !updated {
            return Vec::new();
        }
        self.activate_tab_for_window_events(address.tab_id)
    }

    pub(crate) fn update_terminal_grid_events(
        &mut self,
        id: &str,
        cols: u16,
        rows: u16,
    ) -> Vec<OutboundEvent> {
        let Some(runtime) = self.runtimes.get(id) else {
            return Vec::new();
        };
        self.queue_remote_runtime_command(
            id,
            gwt::project_runtime::events::OwnerCommand::Resize {
                id: runtime.owner_pane_id.clone(),
                incarnation: runtime.incarnation,
                cols: cols.max(20),
                rows: rows.max(6),
            },
            super::RemoteRuntimeCommandKind::Resize,
        )
    }

    pub(crate) fn close_window_events(&mut self, id: &str) -> Vec<OutboundEvent> {
        self.close_window_outcome(id).events
    }

    /// Close a window and report whether the workspace record was actually
    /// removed (Issue #3629 AC-10): the agent pane route must be able to
    /// distinguish a landed close from the silent no-op that an unknown or
    /// already-closed window produces.
    pub(crate) fn close_window_outcome(&mut self, id: &str) -> CloseWindowOutcome {
        self.close_window_outcome_with_monitor_notification(id, true, None, None)
    }

    /// Issue #4234 AC-3: drop every window-keyed entry the closing window owns.
    ///
    /// Window ids are reassigned lowest-free, so a surviving entry is not only
    /// retained memory — it is state a future window with the same id would
    /// read as its own.
    pub(super) fn forget_window_scoped_state(&mut self, id: &str) {
        macro_rules! forget {
            ($runtime:ident, $id:ident, $field:ident) => {
                WindowScopedEntries::forget(&mut $runtime.$field, $id);
            };
        }
        window_scoped_state!(self, id, forget);
        for state in self.project_states.values_mut() {
            state.pending_pm_wakes.remove(id);
            state.pending_pm_launches.remove(id);
        }
    }

    /// Issue #4234 AC-3 / AC-4: names of the window-keyed maps that still hold
    /// `id`. Empty after a close; the regression test reads it directly instead
    /// of inferring release from a heap measurement.
    #[cfg(test)]
    pub(crate) fn window_scoped_state_residue(&self, id: &str) -> Vec<&'static str> {
        let mut residue = Vec::new();
        macro_rules! probe {
            ($runtime:ident, $id:ident, $field:ident) => {
                if WindowScopedEntries::holds(&$runtime.$field, $id) {
                    residue.push(stringify!($field));
                }
            };
        }
        window_scoped_state!(self, id, probe);
        for state in self.project_states.values() {
            if state.pending_pm_wakes.contains_key(id) {
                residue.push("pending_pm_wakes");
            }
            if state.pending_pm_launches.contains_key(id) {
                residue.push("pending_pm_launches");
            }
        }
        residue
    }

    /// Issue #4234 AC-4: total entries held across every window-keyed map. The
    /// open/close regression fixes this number rather than a heap reading, so
    /// it measures retention instead of allocator behaviour.
    #[cfg(test)]
    pub(crate) fn window_scoped_state_entry_count(&self) -> usize {
        let mut total = 0usize;
        macro_rules! count {
            ($runtime:ident, $id:ident, $field:ident) => {
                let _ = $id;
                total += WindowScopedEntries::entry_count(&$runtime.$field);
            };
        }
        let id = "";
        window_scoped_state!(self, id, count);
        total += self
            .project_states
            .values()
            .map(|state| state.pending_pm_wakes.len() + state.pending_pm_launches.len())
            .sum::<usize>();
        total
    }

    /// Close a window whose Issue Monitor lifecycle transition was already
    /// committed by the daemon or the fail-closed local fallback. Re-entering
    /// the normal window-closed hook here would publish a second control for
    /// the same lifecycle edge and could requeue the just-failed issue.
    pub(crate) fn close_window_after_issue_monitor_finalize_events(
        &mut self,
        id: &str,
    ) -> Vec<OutboundEvent> {
        self.close_window_outcome_with_monitor_notification(id, false, None, None)
            .events
    }

    fn close_window_outcome_with_monitor_notification(
        &mut self,
        id: &str,
        notify_issue_monitor: bool,
        self_close_ticket: Option<crate::AgentSelfCloseCapabilityTicket>,
        peer_client_id: Option<String>,
    ) -> CloseWindowOutcome {
        // Issue #4145 AC-1: every close route converges here, and Issue #3783
        // designed the accepted close to stay on the event loop, so this guard
        // measures exactly the latency a person sees when a pane disappears.
        // The detached teardown that follows is deliberately outside it.
        let _perf_route = gwt::perf::RouteTimer::start(gwt::perf::PerfRoute::PaneClose);
        let Some(context) = self
            .window_lookup
            .get(id)
            .and_then(|address| self.project_context(&address.tab_id))
            .or_else(|| {
                self.tabs
                    .iter()
                    .find(|tab| {
                        id.strip_prefix(&format!("{}::", tab.id))
                            .is_some_and(|raw_id| tab.workspace.window(raw_id).is_some())
                    })
                    .and_then(|tab| self.project_context(&tab.id))
            })
        else {
            return CloseWindowOutcome {
                closed: false,
                events: Vec::new(),
            };
        };
        let issue_monitor_project_root = self.issue_monitor_project_root_for_window(id);
        let peer_reply = peer_client_id.and_then(|client_id| {
            let session_id = self
                .active_agent_sessions
                .get(id)
                .map(|active| active.session_id.clone())
                .or_else(|| {
                    let raw_id = id.strip_prefix(&format!("{}::", context.tab_id))?;
                    self.tab(&context.tab_id)?
                        .workspace
                        .window(raw_id)?
                        .session_id
                        .clone()
                });
            (self.runtimes.contains_key(id) || session_id.is_some())
                .then_some(PeerPaneCloseReply { client_id })
        });
        if !close_window_from_workspace(
            &mut self.tabs,
            &mut self.window_lookup,
            &mut self.window_details,
            id,
        ) {
            return CloseWindowOutcome {
                closed: false,
                events: Vec::new(),
            };
        }
        // Issue #4084: the review-dispatch marker dies with its window.
        self.invalidate_workspace_projection_patch(&context.tab_id);
        self.issue_monitor_review_dispatch_windows.remove(id);
        // Issue #4143 (AC-3): window ids are reassigned lowest-free, so an
        // in-flight restore marker must not outlive its window.
        self.record_restore_window_outcome(id, Err(super::startup::RestoreRefusal::WindowClosed));
        self.restore_launch_windows.remove(id);
        // Issue #3783: the accepted close is the in-memory removal above.
        // Everything that may wait on PTY, execution, Session, or Work locks
        // runs in one detached finalizer. Peer completion is sent after reap;
        // visible removal and the event loop remain immediate.
        self.queue_accepted_window_close_finalizer(
            id,
            issue_monitor_project_root,
            notify_issue_monitor,
            self_close_ticket,
            peer_reply,
        );
        // Issue #4234 AC-3: the finalizer above takes the runtime, the session
        // and the capability token because it needs their values. Everything
        // else keyed by this window id is released here, after the finalizer
        // has captured what it owns.
        self.forget_window_scoped_state(id);
        let _ = self.persist();
        let mut events = vec![self.workspace_state_broadcast(&context)];
        if let Some(event) = self.cached_active_work_projection_broadcast_for_tab(&context.tab_id) {
            events.push(event);
        }
        CloseWindowOutcome {
            closed: true,
            events,
        }
    }

    /// Move an already-accepted window lifecycle into its detached finalizer.
    /// Callers must first remove the window from the visible workspace, but
    /// must leave runtime/session ownership intact for this method to capture.
    pub(super) fn queue_accepted_window_close_finalizer(
        &mut self,
        id: &str,
        project_root: Option<std::path::PathBuf>,
        notify_issue_monitor: bool,
        self_close_ticket: Option<crate::AgentSelfCloseCapabilityTicket>,
        peer_reply: Option<PeerPaneCloseReply>,
    ) {
        self.invalidate_launch_delivery_ack(id);
        self.pending_launch_completions.remove(id);
        // SPEC-3431 FR-013: snapshot the closing window's session while the
        // active entry still exists — an explicit close of the PM pane is an
        // intentional stop and clears the durable PM registration below.
        let closing_session_id = self
            .active_agent_sessions
            .get(id)
            .map(|session| session.session_id.clone());
        let closing_window_generation = self
            .window_lifecycle_generations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(id)
            .copied();
        // PR #3787 review: the fence must count only closes of the registered
        // PM session — any other agent pane close in the project would
        // otherwise suppress PM ensure (Automatic/Explicit) and crash respawn
        // through `pending_pm_closes` until its finalizer completes.
        let closing_pm_session = match (project_root.as_ref(), closing_session_id.as_ref()) {
            (Some(project_root), Some(session_id)) => {
                self.pm_session_for_root(project_root) == Some(session_id)
            }
            _ => false,
        };
        if closing_pm_session {
            if let Some(project_root) = project_root.as_ref() {
                if let Some(state) = self.project_state_for_root_mut(project_root) {
                    *state
                        .pending_pm_closes
                        .entry(project_root.clone())
                        .or_default() += 1;
                    state.pm_sessions.remove(project_root);
                }
            }
        }
        self.queue_window_close_finalizer(
            id,
            project_root,
            closing_session_id,
            closing_pm_session,
            notify_issue_monitor,
            self_close_ticket,
            closing_window_generation,
            peer_reply,
        );
        self.profile_selections.remove(id);
    }

    /// SPEC-2356 安心 Addendum (FR-041): stop a single window's agent runtime
    /// (kill the PTY through the existing stop path) but keep the window and its
    /// terminal output in its surface, rendered as `Stopped`. Unlike
    /// [`Self::close_window_events`], the window record is never removed, so the
    /// operator keeps the prior output and can later relaunch it through
    /// [`Self::restart_window_events`]. Idempotent: stopping an already-stopped
    /// window simply re-broadcasts the authoritative `Stopped` status.
    pub(crate) fn stop_window_events(&mut self, id: &str) -> Vec<OutboundEvent> {
        let Some(runtime) = self.runtimes.get(id) else {
            return Vec::new();
        };
        self.queue_remote_runtime_command(
            id,
            gwt::project_runtime::events::OwnerCommand::Close {
                id: runtime.owner_pane_id.clone(),
                incarnation: runtime.incarnation,
            },
            super::RemoteRuntimeCommandKind::Stop,
        )
    }

    /// SPEC-2356 安心 Addendum (FR-042): stop every running agent window's
    /// runtime, keeping all windows in its surface. Iterates the open windows
    /// and applies the per-window [`Self::stop_window_events`] stop to each
    /// window that currently has a live runtime, so the confirm-on-frontend
    /// "stop all" button resolves to the same kept-but-stopped state as the
    /// single-window kill switch.
    pub(crate) fn stop_all_windows_events(
        &mut self,
        context: &super::ProjectContext,
    ) -> Vec<OutboundEvent> {
        if !self.project_context_is_current(context) {
            return Vec::new();
        }
        let running_ids: Vec<String> = self
            .runtimes
            .keys()
            .filter(|id| self.project_key_for_window(id) == Some(&context.project_key))
            .cloned()
            .collect();
        let mut events = Vec::new();
        for window_id in running_ids {
            events.extend(self.stop_window_events(&window_id));
        }
        events
    }

    /// SPEC-2356 安心 Addendum (FR-044): relaunch a window's agent in place. The
    /// window id and Issue association are preserved; the prior terminal
    /// output is appended to (never wiped) because the window record — and the
    /// frontend's xterm buffer bound to its id — survive the restart. Only acts
    /// on a window that is currently `Stopped` or `Error`; restarting a running
    /// window is a no-op so a live agent is never double-spawned.
    ///
    /// Process-spec presets (`Shell` / `Claude` / `Codex`) re-spawn through the
    /// existing [`Self::start_window`] primitive. `Agent` windows re-spawn
    /// through the persisted Session (the same resume path the Workspace /
    /// Board resume buttons use) bound to the existing window id.
    pub(crate) fn restart_window_events(&mut self, id: &str) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id).cloned() else {
            return Vec::new();
        };
        let Some(context) = self.project_context(&address.tab_id) else {
            return Vec::new();
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return Vec::new();
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return Vec::new();
        };
        if !window.preset.requires_process() {
            return Vec::new();
        }
        // Idempotency / safety guard: only relaunch a window that is actually
        // stopped or errored. A running agent must never be double-spawned.
        match self.window_status(id) {
            Some(WindowProcessStatus::Stopped) | Some(WindowProcessStatus::Error) => {}
            _ => return Vec::new(),
        }

        let preset = window.preset;

        if preset == WindowPreset::Agent {
            return self.restart_agent_window_in_place(&address.tab_id, &address.raw_id);
        }

        // Clear any stale "restored window is paused" detail before relaunching
        // so the new PTY's status is the authoritative one.
        self.window_details.remove(id);
        let events = self.start_window(&address.tab_id, &address.raw_id, preset);
        let _ = self.persist();
        let mut all = vec![self.workspace_state_broadcast(&context)];
        all.extend(events);
        all
    }

    pub(crate) fn list_windows_event_for_project(
        &self,
        context: &super::ProjectContext,
    ) -> BackendEvent {
        let windows = if self.project_context_is_current(context) {
            self.tab(&context.tab_id)
                .map(|tab| self.workspace_view_for_tab(tab).windows)
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        BackendEvent::WindowList { windows }
    }

    #[cfg(test)]
    pub(crate) fn list_windows_event(&self) -> BackendEvent {
        // SPEC-3038 (2026-06-20): the Windows popover lists windows from every
        // project tab so it matches the cross-tab open-window badge. Windows
        // carry combined ids, so focusing a non-active-tab entry switches tabs
        // via `focus_window_events`.
        let windows = self
            .tabs
            .iter()
            .flat_map(|tab| self.workspace_view_for_tab(tab).windows)
            .collect();
        BackendEvent::WindowList { windows }
    }
}

#[cfg(test)]
mod singleton_tests {
    use super::*;

    #[test]
    fn work_surface_singleton_excludes_multi_instance_terminal_presets() {
        assert!(shares_work_surface_singleton(WindowPreset::Work));
        assert!(shares_work_surface_singleton(WindowPreset::Branches));
        assert!(!shares_work_surface_singleton(WindowPreset::Agent));
        assert!(!shares_work_surface_singleton(WindowPreset::Shell));
        assert!(!shares_work_surface_singleton(WindowPreset::Claude));
        assert!(!shares_work_surface_singleton(WindowPreset::Codex));
    }
}
