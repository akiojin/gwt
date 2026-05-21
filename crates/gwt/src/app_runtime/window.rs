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
    close_window_from_workspace, combined_window_id, AppRuntime, BackendEvent, OutboundEvent,
    WindowGeometry, WindowPreset,
};

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
        let window = {
            let Some(tab) = self.tab_mut(&tab_id) else {
                return Vec::new();
            };
            let window = tab.workspace.add_window(preset, bounds.clone());
            if preset.opens_maximized_by_default() {
                let _ = tab.workspace.maximize_window(&window.id, bounds.clone());
                tab.workspace.window(&window.id).cloned().unwrap_or(window)
            } else {
                window
            }
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
        let opens_maximized = tab
            .workspace
            .window(&address.raw_id)
            .map(|window| window.preset.opens_maximized_by_default())
            .unwrap_or(false);
        if !tab.workspace.focus_window(
            &address.raw_id,
            if opens_maximized {
                None
            } else {
                bounds.clone()
            },
        ) {
            return Vec::new();
        }
        if opens_maximized {
            if let Some(bounds) = bounds {
                let already_maximized = tab
                    .workspace
                    .window(&address.raw_id)
                    .map(|window| window.maximized)
                    .unwrap_or(false);
                if !already_maximized {
                    let _ = tab.workspace.maximize_window(&address.raw_id, bounds);
                }
            }
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
            for window in &tab.workspace.persisted().windows {
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
        let resize_window_ids = {
            let Some(tab) = self.tab_mut(&address.tab_id) else {
                return Vec::new();
            };
            if !tab
                .workspace
                .dock_window_tab(&address.raw_id, &target_address.raw_id)
            {
                return Vec::new();
            }
            tab.workspace
                .window(&address.raw_id)
                .and_then(|window| window.tab_group_id.clone())
                .map(|group_id| {
                    tab.workspace
                        .persisted()
                        .windows
                        .iter()
                        .filter(|window| window.tab_group_id.as_deref() == Some(group_id.as_str()))
                        .map(|window| combined_window_id(&address.tab_id, &window.id))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_else(|| vec![id.to_string(), target_id.to_string()])
        };
        let _ = self.set_active_tab(address.tab_id);
        for window_id in resize_window_ids {
            self.resize_runtime_to_window(&window_id);
        }
        let _ = self.persist();
        vec![self.workspace_state_broadcast()]
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
        let _ = self.set_active_tab(address.tab_id);
        self.resize_runtime_to_window(id);
        let _ = self.persist();
        vec![self.workspace_state_broadcast()]
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
        let mut events = vec![self.workspace_state_broadcast()];
        if let Some(event) = self.active_work_projection_broadcast_for_active_tab() {
            events.push(event);
        }
        events
    }

    pub(crate) fn list_windows_event(&self) -> BackendEvent {
        let windows = self
            .active_tab_id
            .as_ref()
            .and_then(|tab_id| self.tab(tab_id))
            .map(|tab| self.workspace_view_for_tab(tab).windows)
            .unwrap_or_default();
        BackendEvent::WindowList { windows }
    }
}
