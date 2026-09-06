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
//! - Late runtime wiring setters ([`AppRuntime::set_agent_capability_issuer`],
//!   [`AppRuntime::set_server_url`], [`AppRuntime::set_usage_refresh`])
//!
//! Behavior-preserving move: `AppRuntime::new` and
//! `PendingStartupAutoResumeSession` stay in `mod.rs`.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    thread::JoinHandle,
};

use super::continuation::ActiveOwnerLiveness;
use super::{
    combined_window_id, execute_orphan_intake_worktree_prune, launch_config_from_persisted_session,
    plan_orphan_intake_worktree_prune, same_worktree_path, should_auto_start_restored_window,
    workspace_resume_context_for_work_item, AgentCapabilityIssuer, AppRuntime,
    OrphanIntakePrunePlan, OutboundEvent, PendingStartupAutoResumeSession, WindowGeometry,
    WindowPreset, WindowProcessStatus, WorkspaceResumeContext,
};

/// SPEC-3214 T-006: per-repo cap on orphaned intake worktrees reaped per
/// startup so a pathological pile-up cannot stall boot.
const MAX_STARTUP_INTAKE_PRUNE: usize = 32;
const STARTUP_AUTO_RESUME_STALE_AFTER_SECS: i64 = 24 * 60 * 60;
const STARTUP_AUTO_RESUME_STACK_OFFSET_X: f64 = 28.0;
const STARTUP_AUTO_RESUME_STACK_OFFSET_Y: f64 = 24.0;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct StartupGenerationReaperSummary {
    pub inspected: usize,
    pub reaped: usize,
    pub replayed: usize,
    pub protected: usize,
    pub unchanged: usize,
    pub failures: usize,
}

pub(super) fn spawn_startup_orphan_intake_prune_with<T, F>(
    jobs: Vec<T>,
    mut prune: F,
) -> Option<JoinHandle<()>>
where
    T: Send + 'static,
    F: FnMut(T) -> usize + Send + 'static,
{
    if jobs.is_empty() {
        return None;
    }
    std::thread::Builder::new()
        .name("gwt-startup-intake-recovery".to_string())
        .spawn(move || {
            for job in jobs {
                let _ = prune(job);
            }
        })
        .ok()
}

fn spawn_startup_orphan_intake_prune(plans: Vec<(PathBuf, OrphanIntakePrunePlan)>) {
    let _ = spawn_startup_orphan_intake_prune_with(plans, |(project_root, plan)| {
        let pruned = execute_orphan_intake_worktree_prune(plan, MAX_STARTUP_INTAKE_PRUNE);
        if pruned > 0 {
            tracing::info!(
                project_root = %project_root.display(),
                pruned,
                "reaped orphaned ephemeral intake worktrees on startup"
            );
        }
        pruned
    });
}

pub(super) fn self_heal_managed_hooks_in_worktrees<'a>(
    worktrees: impl IntoIterator<Item = &'a Path>,
) {
    let expected_hook_bin = std::env::var("GWT_HOOK_BIN")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| {
            gwt::managed_assets::resolve_public_gwt_bin_path()
                .ok()
                .map(|path| path.to_string_lossy().into_owned())
        });
    self_heal_managed_hooks_in_worktrees_with_expected(worktrees, expected_hook_bin.as_deref());
}

