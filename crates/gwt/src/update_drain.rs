//! Issue #3906: pure decisions for the drain-and-apply self-update flow.
//!
//! Everything here is side-effect free so the Issue Monitor / update wiring
//! can be table-tested without a daemon, a PTY, or a lease file:
//!
//! - AC-8: [`update_quiescence`] decides whether the host is quiet enough to
//!   restart gwt, and [`UpdateQuiescenceTracker`] enforces the two-tick rule.
//! - AC-9: [`update_drain_notice_due`] paces the "still draining" warning
//!   without ever killing an agent.
//! - AC-5: [`auto_apply_blocked_by_previous_failure`] keeps a failed apply
//!   from being retried automatically.

use serde::{Deserialize, Serialize};
use std::fmt;

use crate::persistence::WindowState;

/// AC-8: how many consecutive clear ticks make the host quiescent. One tick
/// can observe a pane between "claim acquired" and "PTY spawned"; two in a
/// row cannot.
pub const QUIESCENCE_REQUIRED_TICKS: u32 = 2;

/// AC-9: default for `AutonomousTuning::update_drain_notify_after_secs`.
pub const DEFAULT_UPDATE_DRAIN_NOTIFY_AFTER_SECS: u64 = 1800;

/// One agent pane as the drain sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneObservation {
    pub window_id: String,
    /// Human-readable identity for the blocking list (work name / title).
    pub label: String,
    pub state: WindowState,
    /// The always-on PM pane: restored after the restart, never a blocker.
    pub resident_pm: bool,
}

/// Everything [`update_quiescence`] looks at, collected by the caller.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpdateQuiescenceSnapshot {
    pub panes: Vec<PaneObservation>,
    /// Issue numbers with a pending `AcquireClaim` effect.
    pub pending_acquire_claims: Vec<u64>,
    /// Tracked worktrees whose Execution Control Record is `Active`.
    pub active_executions: Vec<String>,
    /// Verification leases held by this gwt process tree.
    pub held_verification_leases: Vec<String>,
}

/// One reason the update cannot be applied yet (AC-8 / AC-12 `blocking[]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UpdateBlocker {
    ActivePane {
        window_id: String,
        label: String,
        state: WindowState,
    },
    PendingAcquireClaim {
        issue_number: u64,
    },
    ActiveExecution {
        worktree: String,
    },
    HeldVerificationLease {
        lease_id: String,
    },
}

impl fmt::Display for UpdateBlocker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ActivePane { label, state, .. } => {
                let state = match state {
                    WindowState::Starting => "starting",
                    _ => "running",
                };
                write!(f, "pane {label} ({state})")
            }
            Self::PendingAcquireClaim { issue_number } => {
                write!(f, "pending claim for #{issue_number}")
            }
            Self::ActiveExecution { worktree } => write!(f, "execution active in {worktree}"),
            Self::HeldVerificationLease { lease_id } => {
                write!(f, "verification lease {lease_id}")
            }
        }
    }
}

/// AC-8: the host is quiescent when no non-PM pane is Running / Starting, no
/// `AcquireClaim` effect is pending, no tracked worktree has an Active
/// execution, and this process holds no verification lease. Idle / Waiting
/// panes and the resident PM pane never block. Returns every blocker so the
/// operator can see the whole list, in snapshot order.
pub fn update_quiescence(snapshot: &UpdateQuiescenceSnapshot) -> Result<(), Vec<UpdateBlocker>> {
    let mut blockers = Vec::new();
    for pane in &snapshot.panes {
        if pane.resident_pm {
            continue;
        }
        if matches!(pane.state, WindowState::Running | WindowState::Starting) {
            blockers.push(UpdateBlocker::ActivePane {
                window_id: pane.window_id.clone(),
                label: pane.label.clone(),
                state: pane.state,
            });
        }
    }
    blockers.extend(snapshot.pending_acquire_claims.iter().map(|issue_number| {
        UpdateBlocker::PendingAcquireClaim {
            issue_number: *issue_number,
        }
    }));
    blockers.extend(snapshot.active_executions.iter().map(|worktree| {
        UpdateBlocker::ActiveExecution {
            worktree: worktree.clone(),
        }
    }));
    blockers.extend(snapshot.held_verification_leases.iter().map(|lease_id| {
        UpdateBlocker::HeldVerificationLease {
            lease_id: lease_id.clone(),
        }
    }));
    if blockers.is_empty() {
        Ok(())
    } else {
        Err(blockers)
    }
}

