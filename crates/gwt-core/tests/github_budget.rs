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
