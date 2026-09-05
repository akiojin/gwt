//! Issue #3891 — GitHub API budget observation and demand-side throttling.
//!
//! AC-3: primary budgets (from `gh api rate_limit`) and a local consumption
//!       counter that approximates the unobservable secondary limit are
//!       exposed together, with the approximation stated explicitly.
//! AC-4: a budget below the reserve, an active rate-limit block, or a local
//!       burst yields a throttle reason that callers can act on and report.

use chrono::{DateTime, Duration, TimeZone, Utc};
use gwt_core::github_budget::{
    self, BudgetLedger, ProbeSnapshot, ResourceWindow, ThrottlePolicy, SECONDARY_LIMIT_NOTE,
};
use gwt_core::github_quota::{GitHubQuota, RateLimitBlock};

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_756_800_000 + seconds, 0)
        .single()
        .expect("timestamp")
}

fn probe(graphql_remaining: u64, probed_at: DateTime<Utc>) -> ProbeSnapshot {
    let mut resources = std::collections::BTreeMap::new();
    resources.insert(
        "graphql".to_string(),
        ResourceWindow {
            limit: 5000,
            remaining: graphql_remaining,
            reset_at: probed_at + Duration::minutes(40),
        },
    );
    resources.insert(
        "core".to_string(),
        ResourceWindow {
            limit: 5000,
            remaining: 4990,
            reset_at: probed_at + Duration::minutes(40),
        },
    );
    ProbeSnapshot {
        probed_at,
        resources,
    }
}

// ── AC-3: local consumption ledger ──

#[test]
fn spawn_ledger_counts_per_resource_inside_minute_and_hour_windows() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ledger = BudgetLedger::at(dir.path());
    ledger.record_spawn(GitHubQuota::GraphQl, at(-4000)); // outside the hour
    ledger.record_spawn(GitHubQuota::GraphQl, at(0));
    ledger.record_spawn(GitHubQuota::GraphQl, at(30));
    ledger.record_spawn(GitHubQuota::GraphQl, at(120));
    ledger.record_spawn(GitHubQuota::Rest, at(0));
    ledger.record_spawn(GitHubQuota::Free, at(125)); // never counted

    let snapshot = ledger.snapshot(at(130));
    let graphql = &snapshot.local["graphql"];
    assert_eq!(graphql.calls_last_minute, 1, "{snapshot:?}");
    assert_eq!(graphql.calls_last_hour, 3, "{snapshot:?}");
    let core = &snapshot.local["core"];
    assert_eq!(core.calls_last_minute, 0);
    assert_eq!(core.calls_last_hour, 1);
    assert_eq!(snapshot.secondary_note, SECONDARY_LIMIT_NOTE);
    assert!(
        SECONDARY_LIMIT_NOTE.contains("approximat"),
        "the secondary estimate must say it is an approximation"
    );
}

#[test]
fn snapshot_of_a_missing_ledger_is_empty_but_well_formed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ledger = BudgetLedger::at(&dir.path().join("never-created"));
    let snapshot = ledger.snapshot(at(0));
    assert!(snapshot.probe.is_none());
    assert!(snapshot.probe_age_secs.is_none());
    assert!(snapshot.last_block.is_none());
    assert_eq!(snapshot.local["graphql"].calls_last_hour, 0);
    assert_eq!(snapshot.local["core"].calls_last_hour, 0);
}

// ── AC-3: primary budgets from the free probe ──

#[test]
fn rate_limit_probe_parses_every_budget_resource() {
    let payload = r#"{
        "resources": {
            "core": {"limit": 5000, "remaining": 4994, "reset": 1756803600},
            "graphql": {"limit": 5000, "remaining": 12, "reset": 1756802400},
            "search": {"limit": 30, "remaining": 30, "reset": 1756800060}
        },
        "rate": {"limit": 5000, "remaining": 4994, "reset": 1756803600}
    }"#;
    let snapshot = github_budget::parse_rate_limit_probe_all(payload, at(0)).expect("probe");
    assert_eq!(snapshot.probed_at, at(0));
    assert_eq!(snapshot.resources["graphql"].remaining, 12);
    assert_eq!(snapshot.resources["graphql"].limit, 5000);
    assert_eq!(snapshot.resources["graphql"].reset_at, at(2400));
    assert_eq!(snapshot.resources["core"].remaining, 4994);
    assert!(
        !snapshot.resources.contains_key("search"),
        "only the budgets gwt spends are reported"
    );
}

