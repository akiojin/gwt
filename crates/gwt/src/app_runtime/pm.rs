//! Resident PM pane lifecycle (SPEC-3431): the GUI-side singleton gate.
//!
//! The durable half of the singleton (project-state `pm.json`) lives in
//! `gwt::pm_registry`; this module supplies the authoritative liveness
//! answer from the in-memory pane registry and drives the ensure flow:
//! live PM → focus, stale registration → resume the same conversation,
//! nothing registered → fresh silent spawn (branchless, explicit PM
//! worktree, `$gwt-pm` bootstrap prompt).
//!
//! Fresh spawns deliberately avoid the Launch Wizard profile machinery:
//! a silent-launch profile does not exist on a fresh project (the Issue
//! Monitor bootstrap trap), so the PM uses a fixed default agent instead.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use gwt::persistence::{WindowGeometry, WindowProcessStatus};
use gwt::pm_registry::{self, PmRegistration};

use super::{AppRuntime, OutboundEvent};

/// Fixed geometry for a freshly spawned PM pane.
const PM_WINDOW_GEOMETRY: WindowGeometry = WindowGeometry {
    x: 96.0,
    y: 96.0,
    width: 860.0,
    height: 520.0,
};

/// Bootstrap prompt: invokes the materialized gwt-pm guidance skill.
const PM_BOOTSTRAP_PROMPT: &str = "$gwt-pm";

impl AppRuntime {
    /// SPEC-3431 FR-001/FR-002: ensure the resident PM pane for `tab_id`.
    ///
    /// Respects the `auto_start` opt-out, focuses a live PM instead of
    /// spawning a duplicate, resumes a stale registration's conversation
    /// when the durable session is still materializable, and otherwise
    /// performs a fresh silent spawn. Never spawns implementation agents.
    pub(crate) fn ensure_pm_agent_for_tab(&mut self, tab_id: &str) -> Vec<OutboundEvent> {
        #[cfg(test)]
        if !test_gate::PM_ENSURE_ENABLED.with(|cell| cell.get()) {
            return Vec::new();
        }
        let Some(tab) = self.tab(tab_id) else {
            return Vec::new();
        };
        if tab.kind != gwt::ProjectKind::Git || tab.migration_pending {
            return Vec::new();
        }
        let project_root = tab.project_root.clone();
        let prefs_path = pm_registry::pm_prefs_path_for_repo_path(&project_root);
        let prefs = match pm_registry::load_pm_prefs(&prefs_path) {
            Ok(prefs) => prefs,
            Err(error) => {
                tracing::warn!(
                    path = %prefs_path.display(),
                    %error,
                    "failed to load PM prefs; skipping PM ensure"
                );
                return Vec::new();
            }
        };
        if !prefs.settings.auto_start {
            return Vec::new();
        }
        let Some(registration) = prefs.registration else {
            return self.spawn_pm_agent(tab_id, &project_root);
        };
        if let Some(window_id) = self.live_pm_window_id(&registration.session_id) {
            return self.focus_existing_live_work_agent_events(&window_id, None);
        }
        // FR-003 crash-loop damper: while the backoff floor is in the future
        // the ensure gate must not respawn; the next project open (or manual
        // action) after the floor recovers the PM.
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        if !pm_registry::pm_respawn_allowed(&registration, &now) {
            return Vec::new();
        }
        // Stale registration (FR-003): resume the same conversation when the
        // durable session is still materializable; otherwise fall back to a
        // fresh spawn whose completion replaces the stale registration.
        let session_path = self
            .sessions_dir
            .join(format!("{}.toml", registration.session_id));
        if let Ok(session) = gwt_agent::Session::load_and_migrate(&session_path) {
            if session.worktree_path.exists() {
                let before = self.pm_window_ids(tab_id);
                let events =
                    self.spawn_restored_agent_session(tab_id, session, None, PM_WINDOW_GEOMETRY);
                self.mark_new_pm_windows(tab_id, &before, &project_root);
                return events;
            }
        }
        self.spawn_pm_agent(tab_id, &project_root)
    }

