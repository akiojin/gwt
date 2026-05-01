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
//! - [`AppRuntime::maximize_window_events`] /
//!   [`AppRuntime::minimize_window_events`] /
//!   [`AppRuntime::restore_window_events`] /
//!   [`AppRuntime::update_window_geometry_events`] — per-window geometry
//!   adjustments (delegates terminal resize to the live runtime)
//! - [`AppRuntime::list_windows_event`] — workspace snapshot for the active tab
//!
//! GUI-side window contracts (canvas viewport pan/zoom, arrange/stack
//! semantics) remain owned by SPEC-2008. This split keeps `mod.rs` lean
//! while preserving the broadcast/persist contract through the existing
//! `workspace_state_broadcast` and `persist` helpers on `AppRuntime`.

use gwt::{ArrangeMode, CanvasViewport, FocusCycleDirection};

use super::{
    close_window_from_workspace, combined_window_id, workspace_view_for_tab, AppRuntime,
    BackendEvent, OutboundEvent, WindowGeometry, WindowPreset,
};

impl AppRuntime {
    pub(crate) fn create_window_events(
        &mut self,
        preset: WindowPreset,
        bounds: WindowGeometry,
    ) -> Vec<OutboundEvent> {
        let Some(tab_id) = self.active_tab_id.clone() else {
            return Vec::new();
        };
        let window = {
            let Some(tab) = self.tab_mut(&tab_id) else {
                return Vec::new();
            };
            tab.workspace.add_window(preset, bounds)
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
        self.active_tab_id = Some(address.tab_id);
        let _ = self.persist();
        vec![self.workspace_state_broadcast()]
    }

    pub(crate) fn cycle_focus_events(
        &mut self,
        direction: FocusCycleDirection,
        bounds: WindowGeometry,
    ) -> Vec<OutboundEvent> {
        let Some(tab) = self.active_tab_mut() else {
            return Vec::new();
        };
        if tab.workspace.cycle_focus(direction, bounds).is_none() {
            return Vec::new();
        }
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
        tab.workspace.update_viewport(viewport);
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
        if let Some(tab) = self.tab(&tab_id) {
            for window in tab.workspace.persisted().windows.iter() {
                self.resize_runtime_to_window(&combined_window_id(&tab_id, &window.id));
            }
        }
        let _ = self.persist();
        vec![self.workspace_state_broadcast()]
    }

    pub(crate) fn maximize_window_events(
        &mut self,
        id: &str,
        bounds: WindowGeometry,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id).cloned() else {
            return Vec::new();
        };
        let updated = {
            let Some(tab) = self.tab_mut(&address.tab_id) else {
                return Vec::new();
            };
            tab.workspace.maximize_window(&address.raw_id, bounds)
        };
        if !updated {
            return Vec::new();
        }
        let _ = self.set_active_tab(address.tab_id);
        self.resize_runtime_to_window(id);
        let _ = self.persist();
        vec![self.workspace_state_broadcast()]
    }

    pub(crate) fn minimize_window_events(&mut self, id: &str) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id).cloned() else {
            return Vec::new();
        };
        let updated = {
            let Some(tab) = self.tab_mut(&address.tab_id) else {
                return Vec::new();
            };
            tab.workspace.minimize_window(&address.raw_id)
        };
        if !updated {
            return Vec::new();
        }
        let _ = self.set_active_tab(address.tab_id);
        self.resize_runtime_to_window(id);
        let _ = self.persist();
        vec![self.workspace_state_broadcast()]
    }

    pub(crate) fn restore_window_events(&mut self, id: &str) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id).cloned() else {
            return Vec::new();
        };
        let updated = {
            let Some(tab) = self.tab_mut(&address.tab_id) else {
                return Vec::new();
            };
            tab.workspace.restore_window(&address.raw_id)
        };
        if !updated {
            return Vec::new();
        }
        let _ = self.set_active_tab(address.tab_id);
        self.resize_runtime_to_window(id);
        let _ = self.persist();
        vec![self.workspace_state_broadcast()]
    }

    pub(crate) fn update_window_geometry_events(
        &mut self,
        id: &str,
        geometry: WindowGeometry,
        cols: u16,
        rows: u16,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id).cloned() else {
            return Vec::new();
        };
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
        self.stop_window_runtime(id);
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
        let _ = self.persist();
        vec![self.workspace_state_broadcast()]
    }

    pub(crate) fn list_windows_event(&self) -> BackendEvent {
        let windows = self
            .active_tab_id
            .as_ref()
            .and_then(|tab_id| self.tab(tab_id))
            .map(|tab| workspace_view_for_tab(tab).windows)
            .unwrap_or_default();
        BackendEvent::WindowList { windows }
    }
}
