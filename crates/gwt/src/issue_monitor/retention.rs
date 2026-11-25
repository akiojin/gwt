//! What a project's durable prefs keep once an Issue is closed (Issue #7550).
//!
//! Every prefs transaction reads and writes the whole record, so a collection
//! that only grows makes each transaction slower without limit. Three
//! collections hold nothing a reader decides on once their Issue is retired,
//! and are pruned here: claim identities, claim diagnostics, and released
//! open-PR holds.
//!
//! What is deliberately kept:
//!
//! - The lifecycle history — closure, completion and settlement records.
//!   Dropping it changes what a later reopen does.
//! - Runtime stop requests. One may still fence an Active execution hold,
//!   which these prefs cannot see.
//! - Runtime observations. A live host leaves its empty observation's
//!   timestamp untouched, so an age rule would drop and re-create it.

use super::*;

impl IssueMonitorEffectPayload {
    pub(crate) fn issue_number(&self) -> u64 {
        match self {
            Self::AcquireClaim { issue_number, .. }
            | Self::RenewClaim { issue_number, .. }
            | Self::ReleaseClaim { issue_number, .. }
            | Self::RemoveQueueLabel { issue_number }
            | Self::ArmAutoMerge { issue_number, .. }
            | Self::RerunFailedCi { issue_number, .. }
            | Self::DisarmAutoMerge { issue_number, .. }
            | Self::SettleMergedIssue { issue_number, .. } => *issue_number,
        }
    }
}

impl IssueMonitorPrefs {
    /// Issues that are closed and that no queue, launch, failure, effect or
    /// observed runtime still names.
    fn retired_issue_numbers(&self) -> BTreeSet<u64> {
        let mut retired: BTreeSet<u64> = self
            .closure_records
            .iter()
            .filter(|record| record.state == IssueClosureState::Closed)
            .map(|record| record.issue_number)
            .collect();
        if retired.is_empty() {
            return retired;
        }
        let live = self
            .priority_order
            .iter()
            .copied()
            .chain(self.priority_launch_now)
            .chain(self.launched_issues.iter().map(|issue| issue.issue_number))
            .chain(self.launching_issues.iter().map(|issue| issue.issue_number))
            .chain(self.launched_claims.keys().copied())
            .chain(self.launch_bindings.values().copied())
            .chain(self.launch_confirmations.keys().copied())
            .chain(self.missing_window_observations.keys().copied())
            .chain(
                self.pending_launch_deliveries
                    .iter()
                    .map(|delivery| delivery.issue_number),
            )
            .chain(self.queued_launch_session_strategies.keys().copied())
            .chain(self.failed_issues.iter().map(|failed| failed.issue_number))
            .chain(
                self.autonomous_records
                    .iter()
                    .map(|record| record.issue_number),
            )
            .chain(
                self.autonomous_handoffs
                    .iter()
                    .map(|handoff| handoff.issue_number),
            )
            .chain(
                self.pending_effects
                    .iter()
                    .map(|effect| effect.payload.issue_number()),
            )
            .chain(
                self.terminal_queues
                    .values()
                    .flat_map(|queue| queue.entries.iter().map(|entry| entry.number)),
            )
            .chain(
                self.monitor_runtime_counts
                    .values()
                    .flat_map(|observation| {
                        observation
                            .counts
                            .keys()
                            .chain(observation.windows.values())
                            .copied()
                    }),
            );
        for issue_number in live {
            retired.remove(&issue_number);
        }
        retired
    }

