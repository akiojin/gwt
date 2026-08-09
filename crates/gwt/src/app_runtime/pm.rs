//! Resident PM pane lifecycle (SPEC-3431): the GUI-side singleton gate.
//!
//! The durable half of the singleton (project-state `pm.json`) lives in
//! `gwt::pm_registry`; this module supplies the authoritative liveness
//! answer from the in-memory pane registry and drives the ensure flow:
//! live PM → focus, stale registration → resume the same conversation,
//! nothing registered → fresh silent spawn (branchless, explicit PM
//! worktree, `$gwt-pm` bootstrap prompt).
//!
//! The gwt-pm guidance skill that prompt resolves against is written by
//! `managed_assets::materialize_managed_gwt_assets_for_targets` — the single
//! writer, keyed on the canonical PM worktree path. Fresh spawn, resume,
//! crash respawn, and every later refresh funnel through it, so this module
//! never materializes the skill itself.
//!
//! Fresh spawns read the project's own `PmSettings::launch_profile` and fall
//! back to a fixed default. They deliberately avoid the Launch Wizard profile
//! machinery: those profiles are derived from unrelated launches and do not
//! exist at all on a fresh project (the Issue Monitor bootstrap trap), and the
//! PM must come up regardless.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use gwt::persistence::{WindowGeometry, WindowProcessStatus};
use gwt::pm_registry::{self, PmLaunchProfile, PmRegistration};
use gwt::PmAgentOption;

use super::{AppRuntime, BackendEvent, OutboundEvent};

/// Fixed geometry for a freshly spawned PM pane.
const PM_WINDOW_GEOMETRY: WindowGeometry = WindowGeometry {
    x: 96.0,
    y: 96.0,
    width: 860.0,
    height: 520.0,
};

/// Bootstrap prompt: invokes the materialized gwt-pm guidance skill.
const PM_BOOTSTRAP_PROMPT: &str = "$gwt-pm";

/// SPEC-3431 T-093 (FR-012): a wake the monitor-event path decided on — which
/// pane receives the prompt and what it says. The window id is only ever the
/// registered PM session's live pane, resolved inside the decision; no caller
/// can point the wake anywhere else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PmWakeDecision {
    pub(crate) window_id: String,
    pub(crate) prompt: String,
}

/// What one monitor inbox snapshot contributes to the wake decision: every
/// issue the monitor is holding, plus an extra marker for the rows a human
/// must look at. A signal string appearing for the first time is "new
/// activity"; the sets are compared, never interpreted.
fn pm_wake_signals(inbox: &[gwt::IssueMonitorInboxItem]) -> std::collections::BTreeSet<String> {
    let mut signals = std::collections::BTreeSet::new();
    for item in inbox {
        signals.insert(format!("issue:{}", item.issue.number));
        if item.state == gwt::MonitorInboxState::NeedsHuman {
            signals.insert(format!("needs_human:{}", item.issue.number));
        }
    }
    signals
}

/// An actively-looping PM picks new events up in its own next cycle, and a PM
/// a human just prompted is busy with that conversation — the wake is only
/// for a loop quiet on both clocks (parked on the budget cap, or dead after a
/// within-floor stop, with no recent user contact). Same interval knob as the
/// Stop-gate driver.
fn pm_wake_loop_is_quiet(state: &pm_registry::PmLoopState, interval_secs: u64, now: &str) -> bool {
    let instant_is_quiet = |instant: Option<&str>| {
        let Some(instant) = instant else {
            return true;
        };
        match (
            chrono::DateTime::parse_from_rfc3339(now),
            chrono::DateTime::parse_from_rfc3339(instant),
        ) {
            (Ok(now_t), Ok(instant_t)) => {
                (now_t - instant_t).num_seconds()
                    >= i64::try_from(interval_secs).unwrap_or(i64::MAX)
            }
            _ => true,
        }
    };
    instant_is_quiet(state.last_continued_at.as_deref())
        && instant_is_quiet(state.last_user_prompt_at.as_deref())
}

/// Who asked for the PM.
///
/// SPEC-3431 FR-002's `auto_start` opt-out scopes to "opening a project starts
/// the PM automatically". It is not a lock on the user: FR-021 requires the PM
/// launcher to start a stopped PM when clicked, and FR-003 promises a crash
/// resumes. Collapsing both into one gate leaves the launcher and the Restart
/// button silently dead with no way back short of editing pm.json by hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PmEnsureTrigger {
    /// Project open / startup restore. Honours the opt-out.
    Automatic,
    /// Launcher click, Restart, crash recovery. Ignores the opt-out.
    Explicit,
}