    /// Authoritative liveness for a stored PM registration (FR-001): the
    /// registered session must have a live pane right now.
    pub(crate) fn pm_registration_is_live(&self, registration: &PmRegistration) -> bool {
        self.live_pm_window_id(&registration.session_id).is_some()
    }

    /// Window id of the live pane bound to `session_id`, if any: the session
    /// is in the active registry, its window is still on the canvas, and the
    /// composed window status is not terminal.
    pub(crate) fn live_pm_window_id(&self, session_id: &str) -> Option<String> {
        self.active_agent_sessions
            .iter()
            .find_map(|(window_id, session)| {
                if session.session_id != session_id || !self.window_lookup.contains_key(window_id) {
                    return None;
                }
                match self.window_status(window_id) {
                    Some(WindowProcessStatus::Stopped) | Some(WindowProcessStatus::Error) => None,
                    _ => Some(window_id.clone()),
                }
            })
    }

    /// SPEC-3431 FR-001: called by `handle_launch_complete` once the PM
    /// launch produced a real session. Writes the durable registration,
    /// replacing a stale one; a concurrently live PM (which the ensure gate
    /// should have prevented) is left untouched and logged.
    pub(crate) fn register_pm_after_launch(
        &mut self,
        project_root: &Path,
        session_id: &str,
        agent_id: &str,
        worktree_path: &Path,
    ) {
        let prefs_path = pm_registry::pm_prefs_path_for_repo_path(project_root);
        let existing_alive = match pm_registry::load_pm_prefs(&prefs_path) {
            Ok(prefs) => prefs
                .registration
                .as_ref()
                .is_some_and(|existing| self.pm_registration_is_live(existing)),
            Err(_) => false,
        };
        let candidate = PmRegistration {
            session_id: session_id.to_string(),
            agent_id: agent_id.to_string(),
            worktree_path: worktree_path.to_string_lossy().into_owned(),
            created_at: Some(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
            consecutive_crashes: 0,
            next_not_before: None,
        };
        match pm_registry::try_register_pm(&prefs_path, candidate, |_| existing_alive) {
            Ok((_, pm_registry::PmRegisterOutcome::RejectedLive { existing })) => {
                tracing::warn!(
                    existing_session = %existing.session_id,
                    new_session = %session_id,
                    "PM launch completed while another live PM is registered; keeping existing"
                );
            }
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(%error, "failed to write PM registration after launch");
            }
        }
    }

    /// FR-013: an explicit close of the PM pane clears the registration so
    /// nothing auto-restarts it. Settings (auto_start) survive.
    pub(super) fn deregister_pm_for_closed_window(
        &mut self,
        project_root: &Path,
        session_id: &str,
    ) {
        let prefs_path = pm_registry::pm_prefs_path_for_repo_path(project_root);
        match pm_registry::deregister_pm(&prefs_path, session_id) {
            Ok((_, true)) => {
                tracing::info!(%session_id, "PM pane closed; registration cleared");
            }
            Ok((_, false)) => {}
            Err(error) => {
                tracing::warn!(%error, "failed to deregister PM on window close");
            }
        }
    }

    /// FR-003: an unexpected exit of the registered PM records one crash on
    /// the backoff ladder and, when the ladder allows, respawns immediately
    /// by re-running the ensure gate (which resumes the same conversation).
    pub(super) fn handle_pm_crash(
        &mut self,
        tab_id: &str,
        project_root: &Path,
        session_id: &str,
    ) -> Vec<OutboundEvent> {
        let prefs_path = pm_registry::pm_prefs_path_for_repo_path(project_root);
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let respawn_now = match pm_registry::mutate_pm_prefs(&prefs_path, |prefs| {
            let registration = prefs.registration.as_mut()?;
            if registration.session_id != session_id {
                return None;
            }
            Some(pm_registry::apply_pm_crash_backoff(registration, &now))
        }) {
            Ok((_, Some(respawn_now))) => respawn_now,
            Ok((_, None)) => return Vec::new(),
            Err(error) => {
                tracing::warn!(%error, "failed to record PM crash backoff");
                return Vec::new();
            }
        };
        if !respawn_now {
            return Vec::new();
        }
        self.ensure_pm_agent_for_tab(tab_id)
    }

