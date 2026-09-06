//! Issue #3604 — GitHub API quota classification, rate-limit identification,
//! and the pre-spawn suppression gate.
//!
//! AC-1: a rate-limited `gh` failure is identifiable as such (not flattened
//!       into a generic network error).
//! AC-2: the identified failure carries the absolute reset time and the
//!       remaining seconds until that reset.
//! AC-3: while the recorded reset time is in the future, further calls against
//!       the exhausted resource are suppressed instead of re-spawning `gh`.
//! AC-4: REST-backed invocations stay available while GraphQL is exhausted.

use chrono::{DateTime, TimeZone, Utc};
use gwt_core::github_quota::{
    self, GitHubQuota, RateLimitBlock, RATE_LIMITED_ERROR_CODE, RATE_LIMIT_PROBE_ARGS,
};

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(seconds, 0).single().expect("timestamp")
}

// ── AC-4 / gating scope: which quota does an argv consume? ──

#[test]
fn pr_and_issue_reads_are_classified_as_graphql() {
    for args in [
        vec!["pr", "list", "--json", "number,title"],
        vec!["pr", "view", "7", "--json", "headRefOid"],
        vec!["pr", "view", "--json", "statusCheckRollup"],
        vec!["pr", "diff", "7"],
        vec!["pr", "checks", "7"],
        vec!["pr", "merge", "7", "--disable-auto"],
        vec!["issue", "list", "--json", "number"],
        vec!["issue", "view", "3604", "--json", "body"],
        vec!["search", "issues", "rate limit"],
        vec!["api", "graphql", "-f", "query=query{viewer{login}}"],
    ] {
        assert_eq!(
            github_quota::classify_gh_args(&args),
            GitHubQuota::GraphQl,
            "expected GraphQL classification for gh {}",
            args.join(" ")
        );
    }
}

#[test]
fn rest_api_paths_are_classified_as_rest() {
    for args in [
        vec!["api", "repos/{owner}/{repo}/pulls?state=all&per_page=20"],
        vec!["api", "-X", "GET", "repos/o/r/commits/abc/status"],
        vec!["api", "--paginate", "repos/o/r/pulls"],
    ] {
        assert_eq!(
            github_quota::classify_gh_args(&args),
            GitHubQuota::Rest,
            "expected REST classification for gh {}",
            args.join(" ")
        );
    }
}

#[test]
fn rate_limit_probe_and_auth_are_free() {
    assert_eq!(
        github_quota::classify_gh_args(RATE_LIMIT_PROBE_ARGS),
        GitHubQuota::Free
    );
    assert_eq!(
        github_quota::classify_gh_args(&["api", "rate_limit"]),
        GitHubQuota::Free
    );
    assert_eq!(
        github_quota::classify_gh_args(&["auth", "status"]),
        GitHubQuota::Free
    );
    assert_eq!(
        github_quota::classify_gh_args(&["--version"]),
        GitHubQuota::Free
    );
}

#[test]
fn unknown_subcommands_are_never_gated() {
    // Conservative default: an argv we do not recognise must not be suppressed
    // by the GraphQL gate.
    assert_ne!(
        github_quota::classify_gh_args(&["some-extension", "run"]),
        GitHubQuota::GraphQl
    );
}

// ── AC-1: identify a rate-limit failure ──

#[test]
fn graphql_exhaustion_stderr_is_identified_as_rate_limited() {
    let stderr = "GraphQL: API rate limit already exceeded for user ID 965624. \
        (repository.pullRequest)";
    assert!(github_quota::is_rate_limit_stderr(stderr));
}

#[test]
fn rest_and_secondary_rate_limit_stderr_are_identified() {
    for stderr in [
        "HTTP 403: API rate limit exceeded for user ID 965624. (https://api.github.com/repos/o/r)",
        "You have exceeded a secondary rate limit. Please wait a few minutes before you try again.",
        "HTTP 429: Too Many Requests",
    ] {
        assert!(
            github_quota::is_rate_limit_stderr(stderr),
            "expected rate-limit identification for: {stderr}"
        );
    }
}

