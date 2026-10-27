//! Resident PM pane lifecycle (SPEC-3431): the GUI-side singleton gate.
//!
//! The project owner commits physical PM delivery and completed-lifecycle
//! registration changes through `gwt::pm_registry`. This coordinator consumes
//! its fenced pane projection and drives the existing ensure flow:
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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use gwt::persistence::WindowProcessStatus;
// Issue #3632: every prompt this module injects closes with the one canonical
// reporting clause the Stop-gate continuation also uses; see
// `pm_registry::PM_CYCLE_REPORTING_CLAUSE` for why it is shared.
use gwt::pm_registry::{
    self, PmLaunchProfile, PmRegistration, PM_CYCLE_REPORTING_CLAUSE,
    PM_GWTD_EXECUTION_WAKE_CLAUSE, PM_PERIODIC_WAKE_INSTRUCTION, PM_STEERING_WAKE_CLAUSE,
};
use gwt::project_runtime::pm_delivery::{
    pm_status_from_launch_prefs, pm_wake_runtime_is_busy, PmWakeInput, PmWakeWrite,
};

use crate::UserEvent;

use super::startup::RestoreOrigin;
use super::{AppRuntime, BackendEvent, OutboundEvent, WindowPreset, WorkspaceResumeContext};

pub(super) use gwt::pm_registry::PM_BOOTSTRAP_PROMPT;

/// SPEC-3431 T-093 (FR-012): a wake the monitor-event path decided on — which
/// pane receives the prompt and what it says. The window id is only ever the
/// registered PM session's live pane, resolved inside the decision; no caller
/// can point the wake anywhere else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PmWakeDecision {
    project_root: PathBuf,
    wake_id: Option<String>,
    pub(crate) window_id: String,
    pub(crate) prompt: String,
    idle_notifications: std::collections::BTreeMap<String, String>,
    // Display-only owner outcome. The owner rechecks its exact retained writer.
    unconfirmed: bool,
}

/// Issue #3932: retain pane-scoped edges independently of the single composer
/// slot used by other wakes. At most one retry and delivery are outstanding.
#[derive(Default)]
pub(crate) struct AgentIdleWakes {
    running: HashSet<String>,
    pending: std::collections::BTreeMap<String, String>,
    retry_scheduled: bool,
    delivery_in_flight: Option<PmIdleWakeInFlight>,
}

impl PmWakeDecision {
    #[cfg(test)]
    pub(crate) fn delivery_prompt(&self) -> String {
        gwt::project_runtime::pm_delivery::pm_wake_delivery_prompt(
            &self.project_root,
            &self.prompt,
            !self.idle_notifications.is_empty(),
        )
    }
}

#[derive(Clone)]
struct PmWakeCancellation {
    client: gwt::project_runtime::client::ProjectRuntimeClient,
    owner_pane_id: String,
    incarnation: u64,
    host_instance_id: String,
    operation_id: String,
}

struct PmIdleWakeInFlight {
    window_id: String,
    permit: Arc<AtomicBool>,
    dispatched: Arc<AtomicBool>,
    cancellation: Option<PmWakeCancellation>,
}

impl PmIdleWakeInFlight {
    fn cancel(&self, tasks: &super::BlockingTaskSpawner) {
        self.permit.store(false, Ordering::Release);
        if !self.dispatched.load(Ordering::Acquire) {
            return;
        }
        let Some(captured) = self.cancellation.clone() else {
            return;
        };
        if let Err(error) = tasks.try_spawn(move || {
            let result = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| error.to_string())
                .and_then(|runtime| {
                    runtime.block_on(captured.client.command(
                        gwt::project_runtime::events::OwnerCommand::CancelPmWake {
                            id: captured.owner_pane_id,
                            incarnation: captured.incarnation,
                            host_instance_id: captured.host_instance_id,
                            operation_id: captured.operation_id,
                        },
                    ))
                });
            if let Err(error) = result {
                tracing::warn!(%error, "PM idle wake cancellation could not reach its exact owner");
            }
        }) {
            tracing::warn!(%error, "PM idle wake cancellation could not be queued");
        }
    }
}

#[derive(Clone)]
pub(crate) struct PmWakeDelivery {
    pub(super) context: super::ProjectContext,
    pub(super) decision: PmWakeDecision,
    owner_pane_id: String,
    incarnation: u64,
    owner_identity: gwt::daemon_subscriber::ProjectOwnerIdentity,
    pub(super) result: Result<PmWakeWrite, String>,
}

impl std::fmt::Debug for PmWakeDelivery {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PmWakeDelivery")
            .field("window_id", &self.decision.window_id)
            .field("result", &self.result)
            .finish_non_exhaustive()
    }
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
        if matches!(
            item.state,
            gwt::MonitorInboxState::LaunchFailed | gwt::MonitorInboxState::AgentFailed
        ) {
            signals.insert(format!("failed:{}", item.issue.number));
        }
    }
    signals
}

pub(crate) use gwt::project_runtime::pm_delivery::has_open_board_escalations;

pub(super) fn pm_has_standing_durable_work(project_root: &Path) -> bool {
    if has_open_board_escalations(project_root) {
        return true;
    }
    match gwt_core::concern::has_unresolved_concerns(project_root) {
        Ok(has_concerns) => has_concerns,
        Err(error) => {
            tracing::warn!(
                %error,
                project_root = %project_root.display(),
                "PM periodic wake retained after Concern store read failure"
            );
            true
        }
    }
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

/// Issue #4258: how many loop intervals a Running PM pane may hold a wake.
/// The newest loop clock marks when the current turn began, so a pane still
/// Running past this bound has most likely missed its Stop hook (#3809) and
/// would otherwise never be woken again.
const PM_WAKE_BUSY_DEFER_MAX_INTERVALS: u64 = 3;

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
    /// Launcher click or crash recovery. Ignores the opt-out.
    Explicit,
    /// Profile restart after the predecessor registration was deliberately
    /// cleared. The accepted close still finalizes in the background, but its
    /// generic anti-respawn fence must not suppress this requested successor.
    Restart,
}

/// Issue #4375: the spawn a PM worktree preparation is holding up.
///
/// Preparing the worktree is Git work — `git worktree add` for a first spawn,
/// `git fetch` plus a detached checkout for every refresh — and its cost scales
/// with the repository's worktree count. Running it inline held the tao event
/// loop for seconds during the surface-ready restore drain, so the preparation
/// moves to a blocking worker and the spawn it gates is replayed from this
/// payload when the worker reports back.
#[derive(Debug, Clone)]
pub(crate) enum PmWorktreeContinuation {
    /// A fresh silent spawn into the canonical PM worktree. The dedicated
    /// detached worktree (research R-10) is created here; its lifecycle is
    /// bound to the PM registration.
    FreshSpawn {
        tab_id: String,
        project_root: PathBuf,
    },
    /// A persisted PM session resuming in the worktree it already owns.
    ResumeSession {
        tab_id: String,
        /// The tab's repository, which keys the in-flight gate. The Git work
        /// itself targets the session's own worktree.
        project_root: PathBuf,
        session: Box<gwt_agent::Session>,
        workspace_resume_context: Option<WorkspaceResumeContext>,
        origin: RestoreOrigin,
        /// SPEC-3431 FR-001: whether the resumed pane must be recorded in
        /// `pending_pm_launches`, so launch completion rewrites `pm.json` to
        /// name the successor session. Only the ensure path asks for this; the
        /// startup restore drain deliberately leaves the registration alone.
        register_pm_launch: bool,
    },
}

impl PmWorktreeContinuation {
    pub(crate) fn tab_id(&self) -> &str {
        match self {
            Self::FreshSpawn { tab_id, .. } | Self::ResumeSession { tab_id, .. } => tab_id,
        }
    }

    /// The repository this preparation holds, i.e. the in-flight gate's key.
    pub(crate) fn project_root(&self) -> &Path {
        match self {
            Self::FreshSpawn { project_root, .. } | Self::ResumeSession { project_root, .. } => {
                project_root
            }
        }
    }

    /// Run the Git-backed preparation and report the prepared PM worktree.
    ///
    /// Called on a blocking worker, never on the GUI event loop.
    pub(crate) fn prepare(&self) -> Result<PathBuf, String> {
        match self {
            Self::FreshSpawn { project_root, .. } => {
                pm_registry::refresh_pm_worktree_for_repo_path(project_root)
                    .map(|outcome| outcome.worktree)
                    .map_err(|error| error.to_string())
            }
            Self::ResumeSession { session, .. } => {
                pm_registry::refresh_pm_worktree_at_safe_boundary(&session.worktree_path)
                    .map(|_| session.worktree_path.clone())
                    .map_err(|error| error.to_string())
            }
        }
    }
}

impl AppRuntime {
    /// Ensure the resident PM pane for this project. Focus the existing pane,
    /// resume a stale registration when materializable, or start one PM.
    pub(crate) fn ensure_pm_agent_for_tab(
        &mut self,
        tab_id: &str,
        trigger: PmEnsureTrigger,
    ) -> Vec<OutboundEvent> {
        #[cfg(test)]
        if !test_gate::PM_ENSURE_ENABLED.with(|cell| cell.get()) {
            return Vec::new();
        }
        let mut events = self.ensure_pm_agent_events(tab_id, trigger);
        // FR-026: the ensure gate is where the PM's live state changes, so it
        // is also where the settings panel learns about it. Skipped ensures
        // (opt-out, non-Git tab, backoff floor) still report — "not running"
        // is exactly the state the panel has to show.
        if let Some(context) = self.project_context(tab_id) {
            events.extend(self.pm_status_broadcast_events(&context));
        }
        events
    }