#[test]
fn recorded_probe_and_block_are_visible_across_ledger_instances() {
    let dir = tempfile::tempdir().expect("tempdir");
    BudgetLedger::at(dir.path()).record_probe(&probe(4200, at(0)));
    BudgetLedger::at(dir.path()).record_block(
        &RateLimitBlock {
            resource: "graphql".to_string(),
            limit: 5000,
            remaining: 0,
            reset_at: at(600),
        },
        at(10),
    );

    let snapshot = BudgetLedger::at(dir.path()).snapshot(at(70));
    assert_eq!(snapshot.probe_age_secs, Some(70));
    assert_eq!(
        snapshot.probe.as_ref().expect("probe").resources["graphql"].remaining,
        4200
    );
    let block = snapshot.last_block.expect("block");
    assert_eq!(block.resource, "graphql");
    assert_eq!(block.reset_at, at(600));
}

// ── AC-4: throttle decision ──

#[test]
fn healthy_budget_yields_no_throttle_reason() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ledger = BudgetLedger::at(dir.path());
    ledger.record_probe(&probe(4200, at(0)));
    ledger.record_spawn(GitHubQuota::GraphQl, at(5));
    let snapshot = ledger.snapshot(at(10));
    assert_eq!(
        github_budget::throttle_reason(
            &snapshot,
            GitHubQuota::GraphQl,
            &ThrottlePolicy::default(),
            at(10)
        ),
        None
    );
}

#[test]
fn remaining_below_reserve_yields_a_throttle_reason() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ledger = BudgetLedger::at(dir.path());
    ledger.record_probe(&probe(400, at(0)));
    let snapshot = ledger.snapshot(at(10));
    let reason = github_budget::throttle_reason(
        &snapshot,
        GitHubQuota::GraphQl,
        &ThrottlePolicy::default(),
        at(10),
    )
    .expect("400 of 5000 is below the 20% reserve");
    assert!(reason.contains("graphql"), "{reason}");
    assert!(reason.contains("remaining=400"), "{reason}");
    assert!(reason.contains("reserve=1000"), "{reason}");
}

#[test]
fn a_stale_probe_never_throttles_on_its_own() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ledger = BudgetLedger::at(dir.path());
    ledger.record_probe(&probe(0, at(0)));
    let policy = ThrottlePolicy::default();
    let later = at(policy.probe_max_age_secs + 1);
    let snapshot = ledger.snapshot(later);
    assert!(github_budget::probe_is_stale(&snapshot, &policy));
    assert_eq!(
        github_budget::throttle_reason(&snapshot, GitHubQuota::GraphQl, &policy, later),
        None,
        "a stale probe is unknown budget, not exhausted budget; callers re-probe instead"
    );
}

#[test]
fn an_active_rate_limit_block_throttles_until_its_reset() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ledger = BudgetLedger::at(dir.path());
    ledger.record_probe(&probe(4900, at(0)));
    ledger.record_block(
        &RateLimitBlock {
            resource: "graphql".to_string(),
            limit: 0,
            remaining: 0,
            reset_at: at(90),
        },
        at(30),
    );
    let policy = ThrottlePolicy::default();
    let reason = github_budget::throttle_reason(
        &ledger.snapshot(at(60)),
        GitHubQuota::GraphQl,
        &policy,
        at(60),
    )
    .expect("block still active");
    assert!(reason.contains("github_rate_limited"), "{reason}");
    assert!(reason.contains("reset_at="), "{reason}");
    assert_eq!(
        github_budget::throttle_reason(
            &ledger.snapshot(at(91)),
            GitHubQuota::GraphQl,
            &policy,
            at(91)
        ),
        None,
        "an elapsed block no longer throttles"
    );
}