pub(super) fn self_heal_managed_hooks_in_worktrees_with_expected<'a>(
    worktrees: impl IntoIterator<Item = &'a Path>,
    expected_hook_bin: Option<&str>,
) {
    let mut seen = HashSet::new();
    for worktree in worktrees {
        let canonical = dunce::canonicalize(worktree).unwrap_or_else(|_| worktree.to_path_buf());
        if !seen.insert(canonical.clone()) {
            continue;
        }
        let mut input = gwt::cli::hook::health::ManagedHookHealthInput::new(&canonical);
        input.runtime_state_path = None;
        if let Some(expected_hook_bin) = expected_hook_bin {
            input.expected_hook_bin = Some(expected_hook_bin.to_string());
        }
        let health = gwt::cli::hook::health::read_managed_hook_health(&input);
        let needs_repair = matches!(
            health.status,
            gwt::cli::hook::health::ManagedHookHealthStatus::NeedsAttention
                | gwt::cli::hook::health::ManagedHookHealthStatus::Degraded
        );
        let only_current_binary_is_missing = expected_hook_bin.is_some()
            && !health.issues.is_empty()
            && health
                .issues
                .iter()
                .all(|issue| issue.starts_with("managed hook binary missing:"));
        if only_current_binary_is_missing {
            continue;
        }
        if !needs_repair {
            continue;
        }
        if let Err(error) =
            gwt::managed_assets::regenerate_existing_managed_hook_configs(&canonical)
        {
            tracing::warn!(
                worktree = %canonical.display(),
                %error,
                "managed hook startup self-heal failed"
            );
        } else if needs_repair {
            if let Err(error) = gwt::cli::hook::health::record_managed_hook_self_healed(&canonical)
            {
                tracing::warn!(
                    worktree = %canonical.display(),
                    %error,
                    "managed hook self-heal marker failed"
                );
            }
        }
    }
}

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

/// Issue #3934: read the holder's durable state to decide whether the reaper
/// is even allowed to consider it. Unreadable and missing records answer
/// `false` so this can only ever widen what the exact stage revalidates.
///
/// Issue #3964 AC-2: a durably `Running` holder is admitted too. A launch that
/// died before its agent ever ran leaves exactly that record with no runtime
/// sidecar anywhere, and only the exact stage can tell that apart from a live
/// agent — it answers `Unchanged` for a live one.
fn durable_holder_status_admits_exact_stage(sessions_dir: &Path, session_id: &str) -> bool {
    match gwt_agent::inspect_session_path(&sessions_dir.join(format!("{session_id}.toml"))) {
        gwt_agent::SessionPathState::Present(session) => {
            gwt::cli::execution_state::holder_status_permits_generation_reclaim(session.status)
                || session.status == gwt_agent::AgentStatus::Running
        }
        gwt_agent::SessionPathState::Missing | gwt_agent::SessionPathState::Error(_) => false,
    }
}