/// What one drain tick concluded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateQuiescenceVerdict {
    /// [`QUIESCENCE_REQUIRED_TICKS`] consecutive clear ticks: apply now.
    Quiesced,
    /// Clear, but not for long enough yet.
    Settling { clear_ticks: u32 },
    /// Something still blocks; the streak restarted.
    Blocked(Vec<UpdateBlocker>),
}

/// AC-8: enforces the consecutive-tick rule over [`update_quiescence`]
/// results. Any blocker resets the streak.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpdateQuiescenceTracker {
    clear_ticks: u32,
}

impl UpdateQuiescenceTracker {
    pub fn observe(&mut self, outcome: Result<(), Vec<UpdateBlocker>>) -> UpdateQuiescenceVerdict {
        match outcome {
            Ok(()) => {
                self.clear_ticks = self.clear_ticks.saturating_add(1);
                if self.clear_ticks >= QUIESCENCE_REQUIRED_TICKS {
                    UpdateQuiescenceVerdict::Quiesced
                } else {
                    UpdateQuiescenceVerdict::Settling {
                        clear_ticks: self.clear_ticks,
                    }
                }
            }
            Err(blockers) => {
                self.clear_ticks = 0;
                UpdateQuiescenceVerdict::Blocked(blockers)
            }
        }
    }

    pub fn reset(&mut self) {
        self.clear_ticks = 0;
    }
}

/// AC-9: whether the "still draining" warning is due. The first notice fires
/// once the drain has lasted `notify_after_secs`; later notices repeat at the
/// same cadence measured from the previous notice. A zero interval never
/// notifies instead of firing every tick. Agents are never killed by this.
pub fn update_drain_notice_due(
    drained_for_secs: u64,
    last_notice_at_secs: Option<u64>,
    notify_after_secs: u64,
) -> bool {
    if notify_after_secs == 0 {
        return false;
    }
    match last_notice_at_secs {
        None => drained_for_secs >= notify_after_secs,
        Some(last) => drained_for_secs.saturating_sub(last) >= notify_after_secs,
    }
}

/// AC-5: a version whose apply already failed is shown to the user, never
/// retried automatically. Leading `v` and whitespace are ignored so the
/// manifest / apply-result spellings compare equal.
pub fn auto_apply_blocked_by_previous_failure(
    last_failed_version: Option<&str>,
    target_version: &str,
) -> bool {
    let normalize = |version: &str| version.trim().trim_start_matches('v').to_string();
    match last_failed_version.map(normalize) {
        Some(failed) if !failed.is_empty() => failed == normalize(target_version),
        _ => false,
    }
}

/// AC-7: default seconds between the "applying soon" notice and the apply,
/// during which the user can cancel from the update banner.
pub const DEFAULT_AUTO_APPLY_GRACE_SECS: u64 = 60;

/// One drain tick as [`UpdateAutoApplyPlanner::tick`] sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateAutoApplyObservation<'a> {
    /// The staged version the drain is about.
    pub version: &'a str,
    /// Wall clock of this tick, in seconds.
    pub now_secs: u64,
    /// Seconds since the drain was raised.
    pub drained_for_secs: u64,
    /// [`update_quiescence`] for this tick.
    pub outcome: Result<(), Vec<UpdateBlocker>>,
    /// `AutonomousTuning::update_drain_notify_after_secs`.
    pub notify_after_secs: u64,
    /// Cancel grace between the notice and the apply (AC-7).
    pub grace_secs: u64,
}