#[test]
fn a_local_burst_throttles_even_when_the_primary_budget_looks_full() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ledger = BudgetLedger::at(dir.path());
    ledger.record_probe(&probe(5000, at(0)));
    let policy = ThrottlePolicy::default();
    for second in 0..policy.burst_calls_per_minute {
        ledger.record_spawn(GitHubQuota::GraphQl, at(second as i64));
    }
    let now = at(59);
    let reason =
        github_budget::throttle_reason(&ledger.snapshot(now), GitHubQuota::GraphQl, &policy, now)
            .expect("burst at the policy limit throttles");
    assert!(reason.contains("burst"), "{reason}");
    assert!(reason.contains("calls_last_minute="), "{reason}");
}

#[test]
fn throttle_looks_only_at_the_requested_resource() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ledger = BudgetLedger::at(dir.path());
    let mut exhausted_core = probe(4900, at(0));
    exhausted_core
        .resources
        .get_mut("core")
        .expect("core")
        .remaining = 0;
    ledger.record_probe(&exhausted_core);
    let snapshot = ledger.snapshot(at(5));
    assert_eq!(
        github_budget::throttle_reason(
            &snapshot,
            GitHubQuota::GraphQl,
            &ThrottlePolicy::default(),
            at(5)
        ),
        None
    );
    assert!(github_budget::throttle_reason(
        &snapshot,
        GitHubQuota::Rest,
        &ThrottlePolicy::default(),
        at(5)
    )
    .is_some());
    assert_eq!(
        github_budget::throttle_reason(
            &snapshot,
            GitHubQuota::Free,
            &ThrottlePolicy::default(),
            at(5)
        ),
        None,
        "free calls are never throttled"
    );
}

// ── AC-3: the spawn choke point feeds the ledger ──

/// Every budget-spending `gh` spawn is recorded in the machine-local ledger
/// so the secondary limit can be approximated across processes — even when
/// the spawn itself fails afterwards. Lives here (not in the spawn-gate
/// binary) because that binary's tests deliberately exhaust the process-global
/// gate, which would suppress the GraphQL spawn this test counts.
#[test]
fn gh_spawns_are_recorded_in_the_machine_local_budget_ledger() {
    use gwt_core::process_console::{
        spawn_logged_blocking, ProcessConsoleHub, ProcessKind, SpawnOptions,
    };
    const MISSING_PROGRAM: &str = "gwt-nonexistent-gh-3891";
    let hub = ProcessConsoleHub::new();
    let before = BudgetLedger::global().snapshot(Utc::now());
    let _ = spawn_logged_blocking(
        &hub,
        ProcessKind::Gh,
        MISSING_PROGRAM,
        &["pr", "list", "--state", "open", "--json", "number"],
        SpawnOptions::new("gh pr list"),
    );
    let _ = spawn_logged_blocking(
        &hub,
        ProcessKind::Gh,
        MISSING_PROGRAM,
        &["api", "rate_limit"],
        SpawnOptions::new("gh api rate_limit"),
    );
    let after = BudgetLedger::global().snapshot(Utc::now());
    assert_eq!(
        after.local["graphql"].calls_last_hour,
        before.local["graphql"].calls_last_hour + 1,
        "one GraphQL spawn must be recorded: {after:?}"
    );
    assert_eq!(
        after.local["core"].calls_last_hour, before.local["core"].calls_last_hour,
        "the free rate_limit probe must not be counted: {after:?}"
    );
}

// ── Issue #3928 AC-1: refusals back off exponentially, and the backoff is
//    persisted so a fresh gwtd process honours it before spawning ──

fn secondary_refusal(now: DateTime<Utc>) -> RateLimitBlock {
    // What `block_from_probe` produces for a secondary (per-minute) refusal:
    // the primary budget still has room, so no measured reset exists.
    RateLimitBlock {
        resource: "graphql".to_string(),
        limit: 0,
        remaining: 0,
        reset_at: now + Duration::seconds(60),
    }
}