/// How the reaper reports owner ledgers it cannot inspect.
///
/// Startup reports each one once at `warn`; the scan cadence would repeat the
/// same permanent set (owners whose worktree was deleted) every tick, so it
/// reports them at `debug` and lets the summary count carry the signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GenerationReaperFailureLog {
    Warn,
    Debug,
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
        let startup_worktrees = self
            .tabs
            .iter()
            .flat_map(|tab| {
                gwt::worktree_inventory::enumerate_worktrees(&tab.project_root, None)
                    .map(|entries| entries.into_iter().map(|entry| entry.path).collect())
                    .unwrap_or_else(|error| {
                        tracing::warn!(
                            project_root = %tab.project_root.display(),
                            %error,
                            "managed hook startup self-heal inventory failed"
                        );
                        vec![tab.project_root.clone()]
                    })
            })
            .collect::<Vec<_>>();
        self_heal_managed_hooks_in_worktrees(startup_worktrees.iter().map(PathBuf::as_path));

        // Fresh linked-owner launch authority is durable in the Session and
        // owner ledger, while readiness capabilities are intentionally
        // process-local. Reconcile that durable pair before startup migrations
        // or auto-resume can observe a partial Activated/Aborted transaction.
        self.reconcile_durable_fresh_execution_launches();

        // SPEC-2359 US-37 / FR-119 / FR-123: One-shot retroactive migration to
        // mark historical merged `work/*` Start Work Workspaces as Done so the
        // Workspace Overview Completed column reflects past completions on the
        // first startup after auto-done emission lands. The scan is idempotent
        // per `work_item_id` and skips silently when journal / work_events
        // files are missing or unreadable.
        let now = chrono::Utc::now();
        let mut orphan_intake_prune_plans = Vec::new();
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
            // Snapshot candidates before the GUI becomes interactive, then
            // inspect/remove only that fixed set on a recovery worker. A new
            // intake launched after startup can never enter this plan.
            if let Some(plan) = plan_orphan_intake_worktree_prune(&tab.project_root) {
                orphan_intake_prune_plans.push((tab.project_root.clone(), plan));
            }
        }
        let planned_orphan_intake_paths = orphan_intake_prune_plans
            .iter()
            .flat_map(|(_, plan)| plan.detached_worktree_paths().iter().cloned())
            .collect::<HashSet<_>>();
        // Issue #4038 (AC-4 / AC-5): if this launch is the tail of an update
        // apply, settle the resume marker first so the auto-resume queue below
        // can bypass the freshness gate for the projects that were open.
        self.settle_update_resume_marker_at_bootstrap(now);
        self.queue_startup_auto_resume_sessions(&planned_orphan_intake_paths);
        // SPEC-2359 W-37 / Issue #3735: restore selection is the protection
        // producer. Complete it before reaping repository owner ledgers, and
        // complete the reaper synchronously before bootstrap returns to the
        // Issue Monitor/daemon dispatch threads.
        self.reap_startup_defunct_active_generations(&startup_worktrees);
        spawn_startup_orphan_intake_prune(orphan_intake_prune_plans);

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
        // SPEC-3431 FR-002: tabs already open at launch get their resident PM
        // pane once the canvas reports bounds (same deferral rule as startup
        // auto-resume — agent panes never spawn before the canvas is ready).
        // Projects opened later get it from `open_project_path_events`.
        self.pending_startup_pm_tabs = self.tabs.iter().map(|tab| tab.id.clone()).collect();
        let _ = self.persist();
    }

    pub(super) fn queue_startup_auto_resume_sessions(
        &mut self,
        planned_orphan_intake_paths: &HashSet<PathBuf>,
    ) {
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
            // The startup prune plan is the authoritative fixed snapshot of
            // detached intake paths that are about to be removed. Exclude
            // those exact worktrees before dispatching the prune worker so a
            // persisted Session/placeholder cannot race remove_force with an
            // auto-resume launch. Use path identity rather than the
            // `.intake-*` basename: a branch-backed worktree may legitimately
            // share that name, and Windows/symlink aliases may differ
            // textually while resolving to the same worktree.
            if planned_orphan_intake_paths
                .iter()
                .any(|planned| same_worktree_path(planned, &session.worktree_path))
            {
                continue;
            }
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
                // Issue #4038 (AC-4): sessions of a project that was open when
                // the update apply began resume regardless of age — the gap
                // was the update, not the operator walking away.
                let resumes_after_update = !self.update_resume_tab_ids.is_empty()
                    && self
                        .auto_resume_tab_id_for_session(&session)
                        .is_some_and(|tab_id| self.update_resume_tab_ids.contains(&tab_id));
                if !resumes_after_update && !startup_auto_resume_is_fresh(&session, now) {
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
            // Issue #3927 (SPEC #3340 FR-047): the canonical terminal
            // predicate fences automatic restore. A settled, closed, or
            // revoked Issue-linked window is marked restore-disabled and its
            // placeholder removed instead of respawning.
            let project_root = tab.project_root.clone();
            let placeholder_window_id = tab
                .workspace
                .persisted()
                .windows
                .iter()
                .find(|window| window.session_id.as_deref() == Some(session.id.as_str()))
                .map(|window| combined_window_id(&tab_id, &window.id));
            if let Some(reason) = self.restore_admission_terminal_reason(
                &session,
                &project_root,
                placeholder_window_id.as_deref(),
            ) {
                self.refuse_terminal_session_restore(&tab_id, &session.id, reason);
                continue;
            }
            let config = launch_config_from_persisted_session(&session);
            if config.session_mode != gwt_agent::SessionMode::Resume {
                continue;
            }
            let project_state_root = session
                .project_state_root
                .as_deref()
                .unwrap_or(&session.worktree_path);
            let workspace_resume_context = Some(workspace_resume_context_for_work_item(
                project_state_root,
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

    /// Reap every integrity-valid stale Active owner visible in the fixed
    /// startup worktree inventory. The canonical non-local liveness predicate
    /// is a conservative prefilter; the execution-state coordinator then
    /// re-proves the complete Session/runtime identity under its leases.
    pub(super) fn reap_startup_defunct_active_generations(
        &self,
        startup_worktrees: &[PathBuf],
    ) -> StartupGenerationReaperSummary {
        let mut protected_exact_sessions = Vec::new();
        let mut protected_unknown_session_ids = HashSet::new();
        for pending in &self.pending_startup_auto_resume_sessions {
            match gwt_agent::SessionExecutionIdentity::from_session(&pending.session) {
                Ok(Some(identity)) => protected_exact_sessions.push(identity),
                Ok(None) | Err(_) => {
                    protected_unknown_session_ids.insert(pending.session.id.clone());
                }
            }
        }
        reap_defunct_active_generations(
            &self.sessions_dir,
            startup_worktrees,
            &protected_exact_sessions,
            &protected_unknown_session_ids,
            GenerationReaperFailureLog::Warn,
        )
    }
}

/// Reap every integrity-valid stale Active owner visible in a worktree
/// inventory. The canonical non-local liveness predicate is a conservative
/// prefilter; the execution-state coordinator then re-proves the complete
/// Session/runtime identity under its leases.
///
/// Issue #3934: this is a free function so the Issue Monitor scan can run the
/// same recovery on its own worker thread. A holder that dies mid-execution
/// used to hold its owner's generation until the next GUI restart, which in
/// practice meant forever: the queue kept refusing the owner and the only
/// recovery left was registering the work under a fresh Issue number.
pub(super) fn reap_defunct_active_generations(
    sessions_dir: &Path,
    worktrees: &[PathBuf],
    protected_exact_sessions: &[gwt_agent::SessionExecutionIdentity],
    protected_unknown_session_ids: &HashSet<String>,
    failure_log: GenerationReaperFailureLog,
) -> StartupGenerationReaperSummary {
    let started_at = std::time::Instant::now();
    let scan = gwt::cli::execution_state::inspect_startup_active_generation_ledgers(worktrees);
    let mut summary = StartupGenerationReaperSummary {
        failures: scan.failures.len(),
        ..StartupGenerationReaperSummary::default()
    };
    for failure in &scan.failures {
        match failure_log {
            GenerationReaperFailureLog::Warn => tracing::warn!(
                path = %failure.path.display(),
                error = %failure.message,
                "startup Active generation owner inspection failed closed"
            ),
            GenerationReaperFailureLog::Debug => tracing::debug!(
                path = %failure.path.display(),
                error = %failure.message,
                "scan Active generation owner inspection failed closed"
            ),
        }
    }

    let liveness_by_session = super::continuation::classify_nonlocal_active_owner_liveness_batch_at(
        sessions_dir,
        scan.candidates
            .iter()
            .filter(|candidate| candidate.replay_operation_id.is_none())
            .map(|candidate| candidate.session_id.as_str()),
    );

    for candidate in scan.candidates {
        summary.inspected += 1;
        if candidate.replay_operation_id.is_some() {
            match gwt::cli::execution_state::repair_startup_defunct_active_generation(&candidate) {
                Ok(gwt::cli::execution_state::StartupActiveGenerationReapOutcome::Replayed) => {
                    summary.replayed += 1;
                }
                Ok(_) => {
                    summary.unchanged += 1;
                }
                Err(error) => {
                    summary.failures += 1;
                    tracing::warn!(
                        owner_kind = candidate.owner.kind.as_str(),
                        owner_number = candidate.owner.number,
                        generation_id = %candidate.generation_id,
                        %error,
                        "startup Active generation replay failed closed"
                    );
                }
            }
            continue;
        }
        if protected_unknown_session_ids.contains(&candidate.session_id) {
            summary.protected += 1;
            continue;
        }
        let liveness = liveness_by_session
            .get(&candidate.session_id)
            .copied()
            .unwrap_or(ActiveOwnerLiveness::Unknown);
        // Issue #3934: the coarse prefilter only decides what is worth the
        // exact revalidation below; it must not be the reason a holder is
        // never looked at. It reports `Unknown` for a durably Idle Session,
        // which is exactly the state a closed agent window leaves behind,
        // so admit every durable state the exact stage is allowed to
        // reclaim and let that stage make the decision.
        if !matches!(liveness, ActiveOwnerLiveness::Stale(_))
            && !durable_holder_status_admits_exact_stage(sessions_dir, &candidate.session_id)
        {
            summary.unchanged += 1;
            continue;
        }
        let exact_holder = match gwt::cli::execution_state::startup_candidate_holder_identity(
            sessions_dir,
            &candidate,
        ) {
            Ok(Some(identity)) => identity,
            Ok(None) => {
                summary.unchanged += 1;
                continue;
            }
            Err(error) => {
                summary.failures += 1;
                tracing::warn!(
                    owner_kind = candidate.owner.kind.as_str(),
                    owner_number = candidate.owner.number,
                    generation_id = %candidate.generation_id,
                    %error,
                    "startup Active generation exact holder inspection failed closed"
                );
                continue;
            }
        };
        match gwt::cli::execution_state::reap_startup_defunct_active_generation(
            &candidate,
            sessions_dir,
            &exact_holder,
            protected_exact_sessions,
        ) {
            Ok(gwt::cli::execution_state::StartupActiveGenerationReapOutcome::Reaped) => {
                summary.reaped += 1;
            }
            Ok(gwt::cli::execution_state::StartupActiveGenerationReapOutcome::Replayed) => {
                summary.replayed += 1;
            }
            Ok(gwt::cli::execution_state::StartupActiveGenerationReapOutcome::Protected) => {
                summary.protected += 1;
            }
            Ok(gwt::cli::execution_state::StartupActiveGenerationReapOutcome::Unchanged) => {
                summary.unchanged += 1;
            }
            Err(error) => {
                summary.failures += 1;
                tracing::warn!(
                    owner_kind = candidate.owner.kind.as_str(),
                    owner_number = candidate.owner.number,
                    generation_id = %candidate.generation_id,
                    %error,
                    "startup Active generation reap failed closed"
                );
            }
        }
    }
    tracing::info!(
        inspected = summary.inspected,
        reaped = summary.reaped,
        replayed = summary.replayed,
        protected = summary.protected,
        unchanged = summary.unchanged,
        failures = summary.failures,
        roots_scanned = scan.roots_scanned,
        owners_inspected = scan.owners_inspected,
        duration_ms = u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX),
        "startup Active generation reaper completed"
    );
    summary
}

impl AppRuntime {
    pub(super) fn startup_auto_resume_ready_events(
        &mut self,
        bounds: WindowGeometry,
    ) -> Vec<OutboundEvent> {
        // Issue #4038 (AC-4 / AC-5): the notification center is a frontend
        // sink, so the bootstrap-time settle is recorded here, on the first
        // canvas-ready round trip, where a client is guaranteed to listen.
        let mut events = self.update_resume_notice_events();
        if self.pending_startup_auto_resume_sessions.is_empty() {
            events.extend(self.startup_pm_ensure_ready_events());
            return events;
        }

        let pending = std::mem::take(&mut self.pending_startup_auto_resume_sessions);
        let total = pending.len();
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
        events.extend(self.startup_pm_ensure_ready_events());
        events
    }

    /// SPEC-3431 FR-002: drain the bootstrap-queued PM ensure once the canvas
    /// is ready. Runs after the auto-resume drain so a resumable PM session
    /// (which the resume queue may already have restarted) is seen as live by
    /// the singleton gate instead of being spawned twice.
    fn startup_pm_ensure_ready_events(&mut self) -> Vec<OutboundEvent> {
        if self.pending_startup_pm_tabs.is_empty() {
            return Vec::new();
        }
        let tabs = std::mem::take(&mut self.pending_startup_pm_tabs);
        tracing::info!(tabs = tabs.len(), "PM ensure: canvas ready, draining queue");
        let mut events = Vec::new();
        for tab_id in tabs {
            events.extend(self.ensure_pm_agent_for_tab(
                &tab_id,
                crate::app_runtime::pm::PmEnsureTrigger::Automatic,
            ));
        }
        events
    }

    /// Spawn a single restored agent window from a persisted session, reusing
    /// the paused placeholder's geometry when present (Issue #2942). Shared by
    /// startup auto-resume and the Open Project restore path so both honor the
    /// "restore everything the user did not explicitly close" rule. Records the
    /// source session in `pending_auto_resume_sources` so the lifecycle handler
    /// retires the old session once the resumed window reports its own id.
    pub(super) fn spawn_restored_agent_session(
        &mut self,
        tab_id: &str,
        session: gwt_agent::Session,
        workspace_resume_context: Option<WorkspaceResumeContext>,
        fallback_geometry: WindowGeometry,
    ) -> Vec<OutboundEvent> {
        if self.restore_would_resurrect_a_foreign_pm(tab_id, &session) {
            return Vec::new();
        }
        if gwt::pm_registry::is_pm_worktree(&session.worktree_path) {
            if let Err(error) =
                gwt::pm_registry::refresh_pm_worktree_at_safe_boundary(&session.worktree_path)
            {
                tracing::warn!(
                    session_id = %session.id,
                    worktree = %session.worktree_path.display(),
                    %error,
                    "failed to refresh the resident PM before resume"
                );
                return Vec::new();
            }
        }
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

    /// Issue #3607 AC-3: refuse to restore a Session rooted in *another*
    /// project store's `pm/worktree`.
    ///
    /// The stopped store in the incident was not open in the app at all, yet
    /// its PM came back: the current store's `workspace.json` still held a
    /// window whose Session pointed at that store's PM worktree, and restore
    /// resolves a window purely by its recorded session id. Nothing on the path
    /// compared the two stores, and `auto_start` cannot close it because
    /// restore never consults a registration.
    ///
    /// Guarding the shared spawn primitive covers every restore entry point at
    /// once (startup auto-resume, Open Project restore, in-place restart) while
    /// leaving a store's own PM resume — which goes through the same primitive
    /// — untouched.
    fn restore_would_resurrect_a_foreign_pm(
        &self,
        tab_id: &str,
        session: &gwt_agent::Session,
    ) -> bool {
        let Some(tab) = self.tab(tab_id) else {
            return false;
        };
        let own_project_dir = gwt_core::paths::gwt_project_dir_for_repo_path(&tab.project_root);
        if !gwt::pm_registry::is_foreign_pm_worktree(&session.worktree_path, &own_project_dir) {
            return false;
        }
        tracing::warn!(
            tab_id,
            session_id = %session.id,
            worktree_path = %session.worktree_path.display(),
            own_project_dir = %own_project_dir.display(),
            "restore refused: the session belongs to another project store's PM worktree"
        );
        true
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
        let project_state_root = session
            .project_state_root
            .as_deref()
            .unwrap_or(&session.worktree_path);
        let workspace_resume_context = Some(workspace_resume_context_for_work_item(
            project_state_root,
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
                // Issue #3927 (SPEC #3340 FR-047): same restore fence as
                // startup auto-resume.
                let project_root = self
                    .tab(tab_id)
                    .map(|tab| tab.project_root.clone())
                    .unwrap_or_default();
                if let Some(reason) =
                    self.restore_admission_terminal_reason(&session, &project_root, Some(&combined))
                {
                    self.refuse_terminal_session_restore(tab_id, &session.id, reason);
                    events.push(self.workspace_state_broadcast());
                    continue;
                }
                let project_state_root = session
                    .project_state_root
                    .as_deref()
                    .unwrap_or(&session.worktree_path);
                let workspace_resume_context = Some(workspace_resume_context_for_work_item(
                    project_state_root,
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

    pub(super) fn remove_stale_paused_agent_window(
        &mut self,
        tab_id: &str,
        session_id: &str,
    ) -> Option<WindowGeometry> {
        let tab = self.tab_mut(tab_id)?;
        // SPEC-1921 Phase 65 (T337): stale placeholder removal must cover the
        // full Agent-family preset set (`Agent`, `Claude`, `Codex`), not just
        // the legacy `Agent` preset — otherwise a resumed Claude/Codex window
        // spawns next to its surviving placeholder and loses the restored
        // geometry.
        let stale = tab
            .workspace
            .persisted()
            .windows
            .iter()
            .find(|w| {
                crate::runtime_support::window_is_agent_pane(w)
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

    /// Issue #4038 (AC-3): the projects to record in the resume marker when an
    /// update apply begins. `update_drain` reports whether the Issue Monitor
    /// of that project holds new launches; the hold itself lands with #4037,
    /// so until then every project reports `false`.
    pub(crate) fn update_resume_projects(&self) -> Vec<gwt_core::update::UpdateResumeProject> {
        let mut seen = HashSet::new();
        self.tabs
            .iter()
            .filter(|tab| tab.kind == gwt::ProjectKind::Git)
            .filter_map(|tab| {
                let hash = gwt_core::paths::project_scope_hash(&tab.project_root).to_string();
                seen.insert(hash.clone())
                    .then_some(gwt_core::update::UpdateResumeProject {
                        hash,
                        update_drain: false,
                    })
            })
            .collect()
    }

    /// Issue #4038 (AC-4 / AC-5): consume `~/.gwt/update-resume/marker.json`.
    /// Success (running `to_version`) and version mismatch (#3807) both
    /// release the projects' `update_drain` holds and queue a notice; only
    /// success bypasses the auto-resume freshness gate. Idempotent: a settled
    /// marker is gone, a mismatched one is re-read with `attempt` bumped.
    fn settle_update_resume_marker_at_bootstrap(&mut self, now: chrono::DateTime<chrono::Utc>) {
        let Some(settlement) =
            gwt_core::update::settle_update_resume_marker(env!("CARGO_PKG_VERSION"), now)
        else {
            return;
        };
        let marker_projects = settlement
            .marker
            .projects
            .iter()
            .map(|project| project.hash.clone())
            .collect::<HashSet<_>>();
        self.update_drain_released_projects = settlement
            .marker
            .projects
            .iter()
            .filter(|project| project.update_drain)
            .map(|project| project.hash.clone())
            .collect();
        let applied = matches!(
            settlement.outcome,
            gwt_core::update::UpdateResumeOutcome::Applied
        );
        self.update_resume_tab_ids = if applied {
            self.tabs
                .iter()
                .filter(|tab| {
                    marker_projects
                        .contains(gwt_core::paths::project_scope_hash(&tab.project_root).as_str())
                })
                .map(|tab| tab.id.clone())
                .collect()
        } else {
            HashSet::new()
        };
        let level = if applied { "info" } else { "error" };
        tracing::info!(
            target: "gwt::startup",
            outcome = ?settlement.outcome,
            to_version = %settlement.marker.to_version,
            attempt = settlement.marker.attempt,
            released_projects = self.update_drain_released_projects.len(),
            resumed_tabs = self.update_resume_tab_ids.len(),
            "settled update resume marker"
        );
        self.pending_update_resume_notice = Some((level.to_string(), settlement.notice()));
    }

    fn update_resume_notice_events(&mut self) -> Vec<OutboundEvent> {
        self.pending_update_resume_notice
            .take()
            .map(|(level, message)| {
                vec![OutboundEvent::broadcast(
                    gwt::BackendEvent::IssueMonitorToast {
                        level,
                        message,
                        issue_number: None,
                    },
                )]
            })
            .unwrap_or_default()
    }

    pub(super) fn load_recovery_sessions(&self) -> Vec<gwt_agent::Session> {
        let Ok(entries) = std::fs::read_dir(&self.sessions_dir) else {
            return Vec::new();
        };
        entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("toml"))
            .filter_map(|path| {
                let session_id = path.file_stem()?.to_str()?;
                gwt_agent::update_session_if_changed(&self.sessions_dir, session_id, |session| {
                    if session.status != gwt_agent::AgentStatus::Interrupted
                        && session.worktree_path.exists()
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

    pub(crate) fn set_agent_capability_issuer(&mut self, issuer: AgentCapabilityIssuer) {
        self.agent_capability_issuer = Some(issuer);
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