impl AppRuntime {
    /// SPEC-3431 FR-001/FR-002: ensure the resident PM pane for `tab_id`.
    ///
    /// Focuses a live PM instead of spawning a duplicate, resumes a stale
    /// registration's conversation when the durable session is still
    /// materializable, and otherwise performs a fresh silent spawn. Never
    /// spawns implementation agents.
    pub(crate) fn ensure_pm_agent_for_tab(
        &mut self,
        tab_id: &str,
        trigger: PmEnsureTrigger,
    ) -> Vec<OutboundEvent> {
        self.ensure_pm_agent_for_tab_with_bounds(tab_id, None, trigger)
    }

    /// [`Self::ensure_pm_agent_for_tab`] with the caller's visible canvas
    /// bounds, so an existing PM is framed in the viewport (FR-019).
    pub(crate) fn ensure_pm_agent_for_tab_with_bounds(
        &mut self,
        tab_id: &str,
        canvas_bounds: Option<WindowGeometry>,
        trigger: PmEnsureTrigger,
    ) -> Vec<OutboundEvent> {
        #[cfg(test)]
        if !test_gate::PM_ENSURE_ENABLED.with(|cell| cell.get()) {
            return Vec::new();
        }
        let mut events = self.ensure_pm_agent_events(tab_id, canvas_bounds, trigger);
        // FR-026: the ensure gate is where the PM's live state changes, so it
        // is also where the settings panel learns about it. Skipped ensures
        // (opt-out, non-Git tab, backoff floor) still report — "not running"
        // is exactly the state the panel has to show.
        if self.active_tab_id.as_deref() == Some(tab_id) {
            events.extend(self.pm_status_broadcast_events());
        }
        events
    }