#[test]
fn consecutive_refusals_back_off_exponentially_up_to_the_cap() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ledger = BudgetLedger::at(dir.path());
    let mut now = at(0);
    let mut waits = Vec::new();
    for _ in 0..6 {
        let effective = ledger.record_block(&secondary_refusal(now), now);
        waits.push(effective.retry_after_secs(now));
        // The next refusal arrives right after the window it was told to wait.
        now = effective.reset_at + Duration::seconds(1);
    }
    assert_eq!(
        waits,
        vec![60, 120, 240, 480, 900, 900],
        "1 → 2 → 4 → 8 minutes, capped at 15"
    );
    let block = ledger.snapshot(now).last_block.expect("block persisted");
    assert_eq!(block.consecutive_refusals, 6);
}

#[test]
fn a_measured_primary_exhaustion_keeps_its_probed_reset() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ledger = BudgetLedger::at(dir.path());
    ledger.record_block(&secondary_refusal(at(0)), at(0));
    let primary = RateLimitBlock {
        resource: "graphql".to_string(),
        limit: 5000,
        remaining: 0,
        reset_at: at(2400),
    };
    let effective = ledger.record_block(&primary, at(61));
    assert_eq!(
        effective.reset_at,
        at(2400),
        "GitHub's own window is authoritative; the schedule only fills in for unmeasured ones"
    );
}

#[test]
fn a_persisted_block_suppresses_graphql_spawns_in_a_fresh_process_but_not_rest() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ledger = BudgetLedger::at(dir.path());
    ledger.record_block(&secondary_refusal(at(0)), at(0));
    // A fresh process starts with an empty in-memory gate; only the ledger
    // knows about the refusal.
    let gate = gwt_core::github_quota::QuotaGate::default();

    let detail = github_budget::suppressed_spawn_detail(&gate, &ledger, &["issue", "list"], at(30))
        .expect("the persisted window suppresses the GraphQL spawn");
    assert!(detail.contains("github_rate_limited"), "{detail}");
    assert!(detail.contains("reset_at="), "{detail}");
    assert!(detail.contains("retry_after_secs=30"), "{detail}");
    assert!(
        github_budget::suppressed_spawn_detail(&gate, &ledger, &["api", "repos/o/r/pulls"], at(30))
            .is_none(),
        "REST keeps its own budget"
    );
    assert!(
        github_budget::suppressed_spawn_detail(&gate, &ledger, &["issue", "list"], at(61))
            .is_none(),
        "an elapsed window no longer suppresses"
    );
}

#[test]
fn a_successful_call_clears_the_persisted_block_and_restarts_the_schedule() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ledger = BudgetLedger::at(dir.path());
    ledger.record_block(&secondary_refusal(at(0)), at(0));
    ledger.record_block(&secondary_refusal(at(61)), at(61));
    assert_eq!(
        ledger
            .snapshot(at(62))
            .last_block
            .expect("block")
            .consecutive_refusals,
        2
    );

    ledger.clear_block(GitHubQuota::Rest);
    assert!(
        ledger.snapshot(at(62)).last_block.is_some(),
        "a success on another resource leaves the GraphQL window alone"
    );
    ledger.clear_block(GitHubQuota::GraphQl);
    assert!(ledger.snapshot(at(62)).last_block.is_none());
    assert!(ledger.active_block(GitHubQuota::GraphQl, at(62)).is_none());

    let effective = ledger.record_block(&secondary_refusal(at(300)), at(300));
    assert_eq!(
        effective.retry_after_secs(at(300)),
        60,
        "the schedule starts over"
    );
}

#[test]
fn a_refusal_long_after_the_last_window_starts_the_schedule_over() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ledger = BudgetLedger::at(dir.path());
    ledger.record_block(&secondary_refusal(at(0)), at(0));
    ledger.record_block(&secondary_refusal(at(61)), at(61));
    let much_later = at(61 + 120 + 2 * 15 * 60 + 1);
    let effective = ledger.record_block(&secondary_refusal(much_later), much_later);
    assert_eq!(effective.retry_after_secs(much_later), 60);
}