    fn ensure_pm_agent_events(
        &mut self,
        tab_id: &str,
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
            // #4486 AC-7: a Restart re-runs this exact refusal, so the panel
            // has to say so instead of offering a button that does nothing.
            Self::record_pm_start_block(
                &tab.project_root,
                pm_registry::PmStartBlockKind::MigrationPending,
                format!(
                    "the tab is not a migration-clear Git project (kind={:?}, migration_pending={})",
                    tab.kind, tab.migration_pending
                ),
            );
            return Vec::new();
        }
        let project_root = tab.project_root.clone();
        if trigger != PmEnsureTrigger::Restart
            && self
                .project_state_for_root(&project_root)
                .is_some_and(|state| state.pending_pm_closes.contains_key(&project_root))
        {
            tracing::info!(
                project_root = %project_root.display(),
                "PM ensure skipped while explicit pane close is finalizing"
            );
            return Vec::new();
        }
        let prefs_path = pm_registry::pm_prefs_path_for_repo_path(&project_root);
        let prefs = match pm_registry::load_pm_prefs(&prefs_path) {
            Ok(prefs) => prefs,
            Err(error) => {
                tracing::warn!(
                    path = %prefs_path.display(),
                    %error,
                    "failed to load PM prefs; skipping PM ensure"
                );
                // #4486 AC-7: an unreadable pm.json refuses every Restart the
                // same way. Recording it is best-effort for the same reason it
                // is needed — the file may be the thing that is broken.
                Self::record_pm_start_block(
                    &project_root,
                    pm_registry::PmStartBlockKind::PrefsUnreadable,
                    format!("{} could not be read: {error}", prefs_path.display()),
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
            // focusing selects its rail surface.
            return self.focus_existing_live_work_agent_events(&window_id);
        }
        // Issue #3607 AC-1/AC-2: the singleton is per *repository*, not per
        // project store. One repository can own two stores after a scope split
        // (#3466), and each store's `pm.json` — `auto_start` included — is
        // invisible to the other, so both auto-started and two PMs ended up
        // rewriting one repository's Issue Monitor order and launch orders.
        // Refusing here rather than at registration keeps the second store from
        // ever spawning the pane.
        if let Some(window_id) = self.live_pm_window_id_in_another_store(&project_root) {
            // #4486 AC-7: the refusing condition lives in the *other* store, so
            // nothing the operator does in this project's panel can clear it.
            // `repository_registrations` already carries the session id that
            // `pm.stop` needs; the block is what points at it.
            Self::record_pm_start_block(
                &project_root,
                pm_registry::PmStartBlockKind::AnotherStoreOwnsThePm,
                "another project store in this repository already owns the resident PM".to_string(),
            );
            return self.focus_existing_live_work_agent_events(&window_id);
        }
        let Some(registration) = prefs.registration else {
            tracing::info!(
                project_root = %project_root.display(),
                "PM ensure: no registration yet, spawning the resident PM"
            );
            Self::clear_pm_start_block(&project_root);
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
        if let Ok(session) = gwt_agent::Session::load(&session_path) {
            if session.worktree_path.exists() {
                Self::clear_pm_start_block(&project_root);
                return self.resume_registered_pm_session(tab_id, &project_root, session);
            }
        }
        Self::clear_pm_start_block(&project_root);
        self.spawn_pm_agent(tab_id, &project_root)
    }

    /// #4486 AC-7: persist why the PM did not start, for the refusals a
    /// Restart cannot clear.
    ///
    /// Best-effort by design. The gate's job is to report the block, not to
    /// fail a second way when the project store is itself the problem — a
    /// `PrefsUnreadable` block would otherwise be the one that can never be
    /// written.
    fn record_pm_start_block(
        project_root: &Path,
        kind: pm_registry::PmStartBlockKind,
        reason: String,
    ) {
        let prefs_path = pm_registry::pm_prefs_path_for_repo_path(project_root);
        if let Err(error) = pm_registry::record_pm_start_block(
            &prefs_path,
            pm_registry::PmStartBlock::new(kind, reason),
        ) {
            tracing::warn!(%error, "failed to record the PM start block");
        }
    }

    /// Drop a recorded block once the gate reaches a live, resumed, or
    /// spawning PM, so it never outlives its cause.
    fn clear_pm_start_block(project_root: &Path) {
        let prefs_path = pm_registry::pm_prefs_path_for_repo_path(project_root);
        if let Err(error) = pm_registry::clear_pm_start_block(&prefs_path) {
            tracing::warn!(%error, "failed to clear the PM start block");
        }
    }

    /// SPEC-3431 FR-026: the PM settings snapshot for the active project tab.
    ///
    /// A non-Git or missing active tab produces an explicit unavailable
    /// snapshot. Shared Settings windows can outlive a project tab, so silence
    /// would leak the previous project's values into the new scope.
    pub(crate) fn pm_status_event(&self, context: &super::ProjectContext) -> BackendEvent {
        let Some(project_root) = self.pm_project_root(context) else {
            let loop_interval_secs = pm_registry::PM_LOOP_INTERVAL_DEFAULT_SECS;
            return BackendEvent::PmStatus {
                available: false,
                auto_start: true,
                paused: false,
                loop_interval_secs,
                loop_interval_secs_decimal: loop_interval_secs.to_string(),
                agent_options: Vec::new(),
                configured_agent_id: String::new(),
                configured_model: None,
                configured_reasoning: None,
                running_agent_id: None,
                running_model: None,
                running_reasoning: None,
                is_running: false,
                // No project in scope, so there is no store to have blocked.
                start_block: None,
            };
        };
        let prefs_path = pm_registry::pm_prefs_path_for_repo_path(&project_root);
        let prefs = pm_registry::load_pm_prefs(&prefs_path).unwrap_or_default();
        // Liveness is the pane registry's answer, not the file's: a
        // registration whose pane is gone is a stale record, and reporting it
        // as running would make the panel offer a restart for nothing.
        let running = prefs
            .registration
            .as_ref()
            .filter(|registration| self.pm_registration_is_live(registration));
        pm_status_from_launch_prefs(&self.sessions_dir, &prefs, running)
    }

    /// The PM settings snapshot as a broadcast, for the call sites that change
    /// PM state. Every PM state transition must pass through here — the panel
    /// has no other source of truth, so a silent transition leaves it stale.
    pub(crate) fn pm_status_broadcast_events(
        &self,
        context: &super::ProjectContext,
    ) -> Vec<OutboundEvent> {
        vec![OutboundEvent::project(
            context.project_key.clone(),
            self.pm_status_event(context),
        )]
    }

    fn pm_project_root(&self, context: &super::ProjectContext) -> Option<PathBuf> {
        if !self.project_context_is_current(context) {
            return None;
        }
        self.tab(&context.tab_id)
            .filter(|tab| tab.kind == gwt::ProjectKind::Git)
            .map(|tab| tab.project_root.clone())
    }

    /// SPEC-3431 FR-026/FR-002: persist the auto-start opt-out.
    ///
    /// Deliberately does not touch the running pane. The flag decides whether
    /// opening the project starts a PM; treating it as a stop switch would end
    /// a conversation the user only meant to stop auto-starting next time.
    pub(crate) fn set_pm_auto_start_events(
        &mut self,
        context: &super::ProjectContext,
        enabled: bool,
    ) -> Vec<OutboundEvent> {
        let Some(project_root) = self.pm_project_root(context) else {
            return Vec::new();
        };
        let prefs_path = pm_registry::pm_prefs_path_for_repo_path(&project_root);
        if let Err(error) = pm_registry::mutate_pm_prefs(&prefs_path, |prefs| {
            prefs.settings.auto_start = enabled;
        }) {
            tracing::warn!(%error, "failed to persist the PM auto-start setting");
            return Vec::new();
        }
        self.pm_status_broadcast_events(context)
    }

    pub(crate) fn set_pm_paused_events(
        &mut self,
        context: &super::ProjectContext,
        paused: bool,
    ) -> Vec<OutboundEvent> {
        let Some(project_root) = self.pm_project_root(context) else {
            return Vec::new();
        };
        if let Err(error) = pm_registry::set_pm_paused(&project_root, paused) {
            tracing::warn!(%error, "failed to persist the PM pause setting");
            return Vec::new();
        }
        self.pm_status_broadcast_events(context)
    }

    /// SPEC-3431 FR-132: persist the active project's resident-loop interval.
    ///
    /// Reject before resolving or opening the preferences file so an invalid
    /// wire value cannot enter the shared read-modify-write path. A committed
    /// write changes only the interval; the live PM and its registration keep
    /// running and reload the preference on their next loop/wake evaluation.
    pub(crate) fn set_pm_loop_interval_events(
        &mut self,
        context: &super::ProjectContext,
        loop_interval_secs: u64,
    ) -> Vec<OutboundEvent> {
        if loop_interval_secs < pm_registry::PM_LOOP_INTERVAL_MIN_SECS {
            tracing::warn!(
                loop_interval_secs,
                minimum = pm_registry::PM_LOOP_INTERVAL_MIN_SECS,
                "rejected PM loop interval below the minimum"
            );
            return Vec::new();
        }
        let Some(project_root) = self.pm_project_root(context) else {
            return Vec::new();
        };
        let prefs_path = pm_registry::pm_prefs_path_for_repo_path(&project_root);
        if let Err(error) = pm_registry::mutate_pm_prefs(&prefs_path, |prefs| {
            prefs.settings.loop_interval_secs = loop_interval_secs;
        }) {
            tracing::warn!(%error, "failed to persist the PM loop interval");
            return Vec::new();
        }
        self.pm_status_broadcast_events(context)
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
        context: &super::ProjectContext,
        agent_id: &str,
        model: Option<String>,
        reasoning: Option<String>,
    ) -> Vec<OutboundEvent> {
        let Some(project_root) = self.pm_project_root(context) else {
            return Vec::new();
        };
        if !pm_registry::pm_agent_is_supported(agent_id) {
            tracing::warn!(
                agent_id,
                "rejected PM launch profile: the agent cannot resolve the gwt-pm skill"
            );
            return Vec::new();
        }
        let profile = PmLaunchProfile {
            agent_id: agent_id.to_string(),
            model,
            reasoning,
            version: None,
        }
        .normalized();
        let prefs_path = pm_registry::pm_prefs_path_for_repo_path(&project_root);
        if let Err(error) = pm_registry::mutate_pm_prefs(&prefs_path, |prefs| {
            prefs.settings.launch_profile = Some(profile.clone());
        }) {
            tracing::warn!(%error, "failed to persist the PM launch profile");
            return Vec::new();
        }
        self.pm_status_broadcast_events(context)
    }

    /// SPEC-3431 FR-026: apply the configured profile by restarting the PM.
    ///
    /// Deregisters first, on purpose: the close path treats a registered PM's
    /// close as an intentional stop and reaps a clean PM worktree with it
    /// (T-016). A restart is not a stop — the worktree holds the PM's own
    /// notes — so clearing the registration up front makes that reap a no-op
    /// and leaves the worktree for the successor.
    pub(crate) fn restart_pm_agent_events(
        &mut self,
        context: &super::ProjectContext,
    ) -> Vec<OutboundEvent> {
        if !self.project_context_is_current(context) {
            return Vec::new();
        }
        let tab_id = context.tab_id.clone();
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
        events.extend(self.ensure_pm_agent_for_tab(&tab_id, PmEnsureTrigger::Restart));
        events
    }

    /// SPEC-3431 T-093 (FR-012): decide whether this inbox snapshot must wake
    /// the resident PM, using `now` as the quiet-loop clock.
    ///
    /// The first snapshot per project is a baseline — GUI startup must not
    /// replay a long-lived backlog as if it just happened. After that, a
    /// signal never seen before wakes the PM iff the Monitor is enabled, a
    /// registered PM pane is live, and the resident loop has gone quiet.
    /// A delta suppressed only by an active loop or a busy PM pane (Issue
    /// #4258) is retained (not consumed), so a loop that dies inside its
    /// floor is still revived by the next snapshot and a turn in progress
    /// gets the accumulated signals once it is Idle; every other outcome
    /// consumes the delta.
    pub(crate) fn pm_wake_decision_at(
        &mut self,
        project_root: &Path,
        inbox: &[gwt::IssueMonitorInboxItem],
        now: &str,
    ) -> Option<PmWakeDecision> {
        let context = self.project_context_for_root(project_root)?;
        let prefs_path = pm_registry::pm_prefs_path_for_repo_path(project_root);
        let prefs = pm_registry::load_pm_prefs(&prefs_path).ok()?;
        if prefs.settings.paused {
            return None;
        }
        if self.retry_held_pm_wake(project_root) {
            return None;
        }
        if self
            .project_state(&context)?
            .pm_agent_idle_wakes
            .delivery_in_flight
            .is_some()
        {
            return None;
        }
        let signals = pm_wake_signals(inbox);
        let Some(seen) = self.project_state(&context)?.pm_wake_seen.get(project_root) else {
            self.project_state_mut(&context)?
                .pm_wake_seen
                .insert(project_root.to_path_buf(), signals);
            return None;
        };
        let fresh: Vec<String> = signals.difference(seen).cloned().collect();
        if fresh.is_empty() {
            self.project_state_mut(&context)?
                .pm_wake_seen
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
            self.project_state_mut(&context)?
                .pm_wake_seen
                .insert(project_root.to_path_buf(), signals);
            return None;
        }
        let Some(registration) = prefs.registration else {
            // No PM to wake; a later PM start reads status in its bootstrap.
            self.project_state_mut(&context)?
                .pm_wake_seen
                .insert(project_root.to_path_buf(), signals);
            return None;
        };
        let Some(window_id) = self.live_pm_window_id(&registration.session_id) else {
            // A dead PM is the crash-resume path's job, never the wake's.
            self.project_state_mut(&context)?
                .pm_wake_seen
                .insert(project_root.to_path_buf(), signals);
            return None;
        };
        let interval_secs = prefs.settings.loop_interval_secs_clamped();
        let loop_path = pm_registry::pm_loop_state_path_for_repo_path(project_root);
        let loop_state = pm_registry::load_pm_loop_state(&loop_path).unwrap_or_default();
        if !pm_wake_loop_is_quiet(&loop_state, interval_secs, now)
            || self.pm_wake_pane_is_busy(&window_id, &loop_state, interval_secs, now)
        {
            // Actively looping or mid-turn: its own next cycle reconciles
            // this. Keep the delta so a floor-stopped loop is revived by the
            // next snapshot.
            return None;
        }
        self.project_state_mut(&context)?
            .pm_wake_seen
            .insert(project_root.to_path_buf(), signals);
        // Re-arm the budget and stamp the wake clock: new actionable work is
        // exactly what the park was waiting for, and the stamp keeps the
        // periodic wake from stacking a second prompt in the same window.
        let wake_id = Some(uuid::Uuid::new_v4().to_string());
        if let Err(error) = pm_registry::save_pm_loop_state(
            &loop_path,
            &pm_registry::PmLoopState {
                last_wake_at: Some(now.to_string()),
                last_wake_id: wake_id.clone(),
                ..pm_registry::PmLoopState::default()
            },
        ) {
            tracing::warn!(%error, "PM wake could not re-arm the loop budget");
            return None;
        }
        let mut reasons = fresh;
        reasons.truncate(5);
        Some(PmWakeDecision {
            project_root: project_root.to_path_buf(),
            wake_id,
            window_id,
            unconfirmed: false,
            idle_notifications: Default::default(),
            prompt: format!(
                "[gwt] Monitor activity while the PM was idle ({}). Reconcile now: fresh \
                 `issue.monitor.status`, triage new items, inventory PRs with `pr.list` \
                 (stale/SUPERSEDED/owner-closed rows: digest, never auto-close). \
                 {PM_STEERING_WAKE_CLAUSE} {PM_GWTD_EXECUTION_WAKE_CLAUSE} \
                 {PM_CYCLE_REPORTING_CLAUSE}\r",
                reasons.join(", "),
            ),
        })
    }

    fn pm_agent_capacity_failure(
        &self,
        window_id: &str,
        current: WindowProcessStatus,
    ) -> Option<&str> {
        if current != WindowProcessStatus::Waiting
            || self.window_approval_waiting.contains_key(window_id)
            || self.provider_quota_holds.contains_key(window_id)
        {
            return None;
        }
        self.provider_api_error_holds
            .get(window_id)
            .filter(|error| {
                error.transient && error.summary.starts_with("Selected model is at capacity")
            })
            .map(|error| error.summary.as_str())
    }

    /// Observe the final composed edge, after approval and quota overlays.
    /// Arm only on an observed running hook; the launch-time Running placeholder
    /// followed by SessionStart is not a finished Agent turn.
    pub(super) fn observe_pm_agent_idle_transition(
        &mut self,
        window_id: &str,
        current: WindowProcessStatus,
    ) {
        let Some(address) = self.window_lookup.get(window_id) else {
            return;
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return;
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return;
        };
        let context = self.project_context_for_root(&tab.project_root);
        let observed_running = current == WindowProcessStatus::Running
            && self.window_hook_states.get(window_id) == Some(&WindowProcessStatus::Running);
        let capacity_failure = self.pm_agent_capacity_failure(window_id, current);
        let needs_wake = current == WindowProcessStatus::Idle || capacity_failure.is_some();
        let label = (window.preset == WindowPreset::Agent && needs_wake).then(|| {
            let title = window.purpose_title.as_deref().unwrap_or(&window.title);
            let title: String = title
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .filter(|ch| !ch.is_control())
                .take(96)
                .collect();
            let label = match window.linked_issue_number {
                Some(number) => format!("{title} (#{number})"),
                None => title,
            };
            match capacity_failure {
                Some(reason) => format!("{label}: {reason}"),
                None => label,
            }
        });
        let Some(context) = context else { return };
        let wake_tasks = self.blocking_tasks.clone();
        let Some(state) = self.project_state_mut(&context) else {
            return;
        };
        if current != WindowProcessStatus::Idle {
            if let Some(delivery) = &state.pm_agent_idle_wakes.delivery_in_flight {
                if delivery.window_id == window_id {
                    delivery.cancel(&wake_tasks);
                }
            }
        }
        let was_running = state.pm_agent_idle_wakes.running.remove(window_id);
        if observed_running {
            state
                .pm_agent_idle_wakes
                .running
                .insert(window_id.to_string());
        }
        if !needs_wake {
            state.pm_agent_idle_wakes.pending.remove(window_id);
        } else if let Some(label) = label.filter(|_| was_running) {
            state
                .pm_agent_idle_wakes
                .pending
                .insert(window_id.to_string(), label);
        }
        self.request_pm_agent_idle_wake(&context, false);
        if current == WindowProcessStatus::Idle
            || self.project_state(&context).is_some_and(|state| {
                state
                    .pending_pm_wakes
                    .get(window_id)
                    .is_some_and(|wake| wake.unconfirmed)
            })
        {
            self.flush_pending_pm_wake(window_id);
        }
    }

    pub(super) fn forget_pm_agent_idle_window(&mut self, window_id: &str) {
        let wake_tasks = self.blocking_tasks.clone();
        for state in self.project_states.values_mut() {
            if let Some(delivery) = &state.pm_agent_idle_wakes.delivery_in_flight {
                if delivery.window_id == window_id {
                    delivery.cancel(&wake_tasks);
                }
            }
            state.pm_agent_idle_wakes.running.remove(window_id);
            state.pm_agent_idle_wakes.pending.remove(window_id);
        }
    }

    fn request_pm_agent_idle_wake(&mut self, context: &super::ProjectContext, delayed: bool) {
        let Some(state) = self.project_state_mut(context) else {
            return;
        };
        if state.pm_agent_idle_wakes.pending.is_empty() || state.pm_agent_idle_wakes.retry_scheduled
        {
            return;
        }
        state.pm_agent_idle_wakes.retry_scheduled = true;
        let context = context.clone();
        let proxy = self.proxy.clone();
        if !delayed {
            proxy.send(UserEvent::PmAgentIdleWake { context });
        } else if let Err(error) = thread::Builder::new()
            .name("gwt-agent-idle-wake".into())
            .spawn({
                let context = context.clone();
                move || {
                    thread::sleep(Duration::from_secs(1));
                    proxy.send(UserEvent::PmAgentIdleWake { context });
                }
            })
        {
            if let Some(state) = self.project_state_mut(&context) {
                state.pm_agent_idle_wakes.retry_scheduled = false;
            }
            tracing::warn!(%error, "could not schedule retained Agent idle wake");
        }
    }

    pub(crate) fn pm_agent_idle_wake_decision_at(
        &mut self,
        project_root: &Path,
        now: &str,
    ) -> Option<PmWakeDecision> {
        let context = self.project_context_for_root(project_root)?;
        let state = self.project_state(&context)?;
        if state.pm_agent_idle_wakes.pending.is_empty()
            || state.pm_agent_idle_wakes.delivery_in_flight.is_some()
        {
            return None;
        }
        let enabled = gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(
            project_root,
        ))
        .map(|prefs| prefs.enabled)
        .unwrap_or(false);
        let prefs =
            pm_registry::load_pm_prefs(&pm_registry::pm_prefs_path_for_repo_path(project_root))
                .ok()?;
        if prefs.settings.paused {
            return None;
        }
        let pm_window = prefs
            .registration
            .as_ref()
            .and_then(|registration| self.live_pm_window_id(&registration.session_id));
        let Some(window_id) = pm_window.filter(|_| enabled) else {
            self.project_state_mut(&context)?
                .pm_agent_idle_wakes
                .pending
                .clear();
            return None;
        };
        let stale: Vec<_> = self
            .project_state(&context)?
            .pm_agent_idle_wakes
            .pending
            .keys()
            .filter(|id| {
                **id == window_id
                    || self.window_status(id).is_none_or(|status| {
                        status != WindowProcessStatus::Idle
                            && self.pm_agent_capacity_failure(id, status).is_none()
                    })
            })
            .cloned()
            .collect();
        for id in stale {
            self.project_state_mut(&context)?
                .pm_agent_idle_wakes
                .pending
                .remove(&id);
        }
        let loop_state = pm_registry::load_pm_loop_state(
            &pm_registry::pm_loop_state_path_for_repo_path(project_root),
        )
        .ok()?;
        if self
            .project_state(&context)?
            .pending_pm_wakes
            .get(&window_id)
            .is_some_and(|wake| wake.wake_id != loop_state.last_wake_id && !wake.unconfirmed)
        {
            self.project_state_mut(&context)?
                .pending_pm_wakes
                .remove(&window_id);
        }
        // A real busy turn must finish; the old stuck-Running override is not
        // permission to inject a new Agent idle notification into that turn.
        if !pm_wake_loop_is_quiet(
            &loop_state,
            prefs.settings.loop_interval_secs_clamped(),
            now,
        ) || matches!(
            self.window_status(&window_id),
            Some(WindowProcessStatus::Running | WindowProcessStatus::Waiting)
        ) || self.pane_has_unsent_user_input(&window_id)
            || self
                .project_state(&context)?
                .pending_pm_wakes
                .contains_key(&window_id)
        {
            return None;
        }
        let mut prompt = String::new();
        let mut idle_notifications = std::collections::BTreeMap::new();
        for (id, label) in &self.project_state(&context)?.pm_agent_idle_wakes.pending {
            let entry = format!("{}{label}", pm_registry::PM_AGENT_IDLE_WAKE_PREFIX);
            if prompt.len() + entry.len() > 800 {
                break;
            }
            if !prompt.is_empty() {
                prompt.push_str(" | ");
            }
            prompt.push_str(&entry);
            idle_notifications.insert(id.clone(), label.clone());
        }
        if idle_notifications.is_empty() {
            return None;
        }
        let wake_id = pm_registry::with_active_pm_loop(
            &pm_registry::pm_prefs_path_for_repo_path(project_root),
            || -> Option<String> {
                let loop_path = pm_registry::pm_loop_state_path_for_repo_path(project_root);
                let mut current = pm_registry::load_pm_loop_state(&loop_path).ok()?;
                if current.last_wake_id != loop_state.last_wake_id {
                    return None;
                }
                let wake_id = uuid::Uuid::new_v4().to_string();
                current.last_wake_id = Some(wake_id.clone());
                pm_registry::save_pm_loop_state(&loop_path, &current).ok()?;
                Some(wake_id)
            },
        )
        .ok()
        .flatten()
        .flatten()?;
        for id in idle_notifications.keys() {
            self.project_state_mut(&context)?
                .pm_agent_idle_wakes
                .pending
                .remove(id);
        }
        prompt.push_str(". Reconcile pane.list and issue.monitor.status; steer the idle agents.\r");
        Some(PmWakeDecision {
            project_root: project_root.to_path_buf(),
            window_id,
            prompt,
            wake_id: Some(wake_id),
            unconfirmed: false,
            idle_notifications,
        })
    }

    pub(crate) fn pm_agent_idle_wake_events(&mut self, context: &super::ProjectContext) {
        if !self.project_context_is_current(context) {
            return;
        }
        self.project_state_mut(context)
            .unwrap()
            .pm_agent_idle_wakes
            .retry_scheduled = false;
        if self
            .project_state(context)
            .unwrap()
            .pm_agent_idle_wakes
            .delivery_in_flight
            .is_some()
        {
            return;
        }
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        if let Some(decision) = self.pm_agent_idle_wake_decision_at(&context.project_root, &now) {
            self.project_state_mut(context)
                .unwrap()
                .pm_agent_idle_wakes
                .delivery_in_flight = Some(PmIdleWakeInFlight {
                window_id: decision.window_id.clone(),
                permit: Arc::new(AtomicBool::new(true)),
                dispatched: Arc::new(AtomicBool::new(false)),
                cancellation: None,
            });
            let result = self.write_pm_wake_prompt(&decision);
            if result == Ok(PmWakeWrite::Queued) {
                return;
            }
            self.project_state_mut(context)
                .unwrap()
                .pm_agent_idle_wakes
                .delivery_in_flight = None;
            match result {
                Ok(PmWakeWrite::Deferred | PmWakeWrite::Unconfirmed | PmWakeWrite::Discarded) => {
                    self.restore_pm_agent_idle_notifications(context, &decision)
                }
                Err(error) => tracing::warn!(%error, "Agent idle wake injection failed"),
                Ok(PmWakeWrite::Injected | PmWakeWrite::Queued) => {}
            }
        }
        self.request_pm_agent_idle_wake(context, true);
    }

    fn restore_pm_agent_idle_notifications(
        &mut self,
        context: &super::ProjectContext,
        decision: &PmWakeDecision,
    ) {
        if let Some(state) = self.project_state_mut(context) {
            for (id, label) in &decision.idle_notifications {
                state
                    .pm_agent_idle_wakes
                    .pending
                    .entry(id.clone())
                    .or_insert_with(|| label.clone());
            }
        }
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
    /// gated out. A busy PM pane (Issue #4258) holds the tick without
    /// stamping the wake clock, so the first tick after it is Idle fires.
    #[cfg(test)]
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

    #[cfg(test)]
    pub(crate) fn pm_periodic_wake_decision_for_monitor_at(
        &mut self,
        project_root: &Path,
        monitor: &gwt::IssueMonitorState,
        now: &str,
    ) -> Option<PmWakeDecision> {
        self.pm_periodic_wake_decision_for_monitor_with_standing_work_at(
            project_root,
            monitor,
            now,
            None,
        )
    }

    fn pm_periodic_wake_decision_for_monitor_with_standing_work_at(
        &mut self,
        project_root: &Path,
        monitor: &gwt::IssueMonitorState,
        now: &str,
        standing_durable_work: Option<bool>,
    ) -> Option<PmWakeDecision> {
        if self
            .project_state_for_root(project_root)?
            .pm_agent_idle_wakes
            .delivery_in_flight
            .is_some()
        {
            return None;
        }
        if !monitor.config.enabled {
            return None;
        }
        let prefs_path = pm_registry::pm_prefs_path_for_repo_path(project_root);
        let prefs = pm_registry::load_pm_prefs(&prefs_path).ok()?;
        if prefs.settings.paused {
            return None;
        }
        let status = monitor.agent_status();
        if self.retry_held_pm_wake(project_root) {
            return None;
        }
        // Issue #3655 AC-5: a standing unblock request is supervision work in
        // its own right. Without this the worst case parks itself — the queue
        // empties precisely because every agent stopped, so the tick that
        // should raise the alarm decides there is nothing to supervise.
        if status.active_launches.is_empty()
            && status.queue.is_empty()
            && status.needs_human.is_empty()
            && !monitor.inbox.iter().any(|item| {
                matches!(
                    item.state,
                    gwt::MonitorInboxState::LaunchFailed | gwt::MonitorInboxState::AgentFailed
                )
            })
            && !standing_durable_work.unwrap_or_else(|| pm_has_standing_durable_work(project_root))
        {
            return None;
        }
        let registration = prefs.registration?;
        let window_id = self.live_pm_window_id(&registration.session_id)?;
        let interval_secs = prefs.settings.loop_interval_secs_clamped();
        let loop_path = pm_registry::pm_loop_state_path_for_repo_path(project_root);
        let loop_state = pm_registry::load_pm_loop_state(&loop_path).unwrap_or_default();
        if !pm_wake_loop_is_quiet(&loop_state, interval_secs, now)
            || self.pm_wake_pane_is_busy(&window_id, &loop_state, interval_secs, now)
        {
            return None;
        }
        let wake_id = Some(uuid::Uuid::new_v4().to_string());
        if let Err(error) = pm_registry::save_pm_loop_state(
            &loop_path,
            &pm_registry::PmLoopState {
                last_wake_at: Some(now.to_string()),
                last_wake_id: wake_id.clone(),
                ..pm_registry::PmLoopState::default()
            },
        ) {
            tracing::warn!(%error, "PM periodic wake could not re-arm the loop budget");
            return None;
        }
        Some(PmWakeDecision {
            project_root: project_root.to_path_buf(),
            wake_id,
            window_id,
            unconfirmed: false,
            idle_notifications: Default::default(),
            prompt: format!(
                "{PM_PERIODIC_WAKE_INSTRUCTION} {PM_STEERING_WAKE_CLAUSE} {PM_GWTD_EXECUTION_WAKE_CLAUSE} \
                 {PM_CYCLE_REPORTING_CLAUSE}\r",
            ),
        })
    }

    #[cfg(test)]
    pub(crate) fn pm_periodic_wake_events_for_monitor_at(
        &mut self,
        project_root: &Path,
        monitor: &gwt::IssueMonitorState,
        now: &str,
    ) -> Vec<OutboundEvent> {
        self.pm_periodic_wake_events_for_monitor_with_standing_work_at(
            project_root,
            monitor,
            now,
            None,
        )
    }

    pub(crate) fn pm_periodic_wake_events_for_prepared_monitor_at(
        &mut self,
        project_root: &Path,
        monitor: &gwt::IssueMonitorState,
        now: &str,
        standing_durable_work: bool,
    ) -> Vec<OutboundEvent> {
        self.pm_periodic_wake_events_for_monitor_with_standing_work_at(
            project_root,
            monitor,
            now,
            Some(standing_durable_work),
        )
    }

    fn pm_periodic_wake_events_for_monitor_with_standing_work_at(
        &mut self,
        project_root: &Path,
        monitor: &gwt::IssueMonitorState,
        now: &str,
        standing_durable_work: Option<bool>,
    ) -> Vec<OutboundEvent> {
        let Some(decision) = self.pm_periodic_wake_decision_for_monitor_with_standing_work_at(
            project_root,
            monitor,
            now,
            standing_durable_work,
        ) else {
            return Vec::new();
        };
        match self.write_pm_wake_prompt(&decision) {
            Ok(PmWakeWrite::Queued | PmWakeWrite::Discarded) => {}
            Ok(PmWakeWrite::Injected) => {
                tracing::info!(
                    window_id = %decision.window_id,
                    "periodic wake re-armed the resident PM from the scheduled snapshot"
                );
            }
            Ok(PmWakeWrite::Deferred | PmWakeWrite::Unconfirmed) => {
                tracing::info!(
                    window_id = %decision.window_id,
                    "periodic wake deferred until the PM composer is empty"
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

    #[cfg(test)]
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
            Ok(PmWakeWrite::Queued | PmWakeWrite::Discarded) => {}
            Ok(PmWakeWrite::Injected) => {
                tracing::info!(
                    window_id = %decision.window_id,
                    "woke the resident PM for new Issue Monitor activity"
                );
            }
            Ok(PmWakeWrite::Deferred | PmWakeWrite::Unconfirmed) => {
                tracing::info!(
                    window_id = %decision.window_id,
                    "Issue Monitor wake deferred until the PM composer is empty"
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
    ///
    /// Issue #3702: if the PM TUI composer already has unsent keystrokes,
    /// hold one coalesced prompt instead of splicing `[gwt]` into the line.
    fn retry_held_pm_wake(&mut self, project_root: &Path) -> bool {
        let pending = self
            .project_state_for_root(project_root)
            .and_then(|state| state.pending_pm_wakes.keys().next().cloned());
        if let Some(window_id) = pending {
            self.flush_pending_pm_wake(&window_id);
        }
        self.project_state_for_root(project_root)
            .is_some_and(|state| {
                !state.pending_pm_wakes.is_empty()
                    || state.pm_agent_idle_wakes.delivery_in_flight.is_some()
            })
    }

    fn write_pm_wake_prompt(&mut self, decision: &PmWakeDecision) -> Result<PmWakeWrite, String> {
        let prefs = pm_registry::load_pm_prefs(&pm_registry::pm_prefs_path_for_repo_path(
            &decision.project_root,
        ))
        .map_err(|error| error.to_string())?;
        if prefs.settings.paused && !decision.unconfirmed {
            if let Some(state) = self
                .project_state_for_root_mut(&decision.project_root)
                .filter(|_| decision.idle_notifications.is_empty())
            {
                state
                    .pending_pm_wakes
                    .insert(decision.window_id.clone(), decision.clone());
            }
            return Ok(PmWakeWrite::Deferred);
        }
        let loop_state = pm_registry::load_pm_loop_state(
            &pm_registry::pm_loop_state_path_for_repo_path(&decision.project_root),
        )
        .map_err(|error| error.to_string())?;
        if loop_state.last_wake_id != decision.wake_id && !decision.unconfirmed {
            if let Some(state) = self
                .project_state_for_root_mut(&decision.project_root)
                .filter(|_| decision.idle_notifications.is_empty())
            {
                state.pending_pm_wakes.remove(&decision.window_id);
            }
            return Ok(PmWakeWrite::Discarded);
        }
        let runtime = self
            .runtimes
            .get(&decision.window_id)
            .ok_or_else(|| format!("no live runtime for pane {}", decision.window_id))?;
        let client = runtime.client.clone();
        let owner_pane_id = runtime.owner_pane_id.clone();
        let incarnation = runtime.incarnation;
        let owner_identity = runtime.owner_identity.clone();
        let composer_hint = runtime.has_unsent_user_input.clone();
        if composer_hint.load(Ordering::Acquire) && !decision.unconfirmed {
            if !decision.idle_notifications.is_empty() {
                return Ok(PmWakeWrite::Deferred);
            }
            let state = self
                .project_state_for_root_mut(&decision.project_root)
                .ok_or_else(|| "PM wake project is closed".to_string())?;
            state
                .pending_pm_wakes
                .insert(decision.window_id.clone(), decision.clone());
            return Ok(PmWakeWrite::Deferred);
        }
        let context = self
            .project_state_for_root(&decision.project_root)
            .map(|state| state.context.clone())
            .ok_or_else(|| "PM wake project is closed".to_string())?;
        let proxy = self.proxy.for_project(context.clone());
        let operation_id = if decision.idle_notifications.is_empty() {
            decision
                .wake_id
                .clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
        } else {
            uuid::Uuid::new_v4().to_string()
        };
        let idle_permit = if decision.idle_notifications.is_empty() {
            let state = self.project_state_mut(&context).unwrap();
            if state.pm_agent_idle_wakes.delivery_in_flight.is_some() {
                state
                    .pending_pm_wakes
                    .insert(decision.window_id.clone(), decision.clone());
                return Ok(PmWakeWrite::Deferred);
            }
            state.pm_agent_idle_wakes.delivery_in_flight = Some(PmIdleWakeInFlight {
                window_id: decision.window_id.clone(),
                permit: Arc::new(AtomicBool::new(true)),
                dispatched: Arc::new(AtomicBool::new(false)),
                cancellation: None,
            });
            None
        } else {
            self.project_state_mut(&context)
                .and_then(|state| state.pm_agent_idle_wakes.delivery_in_flight.as_mut())
                .map(|flight| {
                    flight.cancellation = Some(PmWakeCancellation {
                        client: client.clone(),
                        owner_pane_id: owner_pane_id.clone(),
                        incarnation,
                        host_instance_id: owner_identity.host_instance_id.clone(),
                        operation_id: operation_id.clone(),
                    });
                    (flight.permit.clone(), flight.dispatched.clone())
                })
        };
        let worker_decision = decision.clone();
        let worker_context = context.clone();
        let spawn = self.blocking_tasks.try_spawn(move || {
            let result = (|| {
                if let Some((permit, dispatched)) = &idle_permit {
                    dispatched.store(true, Ordering::Release);
                    if !permit.load(Ordering::Acquire) {
                        return Ok(PmWakeWrite::Deferred);
                    }
                }
                if composer_hint.load(Ordering::Acquire) && !worker_decision.unconfirmed {
                    return Ok(PmWakeWrite::Deferred);
                }
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|error| error.to_string())?;
                let value = runtime.block_on(client.command(
                    gwt::project_runtime::events::OwnerCommand::PmWakeInput {
                        id: owner_pane_id.clone(),
                        incarnation,
                        host_instance_id: owner_identity.host_instance_id.clone(),
                        delivery: PmWakeInput {
                            operation_id,
                            wake_id: worker_decision.wake_id.clone(),
                            prompt: worker_decision.prompt.clone(),
                            idle: !worker_decision.idle_notifications.is_empty(),
                        },
                    },
                ))?;
                serde_json::from_value(
                    value
                        .get("status")
                        .cloned()
                        .ok_or("PM wake owner returned no outcome")?,
                )
                .map_err(|error| error.to_string())
            })();
            proxy.send(UserEvent::PmWakeDeliveryComplete(PmWakeDelivery {
                context: worker_context,
                decision: worker_decision,
                owner_pane_id,
                incarnation,
                owner_identity,
                result,
            }));
        });
        if spawn.is_err() && decision.idle_notifications.is_empty() {
            self.project_state_mut(&context)
                .unwrap()
                .pm_agent_idle_wakes
                .delivery_in_flight = None;
        }
        spawn?;
        if decision.idle_notifications.is_empty() {
            if let Some(state) = self.project_state_for_root_mut(&decision.project_root) {
                state.pending_pm_wakes.remove(&decision.window_id);
            }
        }
        Ok(PmWakeWrite::Queued)
    }

    pub(crate) fn pm_wake_delivery_complete(&mut self, mut delivery: PmWakeDelivery) {
        if !self.project_context_is_current(&delivery.context) {
            return;
        }
        let same_pane = self
            .runtimes
            .get(&delivery.decision.window_id)
            .is_some_and(|runtime| {
                runtime.owner_pane_id == delivery.owner_pane_id
                    && runtime.incarnation == delivery.incarnation
                    && runtime.owner_identity == delivery.owner_identity
            });
        if !delivery.decision.idle_notifications.is_empty() {
            self.project_state_mut(&delivery.context)
                .unwrap()
                .pm_agent_idle_wakes
                .delivery_in_flight = None;
            if !same_pane {
                self.request_pm_agent_idle_wake(&delivery.context, true);
                return;
            }
            match &delivery.result {
                Ok(PmWakeWrite::Deferred | PmWakeWrite::Unconfirmed | PmWakeWrite::Discarded) => {
                    self.restore_pm_agent_idle_notifications(&delivery.context, &delivery.decision)
                }
                Ok(PmWakeWrite::Injected) => {
                    tracing::info!(window_id = %delivery.decision.window_id, "Agent idle wake committed by its project owner");
                }
                Err(error) => tracing::warn!(%error, "Agent idle wake delivery failed"),
                _ => {}
            }
            self.request_pm_agent_idle_wake(&delivery.context, true);
            return;
        }
        self.project_state_mut(&delivery.context)
            .unwrap()
            .pm_agent_idle_wakes
            .delivery_in_flight = None;
        if !same_pane {
            return;
        }
        match delivery.result {
            Ok(PmWakeWrite::Unconfirmed) => {
                delivery.decision.unconfirmed = true;
                self.project_state_mut(&delivery.context)
                    .unwrap()
                    .pending_pm_wakes
                    .entry(delivery.decision.window_id.clone())
                    .or_insert(delivery.decision);
            }
            Ok(PmWakeWrite::Deferred) => {
                let window_id = delivery.decision.window_id.clone();
                let Some(state) = self.project_state_mut(&delivery.context) else {
                    return;
                };
                // A newer wake held during preparation owns the coalesced slot.
                state
                    .pending_pm_wakes
                    .entry(window_id.clone())
                    .or_insert(delivery.decision);
                // The user may already have cleared the composer before this
                // completion arrived. That input event could not flush this slot.
                let busy = self
                    .active_agent_sessions
                    .get(&window_id)
                    .zip(self.runtimes.get(&window_id))
                    .is_some_and(|(session, runtime)| {
                        pm_wake_runtime_is_busy(
                            &gwt_agent::runtime_state_path(&self.sessions_dir, &session.session_id),
                            runtime.incarnation,
                        )
                    });
                let paused = pm_registry::load_pm_prefs(&pm_registry::pm_prefs_path_for_repo_path(
                    &delivery.context.project_root,
                ))
                .map(|prefs| prefs.settings.paused)
                .unwrap_or(true);
                if !busy && !paused {
                    self.flush_pending_pm_wake(&window_id);
                }
            }
            Ok(PmWakeWrite::Injected) => {
                tracing::info!(
                    window_id = %delivery.decision.window_id,
                    "delivered a PM wake prepared outside the GUI event loop"
                );
            }
            Ok(PmWakeWrite::Queued | PmWakeWrite::Discarded) => {}
            Err(error) => {
                self.project_state_mut(&delivery.context)
                    .unwrap()
                    .pending_pm_wakes
                    .entry(delivery.decision.window_id.clone())
                    .or_insert_with(|| delivery.decision.clone());
                tracing::warn!(
                    %error,
                    window_id = %delivery.decision.window_id,
                    "PM wake prompt injection failed"
                );
            }
        }
    }

    /// Issue #3702 AC-2: deliver a held wake once the composer is submitted
    /// or cleared. Missing pending entries are a no-op so every pane submit
    /// can call this cheaply.
    pub(crate) fn flush_pending_pm_wake(&mut self, window_id: &str) {
        let Some(decision) = self
            .project_states
            .values()
            .find_map(|state| state.pending_pm_wakes.get(window_id))
            .cloned()
        else {
            return;
        };
        if self.pane_has_unsent_user_input(window_id) && !decision.unconfirmed {
            return;
        }
        match self.write_pm_wake_prompt(&decision) {
            Ok(PmWakeWrite::Queued | PmWakeWrite::Discarded) => {}
            Ok(PmWakeWrite::Injected) => {
                tracing::info!(
                    window_id,
                    "delivered a PM wake held while the composer was busy"
                );
            }
            Ok(PmWakeWrite::Deferred | PmWakeWrite::Unconfirmed) => {}
            Err(error) => {
                tracing::warn!(
                    %error,
                    window_id,
                    "held PM wake prompt injection failed"
                );
            }
        }
    }

    #[cfg(all(test, unix))]
    pub(crate) fn retain_test_pm_wake(&mut self, id: &str, prompt: &str) {
        let address = self.window_lookup[id].clone();
        let context = self.project_context(&address.tab_id).unwrap();
        let project_root = context.project_root.clone();
        let loop_state = pm_registry::load_pm_loop_state(
            &pm_registry::pm_loop_state_path_for_repo_path(&project_root),
        )
        .unwrap();
        let decision = PmWakeDecision {
            project_root,
            wake_id: loop_state.last_wake_id,
            window_id: id.into(),
            prompt: prompt.into(),
            unconfirmed: false,
            idle_notifications: Default::default(),
        };
        self.project_state_mut(&context)
            .unwrap()
            .pending_pm_wakes
            .insert(id.into(), decision);
    }

    pub(crate) fn pane_has_unsent_user_input(&self, window_id: &str) -> bool {
        self.runtimes
            .get(window_id)
            .is_some_and(|runtime| runtime.has_unsent_user_input.load(Ordering::Acquire))
    }

    /// Authoritative liveness for a stored PM registration (FR-001): the
    /// registered session must have a live pane right now.
    pub(crate) fn pm_registration_is_live(&self, registration: &PmRegistration) -> bool {
        self.live_pm_window_id(&registration.session_id).is_some()
    }

    /// Window id of the live pane bound to `session_id`, if any: the session
    /// is in the active registry, its window is still in the catalog, and the
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

    /// Issue #4258: whether the live PM pane is mid-turn (Running) or at a
    /// prompt (Waiting), where an injected wake would splice into the turn or
    /// be read as the prompt's answer. Both wake paths hold — never drop —
    /// their wake while this is true. A pane Running past the deferral bound
    /// is treated as stuck and woken; a Waiting one is only reported.
    fn pm_wake_pane_is_busy(
        &self,
        window_id: &str,
        loop_state: &pm_registry::PmLoopState,
        interval_secs: u64,
        now: &str,
    ) -> bool {
        let status = self.window_status(window_id);
        if !matches!(
            status,
            Some(WindowProcessStatus::Running | WindowProcessStatus::Waiting)
        ) {
            return false;
        }
        let past_bound = pm_wake_loop_is_quiet(
            loop_state,
            interval_secs.saturating_mul(PM_WAKE_BUSY_DEFER_MAX_INTERVALS),
            now,
        );
        let recent_output = self.window_last_output_at.get(window_id).is_some_and(|at| {
            chrono::DateTime::parse_from_rfc3339(now).is_ok_and(|now| {
                now.signed_duration_since(*at).num_seconds()
                    < interval_secs.saturating_mul(PM_WAKE_BUSY_DEFER_MAX_INTERVALS) as i64
            })
        });
        if !past_bound || recent_output {
            return true;
        }
        if self
            .active_agent_sessions
            .get(window_id)
            .zip(self.runtimes.get(window_id))
            .is_some_and(|(session, runtime)| {
                pm_wake_runtime_is_busy(
                    &gwt_agent::runtime_state_path(&self.sessions_dir, &session.session_id),
                    runtime.incarnation,
                )
            })
        {
            return true;
        }
        if status == Some(WindowProcessStatus::Waiting) {
            tracing::warn!(
                window_id,
                "PM wake still held: the PM pane has waited on a prompt past the deferral bound"
            );
            return true;
        }
        tracing::warn!(
            window_id,
            "PM Running observation passed the wake bound without current busy evidence; attempting protected recovery"
        );
        false
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

    /// Test fixture adapter for a PM whose launch has already completed.
    /// Production registration runs in the launch preparation worker. Fixtures
    /// keep the same stale-replacement and live-singleton registration rules.
    #[cfg(test)]
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
            Ok((prefs, outcome)) => {
                // Issue #4394 AC-2: succession ends the replaced Session as a
                // PM even when it died without `pm.stop`, so restore cannot
                // bring it back as a second, unregistered PM.
                if let pm_registry::PmRegisterOutcome::ReplacedStale { previous } = outcome {
                    if previous.session_id != session_id {
                        super::startup::mark_auto_resume_source_completed(
                            &self.sessions_dir,
                            &previous.session_id,
                        );
                    }
                }
                self.sync_pm_session_cache(project_root, prefs.registration.as_ref());
            }
            Err(error) => {
                tracing::warn!(%error, "failed to write PM registration after launch");
            }
        }
    }

    pub(super) fn pm_session_for_root(&self, project_root: &Path) -> Option<&String> {
        self.project_state_for_root(project_root)?
            .pm_sessions
            .get(project_root)
    }

    /// Keep the per-broadcast PM marker in step with the durable record.
    pub(super) fn sync_pm_session_cache(
        &mut self,
        project_root: &Path,
        registration: Option<&PmRegistration>,
    ) {
        let Some(state) = self.project_state_for_root_mut(project_root) else {
            return;
        };
        match registration {
            Some(registration) => {
                state
                    .pm_sessions
                    .insert(project_root.to_path_buf(), registration.session_id.clone());
            }
            None => {
                state.pm_sessions.remove(project_root);
            }
        }
    }

    pub(super) fn owner_pm_lifecycle_events(
        &mut self,
        context: &super::ProjectContext,
        outcome: gwt::project_runtime::pm_delivery::PmLifecycleOutcome,
    ) -> Vec<OutboundEvent> {
        if self
            .pm_session_for_root(&context.project_root)
            .is_some_and(|current| current != &outcome.session_id)
        {
            return Vec::new();
        }
        self.sync_pm_session_cache(&context.project_root, outcome.prefs.registration.as_ref());
        let mut events = vec![OutboundEvent::project(
            context.project_key.clone(),
            pm_status_from_launch_prefs(&self.sessions_dir, &outcome.prefs, None),
        )];
        if outcome.respawn {
            events.extend(self.ensure_pm_agent_for_tab(&context.tab_id, PmEnsureTrigger::Explicit));
        }
        events
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
        if let Some(state) = self.project_state_for_root_mut(project_root) {
            for combined in new_ids {
                state
                    .pending_pm_launches
                    .insert(combined, project_root.to_path_buf());
            }
        }
    }

    /// Resume the registered PM's own conversation (FR-003), recording the
    /// successor pane so launch completion rewrites `pm.json`.
    ///
    /// Issue #4375: when the Session lives in the canonical PM worktree the
    /// spawn now waits on an off-loop preparation, so the marking travels with
    /// the continuation. Marking here would find no pane and the registration
    /// would keep naming the dead session.
    fn resume_registered_pm_session(
        &mut self,
        tab_id: &str,
        project_root: &Path,
        session: gwt_agent::Session,
    ) -> Vec<OutboundEvent> {
        if pm_registry::is_pm_worktree(&session.worktree_path) {
            return self.spawn_pm_worktree_preparation(PmWorktreeContinuation::ResumeSession {
                tab_id: tab_id.to_string(),
                project_root: project_root.to_path_buf(),
                session: Box::new(session),
                workspace_resume_context: None,
                origin: RestoreOrigin::Automatic,
                register_pm_launch: true,
            });
        }
        let before = self.pm_window_ids(tab_id);
        let events =
            self.spawn_restored_agent_session(tab_id, session, None, RestoreOrigin::Automatic);
        self.mark_new_pm_windows(tab_id, &before, project_root);
        events
    }

    fn spawn_pm_agent(&mut self, tab_id: &str, project_root: &Path) -> Vec<OutboundEvent> {
        self.spawn_pm_worktree_preparation(PmWorktreeContinuation::FreshSpawn {
            tab_id: tab_id.to_string(),
            project_root: project_root.to_path_buf(),
        })
    }

    /// Issue #4375 AC-1: run one PM worktree preparation off the GUI event
    /// loop, then resume the spawn it gates from the completion event.
    ///
    /// The in-flight gate preserves the singleton property the synchronous path
    /// got for free: while a preparation is running, a second ensure for the
    /// same repository must not start another one and land a second PM pane.
    pub(super) fn spawn_pm_worktree_preparation(
        &mut self,
        continuation: PmWorktreeContinuation,
    ) -> Vec<OutboundEvent> {
        let Some(context) = self.project_context(continuation.tab_id()) else {
            return Vec::new();
        };
        let project_root = continuation.project_root().to_path_buf();
        let Some(state) = self.project_state_mut(&context) else {
            return Vec::new();
        };
        if !state
            .pending_pm_worktree_preparations
            .insert(project_root.clone())
        {
            tracing::info!(
                project_root = %project_root.display(),
                "PM worktree preparation already in flight; not starting a second one"
            );
            return Vec::new();
        }
        let proxy = self.proxy.for_project(context);
        let spawned = self.blocking_tasks.try_spawn(move || {
            let result = continuation.prepare();
            proxy.send(UserEvent::PmWorktreePrepared {
                continuation: Box::new(continuation),
                result,
            });
        });
        if let Err(error) = spawned {
            // Nothing will report back, so release the gate here instead of
            // leaving the repository permanently unpreparable.
            if let Some(state) = self.project_state_for_root_mut(&project_root) {
                state.pending_pm_worktree_preparations.remove(&project_root);
            }
            return self.pm_worktree_preparation_failed_events(&project_root, &error);
        }
        Vec::new()
    }

    /// Issue #4375: resume the spawn that the PM worktree preparation gated.
    pub(crate) fn handle_pm_worktree_prepared(
        &mut self,
        continuation: PmWorktreeContinuation,
        result: Result<PathBuf, String>,
    ) -> Vec<OutboundEvent> {
        let Some(state) = self.project_state_for_root_mut(continuation.project_root()) else {
            return Vec::new();
        };
        state
            .pending_pm_worktree_preparations
            .remove(continuation.project_root());
        if result.is_ok() {
            state.project_attention_notice.take();
        }
        let tab_id = continuation.tab_id().to_string();
        let mut events = match result {
            Err(error) => {
                self.pm_worktree_preparation_failed_events(continuation.project_root(), &error)
            }
            Ok(worktree) => match continuation {
                PmWorktreeContinuation::FreshSpawn {
                    tab_id,
                    project_root,
                } => self.spawn_prepared_pm_agent(&tab_id, &project_root, &worktree),
                PmWorktreeContinuation::ResumeSession {
                    tab_id,
                    project_root,
                    session,
                    workspace_resume_context,
                    origin,
                    register_pm_launch,
                } => {
                    let before = register_pm_launch.then(|| self.pm_window_ids(&tab_id));
                    let spawned = self.spawn_prepared_restored_agent_session(
                        &tab_id,
                        *session,
                        workspace_resume_context,
                        origin,
                    );
                    if let Some(before) = before {
                        self.mark_new_pm_windows(&tab_id, &before, &project_root);
                    }
                    spawned
                }
            },
        };
        // FR-026: the preparation is where the PM's live state actually
        // changes now, so it is also where the settings panel learns about it.
        if let Some(context) = self.project_context(&tab_id) {
            events.extend(self.pm_status_broadcast_events(&context));
        }
        events
    }

    /// The fresh silent spawn, once its worktree exists.
    fn spawn_prepared_pm_agent(
        &mut self,
        tab_id: &str,
        project_root: &Path,
        worktree: &Path,
    ) -> Vec<OutboundEvent> {
        let profile = pm_registry::pm_prefs_path_for_repo_path(project_root);
        let profile = pm_registry::load_pm_prefs(&profile)
            .map(|prefs| prefs.settings.launch_profile_or_default())
            .unwrap_or_else(|_| pm_registry::PmLaunchProfile::default_profile());
        // T-052: the `$gwt-pm` bootstrap prompt resolves against the guidance
        // skill that managed-asset materialization writes into this worktree.
        // Writing it here instead would be futile — the launch's own asset
        // refresh prunes unbundled `gwt-*` skills right after.
        let config = Self::pm_launch_config(worktree, &profile);
        let before = self.pm_window_ids(tab_id);
        match self.spawn_agent_window_with_identity(tab_id, config, None, None) {
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

    /// Issue #4375 AC-3: a PM worktree preparation that failed stays visible.
    ///
    /// The synchronous path only logged, so a PM pane that never appeared — a
    /// Git error, a full disk — was indistinguishable from one the project had
    /// deliberately opted out of.
    fn pm_worktree_preparation_failed_events(
        &self,
        project_root: &Path,
        error: &str,
    ) -> Vec<OutboundEvent> {
        if error.contains("Git needs attention") {
            return self.project_attention_events(project_root, Some(error), None);
        }
        let Some(context) = self.project_context_for_root(project_root) else {
            return Vec::new();
        };
        tracing::warn!(
            project_root = %project_root.display(),
            error,
            "failed to prepare the PM worktree; PM not started"
        );
        vec![OutboundEvent::project(
            context.project_key.clone(),
            BackendEvent::IssueMonitorToast {
                notification_transition: None,
                notification_transition_id: None,
                level: "error".to_string(),
                message: format!(
                    "PM worktree preparation failed for {}: {error}",
                    project_root.display()
                ),
                issue_number: None,
            },
        )
        .with_error_project_root(&context.project_root)]
    }

    /// SPEC-3431 FR-026: the launch config for a fresh PM spawn.
    ///
    /// Pure so the resolved agent/model can be asserted without spawning.
    /// The canonical worktree remains the config and Session identity; the
    /// launch worker selects the separate PM runtime as the provider's cwd.
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
        let profile = profile.clone().normalized();
        let agent_id = gwt_agent::resolve_agent_id(&profile.agent_id)
            .unwrap_or(gwt_agent::AgentId::ClaudeCode);
        let mut builder = gwt_agent::AgentLaunchBuilder::new(agent_id)
            .working_dir(worktree.to_path_buf())
            .skip_permissions(true);
        if pm_registry::pm_autonomous_bootstrap_allowed(worktree) {
            builder = builder.extra_arg(PM_BOOTSTRAP_PROMPT);
        }
        if let Some(model) = profile.model.as_deref() {
            builder = builder.model(model);
        }
        if let Some(reasoning) = profile.reasoning.as_deref() {
            builder = builder.reasoning_level(reasoning);
        }
        let mut config = builder.build();
        config.suppress_execution_control = true;
        if let Some(scratch) = pm_registry::pm_scratch_dir_for_pm_worktree(worktree) {
            config.env_vars.insert(
                pm_registry::GWT_PM_SCRATCH_DIR_ENV.to_string(),
                scratch.to_string_lossy().into_owned(),
            );
        }
        config
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

#[cfg(test)]
mod delivery_tests {
    use gwt_core::test_support::{env_lock, ScopedGwtHome};

    use super::*;
    use crate::app_runtime::tests::{
        attach_live_pm_pane, pm_wake_fixture, seed_pm_session_escalation,
    };
    use crate::app_runtime::{AppEventProxy, BlockingTaskSpawner, BlockingTestTaskQueue};

    fn finish_input_task(runtime: &mut AppRuntime, tasks: &BlockingTestTaskQueue, index: usize) {
        let task = tasks.lock().unwrap().remove(index);
        task();
        let AppEventProxy::Stub(recorded) = &runtime.proxy else {
            panic!("input fixture proxy");
        };
        let recorded = recorded.clone();
        let events = std::mem::take(&mut *recorded.lock().unwrap());
        for event in events {
            let Some(event) = runtime.accept_project_completion(event) else {
                continue;
            };
            match event {
                UserEvent::RemoteRuntimeCommandComplete(completion) => {
                    runtime.handle_remote_runtime_command_complete(*completion);
                }
                event => recorded.lock().unwrap().push(event),
            }
        }
    }

    #[test]
    fn pm_wake_idle_delivery_preserves_the_existing_composer_slot() {
        let _env_lock = env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(temp.path());
        let (repo, mut runtime, window_id) = pm_wake_fixture(&temp);
        let _pane = attach_live_pm_pane(&mut runtime, &window_id);
        let held = PmWakeDecision {
            project_root: repo.clone(),
            window_id: window_id.clone(),
            prompt: "HELD-WAKE\r".into(),
            wake_id: None,
            unconfirmed: false,
            idle_notifications: Default::default(),
        };
        runtime
            .project_state_for_root_mut(&repo)
            .unwrap()
            .pending_pm_wakes
            .insert(window_id.clone(), held.clone());
        let idle = PmWakeDecision {
            prompt: "[gwt] Agent idle: worker\r".into(),
            idle_notifications: [("worker".into(), "worker".into())].into(),
            ..held.clone()
        };
        pm_registry::set_pm_paused(&repo, true).unwrap();
        assert_eq!(
            runtime.write_pm_wake_prompt(&idle).unwrap(),
            PmWakeWrite::Deferred
        );
        assert_eq!(
            runtime
                .project_state_for_root(&repo)
                .unwrap()
                .pending_pm_wakes
                .get(&window_id),
            Some(&held),
            "paused idle delivery must preserve the regular slot"
        );
        pm_registry::set_pm_paused(&repo, false).unwrap();
        assert_eq!(
            runtime.write_pm_wake_prompt(&idle).unwrap(),
            PmWakeWrite::Discarded
        );
        assert_eq!(
            runtime
                .project_state_for_root(&repo)
                .unwrap()
                .pending_pm_wakes
                .get(&window_id),
            Some(&held),
            "stale idle delivery must preserve the regular slot"
        );
        let idle = PmWakeDecision {
            wake_id: pm_registry::load_pm_loop_state(
                &pm_registry::pm_loop_state_path_for_repo_path(&repo),
            )
            .unwrap()
            .last_wake_id,
            ..idle
        };
        assert_eq!(
            runtime.write_pm_wake_prompt(&idle).unwrap(),
            PmWakeWrite::Queued
        );
        assert_eq!(
            runtime
                .project_state_for_root(&repo)
                .unwrap()
                .pending_pm_wakes
                .get(&window_id),
            Some(&held)
        );
    }

    #[test]
    fn stale_idle_completion_preserves_paused_and_resumed_loop_state() {
        let _env_lock = env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for resume in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let _home = ScopedGwtHome::set(temp.path());
            let (repo, mut runtime, window_id) = pm_wake_fixture(&temp);
            let _pane = attach_live_pm_pane(&mut runtime, &window_id);
            let target = &runtime.runtimes[&window_id];
            let owner_pane_id = target.owner_pane_id.clone();
            let incarnation = target.incarnation;
            let owner_identity = target.owner_identity.clone();
            let loop_path = pm_registry::pm_loop_state_path_for_repo_path(&repo);
            let state = pm_registry::PmLoopState {
                last_wake_id: Some("old-idle".into()),
                last_user_prompt_at: Some("2026-10-10T00:01:00Z".into()),
                ..Default::default()
            };
            pm_registry::save_pm_loop_state(&loop_path, &state).unwrap();
            let decision = PmWakeDecision {
                project_root: repo.clone(),
                window_id,
                prompt: "[gwt] Agent idle: worker\r".into(),
                wake_id: state.last_wake_id,
                unconfirmed: false,
                idle_notifications: [("worker".into(), "worker".into())].into(),
            };
            pm_registry::set_pm_paused(&repo, true).unwrap();
            if resume {
                pm_registry::set_pm_paused(&repo, false).unwrap();
            }
            let latest = pm_registry::load_pm_loop_state(&loop_path).unwrap();
            let context = runtime.project_context_for_root(&repo).unwrap();
            runtime.pm_wake_delivery_complete(PmWakeDelivery {
                context,
                decision,
                owner_pane_id,
                incarnation,
                owner_identity,
                result: Ok(PmWakeWrite::Injected),
            });
            assert_eq!(
                pm_registry::load_pm_loop_state(&loop_path).unwrap(),
                latest,
                "old idle completion must preserve the current activation and conversation"
            );
        }
    }

    use gwt::project_runtime::pm_delivery::PmWakePromptCapture as PromptCapture;

    fn normal_wake(project_root: PathBuf, window_id: String) -> PmWakeDecision {
        let wake_id = Some(uuid::Uuid::new_v4().to_string());
        pm_registry::save_pm_loop_state(
            &pm_registry::pm_loop_state_path_for_repo_path(&project_root),
            &pm_registry::PmLoopState {
                last_wake_id: wake_id.clone(),
                ..Default::default()
            },
        )
        .unwrap();
        PmWakeDecision {
            project_root,
            window_id,
            wake_id,
            prompt: "WORKER-WAKE\r".into(),
            idle_notifications: Default::default(),
            unconfirmed: false,
        }
    }

    #[test]
    fn pm_wake_worker_defers_a_turn_started_after_enqueue() {
        let _env_lock = env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(temp.path());
        let (repo, mut runtime, window_id) = pm_wake_fixture(&temp);
        let _pane = attach_live_pm_pane(&mut runtime, &window_id);
        let (spawner, tasks) = BlockingTaskSpawner::queued();
        runtime.blocking_tasks = spawner;
        let (proxy, recorded) = AppEventProxy::stub();
        runtime.proxy = proxy;
        let capture = PromptCapture::begin(&repo);
        let decision = normal_wake(repo, window_id.clone());
        runtime.write_pm_wake_prompt(&decision).unwrap();
        let session_id = runtime.active_agent_sessions[&window_id].session_id.clone();
        let path = gwt_agent::runtime_state_path(&runtime.sessions_dir, &session_id);
        let mut state = gwt_agent::SessionRuntimeState::new(gwt_agent::AgentStatus::Running);
        state.runtime_incarnation = Some(runtime.runtimes[&window_id].incarnation);
        state.active_tool_calls = Some(Default::default());
        state.source_event = Some("Stop".into());
        state.last_activity_at = chrono::Utc::now() - chrono::Duration::hours(1);
        state.save(&path).unwrap();
        tasks.lock().unwrap().remove(0)();
        assert!(
            capture.prompts().is_empty(),
            "a busy PM retains the wake outside its composer"
        );
        let event = recorded.lock().unwrap().pop().unwrap();
        let UserEvent::ProjectCompletion { event, .. } = event else {
            panic!("project completion")
        };
        let UserEvent::PmWakeDeliveryComplete(delivery) = *event else {
            panic!("wake completion")
        };
        assert_eq!(delivery.result, Ok(PmWakeWrite::Deferred));
        runtime.pm_wake_delivery_complete(delivery);
        assert!(
            tasks.lock().unwrap().is_empty(),
            "a silent turn must not cause a retry task storm"
        );
        state.status = gwt_agent::AgentStatus::Idle;
        state.last_completed_hook_event_at = Some(state.updated_at);
        state.save(&path).unwrap();
        runtime.flush_pending_pm_wake(&window_id);
        assert_eq!(
            tasks.lock().unwrap().len(),
            1,
            "idle resumes the same held wake"
        );
    }

    #[test]
    fn pm_wake_delivery_prepares_subjects_only_on_worker() {
        let _env_lock = env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(temp.path());
        let (repo, mut runtime, window_id) = pm_wake_fixture(&temp);
        let _pane = attach_live_pm_pane(&mut runtime, &window_id);
        let (spawner, tasks) = BlockingTaskSpawner::queued();
        runtime.blocking_tasks = spawner;
        let (proxy, recorded) = AppEventProxy::stub();
        runtime.proxy = proxy;
        let capture = PromptCapture::begin(&repo);
        let decision = normal_wake(repo, window_id.clone());

        runtime.write_pm_wake_prompt(&decision).unwrap();

        assert!(
            capture.prompts().is_empty(),
            "GUI enqueue must not read escalation subjects or prepare a prompt"
        );
        assert_eq!(tasks.lock().unwrap().len(), 1);
        let reservation = crate::app_runtime::tests::owner_runtime_probe(&runtime, &window_id)
            .pty
            .reserve_input_transaction()
            .expect("enqueue must not begin physical prompt delivery");
        drop(reservation);
        let task = tasks.lock().unwrap().remove(0);
        task();
        assert_eq!(capture.prompts(), vec!["WORKER-WAKE\r"]);
        let event = recorded.lock().unwrap().pop().unwrap();
        let UserEvent::ProjectCompletion { event, .. } = event else {
            panic!("project completion")
        };
        let UserEvent::PmWakeDeliveryComplete(delivery) = *event else {
            panic!("wake completion")
        };
        assert_eq!(
            delivery.result,
            Ok(PmWakeWrite::Unconfirmed),
            "physical CR is not a turn ACK"
        );
        runtime.pm_wake_delivery_complete(delivery);
        let probe = crate::app_runtime::tests::owner_runtime_probe(&runtime, &window_id);
        runtime.terminal_input_events(&window_id, "separate user input");
        assert!(
            probe.pty.reserve_input_transaction().is_err(),
            "ordinary input stays behind the unconfirmed wake transaction"
        );
        pm_registry::set_pm_paused(&decision.project_root, true).unwrap();
        let operation_id = decision.wake_id.as_deref().unwrap();
        let body = decision.prompt.trim_end_matches('\r');
        let session = probe.session.as_ref().unwrap();
        {
            let _session = gwt_core::test_support::ScopedEnvVar::set("GWT_SESSION_ID", &session.id);
            let input = serde_json::json!({"prompt": format!(
                "{body} [gwt-delivery:{operation_id}:{}]", pm_registry::pm_delivery_prompt_sha256(body),
            )}).to_string();
            gwt::cli::hook::pm_loop_stop_check::handle_delivery_acknowledgement(
                &decision.project_root,
                &input,
            );
        }
        let mut ack = crate::app_runtime::tests::runtime_hook_state_for_event(
            "running",
            "UserPromptSubmit",
            &session.id,
        );
        ack.project_root = Some(decision.project_root.to_string_lossy().into_owned());
        ack.branch = Some(session.branch.clone());
        assert!(
            crate::app_runtime::tests::owner_fixture_block_on(probe.runtime_hook(&ack))
                .unwrap()
                .is_success()
        );
        // The authenticated owner releases the captured transaction even if
        // its effective Running status does not change while Pause is active.
        runtime.handle_runtime_hook_event(ack);
        for task in std::mem::take(&mut *tasks.lock().unwrap()) {
            task();
        }
        let completions = std::mem::take(&mut *recorded.lock().unwrap());
        let mut wake_results = completions.into_iter().filter_map(|event| {
            if let Some(UserEvent::PmWakeDeliveryComplete(delivery)) =
                runtime.accept_project_completion(event)
            {
                Some(delivery)
            } else {
                None
            }
        });
        let delivery = wake_results
            .next()
            .expect("unchanged Running hook recovers the late ACK while paused");
        assert!(
            wake_results.next().is_none(),
            "only one recovery is scheduled"
        );
        assert_eq!(delivery.result, Ok(PmWakeWrite::Injected));
        runtime.pm_wake_delivery_complete(delivery);
        probe
            .pty
            .reserve_input_transaction()
            .expect("exact ACK releases ordinary input");
        assert_eq!(capture.prompts(), vec!["WORKER-WAKE\r"]);
    }

    #[test]
    fn pm_wake_delivery_rechecks_subject_after_pending_composer_is_cleared() {
        let _env_lock = env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(temp.path());
        let (repo, mut runtime, window_id) = pm_wake_fixture(&temp);
        let _pane = attach_live_pm_pane(&mut runtime, &window_id);
        let mut subject = gwt_agent::Session::new(&repo, "work/subject", gwt_agent::AgentId::Codex);
        subject.status = gwt_agent::AgentStatus::Running;
        seed_pm_session_escalation(&repo, &subject, "PENDING-SUBJECT");
        let (spawner, tasks) = BlockingTaskSpawner::queued();
        runtime.blocking_tasks = spawner;
        let capture = PromptCapture::begin(&repo);
        let decision = normal_wake(repo, window_id.clone());
        runtime.terminal_input_events(&window_id, "draft");
        finish_input_task(&mut runtime, &tasks, 0);
        assert_eq!(
            runtime.write_pm_wake_prompt(&decision).unwrap(),
            PmWakeWrite::Deferred
        );
        assert!(tasks.lock().unwrap().is_empty());
        assert_eq!(
            runtime
                .project_state(&runtime.test_context())
                .unwrap()
                .pending_pm_wakes[&window_id],
            decision
        );
        runtime.terminal_input_events(&window_id, "\u{0003}");
        finish_input_task(&mut runtime, &tasks, 0);
        assert!(capture.prompts().is_empty());
        subject.status = gwt_agent::AgentStatus::Stopped;
        subject.save(&gwt_core::paths::gwt_sessions_dir()).unwrap();

        let task = tasks.lock().unwrap().remove(0);
        task();

        assert_eq!(capture.prompts(), vec!["WORKER-WAKE\r"]);
    }

    #[test]
    fn pm_wake_delivery_retries_when_composer_clears_before_deferred_completion() {
        let _env_lock = env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(temp.path());
        let (repo, mut runtime, window_id) = pm_wake_fixture(&temp);
        let _pane = attach_live_pm_pane(&mut runtime, &window_id);
        let (proxy, recorded) = AppEventProxy::stub();
        runtime.proxy = proxy;
        let (spawner, tasks) = BlockingTaskSpawner::queued();
        runtime.blocking_tasks = spawner;
        let capture = PromptCapture::begin(&repo);
        let decision = normal_wake(repo, window_id.clone());
        runtime.write_pm_wake_prompt(&decision).unwrap();
        runtime.terminal_input_events(&window_id, "draft");
        // The ordinary input arrived after wake booking but before its worker.
        // Commit that same real owner input, retaining the queued wake task.
        finish_input_task(&mut runtime, &tasks, 1);
        let task = tasks.lock().unwrap().remove(0);
        task();
        let event = recorded.lock().unwrap().pop().unwrap();
        let UserEvent::ProjectCompletion { event, .. } = event else {
            panic!("wake completion must keep its project generation");
        };
        let UserEvent::PmWakeDeliveryComplete(delivery) = *event else {
            panic!("expected wake delivery completion");
        };
        assert_eq!(delivery.result, Ok(PmWakeWrite::Deferred));

        runtime.terminal_input_events(&window_id, "\u{0003}");
        finish_input_task(&mut runtime, &tasks, 0);
        runtime.pm_wake_delivery_complete(delivery);

        assert_eq!(tasks.lock().unwrap().len(), 1);
        assert!(runtime
            .project_state(&runtime.test_context())
            .unwrap()
            .pending_pm_wakes
            .is_empty());
        let task = tasks.lock().unwrap().remove(0);
        task();
        assert_eq!(capture.prompts(), vec!["WORKER-WAKE\r"]);
    }

    #[test]
    fn resumed_pm_drops_a_pre_pause_wake_before_a_same_timestamp_fresh_tick() {
        let _env_lock = env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(temp.path());
        let (repo, mut runtime, window_id) = pm_wake_fixture(&temp);
        let _pane = attach_live_pm_pane(&mut runtime, &window_id);
        let (proxy, recorded) = AppEventProxy::stub();
        runtime.proxy = proxy;
        let (spawner, tasks) = BlockingTaskSpawner::queued();
        runtime.blocking_tasks = spawner;
        let capture = PromptCapture::begin(&repo);
        let monitor = gwt::IssueMonitorState::with_prefs(
            gwt::IssueMonitorConfig::default(),
            gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&repo))
                .unwrap(),
        );
        let now = "2026-10-10T01:00:00Z";
        runtime.pm_periodic_wake_events_for_prepared_monitor_at(&repo, &monitor, now, true);
        pm_registry::set_pm_paused(&repo, true).unwrap();
        let task = tasks.lock().unwrap().remove(0);
        task();
        let event = recorded.lock().unwrap().pop().unwrap();
        let UserEvent::ProjectCompletion { event, .. } = event else {
            panic!("wake completion must keep its project generation");
        };
        let UserEvent::PmWakeDeliveryComplete(delivery) = *event else {
            panic!("expected wake delivery completion");
        };
        assert_eq!(delivery.result, Ok(PmWakeWrite::Deferred));
        let before_resume = capture.prompts().len();

        pm_registry::set_pm_paused(&repo, false).unwrap();
        runtime.pm_wake_delivery_complete(delivery);
        // A same-second fresh scan must replace, rather than revive, the old wake.
        runtime.pm_periodic_wake_events_for_prepared_monitor_at(&repo, &monitor, now, true);
        for task in std::mem::take(&mut *tasks.lock().unwrap()) {
            task();
        }

        assert_eq!(
            capture.prompts().len() - before_resume,
            1,
            "Resume must deliver one fresh reconciliation instead of both old and new wakes"
        );
    }
}
