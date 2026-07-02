//! Bootstrap / startup auto-resume split out of `app_runtime/mod.rs` for
//! SPEC-3064 Phase 1 (Pass 2).
//!
//! Owns:
//! - [`AppRuntime::bootstrap`] (one-shot startup work: retroactive merge
//!   migration, recovery-session restore queueing, ingest kicks)
//! - The startup auto-resume queue and its geometry / freshness helpers
//!   ([`AppRuntime::queue_startup_auto_resume_sessions`],
//!   [`AppRuntime::startup_auto_resume_ready_events`],
//!   `startup_auto_resume_window_geometry`, `startup_auto_resume_is_fresh`,
//!   `mark_auto_resume_source_completed`, ...)
//! - Restoring open-project windows / paused placeholders
//!   ([`AppRuntime::restore_open_project_windows`],
//!   [`AppRuntime::spawn_restored_agent_session`])
//! - Late runtime wiring setters ([`AppRuntime::set_hook_forward_target`],
//!   [`AppRuntime::set_server_url`], [`AppRuntime::set_usage_refresh`])
//!
//! Behavior-preserving move: `AppRuntime::new` and
//! `PendingStartupAutoResumeSession` stay in `mod.rs`.

use std::path::Path;

use super::{
    combined_window_id, launch_config_from_persisted_session, same_worktree_path,
    should_auto_start_restored_window, workspace_resume_context_for_work_item, AppRuntime,
    HookForwardTarget, OutboundEvent, PendingStartupAutoResumeSession, WindowGeometry,
    WindowPreset, WindowProcessStatus, WorkspaceResumeContext,
};

const STARTUP_AUTO_RESUME_STALE_AFTER_SECS: i64 = 24 * 60 * 60;
const STARTUP_AUTO_RESUME_STACK_OFFSET_X: f64 = 28.0;
const STARTUP_AUTO_RESUME_STACK_OFFSET_Y: f64 = 24.0;

fn startup_auto_resume_window_geometry(
    index: usize,
    total: usize,
    bounds: gwt::WindowGeometry,
) -> gwt::WindowGeometry {
    let (width, height) = WindowPreset::Agent.default_size();
    let stack_steps = total.saturating_sub(1) as f64;
    let index = index as f64;
    gwt::WindowGeometry {
        x: bounds.x + (bounds.width - width) / 2.0
            - (stack_steps * STARTUP_AUTO_RESUME_STACK_OFFSET_X) / 2.0
            + index * STARTUP_AUTO_RESUME_STACK_OFFSET_X,
        y: bounds.y + (bounds.height - height) / 2.0
            - (stack_steps * STARTUP_AUTO_RESUME_STACK_OFFSET_Y) / 2.0
            + index * STARTUP_AUTO_RESUME_STACK_OFFSET_Y,
        width,
        height,
    }
}

fn session_project_scope_hash(session: &gwt_agent::Session) -> Option<String> {
    session
        .repo_hash
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            session
                .worktree_path
                .exists()
                .then(|| gwt_core::paths::project_scope_hash(&session.worktree_path).to_string())
        })
}

fn startup_auto_resume_is_fresh(
    session: &gwt_agent::Session,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    now.signed_duration_since(session.last_activity_at)
        <= chrono::Duration::seconds(STARTUP_AUTO_RESUME_STALE_AFTER_SECS)
}

fn startup_auto_resume_window_was_open(session: &gwt_agent::Session) -> bool {
    if session.restore_window_on_startup {
        return true;
    }
    // Compatibility for sessions saved before the explicit GUI restore flag
    // existed, and for files already migrated once with that flag defaulted.
    session.status != gwt_agent::AgentStatus::Stopped
}

pub(super) fn mark_auto_resume_source_completed(sessions_dir: &Path, session_id: &str) {
    let _ = gwt_agent::update_session(sessions_dir, session_id, |session| {
        session.update_status(gwt_agent::AgentStatus::Stopped);
        session.restore_window_on_startup = false;
        Ok(())
    });
}