#[test]
fn a_persisted_refusal_is_annotated_and_recorded_through_observe_refusal() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ledger = BudgetLedger::at(dir.path());
    let gate = gwt_core::github_quota::QuotaGate::default();
    ledger.record_block(&secondary_refusal(at(0)), at(0));

    let annotated = github_budget::observe_refusal(
        &gate,
        &ledger,
        &["pr", "list", "--json", "number"],
        "GraphQL: API rate limit already exceeded for user ID 965624.",
        at(61),
        || None,
    )
    .expect("a rate-limit refusal is identified");
    assert!(annotated.contains("github_rate_limited"), "{annotated}");
    assert!(annotated.contains("retry_after_secs=120"), "{annotated}");
    assert!(annotated.contains("already exceeded"), "{annotated}");
    let gate_block = gate
        .active_block(GitHubQuota::GraphQl, at(62))
        .expect("the in-process gate learns the same window");
    assert_eq!(gate_block.reset_at, at(61 + 120));
    assert_eq!(
        github_budget::observe_refusal(
            &gate,
            &ledger,
            &["pr", "list"],
            "could not resolve to a PullRequest",
            at(62),
            || None,
        ),
        None,
        "an ordinary failure is left alone"
    );
}

/// The two budgets are independent (Issue #3604 measured GraphQL at 0/5000
/// while REST still had 4994). A refusal on one must not reopen the other's
/// window or reset its streak — during the incident both were under pressure,
/// and a REST refusal that wiped the GraphQL window let the Monitor resume
/// spawning straight back into the secondary limit.
#[test]
fn a_refusal_on_one_resource_leaves_the_other_resources_window_intact() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ledger = BudgetLedger::at(dir.path());
    ledger.record_block(&secondary_refusal(at(0)), at(0));
    let graphql = ledger.record_block(&secondary_refusal(at(61)), at(61));
    assert_eq!(graphql.retry_after_secs(at(61)), 120);

    let rest_refusal = RateLimitBlock {
        resource: "core".to_string(),
        limit: 0,
        remaining: 0,
        reset_at: at(90),
    };
    ledger.record_block(&rest_refusal, at(70));

    let still_open = ledger
        .active_block(GitHubQuota::GraphQl, at(100))
        .expect("the GraphQL window outlives a REST refusal");
    assert_eq!(still_open.reset_at, at(181));
    assert!(ledger.active_block(GitHubQuota::Rest, at(100)).is_some());

    // The GraphQL streak is untouched, so its next refusal waits 4 minutes.
    let next = ledger.record_block(&secondary_refusal(at(182)), at(182));
    assert_eq!(next.retry_after_secs(at(182)), 240);

    // Clearing one resource leaves the other's window in force.
    ledger.clear_block(GitHubQuota::Rest);
    assert!(ledger.active_block(GitHubQuota::Rest, at(100)).is_none());
    assert!(ledger.active_block(GitHubQuota::GraphQl, at(200)).is_some());

    let policy = ThrottlePolicy::default();
    let status = github_budget::status_by_resource(&ledger.snapshot(at(200)), &policy, at(200));
    assert!(status["graphql"].throttled);
    assert_eq!(status["graphql"].consecutive_refusals, 3);
    assert!(!status["core"].throttled);
}

// ── Issue #3928 AC-4: the per-minute count carries its sources ──

#[test]
fn spawn_sources_are_broken_down_per_minute() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ledger = BudgetLedger::at(dir.path());
    ledger.record_spawn_from(GitHubQuota::GraphQl, "gwt gh issue view", at(-120));
    for second in 0..3 {
        ledger.record_spawn_from(GitHubQuota::GraphQl, "gwt gh issue view", at(second));
    }
    ledger.record_spawn_from(GitHubQuota::GraphQl, "gwtd gh pr list", at(5));
    ledger.record_spawn_from(GitHubQuota::Rest, "gwtd gh api repos", at(6));
    ledger.record_spawn(GitHubQuota::GraphQl, at(7));

    let snapshot = ledger.snapshot(at(30));
    let graphql = &snapshot.local["graphql"];
    assert_eq!(graphql.calls_last_minute, 5);
    assert_eq!(graphql.calls_last_hour, 6);
    assert_eq!(graphql.sources_last_minute["gwt gh issue view"], 3);
    assert_eq!(graphql.sources_last_minute["gwtd gh pr list"], 1);
    assert_eq!(
        graphql.sources_last_minute["unknown"], 1,
        "a legacy record without a source still counts"
    );
    assert_eq!(
        snapshot.local["core"].sources_last_minute["gwtd gh api repos"],
        1
    );
}