/// What the runtime should do after one drain tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateAutoApplyStep {
    /// Nothing to say: blocked within the notice cadence, settling, waiting
    /// out the grace, or the version was cancelled.
    Idle,
    /// AC-9: still blocked after `notify_after_secs`; warn with the blockers.
    StillDraining(Vec<UpdateBlocker>),
    /// AC-7: the host went quiet; the apply fires at `apply_at_secs` unless
    /// cancelled. Announced once.
    Scheduled { apply_at_secs: u64 },
    /// A blocker appeared during the grace; the scheduled apply is withdrawn.
    Postponed(Vec<UpdateBlocker>),
    /// AC-2: the grace elapsed on a quiet host — apply through the graceful
    /// path now.
    Apply,
}

/// AC-2 / AC-7 / AC-8 / AC-9: the pure state of the automatic apply — the
/// two-tick quiescence streak, the long-drain notice cadence, the cancel
/// grace, and the user's cancellation. The runtime feeds it one observation
/// per tick and acts on the returned step; it never kills an agent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpdateAutoApplyPlanner {
    tracker: UpdateQuiescenceTracker,
    last_notice_at_secs: Option<u64>,
    scheduled_at_secs: Option<u64>,
    cancelled_version: Option<String>,
}

impl UpdateAutoApplyPlanner {
    pub fn tick(&mut self, observation: UpdateAutoApplyObservation<'_>) -> UpdateAutoApplyStep {
        if self.is_cancelled(observation.version) {
            return UpdateAutoApplyStep::Idle;
        }
        match self.tracker.observe(observation.outcome) {
            UpdateQuiescenceVerdict::Blocked(blockers) => {
                if self.scheduled_at_secs.take().is_some() {
                    return UpdateAutoApplyStep::Postponed(blockers);
                }
                if update_drain_notice_due(
                    observation.drained_for_secs,
                    self.last_notice_at_secs,
                    observation.notify_after_secs,
                ) {
                    self.last_notice_at_secs = Some(observation.drained_for_secs);
                    return UpdateAutoApplyStep::StillDraining(blockers);
                }
                UpdateAutoApplyStep::Idle
            }
            UpdateQuiescenceVerdict::Settling { .. } => UpdateAutoApplyStep::Idle,
            UpdateQuiescenceVerdict::Quiesced => match self.scheduled_at_secs {
                None => {
                    self.scheduled_at_secs = Some(observation.now_secs);
                    UpdateAutoApplyStep::Scheduled {
                        apply_at_secs: observation.now_secs.saturating_add(observation.grace_secs),
                    }
                }
                Some(scheduled_at)
                    if observation.now_secs
                        >= scheduled_at.saturating_add(observation.grace_secs) =>
                {
                    UpdateAutoApplyStep::Apply
                }
                Some(_) => UpdateAutoApplyStep::Idle,
            },
        }
    }

    /// AC-7 / AC-13: the user cancelled the automatic apply of `version`;
    /// the tick never reschedules it. A newer staged version starts fresh.
    pub fn cancel(&mut self, version: &str) {
        self.reset();
        self.cancelled_version = Some(version.to_string());
    }

    pub fn is_cancelled(&self, version: &str) -> bool {
        self.cancelled_version.as_deref() == Some(version)
    }

    /// The drain is gone (cleared, applied, or never raised): forget the
    /// streak, the notice cadence and the grace. Keeps the cancellation.
    pub fn reset(&mut self) {
        self.tracker.reset();
        self.last_notice_at_secs = None;
        self.scheduled_at_secs = None;
    }
}

/// AC-5 / AC-6: why a staged update must not be applied automatically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateAutoApplyRefusal {
    /// The install location is not writable by this user (administrator
    /// install): keep the manual button, which asks the OS for elevation.
    RequiresElevation,
    /// This version's apply already failed once; never retry unattended.
    PreviousFailure { version: String },
}