#[test]
fn ordinary_gh_failures_are_not_identified_as_rate_limited() {
    for stderr in [
        "no pull requests found for branch \"work/issue-3604\"",
        "could not resolve to a PullRequest with the number of 999999",
        "error connecting to api.github.com: dial tcp: lookup api.github.com: no such host",
        "gh: Not Found (HTTP 404)",
    ] {
        assert!(
            !github_quota::is_rate_limit_stderr(stderr),
            "expected NO rate-limit identification for: {stderr}"
        );
    }
}

// ── AC-2: reset time + remaining seconds ──

#[test]
fn rate_limit_probe_payload_parses_per_resource_snapshots() {
    let payload = r#"{
      "resources": {
        "core":    {"limit": 5000, "used": 6,    "remaining": 4994, "reset": 1755280000},
        "graphql": {"limit": 5000, "used": 5000, "remaining": 0,    "reset": 1755280163}
      }
    }"#;

    let graphql = github_quota::parse_rate_limit_probe(payload, GitHubQuota::GraphQl)
        .expect("graphql snapshot");
    assert_eq!(graphql.resource, "graphql");
    assert_eq!(graphql.limit, 5000);
    assert_eq!(graphql.remaining, 0);
    assert_eq!(graphql.reset_at, at(1755280163));

    let core =
        github_quota::parse_rate_limit_probe(payload, GitHubQuota::Rest).expect("core snapshot");
    assert_eq!(core.resource, "core");
    assert_eq!(core.remaining, 4994);
    assert_eq!(core.reset_at, at(1755280000));
}

#[test]
fn rate_limit_detail_carries_error_code_reset_time_and_retry_after() {
    let block = RateLimitBlock {
        resource: "graphql".to_string(),
        limit: 5000,
        remaining: 0,
        reset_at: at(1755280163),
    };
    let detail = block.detail(at(1755280000));

    assert!(
        detail.starts_with(RATE_LIMITED_ERROR_CODE),
        "detail must lead with the machine-readable error code: {detail}"
    );
    assert!(detail.contains("resource=graphql"), "{detail}");
    assert!(detail.contains("limit=5000"), "{detail}");
    assert!(detail.contains("remaining=0"), "{detail}");
    assert!(
        detail.contains("reset_at=2025-08-15T17:49:23Z"),
        "detail must carry the absolute reset time: {detail}"
    );
    assert!(
        detail.contains("retry_after_secs=163"),
        "detail must carry the remaining seconds: {detail}"
    );
}

#[test]
fn retry_after_never_reports_negative_seconds_once_the_window_reset() {
    let block = RateLimitBlock {
        resource: "graphql".to_string(),
        limit: 5000,
        remaining: 0,
        reset_at: at(1755280163),
    };
    assert_eq!(block.retry_after_secs(at(1755280200)), 0);
}

#[test]
fn a_secondary_rate_limit_waits_minutes_not_the_whole_hourly_window() {
    // GitHub refuses bursts with a secondary rate limit even when the hourly
    // budget is untouched. Recording the far-away window reset in that case
    // would idle every GraphQL call for up to an hour over a few-second burst,
    // so a probe that still reports budget means "back off briefly" instead.
    let probe = Some(RateLimitBlock {
        resource: "graphql".to_string(),
        limit: 5000,
        remaining: 4300,
        reset_at: at(1755283000),
    });
    let block = github_quota::block_from_probe(GitHubQuota::GraphQl, probe, at(1755280000));

    assert!(
        block.reset_at < at(1755283000),
        "a secondary rate limit must not inherit the hourly window reset: {:?}",
        block.reset_at
    );
    assert!(block.reset_at > at(1755280000));
}

