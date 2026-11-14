//! Floating window handler split out of `app_runtime/mod.rs` for SPEC-2077
//! Phase C (arch-review handoff, 2026-05-01).
//!
//! Owns:
//! - [`AppRuntime::create_window_events`] / [`AppRuntime::close_window_events`]
//!   — window lifecycle on the active project tab
//! - [`AppRuntime::focus_window_events`] / [`AppRuntime::cycle_focus_events`]
//!   — keyboard / pointer focus rotation
//! - [`AppRuntime::update_viewport_events`] /
//!   [`AppRuntime::arrange_windows_events`] — viewport pan/zoom and arrange
//!   tile / stack
//! - [`AppRuntime::update_window_geometry_events`] — per-window geometry
//!   adjustments (delegates terminal resize to the live runtime)
//! - [`AppRuntime::list_windows_event`] — workspace snapshot across all
//!   project tabs (the Command Rail Windows popover is a cross-tab list)
//!
//! GUI-side window contracts (canvas viewport pan/zoom, arrange/stack
//! semantics) remain owned by SPEC-2008. This split keeps `mod.rs` lean
//! while preserving the broadcast/persist contract through the existing
//! `workspace_state_broadcast` and `persist` helpers on `AppRuntime`.

use gwt::{AgentKanbanLane, ArrangeMode, CanvasViewport, FocusCycleDirection};

use super::{
    close_window_from_workspace, AppRuntime, BackendEvent, OutboundEvent, WindowGeometry,
    WindowPreset, WindowProcessStatus,
};

fn shares_work_surface_singleton(preset: WindowPreset) -> bool {
    matches!(preset, WindowPreset::Work | WindowPreset::Branches)
}

#[derive(Debug, Clone)]
struct ModernMonitorCloseExecutionTarget {
    worktree_path: std::path::PathBuf,
    owner: gwt::cli::execution_state::ExecutionOwnerKey,
    session: gwt_agent::SessionExecutionIdentity,
    runtime_incarnation: u64,
    status: gwt::cli::execution_state::ExecutionControlStatus,
}

impl AppRuntime {
    pub(crate) fn create_window_events(
        &mut self,
        preset: WindowPreset,
        bounds: WindowGeometry,
    ) -> Vec<OutboundEvent> {
        if preset.is_removed_legacy() {
            return Vec::new();
        }
        let Some(tab_id) = self.active_tab_id.clone() else {
            return Vec::new();
        };
        if shares_work_surface_singleton(preset) {
            let existing_id = {
                let Some(tab) = self.tab_mut(&tab_id) else {
                    return Vec::new();
                };
                let existing_id = tab
                    .workspace
                    .persisted()
                    .windows
                    .iter()
                    .filter(|window| shares_work_surface_singleton(window.preset))
                    .max_by_key(|window| window.z_index)
                    .map(|window| window.id.clone());
                if let Some(existing_id) = existing_id.as_deref() {
                    let _ = tab.workspace.activate_window_tab(existing_id);
                    let _ = tab
                        .workspace
                        .focus_window(existing_id, Some(bounds.clone()));
                }
                existing_id
            };
            if existing_id.is_some() {
                return self.activate_tab_for_window_events(tab_id);
            }
        }
        let window = {
            let Some(tab) = self.tab_mut(&tab_id) else {
                return Vec::new();
            };
            tab.workspace.add_window(preset, bounds.clone())
        };
        self.register_window(&tab_id, &window.id);
        let runtime_events = self.start_window(&tab_id, &window.id, window.preset, window.geometry);
        let _ = self.persist();
        let mut events = vec![self.workspace_state_broadcast()];
        events.extend(runtime_events);
        events
    }