    fn ensure_pm_agent_events(
        &mut self,
        tab_id: &str,
        canvas_bounds: Option<WindowGeometry>,
        trigger: PmEnsureTrigger,
    ) -> Vec<OutboundEvent> {
        let Some(tab) = self.tab(tab_id) else {
            tracing::info!(tab_id, "PM ensure skipped: no such tab");
            return Vec::new();
        };
        if tab.kind != gwt::ProjectKind::Git || tab.migration_pending {
            tracing::info!(
                tab_id,
                migration_pending = tab.migration_pending,
                "PM ensure skipped: tab is not a migration-clear Git project"
            );
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
        self.sync_pm_session_cache(&project_root, prefs.registration.as_ref());
        if trigger == PmEnsureTrigger::Automatic && !prefs.settings.auto_start {
            tracing::info!(
                project_root = %project_root.display(),
                "PM ensure skipped: auto_start is opted out for this project"
            );
            return Vec::new();
        }
        let Some(registration) = prefs.registration else {
            tracing::info!(
                project_root = %project_root.display(),
                "PM ensure: no registration yet, spawning the resident PM"
            );
            return self.spawn_pm_agent(tab_id, &project_root);
        };
        if let Some(window_id) = self.live_pm_window_id(&registration.session_id) {
            // FR-019: the PM launcher must always land the user on the PM, so
            // focusing frames it in the viewport rather than only raising it.
            return self.focus_existing_live_work_agent_events(&window_id, canvas_bounds);
        }
        // FR-003 crash-loop damper: while the backoff floor is in the future
        // the automatic ladder must not respawn. Scoped to `Automatic` for the
        // same reason as the auto_start opt-out above — FR-021 requires the
        // explicit launcher/Restart click to start a stopped PM regardless.
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        if trigger == PmEnsureTrigger::Automatic
            && !pm_registry::pm_respawn_allowed(&registration, &now)
        {
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

    /// SPEC-3431 FR-026: the PM settings snapshot for the active project tab.
    ///
    /// `None` when there is no Git project to configure — the panel then keeps
    /// showing whatever it last had rather than being fed an empty project's
    /// defaults as if they were this one's.
    pub(crate) fn pm_status_event(&self) -> Option<BackendEvent> {
        let project_root = self.active_pm_project_root()?;
        let prefs_path = pm_registry::pm_prefs_path_for_repo_path(&project_root);
        let prefs = pm_registry::load_pm_prefs(&prefs_path).unwrap_or_default();
        let configured = prefs.settings.launch_profile_or_default();
        // Liveness is the pane registry's answer, not the file's: a
        // registration whose pane is gone is a stale record, and reporting it
        // as running would make the panel offer a restart for nothing.
        let running = prefs
            .registration
            .as_ref()
            .filter(|registration| self.pm_registration_is_live(registration));
        Some(BackendEvent::PmStatus {
            auto_start: prefs.settings.auto_start,
            agent_options: Self::pm_agent_options(&configured.agent_id),
            configured_agent_id: configured.agent_id,
            configured_model: configured.model,
            running_agent_id: running.map(|registration| registration.agent_id.clone()),
            is_running: running.is_some(),
        })
    }

    /// The PM settings snapshot as a broadcast, for the call sites that change
    /// PM state. Every PM state transition must pass through here — the panel
    /// has no other source of truth, so a silent transition leaves it stale.
    pub(crate) fn pm_status_broadcast_events(&self) -> Vec<OutboundEvent> {
        self.pm_status_event()
            .map(OutboundEvent::broadcast)
            .into_iter()
            .collect()
    }

    /// Selectable PM agents: the ones that can resolve `$gwt-pm`, narrowed to
    /// what is actually installed.
    ///
    /// `configured` is always offered even when it is not on PATH, so the
    /// picker can still show (and keep) the project's current choice instead of
    /// silently presenting a different agent as the configured one.
    fn pm_agent_options(configured: &str) -> Vec<PmAgentOption> {
        pm_registry::PM_SUPPORTED_AGENTS
            .iter()
            .filter(|id| **id == configured || which::which(id).is_ok())
            .map(|id| PmAgentOption {
                id: (*id).to_string(),
                name: gwt_agent::builtin_agent_descriptor_for_command(id)
                    .map_or_else(|| (*id).to_string(), |d| d.id.display_name().to_string()),
            })
            .collect()
    }

    fn active_pm_project_root(&self) -> Option<PathBuf> {
        let tab_id = self.active_tab_id.clone()?;
        self.tab(&tab_id).map(|tab| tab.project_root.clone())
    }

    /// SPEC-3431 FR-026/FR-002: persist the auto-start opt-out.
    ///
    /// Deliberately does not touch the running pane. The flag decides whether
    /// opening the project starts a PM; treating it as a stop switch would end
    /// a conversation the user only meant to stop auto-starting next time.
    pub(crate) fn set_pm_auto_start_events(&mut self, enabled: bool) -> Vec<OutboundEvent> {
        let Some(project_root) = self.active_pm_project_root() else {
            return Vec::new();
        };
        let prefs_path = pm_registry::pm_prefs_path_for_repo_path(&project_root);
        if let Err(error) = pm_registry::mutate_pm_prefs(&prefs_path, |prefs| {
            prefs.settings.auto_start = enabled;
        }) {
            tracing::warn!(%error, "failed to persist the PM auto-start setting");
            return Vec::new();
        }
        self.pm_status_broadcast_events()
    }

    /// SPEC-3431 FR-026: persist what the next PM start runs as.
    ///
    /// An agent without a managed `gwt-pm` skills mirror is refused rather than
    /// stored: `PmSettings::launch_profile_or_default` would silently fall back
    /// at launch time, leaving the panel claiming a configuration the PM never
    /// actually uses. The running pane is untouched — applying the change is
    /// [`Self::restart_pm_agent_events`].
    pub(crate) fn set_pm_launch_profile_events(
        &mut self,
        agent_id: &str,
        model: Option<String>,
        reasoning: Option<String>,
    ) -> Vec<OutboundEvent> {
        let Some(project_root) = self.active_pm_project_root() else {
            return Vec::new();
        };
        if !pm_registry::pm_agent_is_supported(agent_id) {
            tracing::warn!(
                agent_id,
                "rejected PM launch profile: the agent cannot resolve the gwt-pm skill"
            );
            return Vec::new();
        }
        let empty_to_none = |value: Option<String>| value.filter(|value| !value.trim().is_empty());
        let profile = PmLaunchProfile {
            agent_id: agent_id.to_string(),
            model: empty_to_none(model),
            reasoning: empty_to_none(reasoning),
            version: None,
        };
        let prefs_path = pm_registry::pm_prefs_path_for_repo_path(&project_root);
        if let Err(error) = pm_registry::mutate_pm_prefs(&prefs_path, |prefs| {
            prefs.settings.launch_profile = Some(profile.clone());
        }) {
            tracing::warn!(%error, "failed to persist the PM launch profile");
            return Vec::new();
        }
        self.pm_status_broadcast_events()
    }

    /// SPEC-3431 FR-026: apply the configured profile by restarting the PM.
    ///
    /// Deregisters first, on purpose: the close path treats a registered PM's
    /// close as an intentional stop and reaps a clean PM worktree with it
    /// (T-016). A restart is not a stop — the worktree holds the PM's own
    /// notes — so clearing the registration up front makes that reap a no-op
    /// and leaves the worktree for the successor.
    pub(crate) fn restart_pm_agent_events(&mut self) -> Vec<OutboundEvent> {
        let Some(tab_id) = self.active_tab_id.clone() else {
            return Vec::new();
        };
        let Some(project_root) = self.tab(&tab_id).map(|tab| tab.project_root.clone()) else {
            return Vec::new();
        };
        let prefs_path = pm_registry::pm_prefs_path_for_repo_path(&project_root);
        let registration = pm_registry::load_pm_prefs(&prefs_path)
            .ok()
            .and_then(|prefs| prefs.registration);
        let mut events = Vec::new();
        if let Some(registration) = registration {
            match pm_registry::deregister_pm(&prefs_path, &registration.session_id) {
                Ok(_) => self.sync_pm_session_cache(&project_root, None),
                Err(error) => {
                    tracing::warn!(%error, "failed to clear the PM registration for a restart");
                    return Vec::new();
                }
            }
            if let Some(window_id) = self.live_pm_window_id(&registration.session_id) {
                events.extend(self.close_window_events(&window_id));
            }
        }
        // The ensure gate broadcasts the post-restart pm_status itself, so the
        // panel is refreshed exactly once rather than twice per restart.
        events.extend(self.ensure_pm_agent_for_tab(&tab_id, PmEnsureTrigger::Explicit));
        events
    }

    /// SPEC-3431 T-093 (FR-012): decide whether this inbox snapshot must wake
    /// the resident PM, using `now` as the quiet-loop clock.
    ///
    /// The first snapshot per project is a baseline — GUI startup must not
    /// replay a long-lived backlog as if it just happened. After that, a
    /// signal never seen before wakes the PM iff the Monitor is enabled, a
    /// registered PM pane is live, and the resident loop has gone quiet.
    /// A delta suppressed only by an active loop is retained (not consumed),
    /// so a loop that dies inside its floor is still revived by the next
    /// snapshot; every other outcome consumes the delta.
    pub(crate) fn pm_wake_decision_at(
        &mut self,
        project_root: &Path,
        inbox: &[gwt::IssueMonitorInboxItem],
        now: &str,
    ) -> Option<PmWakeDecision> {
        let signals = pm_wake_signals(inbox);
        let Some(seen) = self.pm_wake_seen.get(project_root) else {
            self.pm_wake_seen
                .insert(project_root.to_path_buf(), signals);
            return None;
        };
        let fresh: Vec<String> = signals.difference(seen).cloned().collect();
        if fresh.is_empty() {
            self.pm_wake_seen
                .insert(project_root.to_path_buf(), signals);
            return None;
        }
        // Monitor off = the user parked the project (the Stop-gate driver's
        // rule). Consume the delta: re-enabling is a GUI action with a human
        // present, not a moment to replay stale news.
        let monitor_prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(project_root);
        let monitor_enabled = gwt::load_issue_monitor_prefs(&monitor_prefs_path)
            .map(|prefs| prefs.enabled)
            .unwrap_or(false);
        if !monitor_enabled {
            self.pm_wake_seen
                .insert(project_root.to_path_buf(), signals);
            return None;
        }
        let prefs_path = pm_registry::pm_prefs_path_for_repo_path(project_root);
        let prefs = match pm_registry::load_pm_prefs(&prefs_path) {
            Ok(prefs) => prefs,
            Err(error) => {
                tracing::warn!(%error, "PM wake skipped: pm prefs unreadable");
                return None;
            }
        };
        let Some(registration) = prefs.registration else {
            // No PM to wake; a later PM start reads status in its bootstrap.
            self.pm_wake_seen
                .insert(project_root.to_path_buf(), signals);
            return None;
        };
        let Some(window_id) = self.live_pm_window_id(&registration.session_id) else {
            // A dead PM is the crash-resume path's job, never the wake's.
            self.pm_wake_seen
                .insert(project_root.to_path_buf(), signals);
            return None;
        };
        let interval_secs = prefs.settings.loop_interval_secs_clamped();
        let loop_path = pm_registry::pm_loop_state_path_for_repo_path(project_root);
        let loop_state = pm_registry::load_pm_loop_state(&loop_path).unwrap_or_default();
        if !pm_wake_loop_is_quiet(&loop_state, interval_secs, now) {
            // Actively looping: its own next cycle reconciles this. Keep the
            // delta so a floor-stopped loop is revived by the next snapshot.
            return None;
        }
        self.pm_wake_seen
            .insert(project_root.to_path_buf(), signals);
        // Re-arm the budget: new actionable work is exactly what the park was
        // waiting for (the injected prompt's UserPromptSubmit re-arms too;
        // doing it here keeps the state right even if hook wiring drifts).
        if let Err(error) =
            pm_registry::save_pm_loop_state(&loop_path, &pm_registry::PmLoopState::default())
        {
            tracing::warn!(%error, "PM wake could not re-arm the loop budget");
        }
        let mut reasons = fresh;
        reasons.truncate(5);
        Some(PmWakeDecision {
            window_id,
            prompt: format!(
                "[gwt] Issue Monitor activity while the resident PM loop was idle ({}). \
                 Run one reconcile cycle now: read a fresh `issue.monitor.status` snapshot, \
                 triage the new items, and report the milestone digest.\r",
                reasons.join(", ")
            ),
        })
    }

    /// Execute the wake: inject the prompt into the registered PM's pane. A
    /// write failure is logged and dropped — the retained fingerprint was
    /// already consumed, but the next genuinely new event will retry, and the
    /// crash/resume paths own a dead pane.
    pub(crate) fn pm_wake_events(
        &mut self,
        project_root: &Path,
        inbox: &[gwt::IssueMonitorInboxItem],
    ) -> Vec<OutboundEvent> {
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let Some(decision) = self.pm_wake_decision_at(project_root, inbox, &now) else {
            return Vec::new();
        };
        match self.write_pm_wake_prompt(&decision) {
            Ok(()) => {
                tracing::info!(
                    window_id = %decision.window_id,
                    "woke the resident PM for new Issue Monitor activity"
                );
            }
            Err(error) => {
                tracing::warn!(
                    %error,
                    window_id = %decision.window_id,
                    "PM wake prompt injection failed"
                );
            }
        }
        Vec::new()
    }

    /// The one PTY write the wake path performs, against the window id the
    /// decision resolved from the PM registration — mirrors
    /// `pane_send_input_to_window_events` without a client reply.
    fn write_pm_wake_prompt(&mut self, decision: &PmWakeDecision) -> Result<(), String> {
        match self.runtimes.get(&decision.window_id) {
            None => Err(format!("no live runtime for pane {}", decision.window_id)),
            Some(runtime) => runtime
                .pane
                .lock()
                .map_err(|error| error.to_string())
                .and_then(|pane| {
                    pane.write_input(decision.prompt.as_bytes())
                        .map_err(|error| error.to_string())
                }),
        }
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
            Ok((prefs, _)) => {
                self.sync_pm_session_cache(project_root, prefs.registration.as_ref());
            }
            Err(error) => {
                tracing::warn!(%error, "failed to write PM registration after launch");
            }
        }
    }

    /// Keep the per-broadcast PM marker in step with the durable record.
    fn sync_pm_session_cache(
        &mut self,
        project_root: &Path,
        registration: Option<&PmRegistration>,
    ) {
        match registration {
            Some(registration) => {
                self.pm_sessions
                    .insert(project_root.to_path_buf(), registration.session_id.clone());
            }
            None => {
                self.pm_sessions.remove(project_root);
            }
        }
    }

    /// FR-013: an explicit close of the PM pane clears the registration so
    /// nothing auto-restarts it. Settings (auto_start) survive. A clean PM
    /// worktree is reaped with the registration (T-016); local work keeps it
    /// for reuse by the next PM.
    ///
    /// Returns whether this close actually deregistered a PM, so the caller can
    /// refresh the settings panel only for the close that changed PM state.
    pub(super) fn deregister_pm_for_closed_window(
        &mut self,
        project_root: &Path,
        session_id: &str,
    ) -> bool {
        let prefs_path = pm_registry::pm_prefs_path_for_repo_path(project_root);
        match pm_registry::deregister_pm(&prefs_path, session_id) {
            Ok((_, true)) => {
                tracing::info!(%session_id, "PM pane closed; registration cleared");
                self.sync_pm_session_cache(project_root, None);
                Self::cleanup_pm_worktree(project_root);
                true
            }
            Ok((_, false)) => false,
            Err(error) => {
                tracing::warn!(%error, "failed to deregister PM on window close");
                false
            }
        }
    }

    /// T-016: remove the canonical PM worktree when it holds no local work.
    /// Only the canonical `<gwt_project_dir>/pm/worktree` location is ever
    /// touched — a corrupted registration cannot direct the reaper at an
    /// arbitrary path. Fail-closed: any uncertainty keeps the worktree.
    fn cleanup_pm_worktree(project_root: &Path) {
        let worktree = pm_registry::pm_worktree_path_for_repo_path(project_root);
        if !worktree.exists() {
            return;
        }
        let manager = gwt_git::WorktreeManager::new(project_root);
        match manager.ephemeral_worktree_has_local_work(&worktree) {
            Ok(false) => {
                if let Err(error) = manager.remove_force_twice(&worktree) {
                    tracing::warn!(%error, "failed to remove the clean PM worktree");
                }
            }
            Ok(true) => {
                tracing::info!(
                    worktree = %worktree.display(),
                    "PM worktree has local work; keeping it for reuse"
                );
            }
            Err(error) => {
                tracing::warn!(%error, "PM worktree local-work check failed; keeping it");
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
        self.ensure_pm_agent_for_tab(tab_id, PmEnsureTrigger::Explicit)
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
        let profile = pm_registry::pm_prefs_path_for_repo_path(project_root);
        let profile = pm_registry::load_pm_prefs(&profile)
            .map(|prefs| prefs.settings.launch_profile_or_default())
            .unwrap_or_else(|_| pm_registry::PmLaunchProfile::default_profile());
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
        // skill that managed-asset materialization writes into this worktree.
        // Writing it here instead would be futile — the launch's own asset
        // refresh prunes unbundled `gwt-*` skills right after.
        let config = Self::pm_launch_config(&worktree, &profile);
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

    /// SPEC-3431 FR-026: the launch config for a fresh PM spawn.
    ///
    /// Pure so the resolved agent/model can be asserted without spawning.
    /// `suppress_execution_control` is set because the PM is a conversational
    /// role, not an execution-controlled implementation session.
    ///
    /// Permissions are always skipped (FR-012, user ruling 2026-08-06). The PM
    /// subscribes, reconciles, registers Issues, and instructs launches with no
    /// user present, so a permission prompt in that loop is a deadlock nobody
    /// is watching. Same reasoning as the Issue Monitor's
    /// `force_skip_permissions_for_autonomous`, and likewise not a per-project
    /// choice — a PM that can be configured into a hang will eventually hang.
    pub(crate) fn pm_launch_config(
        worktree: &Path,
        profile: &pm_registry::PmLaunchProfile,
    ) -> gwt_agent::LaunchConfig {
        let agent_id = gwt_agent::resolve_agent_id(&profile.agent_id)
            .unwrap_or(gwt_agent::AgentId::ClaudeCode);
        let mut builder = gwt_agent::AgentLaunchBuilder::new(agent_id)
            .working_dir(worktree.to_path_buf())
            .skip_permissions(true)
            .extra_arg(PM_BOOTSTRAP_PROMPT);
        if let Some(model) = profile.model.as_deref().filter(|value| !value.is_empty()) {
            builder = builder.model(model);
        }
        if let Some(reasoning) = profile
            .reasoning
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            builder = builder.reasoning_level(reasoning);
        }
        if let Some(version) = profile.version.as_deref().filter(|value| !value.is_empty()) {
            builder = builder.version(version);
        }
        let mut config = builder.build();
        config.suppress_execution_control = true;
        config
    }

    /// Dedicated detached worktree for the PM session (research R-10). Its
    /// lifecycle is bound to the PM registration; T-016 adds GC.
    fn ensure_pm_worktree(project_root: &Path) -> Result<PathBuf, String> {
        let path = pm_registry::pm_worktree_path_for_repo_path(project_root);
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