    /// Drop what no reader decides on any more. Returns whether anything went.
    ///
    /// The rule reads only these prefs, so every writer that saves them drops
    /// the same entries, and a process that still holds one in memory cannot
    /// publish it again.
    pub(crate) fn enforce_retention(&mut self) -> bool {
        let retired = self.retired_issue_numbers();
        if retired.is_empty() {
            return false;
        }
        let before = (
            self.claim_identities.len(),
            self.claim_diagnostics.len(),
            self.open_pr_observations.len(),
        );
        self.claim_identities
            .retain(|identity| !retired.contains(&identity.issue_number));
        self.claim_diagnostics
            .retain(|issue_number, _| !retired.contains(issue_number));
        // A hold is a decision; only the release of one is history.
        self.open_pr_observations
            .retain(|issue_number, observation| {
                observation.hold.is_some() || !retired.contains(issue_number)
            });
        before
            != (
                self.claim_identities.len(),
                self.claim_diagnostics.len(),
                self.open_pr_observations.len(),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: &str = "2026-11-17T12:00:00Z";

    fn closure(issue_number: u64, state: IssueClosureState) -> IssueClosureRecord {
        IssueClosureRecord {
            issue_number,
            generation: 1,
            state,
            evidence: Default::default(),
            issue_updated_at: Some("2026-11-01T00:00:00Z".into()),
            reopened_after_close: false,
        }
    }

    fn identity(issue_number: u64) -> IssueMonitorClaimIdentity {
        IssueMonitorClaimIdentity {
            issue_number,
            claim_id: format!("gwt-queue:{issue_number}:claim"),
            owner: "host:user:1".into(),
        }
    }

    fn diagnostics(issue_number: u64) -> IssueMonitorClaimDiagnostics {
        IssueMonitorClaimDiagnostics {
            state: IssueMonitorClaimHealth::Active,
            claim_id: Some(format!("gwt-queue:{issue_number}:claim")),
            owner: Some("host:user:1".into()),
            last_attempt_at: Some("2026-11-01T00:00:00Z".into()),
            last_success_at: Some("2026-11-01T00:00:00Z".into()),
            ..Default::default()
        }
    }

    fn release(observed_at: &str) -> IssueMonitorOpenPrObservation {
        IssueMonitorOpenPrObservation {
            observed_at: observed_at.into(),
            hold: None,
        }
    }

    fn runtime(host_pid: u32, issues: &[u64], observed_at: &str) -> IssueMonitorRuntimeCounts {
        IssueMonitorRuntimeCounts {
            host_pid,
            host_started_at: 1,
            counts: issues.iter().map(|issue| (*issue, 1)).collect(),
            windows: BTreeMap::new(),
            observed_at: observed_at.into(),
        }
    }

    /// Issue 1 is retired; 2 is closed but still launched; 3 is closed with a
    /// pending claim effect; 4 is reopened; 5 was never closed.
    fn boundary_prefs() -> IssueMonitorPrefs {
        let mut prefs = IssueMonitorPrefs {
            enabled: true,
            closure_records: vec![
                closure(1, IssueClosureState::Closed),
                closure(2, IssueClosureState::Closed),
                closure(3, IssueClosureState::Closed),
                closure(4, IssueClosureState::Reopened),
            ],
            launched_issues: vec![IssueMonitorLaunchedIssue {
                issue_number: 2,
                window_id: "window-2".into(),
            }],
            pending_effects: vec![PendingIssueMonitorEffect {
                effect_id: "release:3".into(),
                authority_epoch: 1,
                attempt: 0,
                state: IssueMonitorEffectState::Prepared,
                payload: IssueMonitorEffectPayload::RemoveQueueLabel { issue_number: 3 },
            }],
            ..IssueMonitorPrefs::default()
        };
        for issue_number in 1..=5 {
            prefs.claim_identities.push(identity(issue_number));
            prefs
                .claim_diagnostics
                .insert(issue_number, diagnostics(issue_number));
            prefs
                .open_pr_observations
                .insert(issue_number, release("2026-11-01T00:00:00Z"));
        }
        prefs
    }

    fn state(prefs: &IssueMonitorPrefs) -> IssueMonitorState {
        let config = IssueMonitorConfig {
            enabled: true,
            ..IssueMonitorConfig::default()
        };
        IssueMonitorState::with_prefs(config, prefs.clone())
    }

    #[test]
    fn a_retired_issue_loses_its_claim_and_release_history_and_its_neighbours_keep_theirs() {
        let mut prefs = boundary_prefs();

        assert!(prefs.enforce_retention());

        let kept: Vec<u64> = vec![2, 3, 4, 5];
        assert_eq!(
            prefs
                .claim_identities
                .iter()
                .map(|identity| identity.issue_number)
                .collect::<Vec<_>>(),
            kept
        );
        assert_eq!(
            prefs.claim_diagnostics.keys().copied().collect::<Vec<_>>(),
            kept
        );
        assert_eq!(
            prefs
                .open_pr_observations
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            kept
        );
        // The lifecycle history that a later reopen reads stays whole.
        assert_eq!(prefs.closure_records, boundary_prefs().closure_records);
        assert!(!prefs.enforce_retention(), "the rule is idempotent");
    }

    #[test]
    fn readers_decide_the_same_before_and_after_retention() {
        let before = boundary_prefs();
        let mut after = before.clone();
        after.enforce_retention();
        let (before, after) = (state(&before), state(&after));

        // Renewal skips a closed Issue whether or not its identity is kept.
        assert_eq!(
            before.claim_renewal_candidates(NOW),
            after.claim_renewal_candidates(NOW)
        );
        assert_eq!(
            after
                .claim_renewal_candidates(NOW)
                .iter()
                .map(|identity| identity.issue_number)
                .collect::<Vec<_>>(),
            vec![4, 5]
        );
        for issue_number in 1..=5 {
            assert_eq!(
                before.open_pr_launch_held(issue_number),
                after.open_pr_launch_held(issue_number)
            );
            assert_eq!(
                before.issue_is_closed(issue_number),
                after.issue_is_closed(issue_number)
            );
        }
        assert_eq!(before.active_issue_numbers(), after.active_issue_numbers());
    }

    #[test]
    fn a_hold_on_a_retired_issue_is_kept() {
        let mut prefs = boundary_prefs();
        prefs.open_pr_observations.insert(
            1,
            IssueMonitorOpenPrObservation {
                observed_at: "2026-11-01T00:00:00Z".into(),
                hold: Some(IssueMonitorOpenPrHold {
                    pr_number: 99,
                    lifecycle: "open".into(),
                }),
            },
        );

        prefs.enforce_retention();

        assert!(state(&prefs).open_pr_launch_held(1));
        assert!(!prefs.claim_diagnostics.contains_key(&1));
    }

    #[test]
    fn runtime_observations_and_stop_requests_are_never_dropped() {
        let mut prefs = boundary_prefs();
        for (key, observation) in [
            ("1:1", runtime(1, &[], "2026-10-01T00:00:00Z")),
            ("2:1", runtime(2, &[], NOW)),
        ] {
            prefs.monitor_runtime_counts.insert(key.into(), observation);
        }
        let observations = prefs.monitor_runtime_counts.clone();

        assert!(prefs.enforce_retention());

        assert_eq!(prefs.monitor_runtime_counts, observations);
    }

    #[test]
    fn a_runtime_still_observed_for_a_closed_issue_keeps_its_claim() {
        let mut prefs = boundary_prefs();
        prefs
            .monitor_runtime_counts
            .insert("9:1".into(), runtime(9, &[1], NOW));

        assert!(!prefs.enforce_retention());
    }

    /// A process that still holds a retired entry in memory saves prefs
    /// without it, so the entry cannot come back through any writer.
    #[test]
    fn a_stale_holder_cannot_publish_a_retired_entry_again() {
        let stale = state(&boundary_prefs());
        assert!(stale.claim_identities.contains_key(&1));

        let published = stale.prefs();

        assert!(!published
            .claim_identities
            .iter()
            .any(|identity| identity.issue_number == 1));
        assert!(!published.claim_diagnostics.contains_key(&1));
        assert!(!published.open_pr_observations.contains_key(&1));
        assert!(published.claim_diagnostics.contains_key(&2));
    }
}
