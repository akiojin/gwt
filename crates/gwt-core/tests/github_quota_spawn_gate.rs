//! Issue #3604 AC-3 — an exhausted GraphQL budget suppresses the spawn itself.
//!
//! This test owns the process-global [`gwt_core::github_quota::global`] gate,
//! so it lives in its own test binary.

use chrono::{TimeZone, Utc};
use gwt_core::github_quota::{self, GitHubQuota, RateLimitBlock, RATE_LIMITED_ERROR_CODE};
use gwt_core::process_console::{
    spawn_logged_blocking, ProcessConsoleHub, ProcessKind, SpawnOptions,
};

/// A program name that certainly does not exist. If the gate fails to suppress
/// the call, the spawn surfaces a "not found" OS error instead of the
/// rate-limit detail — which is exactly the regression this test catches.
const MISSING_PROGRAM: &str = "gwt-nonexistent-gh-3604";

#[test]
fn exhausted_graphql_budget_suppresses_the_spawn_but_leaves_rest_open() {
    let hub = ProcessConsoleHub::new();
    let reset_at = Utc::now() + chrono::Duration::seconds(300);
    github_quota::global().record_exhaustion(RateLimitBlock {
        resource: "graphql".to_string(),
        limit: 5000,
        remaining: 0,
        reset_at,
    });

    // AC-3: a GraphQL-spending argv is refused without ever spawning.
    let error = spawn_logged_blocking(
        &hub,
        ProcessKind::Gh,
        MISSING_PROGRAM,
        &["pr", "view", "7", "--json", "headRefOid"],
        SpawnOptions::new("gh pr view"),
    )
    .expect_err("an exhausted GraphQL budget must refuse the call");
    let message = error.to_string();
    assert!(
        message.contains(RATE_LIMITED_ERROR_CODE),
        "suppressed spawn must report the rate-limit code, got: {message}"
    );
    // AC-2: the refusal explains when to retry.
    assert!(message.contains("reset_at="), "{message}");
    assert!(message.contains("retry_after_secs="), "{message}");
    assert!(
        !message.contains("No such file or directory"),
        "the process must not have been spawned at all, got: {message}"
    );

    // AC-4: REST kept quota in the observed incident, so it must stay open —
    // reaching the (missing) program proves the gate let it through.
    let rest = spawn_logged_blocking(
        &hub,
        ProcessKind::Gh,
        MISSING_PROGRAM,
        &["api", "repos/o/r/pulls?state=all"],
        SpawnOptions::new("gh api pulls"),
    );
    let rest_message = rest
        .err()
        .map(|error| error.to_string())
        .unwrap_or_default();
    assert!(
        !rest_message.contains(RATE_LIMITED_ERROR_CODE),
        "a GraphQL block must not suppress REST calls, got: {rest_message}"
    );

    github_quota::global().record_success(GitHubQuota::GraphQl);
}

#[test]
fn rate_limited_stderr_is_annotated_with_the_reset_window() {
    let now = Utc.timestamp_opt(1_755_280_000, 0).single().expect("now");
    let block = RateLimitBlock {
        resource: "graphql".to_string(),
        limit: 5000,
        remaining: 0,
        reset_at: Utc.timestamp_opt(1_755_280_163, 0).single().expect("reset"),
    };
    let annotated = github_quota::annotate_rate_limited_stderr(
        &block,
        "GraphQL: API rate limit already exceeded for user ID 965624.",
        now,
    );

    assert!(
        annotated.starts_with(RATE_LIMITED_ERROR_CODE),
        "{annotated}"
    );
    assert!(annotated.contains("retry_after_secs=163"), "{annotated}");
    assert!(
        annotated.contains("API rate limit already exceeded"),
        "the original gh message must be preserved: {annotated}"
    );
}

#[test]
fn suppression_detail_is_only_produced_for_the_exhausted_resource() {
    let gate = github_quota::QuotaGate::default();
    let now = Utc.timestamp_opt(1_755_280_000, 0).single().expect("now");
    gate.record_exhaustion(RateLimitBlock {
        resource: "graphql".to_string(),
        limit: 5000,
        remaining: 0,
        reset_at: Utc.timestamp_opt(1_755_280_163, 0).single().expect("reset"),
    });

    assert!(github_quota::suppressed_spawn_detail(&gate, &["pr", "list"], now).is_some());
    assert!(
        github_quota::suppressed_spawn_detail(&gate, &["api", "repos/o/r/pulls"], now).is_none()
    );
    assert!(github_quota::suppressed_spawn_detail(&gate, &["api", "rate_limit"], now).is_none());
}