    fn pm_window_ids(&self, tab_id: &str) -> HashSet<String> {
        self.tab(tab_id)
            .map(|tab| {
                tab.workspace
                    .persisted()
                    .windows
                    .iter()
                    .map(|window| window.id.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Track windows added by a PM spawn so launch completion can write the
    /// registration (keyed by combined window id, mapped to project root).
    fn mark_new_pm_windows(&mut self, tab_id: &str, before: &HashSet<String>, project_root: &Path) {
        let new_ids: Vec<String> = self
            .tab(tab_id)
            .map(|tab| {
                tab.workspace
                    .persisted()
                    .windows
                    .iter()
                    .filter(|window| !before.contains(&window.id))
                    .map(|window| crate::runtime_support::combined_window_id(tab_id, &window.id))
                    .collect()
            })
            .unwrap_or_default();
        for combined in new_ids {
            self.pending_pm_launches
                .insert(combined, project_root.to_path_buf());
        }
    }

    fn spawn_pm_agent(&mut self, tab_id: &str, project_root: &Path) -> Vec<OutboundEvent> {
        let worktree = match Self::ensure_pm_worktree(project_root) {
            Ok(path) => path,
            Err(error) => {
                tracing::warn!(
                    project_root = %project_root.display(),
                    error,
                    "failed to prepare the PM worktree; PM not started"
                );
                return Vec::new();
            }
        };
        // T-052: the `$gwt-pm` bootstrap prompt resolves against the guidance
        // skill materialized in the PM worktree (both provider mirrors).
        if let Err(error) = gwt_skills::pm_guidance::generate_pm_guidance(&worktree) {
            tracing::warn!(%error, "failed to materialize gwt-pm guidance");
        }
        let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::ClaudeCode)
            .working_dir(worktree)
            .extra_arg(PM_BOOTSTRAP_PROMPT)
            .build();
        config.suppress_execution_control = true;
        let before = self.pm_window_ids(tab_id);
        match self.spawn_agent_window_at_geometry(tab_id, config, PM_WINDOW_GEOMETRY, None) {
            Ok(events) => {
                self.mark_new_pm_windows(tab_id, &before, project_root);
                events
            }
            Err(error) => {
                tracing::warn!(error, "PM silent spawn failed");
                Vec::new()
            }
        }
    }

    /// Dedicated detached worktree for the PM session (research R-10). Its
    /// lifecycle is bound to the PM registration; T-016 adds GC.
    fn ensure_pm_worktree(project_root: &Path) -> Result<PathBuf, String> {
        let path = gwt_core::paths::gwt_project_dir_for_repo_path(project_root).join("pm/worktree");
        if path.join(".git").exists() {
            return Ok(path);
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let manager = gwt_git::WorktreeManager::new(project_root);
        manager
            .create_detached("HEAD", &path)
            .map_err(|error| error.to_string())?;
        Ok(path)
    }
}

/// Test-only gate: the PM ensure is opt-in per test thread. Without it every
/// pre-existing startup/restore test that drives bootstrap or project-open
/// would spawn a PM pane — and its async launch thread — polluting session
/// and window counts across the suite. Production builds compile this out.
#[cfg(test)]
pub(crate) mod test_gate {
    use std::cell::Cell;

    thread_local! {
        pub(crate) static PM_ENSURE_ENABLED: Cell<bool> = const { Cell::new(false) };
    }

    /// RAII enable for PM ensure in a test; resets on drop.
    pub(crate) struct PmEnsureTestGuard;

    impl PmEnsureTestGuard {
        pub(crate) fn enable() -> Self {
            PM_ENSURE_ENABLED.with(|cell| cell.set(true));
            PmEnsureTestGuard
        }
    }

    impl Drop for PmEnsureTestGuard {
        fn drop(&mut self) {
            PM_ENSURE_ENABLED.with(|cell| cell.set(false));
        }
    }
}
