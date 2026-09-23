//! Issue #4366: how a provider quota hold is formed, re-verified, and released.
//!
//! A hold stops every launch on a provider for as long as the provider says,
//! which can be a week. One screen notice used to be enough to form one, and
//! nothing short of `reset_at` or an operator released it — so a provider that
//! had recovered stayed held for days while the pool fell back to another
//! agent. Formation now needs repeated launch failures, and a held provider is
//! periodically given one launch to prove it has recovered.

/// Consecutive rate-limited launch attempts required before a hold forms.
pub(crate) const PROVIDER_QUOTA_HOLD_REQUIRED_FAILURES: usize = 3;

/// Retry delay after the first rate-limited attempt; each further failure
/// doubles it (60 s, 120 s, ...).
pub(crate) const PROVIDER_QUOTA_RETRY_BACKOFF_BASE_SECS: i64 = 60;

/// A rate-limited attempt older than this no longer counts toward a hold: the
/// failures must be consecutive, not scattered across a day.
pub(crate) const PROVIDER_QUOTA_FAILURE_WINDOW_SECS: i64 = 60 * 60;

/// How often a held provider is given one re-verification launch.
pub(crate) const PROVIDER_QUOTA_REVERIFY_INTERVAL_SECS: i64 = 30 * 60;

/// Agent activity on a held provider counts as recovery only this long after
/// its last rate-limited attempt — longer than the 120 s screen settle window,
/// so a launch that is about to be refused cannot vouch for itself.
pub(crate) const PROVIDER_QUOTA_RECOVERY_CONFIRM_SECS: i64 = 5 * 60;

/// Consecutive refusals a hold needs: the constant, or one while a test holds
/// the `hold_provider_quota_on_first_failure_in_this_test` guard.
pub(crate) fn provider_quota_required_failures() -> usize {
    #[cfg(test)]
    if HOLD_ON_FIRST_FAILURE.with(std::cell::Cell::get) {
        return 1;
    }
    PROVIDER_QUOTA_HOLD_REQUIRED_FAILURES
}

