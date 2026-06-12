//! Canonical orchestration for syncing `WorkProjection` title surfaces
//! (per-agent `title_summary` / `current_focus`) into in-memory window state
//! and emitting the consequent broadcasts in one batch.
//!
//! Background (SPEC-2359 US-26 / Phase U-1..U-4):
//! Before this module, every write path that mutated
//! `projection.agents[<i>].title_summary` had to remember each step:
//! (1) update `current.json` + `journal.jsonl`,
//! (2) sync `tab.workspace.windows[<id>].dynamic_title` in memory,
//! (3) broadcast `BackendEvent::ActiveWorkProjection`, and
//! (4) broadcast `BackendEvent::WorkState` so the pane heading
//! `windowData.dynamic_title` consumed by `windowDisplayTitle` on the
//! frontend refreshes. `gwtd workspace update --title-summary` ran (1)
//! and (3) but never (4), so the pane heading kept the `agent_id`
//! fallback ("CLAUDE CODE") even when `projection.agents[<i>].title_summary`
//! had a fresh value.
//!
//! `apply_workspace_projection_title_sync` consolidates (2)..(4) so any
//! caller that has just observed a projection change just dispatches the
//! returned `Vec<OutboundEvent>` and is guaranteed to leave the surfaces
//! consistent. Phase U-1 (this commit) keeps the broadcast surface
//! identical to the pre-refactor behavior to preserve every existing
//! test; Phase U-2 wires the `WorkspaceState` broadcast in;
//! Phase U-3 adds `active_agent_sessions` backfill for sessions that
//! gwt's launch flow has not yet registered.

use std::path::Path;

use gwt_core::work_projection::WorkProjection;

use super::{AppRuntime, OutboundEvent};

impl AppRuntime {
    /// Run the canonical title-sync orchestration for the supplied projection.
    ///
    /// Side effects:
    /// - Mutates `tab.workspace.windows[<id>].dynamic_title` /
    ///   `dynamic_title_detail` from `projection.agents[<i>].title_summary`
    ///   / `current_focus` via
    ///   [`AppRuntime::sync_agent_window_titles_from_workspace_projection`].
    ///
    /// Return value:
    /// - The `OutboundEvent`s that callers should dispatch. Phase U-2
    ///   (SPEC-2359 US-26) makes this emit `BackendEvent::WorkState`
    ///   when an in-memory `dynamic_title` actually changed, so the
    ///   frontend's pane heading (`windowDisplayTitle` →
    ///   `windowData.dynamic_title`) updates immediately without waiting
    ///   for the next hook event or window structure change. The
    ///   `BackendEvent::ActiveWorkProjection` broadcast for the active tab
    ///   is emitted unconditionally — that surface refreshes the Active
    ///   Work card and Workspace Kanban entries regardless of whether a
    ///   pane heading was touched.
    pub(crate) fn apply_workspace_projection_title_sync(
        &mut self,
        project_root: &Path,
        projection: &WorkProjection,
    ) -> Vec<OutboundEvent> {
        let dynamic_title_changed =
            self.sync_agent_window_titles_from_workspace_projection(project_root, projection);

        let mut events = Vec::new();
        if dynamic_title_changed {
            events.push(self.workspace_state_broadcast());
        }
        if let Some(event) = self.active_work_projection_broadcast_for_active_tab() {
            events.push(event);
        }
        events
    }
}
