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
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use gwt::persistence::{WindowGeometry, WindowProcessStatus};
use gwt::pm_registry::{self, PmLaunchProfile, PmRegistration};
use gwt::PmAgentOption;

use crate::embedded_server::AgentPmSendResponder;

use super::{
    AgentCapabilityGrant, AgentCapabilityIssuer, AppRuntime, BackendEvent, ClientId, OutboundEvent,
    WindowPreset,
};

const PM_DELIVERY_MAX_BODY_BYTES: usize = 16 * 1024;

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
        && instant_is_quiet(state.last_wake_at.as_deref())
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
        if let Some(window_id) = prefs
            .registration
            .as_ref()
            .and_then(|registration| self.live_pm_window_id(&registration.session_id))
        {
            // FR-019: the PM launcher must always land the user on the PM, so
            // focusing frames it in the viewport rather than only raising it.
            return self.focus_existing_live_work_agent_events(&window_id, canvas_bounds);
        }
        // Issue #3607 AC-1/AC-2: the singleton is per *repository*, not per
        // project store. One repository can own two stores after a scope split
        // (#3466), and each store's `pm.json` — `auto_start` included — is
        // invisible to the other, so both auto-started and two PMs ended up
        // rewriting one repository's Issue Monitor order and launch orders.
        // Refusing here rather than at registration keeps the second store from
        // ever spawning the pane.
        if let Some(window_id) = self.live_pm_window_id_in_another_store(&project_root) {
            return self.focus_existing_live_work_agent_events(&window_id, canvas_bounds);
        }
        let Some(registration) = prefs.registration else {
            tracing::info!(
                project_root = %project_root.display(),
                "PM ensure: no registration yet, spawning the resident PM"
            );
            return self.spawn_pm_agent(tab_id, &project_root);
        };
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
        // Re-arm the budget and stamp the wake clock: new actionable work is
        // exactly what the park was waiting for, and the stamp keeps the
        // periodic wake from stacking a second prompt in the same window.
        if let Err(error) = pm_registry::save_pm_loop_state(
            &loop_path,
            &pm_registry::PmLoopState {
                last_wake_at: Some(now.to_string()),
                ..pm_registry::PmLoopState::default()
            },
        ) {
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

    /// FR-108(b) (T-201, Issue #3505): the periodic wake — re-arm a quiet
    /// resident PM on the scheduled tick even when no new monitor signal
    /// arrived, as long as there is standing supervision work (running
    /// launches or a non-empty queue). The delta wake (T-093) covers "new
    /// things happened"; this covers "old things still need watching".
    ///
    /// The same quiet gate as the delta wake keeps the two from double-firing:
    /// an actively-looping or freshly-prompted PM is never interrupted, and a
    /// wake re-arms the loop so the next tick inside the interval is quiet-
    /// gated out.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn pm_periodic_wake_decision_at(
        &mut self,
        project_root: &Path,
        now: &str,
    ) -> Option<PmWakeDecision> {
        let monitor_prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(project_root);
        let monitor_prefs = gwt::load_issue_monitor_prefs(&monitor_prefs_path).ok()?;
        let monitor =
            gwt::IssueMonitorState::with_prefs(gwt::IssueMonitorConfig::default(), monitor_prefs);
        self.pm_periodic_wake_decision_for_monitor_at(project_root, &monitor, now)
    }

    pub(crate) fn pm_periodic_wake_decision_for_monitor_at(
        &mut self,
        project_root: &Path,
        monitor: &gwt::IssueMonitorState,
        now: &str,
    ) -> Option<PmWakeDecision> {
        if !monitor.config.enabled {
            return None;
        }
        let status = monitor.agent_status();
        if status.active_launches.is_empty()
            && status.queue.is_empty()
            && status.needs_human.is_empty()
        {
            return None;
        }
        let prefs_path = pm_registry::pm_prefs_path_for_repo_path(project_root);
        let prefs = pm_registry::load_pm_prefs(&prefs_path).ok()?;
        let registration = prefs.registration?;
        let window_id = self.live_pm_window_id(&registration.session_id)?;
        let interval_secs = prefs.settings.loop_interval_secs_clamped();
        let loop_path = pm_registry::pm_loop_state_path_for_repo_path(project_root);
        let loop_state = pm_registry::load_pm_loop_state(&loop_path).unwrap_or_default();
        if !pm_wake_loop_is_quiet(&loop_state, interval_secs, now) {
            return None;
        }
        if let Err(error) = pm_registry::save_pm_loop_state(
            &loop_path,
            &pm_registry::PmLoopState {
                last_wake_at: Some(now.to_string()),
                ..pm_registry::PmLoopState::default()
            },
        ) {
            tracing::warn!(%error, "PM periodic wake could not re-arm the loop budget");
        }
        Some(PmWakeDecision {
            window_id,
            prompt: "[gwt] Scheduled supervision tick: run one PM reconcile cycle now — read a \
                     fresh `issue.monitor.status` snapshot, check the running agents' \
                     `last_activity_at` and any NeedsHuman rows, and report the milestone \
                     digest.\r"
                .to_string(),
        })
    }

    pub(crate) fn pm_periodic_wake_events_for_monitor_at(
        &mut self,
        project_root: &Path,
        monitor: &gwt::IssueMonitorState,
        now: &str,
    ) -> Vec<OutboundEvent> {
        let Some(decision) =
            self.pm_periodic_wake_decision_for_monitor_at(project_root, monitor, now)
        else {
            return Vec::new();
        };
        match self.write_pm_wake_prompt(&decision) {
            Ok(()) => {
                tracing::info!(
                    window_id = %decision.window_id,
                    "periodic wake re-armed the resident PM from the scheduled snapshot"
                );
            }
            Err(error) => {
                tracing::warn!(
                    %error,
                    window_id = %decision.window_id,
                    "PM periodic wake prompt injection failed"
                );
            }
        }
        Vec::new()
    }

    pub(crate) fn pm_periodic_wake_events_at(
        &mut self,
        project_root: &Path,
        now: &str,
    ) -> Vec<OutboundEvent> {
        let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(project_root);
        let Ok(prefs) = gwt::load_issue_monitor_prefs(&prefs_path) else {
            return Vec::new();
        };
        let monitor = gwt::IssueMonitorState::with_prefs(gwt::IssueMonitorConfig::default(), prefs);
        self.pm_periodic_wake_events_for_monitor_at(project_root, &monitor, now)
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

    /// SPEC-3431 FR-111 (T-206): PM-privileged message delivery into an
    /// agent pane. The general SPEC-3050 self-only contract is untouched —
    /// this separate path exists only for the live registered PM, and the
    /// principal is re-verified here, immediately before the injection:
    /// the presented session must be the durable `pm.json` registration of
    /// the project that owns the target window, and that registration's own
    /// pane must be live right now. Everything else is refused with a typed
    /// reply and no write.
    #[cfg(test)]
    pub(crate) fn pm_pane_send_input_events(
        &mut self,
        client_id: ClientId,
        pm_session_id: &str,
        window_id: &str,
        text: &str,
    ) -> Vec<OutboundEvent> {
        let refuse = |error: String| {
            vec![OutboundEvent::reply(
                client_id.clone(),
                BackendEvent::PaneSendResult {
                    ok: false,
                    window_id: Some(window_id.to_string()),
                    error: Some(error),
                },
            )]
        };
        // The window's owning tab decides which project's PM registration is
        // the authority — a PM can never reach a pane of another project.
        let Some(project_root) = self
            .tabs
            .iter()
            .find(|tab| {
                tab.workspace.persisted().windows.iter().any(|window| {
                    crate::runtime_support::combined_window_id(&tab.id, &window.id) == window_id
                })
            })
            .map(|tab| tab.project_root.clone())
        else {
            return refuse(format!(
                "pm pane send: unknown pane {window_id} (FR-111 refuses cross-project delivery)"
            ));
        };
        let prefs_path = pm_registry::pm_prefs_path_for_repo_path(&project_root);
        let registration = match pm_registry::load_pm_prefs(&prefs_path) {
            Ok(prefs) => prefs.registration,
            Err(error) => {
                return refuse(format!("pm pane send: pm registration unreadable: {error}"))
            }
        };
        let Some(registration) = registration else {
            return refuse(
                "pm pane send: no registered PM for this project (FR-111 requires the live PM principal)"
                    .to_string(),
            );
        };
        if registration.session_id != pm_session_id {
            return refuse(
                "pm pane send: presented session is not the registered PM (FR-111 refuses foreign principals)"
                    .to_string(),
            );
        }
        if self.live_pm_window_id(&registration.session_id).is_none() {
            return refuse(
                "pm pane send: the registered PM has no live pane (stale registration; FR-111 refuses)"
                    .to_string(),
            );
        }
        self.pane_send_input_to_window_events(client_id, window_id, text)
    }

    /// Authenticated FR-111 entrypoint. The capability principal supplies the
    /// caller project and Session; no client payload identity participates in
    /// authorization. The direct responder binds the terminal result to the
    /// origin WebSocket.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn authenticated_pm_pane_send_input_events(
        &mut self,
        issuer: &AgentCapabilityIssuer,
        client_id: ClientId,
        grant: AgentCapabilityGrant,
        operation_id: &str,
        window_id: &str,
        text: &str,
        responder: Option<AgentPmSendResponder>,
    ) -> Vec<OutboundEvent> {
        let finish = |status: &str, reason: Option<String>| {
            let event = BackendEvent::PmMessageSendResult {
                operation_id: operation_id.to_string(),
                status: status.to_string(),
                window_id: Some(window_id.to_string()),
                reason,
            };
            if let Some(responder) = responder.as_ref() {
                let _ = responder.send(event);
                Vec::new()
            } else {
                vec![OutboundEvent::reply(client_id.clone(), event)]
            }
        };
        if uuid::Uuid::parse_str(operation_id)
            .ok()
            .is_none_or(|parsed| parsed.hyphenated().to_string() != operation_id)
        {
            return finish(
                "failed",
                Some("pm.message.send requires a canonical operation UUID".to_string()),
            );
        }
        if !issuer.grant_is_current(&grant)
            || responder
                .as_ref()
                .is_some_and(|responder| !responder.mutation_is_current())
        {
            return finish(
                "failed",
                Some("pm.message.send capability is stale".to_string()),
            );
        }

        let project_root = grant.principal().canonical_project_root().to_path_buf();
        let principal_session_id = grant.principal().session_id().to_string();
        let prefs_path = pm_registry::pm_prefs_path_for_repo_path(&project_root);
        let registration = match pm_registry::load_pm_prefs(&prefs_path) {
            Ok(prefs) => prefs.registration,
            Err(error) => {
                return finish(
                    "failed",
                    Some(format!(
                        "pm.message.send registration is unreadable: {error}"
                    )),
                )
            }
        };
        let Some(registration) =
            registration.filter(|registration| registration.session_id == principal_session_id)
        else {
            return finish(
                "failed",
                Some("pm.message.send caller is not the live registered PM".to_string()),
            );
        };
        let Some(principal_window_id) = self.live_pm_window_id(&principal_session_id) else {
            return finish(
                "failed",
                Some("pm.message.send caller is not the live registered PM".to_string()),
            );
        };
        let principal_pty = match self
            .pty_writers
            .read()
            .map_err(|_| "pm.message.send PTY registry is unavailable".to_string())
            .and_then(|writers| {
                writers
                    .get(&principal_window_id)
                    .cloned()
                    .ok_or_else(|| "pm.message.send caller has no live PTY".to_string())
            }) {
            Ok(pty) => pty,
            Err(error) => return finish("failed", Some(error)),
        };

        let Some(body) = text.strip_suffix('\r') else {
            return finish(
                "failed",
                Some(
                    "pm.message.send requires one line terminated by a carriage return".to_string(),
                ),
            );
        };
        if body.len() > PM_DELIVERY_MAX_BODY_BYTES {
            return finish(
                "failed",
                Some(format!(
                    "pm.message.send body exceeds the {PM_DELIVERY_MAX_BODY_BYTES}-byte limit"
                )),
            );
        }
        let protected_prompt = match pm_registry::protected_pm_delivery_prompt(operation_id, body) {
            Ok(prompt) => prompt,
            Err(error) => return finish("failed", Some(error.to_string())),
        };
        let target = self.tabs.iter().find_map(|tab| {
            let canonical_root = dunce::canonicalize(&tab.project_root)
                .map(|path| gwt_core::paths::normalize_windows_child_process_path(&path))
                .ok()?;
            if canonical_root != project_root {
                return None;
            }
            tab.workspace.persisted().windows.iter().find_map(|window| {
                (crate::runtime_support::combined_window_id(&tab.id, &window.id) == window_id).then(
                    || {
                        (
                            tab.id.clone(),
                            window.preset,
                            window.status,
                            window.session_id.clone(),
                        )
                    },
                )
            })
        });
        let target_session_id = target
            .as_ref()
            .and_then(|(_, _, _, session_id)| session_id.clone());
        let target_is_live_agent = target.as_ref().is_some_and(
            |(target_tab_id, target_preset, target_status, target_session_id)| {
                matches!(
                    target_status,
                    WindowProcessStatus::Running
                        | WindowProcessStatus::Idle
                        | WindowProcessStatus::Waiting
                ) && target_session_id
                    .as_deref()
                    .is_some_and(|target_session_id| {
                        self.active_agent_sessions
                            .get(window_id)
                            .is_some_and(|session| {
                                let exact_prompt_ack = matches!(
                                    target_preset,
                                    WindowPreset::Claude | WindowPreset::Codex
                                ) || (*target_preset == WindowPreset::Agent
                                    && matches!(session.agent_id.as_str(), "claude" | "codex"));
                                exact_prompt_ack
                                    && session.window_id == window_id
                                    && session.tab_id == *target_tab_id
                                    && session.session_id == target_session_id
                            })
                    })
            },
        );
        let expected_pty = if target_is_live_agent {
            self.pty_writers
                .read()
                .ok()
                .and_then(|writers| writers.get(window_id).cloned())
        } else {
            None
        };

        let operation_id = operation_id.to_string();
        let window_id = window_id.to_string();
        let body_sha256 = pm_registry::pm_delivery_prompt_sha256(body);
        let receipt_path = pm_registry::pm_delivery_receipts_path_for_repo_path(&project_root);
        let writers = Arc::clone(&self.pty_writers);
        let issuer = issuer.clone();
        let worker_responder = responder.clone();
        let worker_client_id = client_id.clone();
        let proxy = self.proxy.clone();
        let worker_operation_id = operation_id.clone();
        let worker_window_id = window_id.clone();
        let spawn = self.blocking_tasks.try_spawn(move || {
            let send_terminal = |status: &str, reason: Option<String>| {
                let event = BackendEvent::PmMessageSendResult {
                    operation_id: worker_operation_id.clone(),
                    status: status.to_string(),
                    window_id: Some(worker_window_id.clone()),
                    reason,
                };
                if let Some(responder) = worker_responder.as_ref() {
                    let _ = responder.send(event);
                } else {
                    proxy.send(super::UserEvent::Dispatch(vec![OutboundEvent::reply(
                        worker_client_id.clone(),
                        event,
                    )]));
                }
            };
            let deadline = worker_responder
                .as_ref()
                .map(AgentPmSendResponder::deadline)
                .unwrap_or_else(|| Instant::now() + Duration::from_secs(2));
            let _operation_deadline =
                gwt_core::operation_deadline::ScopedOperationDeadline::enter(deadline);
            let mut body_attempted = false;
            let mut receipt_prepared = false;
            let mut receipt_terminalized = false;
            let mut durable_target_session_id = target_session_id.clone();
            let prepare = pm_registry::pm_delivery_receipt_for_operation(
                &receipt_path,
                &worker_operation_id,
            )
            .map_err(|error| format!("PM delivery receipt replay failed: {error}"))
            .and_then(|existing| {
                if let Some(existing) = existing {
                    if existing.principal_session_id != principal_session_id
                        || existing.target_window_id != worker_window_id
                        || existing.body_sha256 != body_sha256
                    {
                        return Err(
                            "PM delivery operation identity conflicts with its durable receipt"
                                .to_string(),
                        );
                    }
                    durable_target_session_id = Some(existing.target_session_id);
                    return Ok(pm_registry::PmDeliveryPrepareOutcome::Existing(
                        existing.status,
                    ));
                }
                let target_session_id = durable_target_session_id.as_ref().ok_or_else(|| {
                    "pm.message.send refused: target is not an authorized live agent pane"
                        .to_string()
                })?;
                if !target_is_live_agent || expected_pty.is_none() {
                    return Err(
                        "pm.message.send refused: target is not an authorized live agent pane"
                            .to_string(),
                    );
                }
                let prepared = pm_registry::PmDeliveryReceipt {
                    operation_id: worker_operation_id.clone(),
                    recorded_at: chrono::Utc::now()
                        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                    status: pm_registry::PmDeliveryReceiptStatus::Prepared,
                    principal_session_id: principal_session_id.clone(),
                    target_window_id: worker_window_id.clone(),
                    target_session_id: target_session_id.clone(),
                    body_sha256: body_sha256.clone(),
                    reason: None,
                };
                pm_registry::prepare_pm_delivery_receipt(&receipt_path, &prepared)
                    .map_err(|error| format!("PM delivery receipt Prepared commit failed: {error}"))
            });
            let result = match prepare {
                Ok(pm_registry::PmDeliveryPrepareOutcome::Prepared) => {
                    receipt_prepared = true;
                    let delivery_target_session_id = durable_target_session_id
                        .clone()
                        .expect("Prepared PM delivery has a target Session");
                    expected_pty
                        .as_ref()
                        .cloned()
                        .ok_or_else(|| {
                            "pm.message.send refused: target is not an authorized live agent pane"
                                .to_string()
                        })
                        .and_then(|expected_pty| {
                        let reservation = Arc::clone(&expected_pty)
                            .reserve_input_transaction()
                            .map_err(|error| {
                                format!("PM pane input transaction unavailable: {error}")
                            })?;
                        super::pty_io::drive_verified_pane_submit(
                            &protected_prompt,
                            2,
                            |bytes| {
                                reservation
                                    .write_input_authorized(bytes, |commit| {
                                        if Instant::now() >= deadline
                                            || worker_responder.as_ref().is_some_and(|responder| {
                                                !responder.mutation_is_current()
                                            })
                                        {
                                            return Err(gwt_terminal::TerminalError::PtyIoError {
                                                details: "PM delivery deadline expired before input mutation"
                                                    .to_string(),
                                            });
                                        }
                                        let current_registration =
                                            pm_registry::load_pm_prefs(&prefs_path)
                                                .map_err(|error| {
                                                    gwt_terminal::TerminalError::PtyIoError {
                                                        details: format!(
                                                            "PM registration revalidation failed: {error}"
                                                        ),
                                                    }
                                                })?
                                                .registration;
                                        if current_registration.as_ref() != Some(&registration) {
                                            return Err(gwt_terminal::TerminalError::PtyIoError {
                                                details: "registered PM authority changed before input mutation"
                                                    .to_string(),
                                            });
                                        }
                                        {
                                            let current = writers.read().map_err(|_| {
                                                gwt_terminal::TerminalError::PtyIoError {
                                                    details: "PM delivery PTY registry is unavailable"
                                                        .to_string(),
                                                }
                                            })?;
                                            if !current
                                                .get(&worker_window_id)
                                                .is_some_and(|pty| Arc::ptr_eq(pty, &expected_pty))
                                                || !current
                                                    .get(&principal_window_id)
                                                    .is_some_and(|pty| {
                                                        Arc::ptr_eq(pty, &principal_pty)
                                                    })
                                            {
                                                return Err(
                                                    gwt_terminal::TerminalError::PtyIoError {
                                                        details: "PM caller or target runtime generation changed"
                                                            .to_string(),
                                                    },
                                                );
                                            }
                                        }
                                        if Instant::now() >= deadline
                                            || worker_responder.as_ref().is_some_and(|responder| {
                                                !responder.mutation_is_current()
                                            })
                                        {
                                            return Err(gwt_terminal::TerminalError::PtyIoError {
                                                details: "PM delivery deadline expired before input commit"
                                                    .to_string(),
                                            });
                                        }
                                        if !issuer.commit_mutation_if_current(&grant, || {
                                            worker_responder.as_ref().is_none_or(
                                                AgentPmSendResponder::try_commit_mutation,
                                            )
                                        }) {
                                            return Err(
                                                gwt_terminal::TerminalError::PtyIoError {
                                                    details: "PM capability or origin changed before input mutation"
                                                        .to_string(),
                                                },
                                            );
                                        }
                                        let mut mark_attempted = || {
                                            if bytes != b"\r" {
                                                body_attempted = true;
                                            }
                                        };
                                        commit(&mut mark_attempted)
                                    })
                                    .map_err(|error| error.to_string())
                            },
                            |settle| {
                                let remaining = deadline.saturating_duration_since(Instant::now());
                                thread::sleep(settle.min(remaining));
                            },
                            || {
                                pm_registry::pm_delivery_receipt_for_operation(
                                    &receipt_path,
                                    &worker_operation_id,
                                )
                                    .map_err(|error| {
                                        format!(
                                            "PM delivery acknowledgement is unavailable: {error}"
                                        )
                                    })
                                    .map(|receipt| {
                                        receipt.is_some_and(|receipt| {
                                            receipt.target_session_id
                                                == delivery_target_session_id
                                                && receipt.body_sha256 == body_sha256
                                                && receipt.status
                                                    == pm_registry::PmDeliveryReceiptStatus::Verified
                                        })
                                    })
                            },
                        )
                    })
                }
                Ok(pm_registry::PmDeliveryPrepareOutcome::Existing(
                    pm_registry::PmDeliveryReceiptStatus::Verified,
                )) => {
                    receipt_terminalized = true;
                    Ok(super::pty_io::VerifiedPaneSubmitOutcome::Verified {
                        submit_attempts: 0,
                    })
                }
                Ok(pm_registry::PmDeliveryPrepareOutcome::Existing(
                    pm_registry::PmDeliveryReceiptStatus::Prepared,
                )) => {
                    let target_session_id = durable_target_session_id
                        .as_ref()
                        .expect("existing Prepared has a target Session");
                    let replay_poll_deadline = deadline
                        .checked_sub(Duration::from_millis(250))
                        .unwrap_or(deadline);
                    let replay_status = (|| -> Result<_, String> {
                        loop {
                            let current = pm_registry::pm_delivery_receipt_for_operation(
                                &receipt_path,
                                &worker_operation_id,
                            )
                            .map_err(|error| {
                                format!("PM delivery replay receipt is unavailable: {error}")
                            })?;
                            let status = current
                                .map(|receipt| receipt.status)
                                .unwrap_or(pm_registry::PmDeliveryReceiptStatus::Prepared);
                            if status != pm_registry::PmDeliveryReceiptStatus::Prepared
                                || Instant::now() >= replay_poll_deadline
                            {
                                return Ok(status);
                            }
                            thread::sleep(
                                Duration::from_millis(25)
                                    .min(replay_poll_deadline.saturating_duration_since(Instant::now())),
                            );
                        }
                    })();
                    match replay_status {
                        Err(error) => Err(error),
                        Ok(pm_registry::PmDeliveryReceiptStatus::Verified) => {
                            receipt_terminalized = true;
                            Ok(super::pty_io::VerifiedPaneSubmitOutcome::Verified {
                                submit_attempts: 0,
                            })
                        }
                        Ok(pm_registry::PmDeliveryReceiptStatus::Ambiguous) => {
                            receipt_terminalized = true;
                            Err("PM delivery may already have staged its prompt body".to_string())
                        }
                        Ok(pm_registry::PmDeliveryReceiptStatus::Refused) => {
                            receipt_terminalized = true;
                            Err("PM delivery operation was already refused".to_string())
                        }
                        Ok(pm_registry::PmDeliveryReceiptStatus::Prepared) => match pm_registry::finish_pm_delivery_receipt(
                        &receipt_path,
                        &worker_operation_id,
                        target_session_id,
                        &body_sha256,
                        pm_registry::PmDeliveryReceiptStatus::Ambiguous,
                        Some("response_lost_after_prepare"),
                    )
                    .map_err(|error| {
                        format!("PM delivery replay could not terminalize Prepared: {error}")
                    }) {
                        Ok(pm_registry::PmDeliveryReceiptStatus::Verified) => {
                            receipt_terminalized = true;
                            Ok(super::pty_io::VerifiedPaneSubmitOutcome::Verified {
                                submit_attempts: 0,
                            })
                        }
                        Ok(_) => {
                            receipt_terminalized = true;
                            Err("PM delivery may already have staged its prompt body".to_string())
                        }
                        Err(error) => Err(error),
                    },
                    }
                }
                Ok(pm_registry::PmDeliveryPrepareOutcome::Existing(
                    pm_registry::PmDeliveryReceiptStatus::Ambiguous,
                )) => {
                    receipt_terminalized = true;
                    Err("PM delivery may already have staged its prompt body".to_string())
                }
                Ok(pm_registry::PmDeliveryPrepareOutcome::Existing(
                    pm_registry::PmDeliveryReceiptStatus::Refused,
                )) => {
                    receipt_terminalized = true;
                    Err("PM delivery operation was already refused".to_string())
                }
                Err(error) => Err(error),
            };

            match result {
                Ok(super::pty_io::VerifiedPaneSubmitOutcome::Verified { .. }) => {
                    send_terminal("delivered", None)
                }
                Ok(super::pty_io::VerifiedPaneSubmitOutcome::Unverified { .. }) => {
                    let target_session_id = durable_target_session_id
                        .as_deref()
                        .expect("Prepared PM delivery has a target Session");
                    match pm_registry::finish_pm_delivery_receipt(
                        &receipt_path,
                        &worker_operation_id,
                        target_session_id,
                        &body_sha256,
                        pm_registry::PmDeliveryReceiptStatus::Ambiguous,
                        Some("submit_unverified"),
                    ) {
                        Ok(pm_registry::PmDeliveryReceiptStatus::Verified) => {
                            send_terminal("delivered", None)
                        }
                        _ => send_terminal(
                            "failed",
                            Some(
                                "PM pane submit was not verified; prompt body may be staged — do not retry with a new operation"
                                    .to_string(),
                            ),
                        ),
                    }
                }
                Err(mut error) => {
                    let mut verified_during_finalization = false;
                    if receipt_prepared && !receipt_terminalized {
                        let target_session_id = durable_target_session_id
                            .as_deref()
                            .expect("Prepared PM delivery has a target Session");
                        let status = if body_attempted {
                            pm_registry::PmDeliveryReceiptStatus::Ambiguous
                        } else {
                            pm_registry::PmDeliveryReceiptStatus::Refused
                        };
                        match pm_registry::finish_pm_delivery_receipt(
                            &receipt_path,
                            &worker_operation_id,
                            target_session_id,
                            &body_sha256,
                            status,
                            Some(if body_attempted {
                                "body_staged_or_unknown"
                            } else {
                                "mutation_refused"
                            }),
                        ) {
                            Ok(pm_registry::PmDeliveryReceiptStatus::Verified) => {
                                verified_during_finalization = true;
                            }
                            Ok(_) => {}
                            Err(receipt_error) => {
                                error.push_str(&format!(
                                    "; PM delivery receipt terminal commit failed: {receipt_error}"
                                ));
                            }
                        }
                    }
                    if verified_during_finalization {
                        send_terminal("delivered", None);
                        return;
                    }
                    if body_attempted {
                        error.push_str(
                            "; prompt body may be staged — do not retry with a new operation",
                        );
                    }
                    send_terminal("failed", Some(error));
                }
            }
        });
        if let Err(error) = spawn {
            return finish(
                "failed",
                Some(format!("pm.message.send worker unavailable: {error}")),
            );
        }
        Vec::new()
    }

    /// The one PTY write the wake path performs, against the window id the
    /// decision resolved from the PM registration — mirrors
    /// `pane_send_input_to_window_events` without a client reply.
    fn write_pm_wake_prompt(&mut self, decision: &PmWakeDecision) -> Result<(), String> {
        match self.runtimes.get(&decision.window_id) {
            None => Err(format!("no live runtime for pane {}", decision.window_id)),
            Some(runtime) => {
                let pane = Arc::clone(&runtime.pane);
                super::pty_io::write_pane_input_then_submit(&pane, &decision.prompt)
            }
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

    /// Issue #3607 AC-1: window id of a live PM registered by *another* project
    /// store of the same repository.
    ///
    /// Only a live one blocks. A dead registration in a split store must never
    /// leave the repository without a PM — the gate exists to stop duplicates,
    /// not to stop recovery.
    pub(crate) fn live_pm_window_id_in_another_store(&self, project_root: &Path) -> Option<String> {
        let repository_key = pm_registry::pm_repository_key(project_root)?;
        let own_project_dir = gwt_core::paths::gwt_project_dir_for_repo_path(project_root);
        pm_registry::pm_registrations_for_repository(&repository_key)
            .into_iter()
            .filter(|record| record.project_dir != own_project_dir)
            .find_map(|record| {
                let window_id = self.live_pm_window_id(&record.registration.session_id)?;
                tracing::warn!(
                    project_root = %project_root.display(),
                    other_store = %record.project_dir.display(),
                    session_id = %record.registration.session_id,
                    "PM ensure refused: this repository already has a live PM in another project store"
                );
                Some(window_id)
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
        let manager = gwt_git::WorktreeManager::new(Self::pm_git_root(project_root));
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
        let manager = gwt_git::WorktreeManager::new(Self::pm_git_root(project_root));
        manager
            .create_detached("HEAD", &path)
            .map_err(|error| error.to_string())?;
        Ok(path)
    }

    /// Issue #3497: the opened project root is not always a repository — the
    /// bare layout (`parent/` holding `parent/<name>.git` plus worktrees) is
    /// exactly what the launch paths already resolve through
    /// `main_worktree_root`. Every PM `WorktreeManager` goes through the same
    /// resolution, or `git worktree` dies with "not a git repository" and the
    /// PM silently never comes up.
    fn pm_git_root(project_root: &Path) -> PathBuf {
        gwt_git::worktree::main_worktree_root(project_root)
            .unwrap_or_else(|_| project_root.to_path_buf())
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