impl AppRuntime {
    pub(crate) fn bootstrap(&mut self) {
        // SPEC-2359 US-37 / FR-119 / FR-123: One-shot retroactive migration to
        // mark historical merged `work/*` Start Work Workspaces as Done so the
        // Workspace Overview Completed column reflects past completions on the
        // first startup after auto-done emission lands. The scan is idempotent
        // per `work_item_id` and skips silently when journal / work_events
        // files are missing or unreadable.
        let now = chrono::Utc::now();
        for tab in &self.tabs {
            let _ =
                gwt_core::workspace_projection::retroactive_auto_done_scan(&tab.project_root, now);
            // SPEC-2359 US-39 / FR-142..145: backfill Phase U-6 schema
            // additions (`summary`, `created_at`, `creator`,
            // `lifecycle_stage`) on legacy `workspace.json` files. Runs
            // alongside the auto-done scan above with independent helpers
            // and an independent `workspace.migration.json` marker, so the
            // two migrations are exactly-once each and never duplicate work.
            // Errors are silently dropped (`let _ = ...`) so a corrupt or
            // unreadable Workspace cannot block daemon startup.
            let _ = gwt_core::workspace_projection_migration::migrate_workspace_projection_for_repo(
                &tab.project_root,
            );
            // SPEC-2359 Phase W-16 (FR-393): decompose legacy mega-items
            // (pre-W-12 records keyed to one projection UUID fusing dozens of
            // branches) into canonical branch-keyed items so each branch row
            // shows its real title / sessions. Idempotent; must run before
            // the intake/reconcile chain so decomposed branches are not
            // redundantly backfilled.
            let _ = gwt_core::workspace_projection::decompose_legacy_multi_branch_work_items(
                &tab.project_root,
            );
            // SPEC-2359 W-16 (FR-387): cross-machine work events intake.
            // Supersedes the one-shot `rebuild_work_items_from_events_for_repo`
            // migration gate — the intake is a permanently-installed idempotent
            // consumer over the same (and more) sources. Runs on a background
            // thread; its completion event then runs the worktree reconcile
            // (intake → reconcile order) and the merge scan.
            self.spawn_work_events_ingest(tab.project_root.clone(), true);
            // SPEC-2359 Phase W-11 (US-58 / FR-346): one-shot, version-guarded
            // clear of legacy prompt-derived title_summary / current_focus so
            // existing broken titles ("あなたの目的は何ですか" etc.) heal via the
            // display fallback and agent re-authoring. Idempotent via
            // `agent_identity.migration.json`; never re-clears agent-authored
            // values written after the marker.
            let _ = gwt_core::workspace_projection::reset_legacy_agent_identity_for_repo(
                &tab.project_root,
            );
        }

        self.queue_startup_auto_resume_sessions();

        // SPEC-3214 T-006: reclaim crash-orphaned `.intake-*` worktrees. Live
        // sessions do not exist yet at bootstrap, but queued auto-resume
        // sessions own their worktrees — keep those. Runs on a background
        // thread (git worktree list + status are IO).
        {
            let live_worktrees: Vec<std::path::PathBuf> = self
                .pending_startup_auto_resume_sessions
                .iter()
                .map(|pending| pending.session.worktree_path.clone())
                .collect();
            let project_roots: Vec<std::path::PathBuf> = self
                .tabs
                .iter()
                .map(|tab| tab.project_root.clone())
                .collect();
            self.blocking_tasks.spawn(move || {
                for project_root in project_roots {
                    let removed = super::launch::prune_orphan_intake_worktrees(
                        &project_root,
                        &live_worktrees,
                    );
                    if removed > 0 {
                        tracing::info!(
                            project_root = %project_root.display(),
                            removed,
                            "pruned orphan intake worktrees at startup"
                        );
                    }
                }
            });
        }

        let windows = self
            .tabs
            .iter()
            .flat_map(|tab| {
                tab.workspace
                    .persisted()
                    .windows
                    .clone()
                    .into_iter()
                    .map(|window| (tab.id.clone(), window))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        for (tab_id, window) in windows {
            if !should_auto_start_restored_window(&window) {
                continue;
            }
            let _ = self.start_window(&tab_id, &window.id, window.preset, window.geometry.clone());
        }
        let _ = self.persist();
    }

    fn queue_startup_auto_resume_sessions(&mut self) {
        self.pending_startup_auto_resume_sessions.clear();
        let mut sessions = self.load_recovery_sessions();
        sessions.sort_by(|left, right| {
            right
                .last_activity_at
                .cmp(&left.last_activity_at)
                .then_with(|| left.id.cmp(&right.id))
        });

        let now = chrono::Utc::now();
        let mut resumed_native_sessions = std::collections::HashSet::new();
        for session in sessions {
            // Issue #2942: a persisted Stopped agent placeholder means the user
            // did not explicitly close the window (closing removes it from the
            // workspace). Such "still open" windows must restore regardless of
            // the session's status drift (e.g. idle-timeout -> Stopped) or age,
            // honoring "restore everything not explicitly closed". Sessions with
            // no placeholder are orphans (the workspace lost the window); keep
            // the conservative status / freshness gates so old, windowless
            // sessions are not resurrected at startup.
            // SPEC-2359 G: a Session whose worktree no longer exists on this
            // machine (moved machines, deleted repo, a path from another OS)
            // cannot be auto-resumed; skip here so a stale path never reaches an
            // async spawn that fails later. Applies to both placeholder and
            // orphan sessions (orphans previously skipped this check).
            if !session.worktree_path.exists() {
                continue;
            }
            let placeholder_tab = self.paused_placeholder_tab_for_session(&session.id);
            // Orphan sessions (workspace lost the window) keep the conservative
            // status / freshness gates so old, windowless sessions are not
            // resurrected; placeholder sessions restore regardless (Issue #2942).
            if placeholder_tab.is_none() {
                if !startup_auto_resume_window_was_open(&session) {
                    continue;
                }
                if !session.exact_auto_resume_candidate() {
                    continue;
                }
                if !startup_auto_resume_is_fresh(&session, now) {
                    continue;
                }
            }
            let Some(native_session_id) = session.exact_resume_session_id() else {
                continue;
            };
            if !resumed_native_sessions.insert(native_session_id.to_string()) {
                continue;
            }
            if self
                .active_agent_sessions
                .values()
                .any(|active| active.session_id == session.id)
            {
                continue;
            }
            let Some(tab_id) =
                placeholder_tab.or_else(|| self.auto_resume_tab_id_for_session(&session))
            else {
                continue;
            };
            let Some(tab) = self.tab(&tab_id) else {
                continue;
            };
            if tab.kind != gwt::ProjectKind::Git || tab.migration_pending {
                continue;
            }
            let config = launch_config_from_persisted_session(&session);
            if config.session_mode != gwt_agent::SessionMode::Resume {
                continue;
            }
            let workspace_resume_context = Some(workspace_resume_context_for_work_item(
                &session.worktree_path,
                Some(session.branch.as_str()),
                &session.worktree_path,
            ));
            self.pending_startup_auto_resume_sessions
                .push(PendingStartupAutoResumeSession {
                    tab_id,
                    session,
                    workspace_resume_context,
                });
        }
    }

    pub(super) fn startup_auto_resume_ready_events(
        &mut self,
        bounds: WindowGeometry,
    ) -> Vec<OutboundEvent> {
        if self.pending_startup_auto_resume_sessions.is_empty() {
            return Vec::new();
        }

        let pending = std::mem::take(&mut self.pending_startup_auto_resume_sessions);
        let total = pending.len();
        let mut events = Vec::new();
        for (index, pending_session) in pending.into_iter().enumerate() {
            let fallback_geometry =
                startup_auto_resume_window_geometry(index, total, bounds.clone());
            let mut spawned = self.spawn_restored_agent_session(
                &pending_session.tab_id,
                pending_session.session,
                pending_session.workspace_resume_context,
                fallback_geometry,
            );
            events.append(&mut spawned);
        }
        events
    }

    /// Spawn a single restored agent window from a persisted session, reusing
    /// the paused placeholder's geometry when present (Issue #2942). Shared by
    /// startup auto-resume and the Open Project restore path so both honor the
    /// "restore everything the user did not explicitly close" rule. Records the
    /// source session in `pending_auto_resume_sources` so the lifecycle handler
    /// retires the old session once the resumed window reports its own id.
    fn spawn_restored_agent_session(
        &mut self,
        tab_id: &str,
        session: gwt_agent::Session,
        workspace_resume_context: Option<WorkspaceResumeContext>,
        fallback_geometry: WindowGeometry,
    ) -> Vec<OutboundEvent> {
        let config = launch_config_from_persisted_session(&session);
        let geometry = self
            .remove_stale_paused_agent_window(tab_id, &session.id)
            .unwrap_or(fallback_geometry);
        // Snapshot the window registry *after* the paused placeholder is
        // removed: the freshly spawned window may reuse the placeholder's id
        // (ids are assigned lowest-free), so a pre-removal snapshot would fail
        // to detect it and the source session would never be retired.
        let existing_windows = self
            .window_lookup
            .keys()
            .cloned()
            .collect::<std::collections::HashSet<_>>();
        match self.spawn_agent_window_at_geometry(
            tab_id,
            config,
            geometry,
            workspace_resume_context,
        ) {
            Ok(events) => {
                if let Some(window_id) = self
                    .window_lookup
                    .keys()
                    .find(|window_id| !existing_windows.contains(*window_id))
                    .cloned()
                {
                    self.pending_auto_resume_sources
                        .insert(window_id, session.id);
                }
                events
            }
            Err(error) => {
                tracing::warn!(
                    session_id = %session.id,
                    error = %error,
                    "failed to spawn restored agent window"
                );
                Vec::new()
            }
        }
    }

    /// SPEC-2356 安心 Addendum (FR-044): relaunch a stopped/errored `Agent`
    /// window in place. Reuses the same persisted-Session resume primitive the
    /// startup window restore uses ([`Self::spawn_restored_agent_session`]),
    /// which removes the paused placeholder and re-spawns the agent into the
    /// reused window id, preserving the window and appending to its prior
    /// output. Returns an empty event list when the window has no resumable
    /// Session (e.g. a never-launched placeholder) so the kill-switch UI can
    /// surface "nothing to restart" instead of spawning a blank agent.
    pub(crate) fn restart_agent_window_in_place(
        &mut self,
        tab_id: &str,
        raw_id: &str,
        fallback_geometry: WindowGeometry,
    ) -> Vec<OutboundEvent> {
        let Some(session_id) = self
            .tab(tab_id)
            .and_then(|tab| tab.workspace.window(raw_id))
            .and_then(|window| window.session_id.clone())
        else {
            return Vec::new();
        };
        let path = self.sessions_dir.join(format!("{session_id}.toml"));
        let Ok(session) = gwt_agent::Session::load_and_migrate(&path) else {
            return Vec::new();
        };
        let workspace_resume_context = Some(workspace_resume_context_for_work_item(
            &session.worktree_path,
            Some(session.branch.as_str()),
            &session.worktree_path,
        ));
        let mut events = vec![self.workspace_state_broadcast()];
        events.append(&mut self.spawn_restored_agent_session(
            tab_id,
            session,
            workspace_resume_context,
            fallback_geometry,
        ));
        events
    }

    /// Restore every process window the user did not explicitly close in a
    /// freshly opened/restored project tab (Issue #2942). Closing a window
    /// removes it from the persisted workspace, so the persisted process
    /// windows are exactly the set to restart: agents resume via their native
    /// session id (or launch fresh when none exists), and non-agent process
    /// windows (e.g. Shell) launch fresh. Runs synchronously because each
    /// placeholder already carries its geometry, so no frontend canvas bounds
    /// round-trip is required. The startup `bootstrap` queue only covers tabs
    /// open at launch, so projects opened via Open Project / Reopen Recent were
    /// never restored before this path existed.
    pub(super) fn restore_open_project_windows(&mut self, tab_id: &str) -> Vec<OutboundEvent> {
        let windows = match self.tab(tab_id) {
            Some(tab) if tab.kind == gwt::ProjectKind::Git && !tab.migration_pending => tab
                .workspace
                .persisted()
                .windows
                .iter()
                .filter(|window| {
                    window.preset.requires_process()
                        && window.status == WindowProcessStatus::Stopped
                })
                .cloned()
                .collect::<Vec<_>>(),
            _ => return Vec::new(),
        };

        let mut events = Vec::new();
        for window in windows {
            let combined = combined_window_id(tab_id, &window.id);
            // A window with a live PTY/runtime is already running (e.g. when an
            // already-open project tab is re-selected); only paused placeholders
            // should be restarted. `window_lookup` is the registry of known
            // windows, not the set of running ones, so it must not gate here.
            if self.runtimes.contains_key(&combined) {
                continue;
            }
            if crate::runtime_support::window_is_agent_pane(&window) {
                let Some(session_id) = window.session_id.clone() else {
                    continue;
                };
                let path = self.sessions_dir.join(format!("{session_id}.toml"));
                let Ok(session) = gwt_agent::Session::load_and_migrate(&path) else {
                    continue;
                };
                if !session.worktree_path.exists() {
                    continue;
                }
                if self
                    .active_agent_sessions
                    .values()
                    .any(|active| active.session_id == session.id)
                {
                    continue;
                }
                let workspace_resume_context = Some(workspace_resume_context_for_work_item(
                    &session.worktree_path,
                    Some(session.branch.as_str()),
                    &session.worktree_path,
                ));
                let fallback_geometry = window.geometry.clone();
                let mut spawned = self.spawn_restored_agent_session(
                    tab_id,
                    session,
                    workspace_resume_context,
                    fallback_geometry,
                );
                events.append(&mut spawned);
            } else {
                events.extend(self.start_window(
                    tab_id,
                    &window.id,
                    window.preset,
                    window.geometry.clone(),
                ));
            }
        }
        events
    }

    /// Find the tab holding a persisted, paused (`Stopped`) agent placeholder
    /// window backed by `session_id`. Its presence proves the user did not
    /// explicitly close that window (Issue #2942), so the session must restore
    /// regardless of status drift or age.
    fn paused_placeholder_tab_for_session(&self, session_id: &str) -> Option<String> {
        self.tabs
            .iter()
            .filter(|tab| tab.kind == gwt::ProjectKind::Git && !tab.migration_pending)
            .find(|tab| {
                tab.workspace.persisted().windows.iter().any(|window| {
                    window.status == WindowProcessStatus::Stopped
                        && crate::runtime_support::window_is_agent_pane(window)
                        && window.session_id.as_deref() == Some(session_id)
                })
            })
            .map(|tab| tab.id.clone())
    }

    fn remove_stale_paused_agent_window(
        &mut self,
        tab_id: &str,
        session_id: &str,
    ) -> Option<WindowGeometry> {
        let tab = self.tab_mut(tab_id)?;
        let stale = tab
            .workspace
            .persisted()
            .windows
            .iter()
            .find(|w| {
                w.preset == WindowPreset::Agent
                    && w.status == WindowProcessStatus::Stopped
                    && w.session_id.as_deref() == Some(session_id)
            })
            .map(|w| (w.id.clone(), w.geometry.clone()));
        let (raw_id, geometry) = stale?;
        tab.workspace.close_window(&raw_id);
        let combined = combined_window_id(tab_id, &raw_id);
        self.window_lookup.remove(&combined);
        self.window_details.remove(&combined);
        Some(geometry)
    }

    fn auto_resume_tab_id_for_session(&self, session: &gwt_agent::Session) -> Option<String> {
        if let Some(tab) = self.tabs.iter().find(|tab| {
            tab.kind == gwt::ProjectKind::Git
                && !tab.migration_pending
                && same_worktree_path(&tab.project_root, &session.worktree_path)
        }) {
            return Some(tab.id.clone());
        }

        // Issue #2942: a session's worktree belongs to the tab whose project
        // shares the same main worktree root (the gwt workspace home / bare
        // layout root). `repo_hash` / `project_scope_hash` differ between a
        // workspace-home project_root and its linked worktrees, so scope-hash
        // equality alone fails to associate worktree-backed agent sessions with
        // the parent tab and they never auto-resume on startup.
        if let Ok(session_root) = gwt_git::worktree::main_worktree_root(&session.worktree_path) {
            if let Some(tab) = self.tabs.iter().find(|tab| {
                tab.kind == gwt::ProjectKind::Git
                    && !tab.migration_pending
                    && same_worktree_path(&tab.main_worktree_root(), &session_root)
            }) {
                return Some(tab.id.clone());
            }
        }

        let session_scope = session_project_scope_hash(session)?;
        self.tabs
            .iter()
            .find(|tab| {
                tab.kind == gwt::ProjectKind::Git
                    && !tab.migration_pending
                    && gwt_core::paths::project_scope_hash(&tab.project_root).to_string()
                        == session_scope
            })
            .map(|tab| tab.id.clone())
    }

    fn load_recovery_sessions(&self) -> Vec<gwt_agent::Session> {
        let Ok(entries) = std::fs::read_dir(&self.sessions_dir) else {
            return Vec::new();
        };
        entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("toml"))
            .filter_map(|path| {
                let session_id = path.file_stem()?.to_str()?;
                gwt_agent::update_session(&self.sessions_dir, session_id, |session| {
                    if session.worktree_path.exists()
                        && session.should_mark_interrupted_from_lifecycle()
                    {
                        session.update_status(gwt_agent::AgentStatus::Interrupted);
                    }
                    Ok(())
                })
                .ok()
            })
            .collect()
    }

    pub(crate) fn set_hook_forward_target(&mut self, target: HookForwardTarget) {
        self.hook_forward_target = Some(target);
    }

    /// SPEC-2785 FR-E: capture the embedded server URL after the axum bind
    /// completes so `open_server_url_events` can reject mismatched origin
    /// requests before invoking the OS opener.
    pub(crate) fn set_server_url(&mut self, url: String) {
        self.server_url = Some(url);
    }

    /// SPEC-2970: wire the usage poller's refresh handle so frontend toggles
    /// can request an immediate re-poll.
    pub(crate) fn set_usage_refresh(&mut self, refresh: std::sync::Arc<tokio::sync::Notify>) {
        self.usage_refresh = Some(refresh);
    }
}