#[test]
fn spawn_source_names_the_process_and_the_gh_command_shape() {
    let source = github_budget::spawn_source(&["issue", "view", "42", "--json", "body"]);
    assert!(source.ends_with(" gh issue view"), "{source}");
    assert!(
        github_budget::spawn_source(&["api", "repos/o/r/pulls?state=all"])
            .ends_with(" gh api repos")
    );
    assert!(
        github_budget::spawn_source(&["api", "graphql", "-f", "q"]).ends_with(" gh api graphql")
    );
    assert!(github_budget::spawn_source(&["--version"]).ends_with(" gh"));
}

#[test]
fn budget_status_names_the_throttle_the_backoff_and_the_sources() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ledger = BudgetLedger::at(dir.path());
    ledger.record_block(&secondary_refusal(at(0)), at(0));
    ledger.record_block(&secondary_refusal(at(61)), at(61));
    ledger.record_spawn_from(GitHubQuota::GraphQl, "gwt gh issue view", at(62));
    ledger.record_spawn_from(GitHubQuota::GraphQl, "gwt gh issue view", at(63));

    let policy = ThrottlePolicy::default();
    let status = github_budget::status_by_resource(&ledger.snapshot(at(90)), &policy, at(90));
    let graphql = &status["graphql"];
    assert!(graphql.throttled);
    assert!(graphql
        .reason
        .as_deref()
        .is_some_and(|reason| reason.contains("github_rate_limited")));
    assert_eq!(graphql.backoff_until, Some(at(61 + 120)));
    assert_eq!(graphql.retry_after_secs, Some(91));
    assert_eq!(graphql.consecutive_refusals, 2);
    assert_eq!(graphql.calls_last_minute, 2);
    assert_eq!(graphql.burst_limit, policy.burst_calls_per_minute);
    assert_eq!(graphql.sources_last_minute["gwt gh issue view"], 2);
    let core = &status["core"];
    assert!(!core.throttled);
    assert_eq!(core.backoff_until, None);
    assert_eq!(core.consecutive_refusals, 0);
}

// ── Issue #3928 AC-3: the cache resync paces itself under the burst limit ──

#[test]
fn burst_wait_keeps_the_next_call_strictly_under_the_per_minute_limit() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ledger = BudgetLedger::at(dir.path());
    let policy = ThrottlePolicy::default();
    for second in 0..58 {
        ledger.record_spawn(GitHubQuota::GraphQl, at(second));
    }
    assert_eq!(
        ledger.burst_wait(GitHubQuota::GraphQl, &policy, at(59)),
        None,
        "58 calls plus this one stay under 60"
    );
    ledger.record_spawn(GitHubQuota::GraphQl, at(58));
    assert_eq!(
        ledger.burst_wait(GitHubQuota::GraphQl, &policy, at(59)),
        Some(std::time::Duration::from_secs(2)),
        "wait until the oldest call leaves the window (plus one second of slack)"
    );
    ledger.record_spawn(GitHubQuota::GraphQl, at(59));
    assert_eq!(
        ledger.burst_wait(GitHubQuota::GraphQl, &policy, at(59)),
        Some(std::time::Duration::from_secs(3))
    );
    assert_eq!(
        ledger.burst_wait(GitHubQuota::Rest, &policy, at(59)),
        None,
        "another resource's burst does not pace this one"
    );
    assert_eq!(
        ledger.burst_wait(GitHubQuota::GraphQl, &policy, at(130)),
        None
    );
}
