//! Issue #4908: a refused launch immediately holds its provider. Known
//! resets restore admission at that time; unknown resets require an explicit
//! release or account replacement. Usage readings never select a candidate.

/// Issue #4908 AC-3: the `provider_quota_holds` deadline of a hold whose
/// refusal stated no reset. A far-future instant rather than a new field, so
/// a gwt that predates it still reads the provider as held; every projection
/// reports it as [`PROVIDER_QUOTA_UNKNOWN_RESET_LABEL`] instead.
pub(crate) const PROVIDER_QUOTA_UNKNOWN_RESET_AT: &str = "9999-12-31T23:59:59Z";

/// How a hold with no stated reset reads in `issue.monitor.status`.
pub(crate) const PROVIDER_QUOTA_UNKNOWN_RESET_LABEL: &str = "unknown";

/// A hold's reset as readers see it: the instant, or `unknown`.
pub(crate) fn provider_quota_reset_label(reset_at: &str) -> &str {
    if reset_at == PROVIDER_QUOTA_UNKNOWN_RESET_AT {
        PROVIDER_QUOTA_UNKNOWN_RESET_LABEL
    } else {
        reset_at
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::*;

    const LEGACY_REVERIFY_INTERVAL_SECS: i64 = 30 * 60;

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
            max_active_agents_mode: crate::issue_monitor::IssueMonitorMaxActiveMode::Manual,
            provider_quota_holds: holds
                .iter()
                .map(|(provider, reset_at)| (provider.to_string(), reset_at.to_string()))
                .collect(),
            ..IssueMonitorPrefs::default()
        };
        prefs.set_launch_profile_pool(agents.iter().map(|agent| profile(agent)).collect());
        let mut monitor = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), prefs);
        monitor.set_gui_connected(true);
        monitor.terminal_queue_push(&[42], "quota-test", FIRST_FAILURE_AT);
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
        refused_launch(monitor, "codex", window_id, at, Some(RESET_AT))
    }

    /// Launch Issue 42 on `window_id` and have `provider` refuse it, stating
    /// `resets_at` (or no reset at all).
    fn refused_launch(
        monitor: &mut IssueMonitorState,
        provider: &str,
        window_id: &str,
        at: &str,
        resets_at: Option<&str>,
    ) -> IssueMonitorProviderUsageLimitOutcome {
        monitor.complete_active_launch_at(42, window_id, at);
        monitor.try_hold_provider_usage_limit(
            42,
            window_id,
            provider,
            format!("{provider} usage limit reached"),
            resets_at,
            Some(IssueMonitorProviderQuotaHoldEvidence::screen_notice(
                at, window_id, SCREEN,
            )),
            at,
        )
    }

    fn form_codex_hold(monitor: &mut IssueMonitorState) -> String {
        assert_eq!(
            rate_limited_launch(monitor, "tab-1::agent-1", FIRST_FAILURE_AT),
            IssueMonitorProviderUsageLimitOutcome::Held
        );
        FIRST_FAILURE_AT.to_string()
    }

    fn refuse_until_held(
        monitor: &mut IssueMonitorState,
        provider: &str,
        first_at: &str,
        resets_at: Option<&str>,
    ) -> String {
        refused_launch(
            monitor,
            provider,
            &format!("tab-1::agent-{provider}"),
            first_at,
            resets_at,
        );
        assert!(monitor.prefs().provider_quota_holds.contains_key(provider));
        first_at.to_string()
    }

    fn codex_evidence(monitor: &IssueMonitorState) -> IssueMonitorProviderQuotaHoldEvidence {
        monitor
            .prefs()
            .provider_quota_hold_evidence
            .get("codex")
            .cloned()
            .expect("the hold carries its evidence")
    }

    /// Issue #4908 D/B: even the last provider is held on its first refusal.
    #[test]
    fn one_rate_limited_launch_holds_the_last_provider_until_reset() {
        let mut monitor = monitor_with_pool(&["codex"]);
        assert_eq!(
            rate_limited_launch(&mut monitor, "tab-1::agent-1", FIRST_FAILURE_AT),
            IssueMonitorProviderUsageLimitOutcome::Held
        );
        assert_eq!(
            monitor
                .prefs()
                .provider_quota_holds
                .get("codex")
                .map(String::as_str),
            Some(RESET_AT)
        );
        let before_reset = after(RESET_AT, -1);
        assert!(!monitor.retry_ready_for_saved_profile(42, &before_reset));
        assert!(monitor.next_launch_request(&before_reset).is_none());
        assert!(monitor
            .agent_status_at(&before_reset)
            .needs_human_fleet
            .is_none());
        assert!(monitor.retry_ready_for_saved_profile(42, RESET_AT));
        assert!(monitor.next_launch_request(RESET_AT).is_some());
    }

    /// The first refusal and any racing refusal retain their evidence.
    #[test]
    fn refused_launches_keep_every_attempt_as_evidence_without_scheduling_probes() {
        let mut monitor = monitor_with_pool(&["codex"]);
        form_codex_hold(&mut monitor);
        let later = after(FIRST_FAILURE_AT, 60);
        rate_limited_launch(&mut monitor, "tab-1::agent-racing", &later);
        let evidence = codex_evidence(&monitor);
        assert_eq!(evidence.source, "launch_attempts");
        assert_eq!(evidence.attempts.len(), 2);
        assert_eq!(evidence.attempts[0].at, FIRST_FAILURE_AT);
        assert_eq!(evidence.attempts[1].at, later);
        assert_eq!(
            evidence.attempts[1].window_id.as_deref(),
            Some("tab-1::agent-racing")
        );
        for attempt in &evidence.attempts {
            assert_eq!(attempt.outcome, "rate_limited");
            assert_eq!(attempt.issue_number, Some(42));
            assert!(attempt
                .screen_text
                .as_deref()
                .unwrap()
                .contains("usage limit"));
        }
        assert!(evidence.next_reverify_at.is_none());
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
    fn launch_choice(prefs: &IssueMonitorPrefs, now: &str, _reported_healthy: bool) -> String {
        let pool = prefs.launch_profile_pool();
        let selection = select_launch_profile(
            &pool,
            &prefs.launch_admission_provider_quota_holds(),
            &[],
            None,
            now,
        );
        pool[selection.selected.expect("a candidate is selectable")]
            .agent_id
            .clone()
    }

    /// A held head stays skipped even with a legacy probe schedule.
    #[test]
    fn a_due_reverification_does_not_preempt_a_free_candidate() {
        let mut monitor = monitor_with_pool(&["codex", "claude"]);
        let formed_at = form_codex_hold(&mut monitor);
        let reverify_at = after(&formed_at, LEGACY_REVERIFY_INTERVAL_SECS);
        monitor
            .provider_quota_hold_evidence
            .get_mut("codex")
            .unwrap()
            .next_reverify_at = Some(reverify_at.clone());
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
            "claude",
            "usage readings cannot bypass the held provider"
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
    /// launches until the earliest known reset, and the stop is
    /// reported as a blackout instead of being silent.
    #[test]
    fn every_candidate_held_stops_launches_and_reports_why() {
        let claude_reset = "2026-09-20T00:00:00Z";
        let mut monitor = monitor_with_pool_holding(
            &["codex", "claude", "grok-build"],
            &[
                ("claude", claude_reset),
                ("grok-build", PROVIDER_QUOTA_UNKNOWN_RESET_AT),
            ],
        );
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

        // A persisted legacy probe schedule cannot reopen admission before reset.
        let reverify_at = after(&formed_at, LEGACY_REVERIFY_INTERVAL_SECS);
        monitor
            .provider_quota_hold_evidence
            .get_mut("codex")
            .unwrap()
            .next_reverify_at = Some(reverify_at.clone());
        assert!(monitor.next_launch_request(&reverify_at).is_none());
        assert!(monitor
            .agent_status_at(&reverify_at)
            .needs_human_fleet
            .is_none());
        assert_eq!(
            launch_choice(&monitor.prefs(), claude_reset, false),
            "claude"
        );
        assert!(monitor.next_launch_request(claude_reset).is_some());
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
        let mut monitor = monitor_with_pool_holding(
            &["codex", "claude", "grok-build"],
            &[
                ("claude", claude_reset),
                ("grok-build", PROVIDER_QUOTA_UNKNOWN_RESET_AT),
            ],
        );

        form_codex_hold(&mut monitor);

        assert_eq!(
            monitor
                .autonomous_record(42)
                .and_then(|record| record.retry_not_before.clone())
                .as_deref(),
            Some(claude_reset)
        );
    }

    /// Issue #4908 AC-1 / AC-3 / AC-7: with another candidate free, the first
    /// refusal switches launches to it — no retry on the refused provider, no
    /// backoff on the Issue — and the refused provider stays out until the
    /// reset the refusal stated.
    #[test]
    fn the_first_refusal_switches_launches_to_the_next_free_candidate() {
        let mut monitor = monitor_with_pool(&["codex", "claude"]);
        assert_eq!(
            launch_choice(&monitor.prefs(), FIRST_FAILURE_AT, false),
            "codex"
        );

        assert_eq!(
            rate_limited_launch(&mut monitor, "tab-1::agent-1", FIRST_FAILURE_AT),
            IssueMonitorProviderUsageLimitOutcome::Held
        );

        let prefs = monitor.prefs();
        assert_eq!(
            prefs.provider_quota_holds.get("codex").map(String::as_str),
            Some(RESET_AT),
            "one observed refusal takes the provider out of the pool"
        );
        assert_eq!(launch_choice(&prefs, FIRST_FAILURE_AT, false), "claude");
        assert!(
            monitor.retry_ready(42, FIRST_FAILURE_AT),
            "the refused Issue relaunches on the next candidate at once"
        );
        assert_eq!(monitor.queued_issue_numbers(), vec![42]);
        assert_eq!(launch_choice(&prefs, &after(RESET_AT, -1), false), "claude");
        assert_eq!(launch_choice(&prefs, RESET_AT, false), "codex");
    }

    /// Issue #4908 AC-1: nothing but a refusal switches launches. A poller
    /// reading that shows the account at its limit forms no hold and leaves
    /// the pool head the launch choice — the account may still be serving.
    #[test]
    fn a_usage_reading_at_the_limit_never_switches_the_launch_candidate() {
        let mut monitor = monitor_with_pool(&["codex", "claude"]);
        let fetched_at = parse_rfc3339_utc(FIRST_FAILURE_AT).expect("fixture instant");
        let exhausted = gwt_core::usage::ProviderUsage {
            provider: gwt_core::usage::UsageProvider::Codex,
            account_id: Some("acct".to_string()),
            account_label: None,
            plan: None,
            windows: vec![gwt_core::usage::UsageWindow::new(
                gwt_core::usage::WindowKind::Weekly,
                100.0,
                None,
            )],
            limit_reached: true,
            state: gwt_core::usage::UsageState::Ok,
            fetched_at: Some(fetched_at),
        };

        monitor.reconcile_provider_usage(&exhausted, FIRST_FAILURE_AT);

        let prefs = monitor.prefs();
        assert!(
            prefs.provider_quota_holds.is_empty(),
            "a reading is not a refusal: {:?}",
            prefs.provider_quota_holds
        );
        assert_eq!(launch_choice(&prefs, FIRST_FAILURE_AT, false), "codex");
        let status = monitor.agent_status_at(FIRST_FAILURE_AT);
        assert!(status.effective_launch_profile.is_none());
        assert!(status.needs_human_fleet.is_none());
    }

    /// Issue #4908 AC-2: the status says launches were switched, and names
    /// the provider that refused, when, and with what wording.
    #[test]
    fn status_names_the_provider_and_the_refusal_behind_a_switch() {
        let mut monitor = monitor_with_pool(&["codex", "claude"]);
        rate_limited_launch(&mut monitor, "tab-1::agent-1", FIRST_FAILURE_AT);
        let now = after(FIRST_FAILURE_AT, 30);

        for status in [
            serde_json::to_value(monitor.agent_status_at(&now)).expect("agent status"),
            serde_json::to_value(monitor.status_view_at(&now)).expect("gui status"),
        ] {
            assert_eq!(
                status.pointer("/effective_launch_profile/agent_id"),
                Some(&serde_json::json!("claude")),
                "{status}"
            );
            assert_eq!(
                status.pointer("/effective_launch_profile/refused_provider"),
                Some(&serde_json::json!("codex")),
                "{status}"
            );
            assert_eq!(
                status.pointer("/effective_launch_profile/refused_at"),
                Some(&serde_json::json!(FIRST_FAILURE_AT)),
                "{status}"
            );
            let text = |pointer: &str| {
                status
                    .pointer(pointer)
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string()
            };
            assert!(
                text("/effective_launch_profile/refusal").contains("hit your usage limit"),
                "{status}"
            );
            let reason = text("/effective_launch_profile/reason");
            assert!(
                reason.contains("codex")
                    && reason.contains("hit your usage limit")
                    && reason.contains(RESET_AT),
                "the reason names the provider, its refusal and its reset: {reason}"
            );
        }
    }

    /// Issue #4908 AC-3: a refusal that states no reset is recorded as
    /// unknown, and the provider does not come back on a timer.
    #[test]
    fn a_refusal_without_a_reset_is_recorded_as_unknown_and_never_returns_on_a_timer() {
        let mut monitor = monitor_with_pool(&["codex", "claude"]);

        refused_launch(
            &mut monitor,
            "codex",
            "tab-1::agent-1",
            FIRST_FAILURE_AT,
            None,
        );

        let prefs = monitor.prefs();
        for secs in [61, 60 * 60 + 1, 7 * 24 * 60 * 60, 365 * 24 * 60 * 60] {
            assert_eq!(
                launch_choice(&prefs, &after(FIRST_FAILURE_AT, secs), false),
                "claude",
                "{secs}s after a refusal with no stated reset"
            );
        }
        let status = serde_json::to_value(monitor.agent_status_at(&after(FIRST_FAILURE_AT, 120)))
            .expect("agent status");
        assert_eq!(
            status.pointer("/provider_quota_holds/0/reset_at"),
            Some(&serde_json::json!("unknown")),
            "{status}"
        );
        assert_eq!(
            status.pointer("/launch_profile_candidates/0/held_until"),
            Some(&serde_json::json!("unknown")),
            "{status}"
        );
        let reason = status
            .pointer("/effective_launch_profile/reason")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        assert!(
            reason.contains("reset unknown") && !reason.contains("9999"),
            "{reason}"
        );
    }

    /// Issue #4908 AC-3: a reset read from a later refusal replaces the
    /// unknown one.
    #[test]
    fn a_later_refusal_that_states_its_reset_replaces_the_unknown_one() {
        let mut monitor = monitor_with_pool(&["codex", "claude"]);
        refused_launch(
            &mut monitor,
            "codex",
            "tab-1::agent-1",
            FIRST_FAILURE_AT,
            None,
        );

        rate_limited_launch(
            &mut monitor,
            "tab-1::agent-2",
            &after(FIRST_FAILURE_AT, 600),
        );

        assert_eq!(
            monitor
                .prefs()
                .provider_quota_holds
                .get("codex")
                .map(String::as_str),
            Some(RESET_AT)
        );
        assert_eq!(codex_evidence(&monitor).attempts.len(), 2);
    }

    /// Issue #4908 AC-4: once every candidate has refused, nothing launches
    /// and the status says a human is needed — the stop is never silent.
    #[test]
    fn every_candidate_refused_is_reported_as_needing_a_human() {
        let mut monitor = monitor_with_pool(&["codex", "claude"]);
        refused_launch(
            &mut monitor,
            "codex",
            "tab-1::agent-1",
            FIRST_FAILURE_AT,
            None,
        );
        assert!(monitor
            .agent_status_at(FIRST_FAILURE_AT)
            .needs_human_fleet
            .is_none());
        let at = after(FIRST_FAILURE_AT, 60);
        refused_launch(&mut monitor, "claude", "tab-1::agent-2", &at, None);
        for now in [&at, &after(&at, 31 * 60), &after(&at, 365 * 24 * 60 * 60)] {
            let status = monitor.agent_status_at(now);
            let needs_human = status
                .needs_human_fleet
                .expect("all unknown resets need a human immediately");
            assert_eq!(needs_human.kind, "launch_candidates_exhausted");
            assert!(needs_human.reason.contains("codex (reset unknown)"));
            assert!(needs_human.reason.contains("claude (reset unknown)"));
            assert!(monitor.next_launch_request(now).is_none());
            assert!(status.agent_blackout.is_some());
            assert_eq!(
                status.stall_reason,
                Some(IssueMonitorStallReason::QuotaHold)
            );
        }
    }

    /// Issue #4908 AC-3 / AC-4: an Issue parked behind an exhausted pool
    /// whose resets are unknown is admitted again as soon as one provider is
    /// released — its own floor never passes, so the pool decides.
    #[test]
    fn releasing_one_provider_of_an_exhausted_pool_readmits_the_parked_issue() {
        let mut monitor = monitor_with_pool(&["codex", "claude"]);
        refused_launch(
            &mut monitor,
            "codex",
            "tab-1::agent-1",
            FIRST_FAILURE_AT,
            None,
        );
        let at = refuse_until_held(&mut monitor, "claude", &after(FIRST_FAILURE_AT, 60), None);
        let now = after(&at, 30);
        assert_eq!(
            monitor
                .autonomous_record(42)
                .and_then(|record| record.retry_not_before.as_deref()),
            Some(PROVIDER_QUOTA_UNKNOWN_RESET_AT)
        );
        assert!(!monitor.retry_ready_for_saved_profile(42, &now));
        assert_eq!(monitor.runnable_backlog_len(&now), 0);
        assert!(monitor.next_launch_request(&now).is_none());

        monitor.clear_provider_quota_hold("codex", "test", &now);

        assert!(
            monitor.retry_ready_for_saved_profile(42, &now),
            "a free candidate outranks a floor that mirrors another provider's hold"
        );
        assert_eq!(
            monitor.runnable_backlog_len(&now),
            1,
            "the parked Issue counts as runnable again"
        );
        let status = monitor.agent_status_at(&now);
        assert!(status.needs_human_fleet.is_none());
        assert!(
            status.quota_hold.is_none(),
            "launch admission is open again"
        );
    }

    /// Issue #4908 AC-3: between two processes, the newer refusal decides
    /// whether a hold carries a stated reset or an unknown one — the unknown
    /// deadline is not "later" than a real reset.
    #[test]
    fn the_newer_refusal_decides_between_an_unknown_and_a_stated_reset_across_processes() {
        let mut stale = monitor_with_pool(&["codex", "claude"]);
        refused_launch(
            &mut stale,
            "codex",
            "tab-1::agent-1",
            FIRST_FAILURE_AT,
            None,
        );
        let mut newer = monitor_with_pool(&["codex", "claude"]);
        refused_launch(
            &mut newer,
            "codex",
            "tab-1::agent-1",
            FIRST_FAILURE_AT,
            None,
        );
        rate_limited_launch(&mut newer, "tab-1::agent-2", &after(FIRST_FAILURE_AT, 600));
        let hold = |monitor: &IssueMonitorState| {
            monitor.prefs().provider_quota_holds.get("codex").cloned()
        };
        assert_eq!(
            hold(&stale).as_deref(),
            Some(PROVIDER_QUOTA_UNKNOWN_RESET_AT)
        );
        assert_eq!(hold(&newer).as_deref(), Some(RESET_AT));

        let stale_prefs = stale.prefs();
        newer.merge_provider_quota_holds_from_prefs(&stale_prefs);
        assert_eq!(
            hold(&newer).as_deref(),
            Some(RESET_AT),
            "an older unknown hold on disk must not erase the stated reset"
        );

        let newer_prefs = newer.prefs();
        stale.merge_provider_quota_holds_from_prefs(&newer_prefs);
        assert_eq!(
            hold(&stale).as_deref(),
            Some(RESET_AT),
            "the newer refusal's stated reset replaces the unknown one"
        );
    }
}