impl UpdateAutoApplyRefusal {
    /// The notification-center line that names the manual fallback.
    pub fn notice(&self, version: &str) -> String {
        match self {
            Self::RequiresElevation => format!(
                "Update v{version} needs elevated permissions to install — apply it manually from the update button."
            ),
            Self::PreviousFailure { .. } => format!(
                "Update v{version} was not applied automatically because its previous apply failed — retry manually from the update button."
            ),
        }
    }
}

/// AC-5 / AC-6: decide whether the staged `target_version` may be applied
/// unattended. `last_failed_version` is the version of the last recorded
/// failed apply, if any.
pub fn auto_apply_refusal(
    requires_elevation: bool,
    last_failed_version: Option<&str>,
    target_version: &str,
) -> Option<UpdateAutoApplyRefusal> {
    if requires_elevation {
        return Some(UpdateAutoApplyRefusal::RequiresElevation);
    }
    if auto_apply_blocked_by_previous_failure(last_failed_version, target_version) {
        return Some(UpdateAutoApplyRefusal::PreviousFailure {
            version: target_version.trim().trim_start_matches('v').to_string(),
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::WindowState;

    fn pane(window_id: &str, state: WindowState) -> PaneObservation {
        PaneObservation {
            window_id: window_id.to_string(),
            label: format!("work/{window_id}"),
            state,
            resident_pm: false,
        }
    }

    #[test]
    fn empty_snapshot_is_quiescent() {
        assert_eq!(
            update_quiescence(&UpdateQuiescenceSnapshot::default()),
            Ok(())
        );
    }

    #[test]
    fn idle_waiting_stopped_and_error_panes_do_not_block() {
        // AC-8: Idle / Waiting panes never hold the update back; neither do
        // panes that already exited.
        let snapshot = UpdateQuiescenceSnapshot {
            panes: vec![
                pane("idle", WindowState::Idle),
                pane("waiting", WindowState::Waiting),
                pane("stopped", WindowState::Stopped),
                pane("error", WindowState::Error),
            ],
            ..Default::default()
        };
        assert_eq!(update_quiescence(&snapshot), Ok(()));
    }

    #[test]
    fn running_and_starting_panes_block_with_their_identity() {
        let snapshot = UpdateQuiescenceSnapshot {
            panes: vec![
                pane("a", WindowState::Running),
                pane("b", WindowState::Starting),
                pane("c", WindowState::Idle),
            ],
            ..Default::default()
        };
        assert_eq!(
            update_quiescence(&snapshot),
            Err(vec![
                UpdateBlocker::ActivePane {
                    window_id: "a".to_string(),
                    label: "work/a".to_string(),
                    state: WindowState::Running,
                },
                UpdateBlocker::ActivePane {
                    window_id: "b".to_string(),
                    label: "work/b".to_string(),
                    state: WindowState::Starting,
                },
            ])
        );
    }

    #[test]
    fn resident_pm_pane_never_blocks_even_while_running() {
        // AC-8: the always-on PM pane is a Claude Code process that is
        // Running by design; it is restored after the restart (#4038) and
        // must not keep the drain open forever.
        let snapshot = UpdateQuiescenceSnapshot {
            panes: vec![PaneObservation {
                resident_pm: true,
                ..pane("pm", WindowState::Running)
            }],
            ..Default::default()
        };
        assert_eq!(update_quiescence(&snapshot), Ok(()));
    }

    #[test]
    fn pending_acquire_claims_active_executions_and_held_leases_block() {
        let snapshot = UpdateQuiescenceSnapshot {
            panes: Vec::new(),
            pending_acquire_claims: vec![3906],
            active_executions: vec!["/w/issue-3906".to_string()],
            held_verification_leases: vec!["repo--verification--abc".to_string()],
        };
        assert_eq!(
            update_quiescence(&snapshot),
            Err(vec![
                UpdateBlocker::PendingAcquireClaim { issue_number: 3906 },
                UpdateBlocker::ActiveExecution {
                    worktree: "/w/issue-3906".to_string(),
                },
                UpdateBlocker::HeldVerificationLease {
                    lease_id: "repo--verification--abc".to_string(),
                },
            ])
        );
    }

    #[test]
    fn blockers_render_for_the_blocking_list() {
        // AC-12: `update_drain.blocking[]` and the notification body reuse
        // the Display form, so it must name the thing an operator can act on.
        let cases: Vec<(UpdateBlocker, &str)> = vec![
            (
                UpdateBlocker::ActivePane {
                    window_id: "w1".to_string(),
                    label: "work/issue-1".to_string(),
                    state: WindowState::Running,
                },
                "pane work/issue-1 (running)",
            ),
            (
                UpdateBlocker::ActivePane {
                    window_id: "w2".to_string(),
                    label: "work/issue-2".to_string(),
                    state: WindowState::Starting,
                },
                "pane work/issue-2 (starting)",
            ),
            (
                UpdateBlocker::PendingAcquireClaim { issue_number: 42 },
                "pending claim for #42",
            ),
            (
                UpdateBlocker::ActiveExecution {
                    worktree: "/w/issue-42".to_string(),
                },
                "execution active in /w/issue-42",
            ),
            (
                UpdateBlocker::HeldVerificationLease {
                    lease_id: "lease-1".to_string(),
                },
                "verification lease lease-1",
            ),
        ];
        for (blocker, expected) in cases {
            assert_eq!(blocker.to_string(), expected);
        }
    }

    #[test]
    fn blockers_serialize_with_a_kind_tag() {
        let json = serde_json::to_value(UpdateBlocker::PendingAcquireClaim { issue_number: 7 })
            .expect("serialize");
        assert_eq!(json["kind"], "pending_acquire_claim");
        assert_eq!(json["issue_number"], 7);
    }

    #[test]
    fn tracker_requires_two_consecutive_clear_ticks() {
        // AC-8: one clear tick is `Settling`, the second consecutive one is
        // `Quiesced`; the required count is the documented constant.
        assert_eq!(QUIESCENCE_REQUIRED_TICKS, 2);
        let mut tracker = UpdateQuiescenceTracker::default();
        assert_eq!(
            tracker.observe(Ok(())),
            UpdateQuiescenceVerdict::Settling { clear_ticks: 1 }
        );
        assert_eq!(tracker.observe(Ok(())), UpdateQuiescenceVerdict::Quiesced);
        // Staying quiet keeps reporting quiesced.
        assert_eq!(tracker.observe(Ok(())), UpdateQuiescenceVerdict::Quiesced);
    }

    #[test]
    fn tracker_resets_the_streak_on_any_blocker() {
        let blocker = UpdateBlocker::PendingAcquireClaim { issue_number: 1 };
        let mut tracker = UpdateQuiescenceTracker::default();
        assert_eq!(
            tracker.observe(Ok(())),
            UpdateQuiescenceVerdict::Settling { clear_ticks: 1 }
        );
        assert_eq!(
            tracker.observe(Err(vec![blocker.clone()])),
            UpdateQuiescenceVerdict::Blocked(vec![blocker])
        );
        // The streak restarts from one; a single clear tick after a blocker
        // is not enough.
        assert_eq!(
            tracker.observe(Ok(())),
            UpdateQuiescenceVerdict::Settling { clear_ticks: 1 }
        );
        assert_eq!(tracker.observe(Ok(())), UpdateQuiescenceVerdict::Quiesced);
    }

    #[test]
    fn drain_notice_is_due_at_the_threshold_and_then_every_interval() {
        // AC-9: default 1800 s; warn once the drain has lasted that long and
        // re-warn at the same cadence, never earlier.
        assert_eq!(DEFAULT_UPDATE_DRAIN_NOTIFY_AFTER_SECS, 1800);
        let cases: Vec<(u64, Option<u64>, u64, bool)> = vec![
            // (drained_for, last_notice_at, notify_after, expected)
            (0, None, 1800, false),
            (1799, None, 1800, false),
            (1800, None, 1800, true),
            (5000, None, 1800, true),
            (1800, Some(1800), 1800, false),
            (3599, Some(1800), 1800, false),
            (3600, Some(1800), 1800, true),
            (7300, Some(3600), 1800, true),
            // A zero interval degenerates to "never" instead of spamming.
            (10, None, 0, false),
        ];
        for (drained_for, last_notice_at, notify_after, expected) in cases {
            assert_eq!(
                update_drain_notice_due(drained_for, last_notice_at, notify_after),
                expected,
                "drained_for={drained_for} last={last_notice_at:?} after={notify_after}"
            );
        }
    }

    #[test]
    fn auto_apply_is_blocked_only_for_the_version_that_already_failed() {
        // AC-5: a failed apply is surfaced, never retried automatically; a
        // newer release is allowed again.
        let cases: Vec<(Option<&str>, &str, bool)> = vec![
            (None, "9.90.0", false),
            (Some("9.90.0"), "9.90.0", true),
            (Some("v9.90.0"), "9.90.0", true),
            (Some("9.90.0"), "v9.90.0", true),
            (Some("9.90.0"), "9.91.0", false),
            (Some(""), "9.90.0", false),
        ];
        for (failed, target, expected) in cases {
            assert_eq!(
                auto_apply_blocked_by_previous_failure(failed, target),
                expected,
                "failed={failed:?} target={target}"
            );
        }
    }

    fn observation<'a>(
        version: &'a str,
        now_secs: u64,
        drained_for_secs: u64,
        outcome: Result<(), Vec<UpdateBlocker>>,
    ) -> UpdateAutoApplyObservation<'a> {
        UpdateAutoApplyObservation {
            version,
            now_secs,
            drained_for_secs,
            outcome,
            notify_after_secs: 1800,
            grace_secs: 60,
        }
    }

    fn blocked() -> Result<(), Vec<UpdateBlocker>> {
        Err(vec![UpdateBlocker::PendingAcquireClaim { issue_number: 7 }])
    }

    #[test]
    fn planner_applies_only_after_two_clear_ticks_and_the_cancel_grace() {
        // AC-2 / AC-7 / AC-8: one clear tick settles, the second schedules the
        // apply and announces the grace, and the apply itself fires only once
        // the grace has elapsed on a still-quiet host.
        let mut planner = UpdateAutoApplyPlanner::default();
        assert_eq!(
            planner.tick(observation("9.99.0", 1_000, 30, Ok(()))),
            UpdateAutoApplyStep::Idle
        );
        assert_eq!(
            planner.tick(observation("9.99.0", 1_015, 45, Ok(()))),
            UpdateAutoApplyStep::Scheduled {
                apply_at_secs: 1_075
            }
        );
        assert_eq!(
            planner.tick(observation("9.99.0", 1_030, 60, Ok(()))),
            UpdateAutoApplyStep::Idle,
            "the grace is announced once, then waited out silently"
        );
        assert_eq!(
            planner.tick(observation("9.99.0", 1_074, 104, Ok(()))),
            UpdateAutoApplyStep::Idle
        );
        assert_eq!(
            planner.tick(observation("9.99.0", 1_075, 105, Ok(()))),
            UpdateAutoApplyStep::Apply
        );
    }

    #[test]
    fn planner_postpones_when_a_blocker_appears_during_the_grace() {
        // AC-8: the two-tick streak restarts on any blocker, and a scheduled
        // apply is withdrawn instead of firing over a busy host.
        let mut planner = UpdateAutoApplyPlanner::default();
        planner.tick(observation("9.99.0", 1_000, 30, Ok(())));
        planner.tick(observation("9.99.0", 1_015, 45, Ok(())));
        assert_eq!(
            planner.tick(observation("9.99.0", 1_030, 60, blocked())),
            UpdateAutoApplyStep::Postponed(vec![UpdateBlocker::PendingAcquireClaim {
                issue_number: 7
            }])
        );
        assert_eq!(
            planner.tick(observation("9.99.0", 1_045, 75, Ok(()))),
            UpdateAutoApplyStep::Idle,
            "one clear tick after a blocker is not enough"
        );
        assert_eq!(
            planner.tick(observation("9.99.0", 1_060, 90, Ok(()))),
            UpdateAutoApplyStep::Scheduled {
                apply_at_secs: 1_120
            }
        );
    }

    #[test]
    fn planner_notices_a_long_drain_at_the_configured_cadence_without_killing() {
        // AC-9: a blocked host is reported once per `notify_after_secs`, with
        // the blockers, and the planner never returns Apply while blocked.
        let mut planner = UpdateAutoApplyPlanner::default();
        assert_eq!(
            planner.tick(observation("9.99.0", 1_000, 1_799, blocked())),
            UpdateAutoApplyStep::Idle
        );
        assert_eq!(
            planner.tick(observation("9.99.0", 1_001, 1_800, blocked())),
            UpdateAutoApplyStep::StillDraining(vec![UpdateBlocker::PendingAcquireClaim {
                issue_number: 7
            }])
        );
        assert_eq!(
            planner.tick(observation("9.99.0", 1_002, 3_599, blocked())),
            UpdateAutoApplyStep::Idle
        );
        assert_eq!(
            planner.tick(observation("9.99.0", 1_003, 3_600, blocked())),
            UpdateAutoApplyStep::StillDraining(vec![UpdateBlocker::PendingAcquireClaim {
                issue_number: 7
            }])
        );
    }

    #[test]
    fn planner_stays_idle_for_a_cancelled_version_until_reset() {
        // AC-7: after the user cancels, the same staged version is never
        // rescheduled by the tick; a newer version is.
        let mut planner = UpdateAutoApplyPlanner::default();
        planner.tick(observation("9.99.0", 1_000, 30, Ok(())));
        planner.tick(observation("9.99.0", 1_015, 45, Ok(())));
        planner.cancel("9.99.0");
        assert!(planner.is_cancelled("9.99.0"));
        for now in [1_030, 1_045, 1_200] {
            assert_eq!(
                planner.tick(observation("9.99.0", now, now - 970, Ok(()))),
                UpdateAutoApplyStep::Idle
            );
        }
        assert_eq!(
            planner.tick(observation("9.99.1", 1_215, 15, Ok(()))),
            UpdateAutoApplyStep::Idle
        );
        assert_eq!(
            planner.tick(observation("9.99.1", 1_230, 30, Ok(()))),
            UpdateAutoApplyStep::Scheduled {
                apply_at_secs: 1_290
            }
        );
    }

    #[test]
    fn auto_apply_refusal_falls_back_to_manual_for_elevation_and_previous_failure() {
        // AC-5 / AC-6: an install that needs elevation and a version whose
        // apply already failed both keep the manual button path.
        assert_eq!(auto_apply_refusal(false, None, "9.99.0"), None);
        assert_eq!(
            auto_apply_refusal(true, None, "9.99.0"),
            Some(UpdateAutoApplyRefusal::RequiresElevation)
        );
        assert_eq!(
            auto_apply_refusal(false, Some("v9.99.0"), "9.99.0"),
            Some(UpdateAutoApplyRefusal::PreviousFailure {
                version: "9.99.0".to_string()
            })
        );
        assert_eq!(auto_apply_refusal(false, Some("9.98.0"), "9.99.0"), None);
        assert!(UpdateAutoApplyRefusal::RequiresElevation
            .notice("9.99.0")
            .contains("manually"));
    }
}