    pub(crate) fn focus_window_events(
        &mut self,
        id: &str,
        bounds: Option<WindowGeometry>,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id).cloned() else {
            return Vec::new();
        };
        let Some(tab) = self.tab_mut(&address.tab_id) else {
            return Vec::new();
        };
        if !tab.workspace.focus_window(&address.raw_id, bounds) {
            return Vec::new();
        }
        self.activate_tab_for_window_events(address.tab_id)
    }

    fn activate_tab_for_window_events(&mut self, tab_id: String) -> Vec<OutboundEvent> {
        let previous_tab_id = self.active_tab_id.clone();
        let wizard_closed = self.set_active_tab(tab_id);
        let tab_changed = self.active_tab_id != previous_tab_id;
        let _ = self.persist();
        let mut events = vec![self.workspace_state_broadcast()];
        if tab_changed {
            if let Some(event) = self.active_work_projection_broadcast_on_tab_change() {
                events.push(event);
            }
        }
        if wizard_closed {
            events.push(self.launch_wizard_state_broadcast(None));
        }
        events
    }

    pub(crate) fn cycle_focus_events(
        &mut self,
        direction: FocusCycleDirection,
        bounds: WindowGeometry,
    ) -> Vec<OutboundEvent> {
        let Some(tab_id) = self.active_tab_id.clone() else {
            return Vec::new();
        };
        let focused = {
            let Some(tab) = self.tab_mut(&tab_id) else {
                return Vec::new();
            };
            tab.workspace.cycle_focus(direction, bounds)
        };
        if focused.is_none() {
            return Vec::new();
        }
        // Issue #2937: do NOT resize PTYs here. PTY size is owned by the
        // frontend's real xterm fit (sendGeometry → update_window_geometry_events).
        // The frontend re-fits the focused window (scheduleTerminalFocusActivation)
        // and any geometry-changed window (applyWorkspaceGeometry on the
        // geometry_revision bump from a maximize-restore) on the resulting
        // workspace_state render. Re-running geometry_to_pty_size here only
        // clobbered the accurate cols/rows back to the spawn-time approximation
        // on every window switch, desyncing the child's grid from xterm.
        let _ = self.persist();
        vec![self.workspace_state_broadcast()]
    }

    pub(crate) fn update_viewport_events(
        &mut self,
        viewport: CanvasViewport,
    ) -> Vec<OutboundEvent> {
        let Some(tab) = self.active_tab_mut() else {
            return Vec::new();
        };
        if !tab.workspace.update_viewport(viewport) {
            return Vec::new();
        }
        let _ = self.persist();
        vec![self.workspace_state_broadcast()]
    }

    pub(crate) fn arrange_windows_events(
        &mut self,
        mode: ArrangeMode,
        bounds: WindowGeometry,
    ) -> Vec<OutboundEvent> {
        let Some(tab_id) = self.active_tab_id.clone() else {
            return Vec::new();
        };
        let arranged = {
            let Some(tab) = self.tab_mut(&tab_id) else {
                return Vec::new();
            };
            tab.workspace.arrange_windows(mode, bounds)
        };
        if !arranged {
            return Vec::new();
        }
        // Issue #2937: do NOT resize PTYs here (same rationale as
        // cycle_focus_events). arrange_windows bumps geometry_revision for
        // every window it moves, so the frontend re-fits each affected
        // terminal (applyWorkspaceGeometry → fitTerminal → sendGeometry) on
        // the resulting workspace_state render. The backend approximation
        // must not clobber those frontend-fitted cols/rows.
        let _ = self.persist();
        vec![self.workspace_state_broadcast()]
    }

    pub(crate) fn dock_window_tab_events(
        &mut self,
        id: &str,
        target_id: &str,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id).cloned() else {
            return Vec::new();
        };
        let Some(target_address) = self.window_lookup.get(target_id).cloned() else {
            return Vec::new();
        };
        if address.tab_id != target_address.tab_id {
            return Vec::new();
        }
        let updated = {
            let Some(tab) = self.tab_mut(&address.tab_id) else {
                return Vec::new();
            };
            tab.workspace
                .dock_window_tab(&address.raw_id, &target_address.raw_id)
        };
        if !updated {
            return Vec::new();
        }
        // Docking mutates shared canvas geometry, but the frontend's
        // revisioned xterm fit remains authoritative for the live PTY grid.
        self.activate_tab_for_window_events(address.tab_id)
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
        // Tab activation only changes the active marker/z-order. The revealed
        // terminal's real grid is owned by the frontend xterm fit; resizing
        // from backend geometry here clobbers that fit, especially for
        // maximized tab groups where shared window geometry is only an
        // approximation of the visible terminal body.
        self.activate_tab_for_window_events(address.tab_id)
    }

    pub(crate) fn detach_window_tab_events(
        &mut self,
        id: &str,
        geometry: WindowGeometry,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id).cloned() else {
            return Vec::new();
        };
        let updated = {
            let Some(tab) = self.tab_mut(&address.tab_id) else {
                return Vec::new();
            };
            tab.workspace.detach_window_tab(&address.raw_id, geometry)
        };
        if !updated {
            return Vec::new();
        }
        // Detaching mutates canvas geometry, but the frontend's revisioned
        // xterm fit remains authoritative for the live PTY grid.
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

    pub(crate) fn undock_agent_window_events(
        &mut self,
        id: &str,
        geometry: Option<WindowGeometry>,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id).cloned() else {
            return Vec::new();
        };
        let updated = {
            let Some(tab) = self.tab_mut(&address.tab_id) else {
                return Vec::new();
            };
            tab.workspace.undock_agent_window(&address.raw_id, geometry)
        };
        if !updated {
            return Vec::new();
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
        if let Ok(mut pane) = runtime.pane.lock() {
            let _ = pane.resize(cols.max(20), rows.max(6));
        }
        Vec::new()
    }

    pub(crate) fn update_window_geometry_events(
        &mut self,
        id: &str,
        geometry: WindowGeometry,
        cols: u16,
        rows: u16,
        base_geometry_revision: Option<u64>,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id).cloned() else {
            return Vec::new();
        };
        if let Some(base_geometry_revision) = base_geometry_revision {
            let Some(tab) = self.tab(&address.tab_id) else {
                return Vec::new();
            };
            let Some(window) = tab.workspace.window(&address.raw_id) else {
                return Vec::new();
            };
            if window.geometry_revision != base_geometry_revision {
                return vec![self.workspace_state_broadcast()];
            }
        }
        let updated = {
            let Some(tab) = self.tab_mut(&address.tab_id) else {
                return Vec::new();
            };
            tab.workspace.update_geometry(&address.raw_id, geometry)
        };
        if !updated {
            return Vec::new();
        }
        if let Some(runtime) = self.runtimes.get(id) {
            if let Ok(mut pane) = runtime.pane.lock() {
                let _ = pane.resize(cols.max(20), rows.max(6));
            }
        }
        let _ = self.persist();
        vec![self.workspace_state_broadcast()]
    }

    pub(crate) fn close_window_events(&mut self, id: &str) -> Vec<OutboundEvent> {
        self.close_window_events_with_monitor_notification(id, true)
    }

    fn modern_monitor_close_execution_target(
        &self,
        window_id: &str,
        launched: &gwt::IssueMonitorLaunchedIssue,
    ) -> Result<ModernMonitorCloseExecutionTarget, String> {
        let active = self
            .active_agent_sessions
            .get(window_id)
            .ok_or_else(|| "modern Monitor launch has no exact active Session".to_string())?;
        let runtime_incarnation = self
            .runtimes
            .get(window_id)
            .map(|runtime| runtime.incarnation)
            .filter(|incarnation| *incarnation > 0)
            .ok_or_else(|| "modern Monitor launch has no exact local runtime".to_string())?;
        let session_path = self
            .sessions_dir
            .join(format!("{}.toml", active.session_id));
        let session = gwt_agent::Session::load(&session_path)
            .map_err(|error| format!("modern Monitor launch Session is unavailable: {error}"))?;
        let identity = gwt_agent::SessionExecutionIdentity::from_session(&session)
            .map_err(|error| format!("modern Monitor launch Session identity is invalid: {error}"))?
            .ok_or_else(|| "modern Monitor launch Session has no execution binding".to_string())?;
        if identity.session_id != active.session_id
            || identity.execution_binding.session_id != active.session_id
            || !crate::runtime_support::same_worktree_path(
                &identity.worktree_path,
                &active.worktree_path,
            )
        {
            return Err(
                "modern Monitor launch Session no longer matches the active worktree identity"
                    .to_string(),
            );
        }
        let owner_kind = match identity.execution_binding.owner_kind.as_str() {
            "issue" => gwt::cli::execution_state::ExecutionOwnerKind::Issue,
            "spec" => gwt::cli::execution_state::ExecutionOwnerKind::Spec,
            other => return Err(format!("execution owner kind is not canonical: {other}")),
        };
        let owner = gwt::cli::execution_state::ExecutionOwnerKey {
            kind: owner_kind,
            number: identity.execution_binding.owner_number,
        };
        if owner.kind != gwt::cli::execution_state::ExecutionOwnerKind::Issue
            || owner.number != launched.issue_number
        {
            return Err("execution owner does not match the Monitor issue".to_string());
        }
        let record = gwt::cli::execution_state::load(&identity.worktree_path)
            .map_err(|error| format!("exact worktree Execution could not be read: {error}"))?
            .ok_or_else(|| "modern Monitor launch has no worktree Execution".to_string())?;
        if record.owner_kind != owner.kind
            || record.owner_number != owner.number
            || record.primary_session_id != identity.session_id
        {
            return Err(
                "worktree Execution does not match the Monitor Session identity".to_string(),
            );
        }
        let ledger =
            gwt::cli::execution_state::load_generation_ledger(&identity.worktree_path, owner)
                .map_err(|error| {
                    format!("exact worktree execution ledger could not be read: {error}")
                })?
                .ok_or_else(|| "modern Monitor launch has no execution ledger".to_string())?;
        let current = ledger
            .current_generation()
            .ok_or_else(|| "execution ledger has no current generation".to_string())?;
        if current.identity.generation_id != identity.execution_binding.identity.generation_id
            || current.identity.session_binding_id != identity.execution_binding.identity.binding_id
            || ledger.current_effective_status() != Some(record.status)
        {
            return Err(
                "modern Monitor Session is not the exact current execution generation".to_string(),
            );
        }
        if ledger.continuation_attempts.iter().any(|attempt| {
            attempt.predecessor.generation_id == current.identity.generation_id
                && attempt.status == gwt::cli::execution_state::ContinuationAttemptStatus::Prepared
        }) || ledger.takeover_attempts.iter().any(|attempt| {
            attempt.generation_id == current.identity.generation_id
                && attempt.status
                    == gwt::cli::execution_state::GenerationTakeoverAttemptStatus::Prepared
        }) {
            return Err(
                "modern Monitor execution has a Prepared successor or takeover".to_string(),
            );
        }
        if !gwt::cli::execution_state::execution_binding_authorizes_current_generation(
            &identity.worktree_path,
            owner,
            &identity.session_id,
            &identity.execution_binding.identity,
        )
        .map_err(|error| format!("exact execution binding could not be verified: {error}"))?
        {
            return Err("modern Monitor Session binding is stale".to_string());
        }
        let runtime = gwt::cli::execution_state::classify_exact_session_runtime(
            &self.sessions_dir,
            &identity,
        )
        .map_err(|error| format!("exact runtime proof could not be classified: {error}"))?;
        match record.status {
            gwt::cli::execution_state::ExecutionControlStatus::Active => {
                if runtime != gwt::cli::execution_state::ExactSessionRuntimeDisposition::Live {
                    return Err(
                        "modern Monitor Active execution has missing or ambiguous runtime proof"
                            .to_string(),
                    );
                }
            }
            gwt::cli::execution_state::ExecutionControlStatus::Completed
            | gwt::cli::execution_state::ExecutionControlStatus::Blocked => {
                if !matches!(
                    runtime,
                    gwt::cli::execution_state::ExactSessionRuntimeDisposition::Terminal(proof)
                        if proof.runtime_incarnation == runtime_incarnation
                ) {
                    return Err(
                        "terminal Monitor execution has live or ambiguous runtime proof"
                            .to_string(),
                    );
                }
            }
        }
        Ok(ModernMonitorCloseExecutionTarget {
            worktree_path: identity.worktree_path.clone(),
            owner,
            session: identity,
            runtime_incarnation,
            status: record.status,
        })
    }

    fn modern_monitor_close_execution_readback_is_exact(
        &self,
        target: &ModernMonitorCloseExecutionTarget,
    ) -> Result<bool, String> {
        let current_session = gwt_agent::Session::load(
            &self
                .sessions_dir
                .join(format!("{}.toml", target.session.session_id)),
        )
        .map_err(|error| format!("exact Session readback failed: {error}"))?;
        if gwt_agent::SessionExecutionIdentity::from_session(&current_session)
            .map_err(|error| format!("exact Session readback is invalid: {error}"))?
            .as_ref()
            != Some(&target.session)
        {
            return Ok(false);
        }
        let record = gwt::cli::execution_state::load(&target.worktree_path)
            .map_err(|error| format!("exact Execution readback failed: {error}"))?;
        let Some(record) = record else {
            return Ok(false);
        };
        if record.owner_kind != target.owner.kind
            || record.owner_number != target.owner.number
            || record.primary_session_id != target.session.session_id
            || record.status == gwt::cli::execution_state::ExecutionControlStatus::Active
        {
            return Ok(false);
        }
        gwt::cli::execution_state::execution_binding_authorizes_current_generation(
            &target.worktree_path,
            target.owner,
            &target.session.session_id,
            &target.session.execution_binding.identity,
        )
        .map_err(|error| format!("exact ledger readback failed: {error}"))
    }

    /// Close a window whose Issue Monitor lifecycle transition was already
    /// committed by the daemon or the fail-closed local fallback. Re-entering
    /// the normal window-closed hook here would publish a second control for
    /// the same lifecycle edge and could requeue the just-failed issue.
    pub(crate) fn close_window_after_issue_monitor_finalize_events(
        &mut self,
        id: &str,
    ) -> Vec<OutboundEvent> {
        self.close_window_events_with_monitor_notification(id, false)
    }

    fn close_window_events_with_monitor_notification(
        &mut self,
        id: &str,
        notify_issue_monitor: bool,
    ) -> Vec<OutboundEvent> {
        let issue_monitor_project_root = self.issue_monitor_project_root_for_window(id);
        // SPEC-3431 FR-013: snapshot the closing window's session while the
        // active entry still exists — an explicit close of the PM pane is an
        // intentional stop and clears the durable PM registration below.
        // The tab owns Monitor state, but an agent Session may execute from a
        // distinct managed worktree. Snapshot both identities before runtime
        // teardown so an ordinary close can fence the exact worktree ECR.
        let active_execution_target = self
            .active_agent_sessions
            .get(id)
            .map(|session| (session.session_id.clone(), session.worktree_path.clone()));
        let closing_session_id = active_execution_target
            .as_ref()
            .map(|(session_id, _)| session_id.clone())
            .or_else(|| {
                let address = self.window_lookup.get(id)?;
                self.tab(&address.tab_id)?
                    .workspace
                    .window(&address.raw_id)?
                    .session_id
                    .clone()
            });
        let launched_monitor_identity = issue_monitor_project_root
            .as_ref()
            .and_then(|project_root| {
                gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(
                    project_root,
                ))
                .ok()
            })
            .and_then(|prefs| {
                prefs
                    .launched_issues
                    .into_iter()
                    .find(|identity| identity.window_id == id)
            });
        let closes_launched_monitor_identity = launched_monitor_identity.is_some();
        let modern_execution_target = if notify_issue_monitor {
            match launched_monitor_identity
                .as_ref()
                .filter(|identity| identity.generation > 0)
                .map(|identity| self.modern_monitor_close_execution_target(id, identity))
                .transpose()
            {
                Ok(target) => target,
                Err(error) => {
                    tracing::warn!(
                        window_id = id,
                        %error,
                        "modern Monitor pane close retained its exact identity"
                    );
                    return Vec::new();
                }
            }
        } else {
            None
        };
        let mut runtime_already_stopped = false;
        if let Some(target) = modern_execution_target.as_ref() {
            if target.status == gwt::cli::execution_state::ExecutionControlStatus::Active {
                if let Err(error) = self.stop_exact_manual_holder_runtime(
                    id,
                    target.runtime_incarnation,
                    &target.session,
                    false,
                ) {
                    tracing::warn!(
                        window_id = id,
                        %error,
                        "modern Monitor pane close retained identity because exact runtime stop failed"
                    );
                    return Vec::new();
                }
                runtime_already_stopped = true;
                let terminal = gwt::cli::execution_state::classify_exact_session_runtime(
                    &self.sessions_dir,
                    &target.session,
                );
                if !matches!(
                    terminal,
                    Ok(gwt::cli::execution_state::ExactSessionRuntimeDisposition::Terminal(proof))
                        if proof.runtime_incarnation == target.runtime_incarnation
                ) {
                    tracing::warn!(
                        window_id = id,
                        ?terminal,
                        "modern Monitor pane close retained identity because terminal runtime proof is inconclusive"
                    );
                    return Vec::new();
                }
                let settlement = gwt::cli::execution_state::settle(
                    &target.worktree_path,
                    &target.session.session_id,
                    gwt::cli::execution_state::ExecutionSettlement::Blocked {
                        reason: "Issue Monitor window closed".to_string(),
                        missing_verification: None,
                    },
                );
                if !matches!(
                    settlement,
                    Ok(gwt::cli::execution_state::SettleResult::Settled(_))
                        | Ok(gwt::cli::execution_state::SettleResult::AlreadySettled(_))
                ) {
                    tracing::warn!(
                        window_id = id,
                        ?settlement,
                        "modern Monitor pane close retained identity because exact Execution settlement failed"
                    );
                    return Vec::new();
                }
            } else {
                // An exact terminal predecessor replay needs no second ECR
                // mutation, but still tears down the inert local runtime
                // registry before the Monitor identity can be released.
                self.stop_window_runtime(id);
                runtime_already_stopped = true;
            }
            match self.modern_monitor_close_execution_readback_is_exact(target) {
                Ok(true) => {}
                Ok(false) => {
                    tracing::warn!(
                        window_id = id,
                        "modern Monitor pane close retained identity after exact readback mismatch"
                    );
                    return Vec::new();
                }
                Err(error) => {
                    tracing::warn!(
                        window_id = id,
                        %error,
                        "modern Monitor pane close retained identity because exact readback failed"
                    );
                    return Vec::new();
                }
            }
        }
        if notify_issue_monitor
            && (closing_session_id.is_some() || closes_launched_monitor_identity)
            && modern_execution_target.is_none()
        {
            if let Some(project_root) = issue_monitor_project_root.as_ref() {
                let execution_root = active_execution_target
                    .as_ref()
                    .map(|(_, worktree_path)| worktree_path.as_path())
                    .unwrap_or(project_root.as_path());
                let active_execution = match gwt::cli::execution_state::load(execution_root) {
                    Ok(record) => record.filter(|record| {
                        record.status == gwt::cli::execution_state::ExecutionControlStatus::Active
                    }),
                    Err(error) => {
                        tracing::warn!(
                            window_id = id,
                            project_root = %project_root.display(),
                            execution_root = %execution_root.display(),
                            %error,
                            "agent pane close deferred because Execution snapshot failed"
                        );
                        return Vec::new();
                    }
                };
                if let Some(record) = active_execution {
                    let Some(session_id) = closing_session_id.as_deref() else {
                        tracing::warn!(
                            window_id = id,
                            expected_session_id = %record.primary_session_id,
                            "agent pane close deferred because its Session identity is missing"
                        );
                        return Vec::new();
                    };
                    if record.primary_session_id != session_id {
                        tracing::warn!(
                            window_id = id,
                            expected_session_id = %record.primary_session_id,
                            observed_session_id = session_id,
                            "agent pane close deferred by Execution session fence"
                        );
                        return Vec::new();
                    }
                    let settlement = gwt::cli::execution_state::settle(
                        execution_root,
                        session_id,
                        gwt::cli::execution_state::ExecutionSettlement::Blocked {
                            reason: "Agent pane closed".to_string(),
                            missing_verification: None,
                        },
                    );
                    if !matches!(
                        settlement,
                        Ok(gwt::cli::execution_state::SettleResult::Settled(_))
                            | Ok(gwt::cli::execution_state::SettleResult::AlreadySettled(_))
                    ) {
                        tracing::warn!(
                            window_id = id,
                            ?settlement,
                            "agent pane close deferred until exact Execution settlement succeeds"
                        );
                        return Vec::new();
                    }
                }
            }
        }
        self.clear_agent_window_startup_restore(id);
        if !runtime_already_stopped {
            self.stop_window_runtime(id);
        }
        self.remove_window_state_tracking(id);
        self.profile_selections.remove(id);
        if !close_window_from_workspace(
            &mut self.tabs,
            &mut self.window_lookup,
            &mut self.window_details,
            id,
        ) {
            return Vec::new();
        }
        let pm_deregistered = match (
            closing_session_id.as_deref(),
            issue_monitor_project_root.as_ref(),
        ) {
            (Some(session_id), Some(project_root)) => {
                self.deregister_pm_for_closed_window(project_root, session_id)
            }
            _ => false,
        };
        let _ = self.persist();
        let mut events = vec![self.workspace_state_broadcast()];
        // SPEC-3431 FR-026: closing the PM is a PM state change, so the
        // settings panel has to hear about it or it keeps offering a restart
        // for a pane that is already gone.
        if pm_deregistered {
            events.extend(self.pm_status_broadcast_events());
        }
        if let Some(event) = self.active_work_projection_broadcast_for_active_tab() {
            events.push(event);
        }
        if notify_issue_monitor {
            if let Some(project_root) = issue_monitor_project_root {
                events.extend(
                    self.issue_monitor_windows_closed_events(&project_root, &[id.to_string()]),
                );
            }
        }
        events
    }

    /// SPEC-2356 安心 Addendum (FR-041): stop a single window's agent runtime
    /// (kill the PTY through the existing stop path) but keep the window and its
    /// terminal output on the canvas, rendered as `Stopped`. Unlike
    /// [`Self::close_window_events`], the window record is never removed, so the
    /// operator keeps the prior output and can later relaunch it through
    /// [`Self::restart_window_events`]. Idempotent: stopping an already-stopped
    /// window simply re-broadcasts the authoritative `Stopped` status.
    pub(crate) fn stop_window_events(&mut self, id: &str) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id).cloned() else {
            return Vec::new();
        };
        // Tear down the live runtime (kills the PTY + joins reader/status
        // threads + deregisters the writer). This reuses the exact same stop
        // primitive that `close_window_events` uses, so no PTY management is
        // reimplemented. `stop_window_runtime` also persists the Paused Work
        // marker for an agent session via `mark_agent_session_stopped`.
        self.stop_window_runtime(id);
        // Render the surviving window as Stopped. `stop_window_runtime` already
        // removed the pty/hook tracking, so the workspace record is the source
        // of truth for the post-stop status.
        self.set_window_status(
            &address.tab_id,
            &address.raw_id,
            WindowProcessStatus::Stopped,
        );
        let _ = self.persist();
        let mut events = vec![self.workspace_state_broadcast()];
        events.extend(Self::status_events(
            id.to_string(),
            WindowProcessStatus::Stopped,
            None,
        ));
        if let Some(event) = self.active_work_projection_broadcast_for_active_tab() {
            events.push(event);
        }
        events
    }

    /// SPEC-2356 安心 Addendum (FR-042): stop every running agent window's
    /// runtime, keeping all windows on the canvas. Iterates the open windows
    /// and applies the per-window [`Self::stop_window_events`] stop to each
    /// window that currently has a live runtime, so the confirm-on-frontend
    /// "stop all" button resolves to the same kept-but-stopped state as the
    /// single-window kill switch.
    pub(crate) fn stop_all_windows_events(&mut self) -> Vec<OutboundEvent> {
        let running_ids: Vec<String> = self.runtimes.keys().cloned().collect();
        let mut events = Vec::new();
        for window_id in running_ids {
            events.extend(self.stop_window_events(&window_id));
        }
        events
    }

    /// SPEC-2356 安心 Addendum (FR-044): relaunch a window's agent in place. The
    /// window id, geometry, and placement are preserved; the prior terminal
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
        let geometry = window.geometry.clone();

        if preset == WindowPreset::Agent {
            return self.restart_agent_window_in_place(&address.tab_id, &address.raw_id, geometry);
        }

        // Clear any stale "restored window is paused" detail before relaunching
        // so the new PTY's status is the authoritative one.
        self.window_details.remove(id);
        let events = self.start_window(&address.tab_id, &address.raw_id, preset, geometry);
        let _ = self.persist();
        let mut all = vec![self.workspace_state_broadcast()];
        all.extend(events);
        all
    }

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