#[test]
fn probe_failure_still_yields_an_identified_block_with_a_fallback_window() {
    // The probe itself can fail (offline, gh missing). AC-1 must still hold:
    // the failure is identified as rate limiting rather than a network error,
    // and AC-2's reset time falls back to the conservative GitHub window.
    let block = github_quota::block_from_probe(GitHubQuota::GraphQl, None, at(1755280000));
    assert_eq!(block.resource, "graphql");
    assert!(block.reset_at > at(1755280000));
    assert!(block
        .detail(at(1755280000))
        .contains(RATE_LIMITED_ERROR_CODE));
}

// ── AC-3 / AC-4: the suppression gate ──

#[test]
fn graphql_block_suppresses_graphql_but_not_rest_or_free() {
    let gate = github_quota::QuotaGate::default();
    gate.record_exhaustion(RateLimitBlock {
        resource: "graphql".to_string(),
        limit: 5000,
        remaining: 0,
        reset_at: at(1755280163),
    });

    let blocked = gate
        .active_block(GitHubQuota::GraphQl, at(1755280000))
        .expect("graphql must be suppressed before reset");
    assert_eq!(blocked.reset_at, at(1755280163));

    // AC-4: REST still has quota in the observed incident, so it must stay open.
    assert!(gate
        .active_block(GitHubQuota::Rest, at(1755280000))
        .is_none());
    assert!(gate
        .active_block(GitHubQuota::Free, at(1755280000))
        .is_none());
}

#[test]
fn block_expires_exactly_at_the_reset_time() {
    let gate = github_quota::QuotaGate::default();
    gate.record_exhaustion(RateLimitBlock {
        resource: "graphql".to_string(),
        limit: 5000,
        remaining: 0,
        reset_at: at(1755280163),
    });

    assert!(gate
        .active_block(GitHubQuota::GraphQl, at(1755280162))
        .is_some());
    assert!(gate
        .active_block(GitHubQuota::GraphQl, at(1755280163))
        .is_none());
}

#[test]
fn rest_exhaustion_suppresses_rest_without_touching_graphql() {
    let gate = github_quota::QuotaGate::default();
    gate.record_exhaustion(RateLimitBlock {
        resource: "core".to_string(),
        limit: 5000,
        remaining: 0,
        reset_at: at(1755280163),
    });

    assert!(gate
        .active_block(GitHubQuota::Rest, at(1755280000))
        .is_some());
    assert!(gate
        .active_block(GitHubQuota::GraphQl, at(1755280000))
        .is_none());
}

#[test]
fn a_later_reset_extends_the_block_and_an_earlier_one_does_not_shorten_it() {
    let gate = github_quota::QuotaGate::default();
    gate.record_exhaustion(RateLimitBlock {
        resource: "graphql".to_string(),
        limit: 5000,
        remaining: 0,
        reset_at: at(1755280163),
    });
    gate.record_exhaustion(RateLimitBlock {
        resource: "graphql".to_string(),
        limit: 5000,
        remaining: 0,
        reset_at: at(1755280100),
    });
    assert_eq!(
        gate.active_block(GitHubQuota::GraphQl, at(1755280000))
            .expect("still blocked")
            .reset_at,
        at(1755280163),
        "an earlier reset must not shorten an active block"
    );

    gate.record_exhaustion(RateLimitBlock {
        resource: "graphql".to_string(),
        limit: 5000,
        remaining: 0,
        reset_at: at(1755280500),
    });
    assert_eq!(
        gate.active_block(GitHubQuota::GraphQl, at(1755280000))
            .expect("still blocked")
            .reset_at,
        at(1755280500)
    );
}

#[test]
fn a_successful_call_after_reset_clears_the_recorded_block() {
    let gate = github_quota::QuotaGate::default();
    gate.record_exhaustion(RateLimitBlock {
        resource: "graphql".to_string(),
        limit: 5000,
        remaining: 0,
        reset_at: at(1755280163),
    });
    gate.record_success(GitHubQuota::GraphQl);
    assert!(gate
        .active_block(GitHubQuota::GraphQl, at(1755280000))
        .is_none());
}
