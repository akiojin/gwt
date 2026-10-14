//! Issue #3928 AC-1 — a rate-limit refusal persisted by one process suppresses
//! the GraphQL spawns of the next one.
//!
//! `gwtd` is one short-lived process per command, so the in-process
//! [`gwt_core::github_quota::global`] gate is empty on every start. Without
//! the persisted window each new process re-spawned `gh` straight into the
//! same secondary limit. This test owns the process-global gate and the
//! machine-local ledger of its own test process, so it lives in its own binary.

use chrono::Utc;
use gwt_core::github_budget::BudgetLedger;
use gwt_core::github_quota::{self, GitHubQuota, RateLimitBlock, RATE_LIMITED_ERROR_CODE};
use gwt_core::process_console::{
    spawn_logged_blocking, ProcessConsoleHub, ProcessKind, SpawnOptions,
};

/// A program name that certainly does not exist: reaching it proves the gate
/// let the call through.
const MISSING_PROGRAM: &str = "gwt-nonexistent-gh-3928";

#[test]
fn a_refusal_persisted_by_another_process_suppresses_graphql_spawns_but_not_rest() {
    let hub = ProcessConsoleHub::new();
    let now = Utc::now();
    // The previous process observed a secondary refusal and persisted it.
    let effective = BudgetLedger::global().record_block(
        &RateLimitBlock {
            resource: "graphql".to_string(),
            limit: 0,
            remaining: 0,
            reset_at: now + chrono::Duration::seconds(60),
        },
        now,
    );
    assert!(
        github_quota::global()
            .active_block(GitHubQuota::GraphQl, now)
            .is_none(),
        "this process's in-memory gate knows nothing yet"
    );

    let error = spawn_logged_blocking(
        &hub,
        ProcessKind::Gh,
        MISSING_PROGRAM,
        &["issue", "list", "--json", "number"],
        SpawnOptions::new("gh issue list"),
    )
    .expect_err("the persisted window must refuse the GraphQL spawn");
    let message = error.to_string();
    assert!(message.contains(RATE_LIMITED_ERROR_CODE), "{message}");
    assert!(
        message.contains(&format!(
            "reset_at={}",
            effective
                .reset_at
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        )),
        "{message}"
    );
    assert!(
        !message.contains("No such file or directory"),
        "the process must not have been spawned at all, got: {message}"
    );

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
        "a GraphQL window must not suppress REST calls, got: {rest_message}"
    );

    BudgetLedger::global().clear_block(GitHubQuota::GraphQl);
}
