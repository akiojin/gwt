//! Canonical orchestration for syncing `WorkspaceProjection` title surfaces
//! (per-agent `title_summary` / `current_focus`) into in-memory window state
//! and emitting the consequent broadcasts in one batch.
//!
//! Background (SPEC-2359 US-26 / Phase U-1..U-4):
//! Before this module, every write path that mutated
//! `projection.agents[<i>].title_summary` had to remember each step:
//! (1) update `current.json` + `journal.jsonl`,
//! (2) sync `tab.workspace.windows[<id>].dynamic_title` in memory,
//! (3) broadcast `BackendEvent::ActiveWorkProjection`, and
//! (4) broadcast `BackendEvent::WindowCanvasState` so the pane heading
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
//! test; Phase U-2 wires the `WindowCanvasState` broadcast in;
//! Phase U-3 adds `active_agent_sessions` backfill for sessions that
//! gwt's launch flow has not yet registered.

use std::path::Path;

use gwt_core::workspace_projection::WorkspaceProjection;

use crate::same_worktree_path;

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
    ///   (SPEC-2359 US-26) makes this emit `BackendEvent::WindowCanvasState`
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
        projection: &WorkspaceProjection,
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

    /// Sync `projection.agents[<i>].title_summary` / `current_focus` into the
    /// matching `tab.workspace.windows[<id>].dynamic_title` /
    /// `dynamic_title_detail`. Returns `true` if at least one window was
    /// touched.
    ///
    /// Callers should generally go through
    /// [`AppRuntime::apply_workspace_projection_title_sync`] (Phase U-1+)
    /// rather than calling this directly, so that the consequent broadcasts
    /// are emitted in the same batch.
    pub(crate) fn sync_agent_window_titles_from_workspace_projection(
        &mut self,
        project_root: &Path,
        projection: &gwt_core::workspace_projection::WorkspaceProjection,
    ) -> bool {
        // SPEC-2359 Phase W-11 (US-58 / FR-344): resolve the effective window
        // title with the display fallback chain — the agent-authored
        // `title_summary` first, then the linked Issue/SPEC title, then `None`
        // (which lets the frontend fall back to the neutral agent label). The
        // raw prompt is never written into a title, so it can never appear here.
        let issue_fallback_title = projection
            .linked_issues
            .first()
            .map(|issue| issue.number)
            .and_then(|number| {
                let cache_root =
                    gwt::issue_cache::issue_cache_root_for_repo_path_or_detached(project_root);
                gwt::issue_cache::load_issue_title_from_cache(&cache_root, number)
            });

        let updates = projection
            .agents
            .iter()
            .filter_map(|agent| {
                let window_id = self.resolve_title_sync_window_id(agent, project_root)?;
                let title = agent
                    .title_summary
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
                    .or_else(|| issue_fallback_title.clone());
                Some((window_id, title, agent.current_focus.clone()))
            })
            .collect::<Vec<_>>();

        let mut changed = false;
        for (window_id, title, detail) in updates {
            let Some(address) = self.window_lookup.get(&window_id).cloned() else {
                continue;
            };
            let Some(tab) = self.tab_mut(&address.tab_id) else {
                continue;
            };
            if tab
                .workspace
                .set_dynamic_title_with_detail(&address.raw_id, title, detail)
            {
                changed = true;
            }
        }
        changed
    }

    /// Resolve the window_id that title sync should target for a given
    /// projection agent.
    ///
    /// Fast path: `active_agent_sessions` (gwt's live launch tracking).
    ///
    /// Phase U-3 fallback (SPEC-2359 US-26): for sessions that gwt's
    /// launch flow has not (yet) registered — e.g. GUI restarted after a
    /// session started, a session that was launched outside the tracked
    /// `gwtd` path but still publishes its `GWT_SESSION_ID` — use the
    /// `window_id` / `worktree_path` carried by the projection itself. The
    /// fallback intentionally does **not** mutate `active_agent_sessions`
    /// (that lifecycle stays in the launch flow, see US-24). It only
    /// resolves the lookup needed for title sync.
    ///
    /// Phase U-4 fallback: when the projection record only carries
    /// `worktree_path` (e.g. SessionStart hook registered the agent
    /// before any GUI launch picked it up so `window_id` is `None`),
    /// try to match against `active_agent_sessions` by worktree alone.
    /// Only resolves when there is exactly one matching session in the
    /// worktree with the same `agent_id`, to avoid mis-targeting when
    /// the worktree has multiple panes.
    ///
    /// Phase U-7 (SPEC-2359): the fast path used to require
    /// `same_worktree_path(session.worktree_path, project_root)` so that
    /// only the watcher firing for the *agent's own* tab would resolve
    /// the window. In practice this filter prevented title updates
    /// whenever the watcher event came from a different tab (e.g. the
    /// startup tab's watcher firing for a change in another tab's
    /// agent, since both tabs share `current.json`). `session_id` is
    /// globally unique to one launched window — finding it in
    /// `active_agent_sessions` is sufficient to identify the target.
    fn resolve_title_sync_window_id(
        &self,
        agent: &gwt_core::workspace_projection::WorkspaceAgentSummary,
        project_root: &Path,
    ) -> Option<String> {
        if let Some((window_id, _session)) = self
            .active_agent_sessions
            .iter()
            .find(|(_, session)| session.session_id == agent.session_id)
        {
            return Some(window_id.clone());
        }
        if let Some(worktree) = agent.worktree_path.as_deref() {
            if same_worktree_path(worktree, project_root) {
                if let Some(projected_window_id) = agent.window_id.as_deref() {
                    if self.window_lookup.contains_key(projected_window_id) {
                        return Some(projected_window_id.to_string());
                    }
                }
                let mut matches = self.active_agent_sessions.iter().filter(|(_, session)| {
                    same_worktree_path(&session.worktree_path, worktree)
                        && session.agent_id == agent.agent_id
                });
                if let Some((window_id, _)) = matches.next() {
                    if matches.next().is_none() {
                        return Some(window_id.clone());
                    }
                }
            }
        }
        None
    }
}