#[cfg(test)]
thread_local! {
    static HOLD_ON_FIRST_FAILURE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Test-only: while the returned guard lives, one refused launch forms the
/// hold, as it did before #4366. For tests about what a hold *does* once it
/// exists; covers every monitor the test builds, including ones rebuilt from
/// prefs.
#[cfg(test)]
pub(crate) fn hold_provider_quota_on_first_failure_in_this_test() -> HoldOnFirstFailure {
    HOLD_ON_FIRST_FAILURE.with(|flag| flag.set(true));
    HoldOnFirstFailure(())
}

#[cfg(test)]
pub(crate) struct HoldOnFirstFailure(());

#[cfg(test)]
impl Drop for HoldOnFirstFailure {
    fn drop(&mut self) {
        HOLD_ON_FIRST_FAILURE.with(|flag| flag.set(false));
    }
}

/// Retry delay after `failures` consecutive rate-limited attempts.
pub(crate) fn provider_quota_retry_backoff_secs(failures: usize) -> i64 {
    let doublings = failures.saturating_sub(1).min(16) as u32;
    PROVIDER_QUOTA_RETRY_BACKOFF_BASE_SECS.saturating_mul(1_i64 << doublings)
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::*;

    const RESET_AT: &str = "2026-09-21T08:41:00Z";
    const FIRST_FAILURE_AT: &str = "2026-09-14T16:42:18Z";
    const SCREEN: &str =
        "You've hit your usage limit. Visit https://chatgpt.com/codex/settings/usage \
                          to purchase more credits or try again at Sep 21st, 2026 5:41 PM.";

    fn after(base: &str, secs: i64) -> String {
        format_rfc3339_utc(
            parse_rfc3339_utc(base).expect("fixture instant") + chrono::Duration::seconds(secs),
        )
    }

    fn profile(agent_id: &str) -> IssueMonitorLaunchProfile {
        IssueMonitorLaunchProfile {
            agent_id: agent_id.to_string(),
            model: None,
            reasoning: None,
            version: None,
            session_mode: Default::default(),
            skip_permissions: false,
            fast_mode: false,
            runtime_target: Default::default(),
            docker_service: None,
            docker_lifecycle_intent: Default::default(),
            windows_shell: None,
            prefer_for: Vec::new(),
        }
    }

    fn monitor_with_pool(agents: &[&str]) -> IssueMonitorState {
        monitor_with_pool_holding(agents, &[])
    }

    fn monitor_with_pool_holding(agents: &[&str], holds: &[(&str, &str)]) -> IssueMonitorState {
        let mut prefs = IssueMonitorPrefs {
            enabled: true,
            provider_quota_holds: holds
                .iter()
                .map(|(provider, reset_at)| (provider.to_string(), reset_at.to_string()))
                .collect(),
            ..IssueMonitorPrefs::default()
        };
        prefs.set_launch_profile_pool(agents.iter().map(|agent| profile(agent)).collect());
        let mut monitor = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), prefs);
        monitor.set_gui_connected(true);
        monitor.record_candidate(IssueMonitorIssue {
            number: 42,
            title: "Issue 42".to_string(),
            labels: Vec::new(),
            state: IssueMonitorIssueState::Open,
            body: None,
            url: None,
            readiness: IssueMonitorReadiness::NotApplicable,
            updated_at: Some("2026-09-14T00:00:00Z".to_string()),
        });
        monitor
    }

    /// Launch Issue 42 on `window_id` and have the provider refuse it.
    fn rate_limited_launch(
        monitor: &mut IssueMonitorState,
        window_id: &str,
        at: &str,
    ) -> IssueMonitorProviderUsageLimitOutcome {
        monitor.complete_active_launch_at(42, window_id, at);
        monitor.try_hold_provider_usage_limit(
            42,
            window_id,
            "codex",
            "Codex usage limit reached",
            Some(RESET_AT),
            Some(IssueMonitorProviderQuotaHoldEvidence::screen_notice(
                at, window_id, SCREEN,
            )),
            at,
        )
    }

    /// Drive the required failures, each after the backoff the previous one
    /// set, and return the instant of the last one.
    fn form_codex_hold(monitor: &mut IssueMonitorState) -> String {
        let mut at = FIRST_FAILURE_AT.to_string();
        for attempt in 1..=PROVIDER_QUOTA_HOLD_REQUIRED_FAILURES {
            assert_eq!(
                rate_limited_launch(monitor, &format!("tab-1::agent-{attempt}"), &at),
                IssueMonitorProviderUsageLimitOutcome::Held
            );
            if attempt < PROVIDER_QUOTA_HOLD_REQUIRED_FAILURES {
                at = after(&at, provider_quota_retry_backoff_secs(attempt));
            }
        }
        at
    }

    fn codex_evidence(monitor: &IssueMonitorState) -> IssueMonitorProviderQuotaHoldEvidence {
        monitor
            .prefs()
            .provider_quota_hold_evidence
            .get("codex")
            .cloned()
            .expect("the hold carries its evidence")
    }

    /// AC-1 / AC-2 / AC-7: one refused launch is a retry, not a week-long hold.
    #[test]
    fn one_rate_limited_launch_does_not_form_a_provider_hold() {
        let mut monitor = monitor_with_pool(&["codex"]);

        assert_eq!(
            rate_limited_launch(&mut monitor, "tab-1::agent-1", FIRST_FAILURE_AT),
            IssueMonitorProviderUsageLimitOutcome::Held,
            "the refused launch still frees its slot"
        );

        assert!(
            monitor.prefs().provider_quota_holds.is_empty(),
            "a single observation must not hold the provider"
        );
        assert!(monitor
            .agent_status_at(FIRST_FAILURE_AT)
            .quota_hold
            .is_none());
        let retry_at = after(FIRST_FAILURE_AT, PROVIDER_QUOTA_RETRY_BACKOFF_BASE_SECS);
        assert!(!monitor.retry_ready(42, &after(&retry_at, -1)));
        assert!(
            monitor.retry_ready(42, &retry_at),
            "the Issue is relaunched after the first backoff, not after the notice's reset"
        );
    }

    /// AC-2 / AC-3 / AC-7: the hold forms on the N-th consecutive failure, and
    /// its evidence lists every attempt.
    #[test]
    fn the_required_rate_limited_launches_form_the_hold_with_every_attempt_as_evidence() {
        let mut monitor = monitor_with_pool(&["codex"]);
        let mut at = FIRST_FAILURE_AT.to_string();
        for attempt in 1..PROVIDER_QUOTA_HOLD_REQUIRED_FAILURES {
            rate_limited_launch(&mut monitor, &format!("tab-1::agent-{attempt}"), &at);
            assert!(
                monitor.prefs().provider_quota_holds.is_empty(),
                "attempt {attempt} of {PROVIDER_QUOTA_HOLD_REQUIRED_FAILURES} must not hold yet"
            );
            let backoff = provider_quota_retry_backoff_secs(attempt);
            assert_eq!(
                monitor
                    .autonomous_record(42)
                    .and_then(|record| record.retry_not_before.clone()),
                Some(after(&at, backoff)),
                "attempt {attempt} backs off exponentially"
            );
            at = after(&at, backoff);
        }
        assert_eq!(
            provider_quota_retry_backoff_secs(2),
            2 * provider_quota_retry_backoff_secs(1)
        );

        rate_limited_launch(
            &mut monitor,
            &format!("tab-1::agent-{PROVIDER_QUOTA_HOLD_REQUIRED_FAILURES}"),
            &at,
        );

        assert_eq!(
            monitor
                .prefs()
                .provider_quota_holds
                .get("codex")
                .map(String::as_str),
            Some(RESET_AT)
        );
        let evidence = codex_evidence(&monitor);
        assert_eq!(evidence.source, "launch_attempts");
        assert_eq!(
            evidence.attempts.len(),
            PROVIDER_QUOTA_HOLD_REQUIRED_FAILURES
        );
        for (index, attempt) in evidence.attempts.iter().enumerate() {
            assert_eq!(attempt.outcome, "rate_limited");
            assert_eq!(
                attempt.window_id.as_deref(),
                Some(format!("tab-1::agent-{}", index + 1).as_str())
            );
            assert_eq!(attempt.issue_number, Some(42));
            assert!(
                attempt
                    .screen_text
                    .as_deref()
                    .is_some_and(|text| text.contains("usage limit")),
                "each attempt keeps the wording it was refused with"
            );
        }
        assert_eq!(evidence.attempts[0].at, FIRST_FAILURE_AT);
        assert_eq!(
            evidence.next_reverify_at.as_deref(),
            Some(after(&at, PROVIDER_QUOTA_REVERIFY_INTERVAL_SECS).as_str())
        );
    }

    /// AC-2: failures must be consecutive; one from an hour ago is not the
    /// start of a streak.
    #[test]
    fn a_stale_rate_limited_attempt_does_not_count_toward_the_hold() {
        let mut monitor = monitor_with_pool(&["codex"]);
        let mut at = FIRST_FAILURE_AT.to_string();
        for attempt in 1..PROVIDER_QUOTA_HOLD_REQUIRED_FAILURES {
            rate_limited_launch(&mut monitor, &format!("tab-1::agent-{attempt}"), &at);
            at = after(&at, provider_quota_retry_backoff_secs(attempt));
        }

        let much_later = after(&at, PROVIDER_QUOTA_FAILURE_WINDOW_SECS + 1);
        rate_limited_launch(&mut monitor, "tab-1::agent-late", &much_later);

        assert!(monitor.prefs().provider_quota_holds.is_empty());
    }

    /// AC-4 / AC-7: a held provider gets one launch per interval, and agent
    /// activity on that launch releases the hold with a recorded reason.
    #[test]
    fn a_reverification_launch_that_shows_activity_releases_the_hold() {
        let mut monitor = monitor_with_pool(&["codex"]);
        let formed_at = form_codex_hold(&mut monitor);
        let reverify_at = after(&formed_at, PROVIDER_QUOTA_REVERIFY_INTERVAL_SECS);

        assert!(
            monitor
                .agent_status_at(&after(&reverify_at, -1))
                .quota_hold
                .is_some(),
            "held until the re-verification is due"
        );
        assert!(
            monitor.agent_status_at(&reverify_at).quota_hold.is_none(),
            "a due re-verification admits a launch before reset_at"
        );

        monitor.complete_active_launch_at(42, "tab-1::agent-probe", &reverify_at);
        assert!(
            monitor
                .agent_status_at(&after(&reverify_at, 1))
                .quota_hold
                .is_some(),
            "only one launch re-verifies the provider"
        );

        assert!(
            !monitor.record_provider_activity(42, "codex", &after(&reverify_at, 10)),
            "activity right after the launch cannot yet prove recovery"
        );
        let confirmed_at = after(&reverify_at, PROVIDER_QUOTA_RECOVERY_CONFIRM_SECS);
        assert!(monitor.record_provider_activity(42, "codex", &confirmed_at));

        assert!(monitor.prefs().provider_quota_holds.is_empty());
        let release = monitor
            .prefs()
            .provider_quota_hold_releases
            .get("codex")
            .cloned()
            .expect("the release is recorded");
        assert!(release.reason.contains("re-verification"), "{release:?}");
        assert_eq!(release.released_reset_at.as_deref(), Some(RESET_AT));
    }

    /// AC-3 / AC-4: a re-verification that is refused again keeps the hold,
    /// adds the attempt to the evidence, and waits a full interval again.
    #[test]
    fn a_refused_reverification_keeps_the_hold_and_is_recorded() {
        let mut monitor = monitor_with_pool(&["codex"]);
        let formed_at = form_codex_hold(&mut monitor);
        let reverify_at = after(&formed_at, PROVIDER_QUOTA_REVERIFY_INTERVAL_SECS);

        rate_limited_launch(&mut monitor, "tab-1::agent-probe", &reverify_at);

        assert_eq!(
            monitor
                .prefs()
                .provider_quota_holds
                .get("codex")
                .map(String::as_str),
            Some(RESET_AT)
        );
        let evidence = codex_evidence(&monitor);
        assert_eq!(
            evidence.attempts.len(),
            PROVIDER_QUOTA_HOLD_REQUIRED_FAILURES + 1
        );
        assert_eq!(
            evidence.next_reverify_at.as_deref(),
            Some(after(&reverify_at, PROVIDER_QUOTA_REVERIFY_INTERVAL_SECS).as_str())
        );
        assert!(monitor
            .agent_status_at(&after(&reverify_at, 60))
            .quota_hold
            .is_some());
    }

    /// AC-5: a poller reading below the limit brings the re-verification
    /// forward instead of waiting out the interval.
    #[test]
    fn a_healthy_poller_reading_brings_the_reverification_forward() {
        let mut monitor = monitor_with_pool(&["codex"]);
        let formed_at = form_codex_hold(&mut monitor);
        let poller_at = after(&formed_at, 60);
        assert!(monitor.agent_status_at(&poller_at).quota_hold.is_some());

        assert!(monitor.hasten_provider_quota_reverification("codex", &poller_at));

        assert!(
            monitor.agent_status_at(&poller_at).quota_hold.is_none(),
            "the re-verification launch is admitted now"
        );
        assert!(
            !monitor.hasten_provider_quota_reverification("codex", &after(&poller_at, 5)),
            "an already due re-verification is not moved again"
        );
    }

    /// AC-6c: the saved head, the candidate actually launched, and the reason
    /// are three separate fields.
    #[test]
    fn status_separates_the_saved_profile_from_the_effective_candidate() {
        let mut monitor = monitor_with_pool(&["codex", "claude"]);
        let formed_at = form_codex_hold(&mut monitor);
        let now = after(&formed_at, 60);

        for status in [
            serde_json::to_value(monitor.agent_status_at(&now)).expect("agent status"),
            serde_json::to_value(monitor.status_view_at(&now)).expect("gui status"),
        ] {
            assert_eq!(
                status.pointer("/launch_profile_candidates/0/agent_id"),
                Some(&serde_json::json!("codex")),
                "the saved head stays first: {status}"
            );
            assert_eq!(
                status.pointer("/effective_launch_profile/agent_id"),
                Some(&serde_json::json!("claude")),
                "{status}"
            );
            let reason = status
                .pointer("/effective_launch_profile/reason")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            assert!(
                reason.contains("codex") && reason.contains(RESET_AT),
                "the reason names the held provider and its reset: {reason}"
            );
        }

        monitor.clear_provider_quota_hold("codex", "test", &now);
        assert!(monitor
            .agent_status_at(&now)
            .effective_launch_profile
            .is_none());
    }

    /// The candidate `prefs` would launch at `now`, with the poller reading
    /// every provider as `reported_healthy`.
    fn launch_choice(prefs: &IssueMonitorPrefs, now: &str, reported_healthy: bool) -> String {
        let pool = prefs.launch_profile_pool();
        let selection = select_launch_profile(
            &pool,
            &prefs.launch_admission_provider_quota_holds(now, |_| reported_healthy),
            &[],
            prefs.launch_usage_threshold_percent,
            &[],
            None,
            now,
        );
        pool[selection.selected.expect("a candidate is selectable")]
            .agent_id
            .clone()
    }

    /// Issue #4636 AC-1 / AC-5(a) / AC-6: a held head is skipped for a free
    /// candidate even when its re-verification is due. The re-verification
    /// only preempts a free candidate when the poller contradicts the hold.
    #[test]
    fn a_due_reverification_does_not_preempt_a_free_candidate() {
        let mut monitor = monitor_with_pool(&["codex", "claude"]);
        let formed_at = form_codex_hold(&mut monitor);
        let reverify_at = after(&formed_at, PROVIDER_QUOTA_REVERIFY_INTERVAL_SECS);
        let prefs = monitor.prefs();

        assert_eq!(
            launch_choice(&prefs, &after(&formed_at, 60), false),
            "claude"
        );
        assert_eq!(
            launch_choice(&prefs, &reverify_at, false),
            "claude",
            "a due re-verification must not spend a launch on a provider the poller still reads as exhausted"
        );
        assert_eq!(
            launch_choice(&prefs, &reverify_at, true),
            "codex",
            "a poller reading that contradicts the hold still gets its re-verification launch"
        );
        assert_eq!(
            monitor
                .agent_status_at(&reverify_at)
                .effective_launch_profile
                .and_then(|effective| effective.agent_id)
                .as_deref(),
            Some("claude")
        );
        assert_eq!(
            monitor
                .prefs()
                .provider_quota_holds
                .get("codex")
                .map(String::as_str),
            Some(RESET_AT),
            "AC-6: choosing around a hold never shortens it"
        );
    }

    /// Issue #4636 AC-2 / AC-5(b): with every candidate held, nothing
    /// launches until the earliest reset or re-verification, and the stop is
    /// reported as a blackout instead of being silent.
    #[test]
    fn every_candidate_held_stops_launches_and_reports_why() {
        let claude_reset = "2026-09-20T00:00:00Z";
        let mut monitor =
            monitor_with_pool_holding(&["codex", "claude"], &[("claude", claude_reset)]);
        let formed_at = form_codex_hold(&mut monitor);
        let now = after(&formed_at, 60);

        let status = monitor.agent_status_at(&now);
        assert_eq!(
            status
                .quota_hold
                .as_ref()
                .map(|hold| hold.reset_at.as_str()),
            Some(claude_reset)
        );
        assert!(monitor.next_launch_request(&now).is_none());
        let blackout = status.agent_blackout.unwrap_or_default();
        assert!(
            blackout.contains("held") && blackout.contains(claude_reset),
            "the all-held stop names when launches resume: {blackout:?}"
        );

        // The re-verification is the one launch a fully held pool admits.
        let reverify_at = after(&formed_at, PROVIDER_QUOTA_REVERIFY_INTERVAL_SECS);
        assert_eq!(
            launch_choice(&monitor.prefs(), &reverify_at, false),
            "codex"
        );
    }

    /// Issue #4636 AC-4 / AC-5(c): once `held_until` passes, the provider is a
    /// candidate again without anyone clearing the hold.
    #[test]
    fn an_expired_hold_returns_the_provider_to_the_pool() {
        let mut monitor = monitor_with_pool(&["codex", "claude"]);
        form_codex_hold(&mut monitor);
        let prefs = monitor.prefs();

        assert_eq!(launch_choice(&prefs, &after(RESET_AT, -1), false), "claude");
        assert_eq!(launch_choice(&prefs, RESET_AT, false), "codex");
        assert_eq!(
            prefs.provider_quota_holds.get("codex").map(String::as_str),
            Some(RESET_AT),
            "AC-6: the hold record itself is left in place"
        );
    }

    /// Issue #4636 AC-7 / AC-9(a): a provider hold is not copied onto the
    /// Issue while another candidate can run it.
    #[test]
    fn a_provider_hold_does_not_park_the_issue_while_another_candidate_is_free() {
        let mut monitor = monitor_with_pool(&["codex", "claude"]);
        let formed_at = form_codex_hold(&mut monitor);

        assert_eq!(
            monitor
                .autonomous_record(42)
                .and_then(|record| record.retry_not_before.clone()),
            None,
            "the provider's reset must not become the Issue's retry floor"
        );
        assert!(monitor.retry_ready(42, &after(&formed_at, 1)));
        assert_eq!(monitor.queued_issue_numbers(), vec![42]);
    }

    /// Issue #4636 AC-8 / AC-9(b): with every candidate held the Issue waits
    /// for the earliest reset, not for the provider it last tried.
    #[test]
    fn an_all_held_pool_parks_the_issue_until_the_earliest_reset() {
        let claude_reset = "2026-09-20T00:00:00Z";
        let mut monitor =
            monitor_with_pool_holding(&["codex", "claude"], &[("claude", claude_reset)]);

        form_codex_hold(&mut monitor);

        assert_eq!(
            monitor
                .autonomous_record(42)
                .and_then(|record| record.retry_not_before.clone())
                .as_deref(),
            Some(claude_reset)
        );
    }
}
