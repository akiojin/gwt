//! `github.budget` — observe the GitHub API budget (Issue #3891 AC-3).
//!
//! Renders the primary budgets GitHub reports (`gh api rate_limit`, a free
//! endpoint), the machine-local approximation of the secondary limit, the
//! newest observed refusal, and the throttle decision a periodic read would
//! get right now — so a PM can see *why* `pr.list` was thinned out and an
//! operator can see the budget before it is gone.

use chrono::{DateTime, Utc};
use gwt_core::github_budget::{self, BudgetLedger, ThrottlePolicy};
use gwt_core::github_quota::GitHubQuota;
use gwt_github::SpecOpsError;

use crate::cli::CliEnv;

pub(super) fn run<E: CliEnv>(
    env: &mut E,
    refresh: bool,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    run_with(env, refresh, &BudgetLedger::global(), Utc::now(), out)
}

fn run_with<E: CliEnv>(
    env: &mut E,
    refresh: bool,
    ledger: &BudgetLedger,
    now: DateTime<Utc>,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let policy = ThrottlePolicy::default();
    let mut snapshot = ledger.snapshot(now);
    let mut probe_error = None;
    if refresh || github_budget::probe_is_stale(&snapshot, &policy) {
        match env.probe_github_rate_limit() {
            Ok(payload) => match github_budget::parse_rate_limit_probe_all(&payload, now) {
                Some(probe) => {
                    ledger.record_probe(&probe);
                    snapshot = ledger.snapshot(now);
                }
                None => probe_error = Some("gh api rate_limit: unparsable payload".to_string()),
            },
            Err(error) => probe_error = Some(error.to_string()),
        }
    }

    let throttle: serde_json::Map<String, serde_json::Value> =
        [GitHubQuota::GraphQl, GitHubQuota::Rest]
            .into_iter()
            .filter_map(|quota| {
                let resource = quota.resource_name()?;
                let reason = github_budget::throttle_reason(&snapshot, quota, &policy, now);
                Some((resource.to_string(), serde_json::json!(reason)))
            })
            .collect();
    let payload = serde_json::json!({
        "taken_at": snapshot.taken_at,
        "primary": snapshot.probe.as_ref().map(|probe| &probe.resources),
        "probe_at": snapshot.probe.as_ref().map(|probe| probe.probed_at),
        "probe_age_secs": snapshot.probe_age_secs,
        "probe_error": probe_error,
        "secondary_estimate": snapshot.local,
        "secondary_note": snapshot.secondary_note,
        "last_block": snapshot.last_block,
        "throttle": throttle,
        "policy": {
            "reserve_fraction": policy.reserve_fraction,
            "burst_calls_per_minute": policy.burst_calls_per_minute,
            "probe_max_age_secs": policy.probe_max_age_secs,
        },
        "ledger_dir": ledger.dir().display().to_string(),
    });
    match serde_json::to_string_pretty(&payload) {
        Ok(rendered) => {
            out.push_str(&rendered);
            out.push('\n');
        }
        Err(error) => out.push_str(&format!("github.budget JSON: {error}\n")),
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(seconds: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_756_800_000 + seconds, 0)
            .single()
            .expect("timestamp")
    }

    fn payload(graphql_remaining: u64) -> String {
        format!(
            r#"{{"resources":{{"core":{{"limit":5000,"remaining":4990,"reset":1756803600}},
                "graphql":{{"limit":5000,"remaining":{graphql_remaining},"reset":1756802400}}}}}}"#
        )
    }

    #[test]
    fn budget_renders_primary_secondary_and_throttle_decisions() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(tmp.path().to_path_buf());
        env.github_rate_limit_payload = Some(payload(300));
        let ledger = BudgetLedger::at(&tmp.path().join("budget"));
        ledger.record_spawn(GitHubQuota::GraphQl, at(-10));

        let mut out = String::new();
        let code = run_with(&mut env, false, &ledger, at(0), &mut out).expect("run");
        assert_eq!(code, 0);
        assert_eq!(
            env.github_rate_limit_probe_count, 1,
            "a missing probe is refreshed through the free endpoint"
        );
        let value: serde_json::Value = serde_json::from_str(&out).expect("json");
        assert_eq!(value["primary"]["graphql"]["remaining"], 300);
        assert_eq!(value["primary"]["core"]["remaining"], 4990);
        assert_eq!(value["probe_age_secs"], 0);
        assert_eq!(
            value["secondary_estimate"]["graphql"]["calls_last_minute"],
            1
        );
        assert!(value["secondary_note"]
            .as_str()
            .expect("note")
            .contains("approximat"));
        let reason = value["throttle"]["graphql"].as_str().expect("throttled");
        assert!(reason.contains("budget_reserve"), "{reason}");
        assert!(value["throttle"]["core"].is_null(), "{out}");
        assert!(value["probe_error"].is_null(), "{out}");
    }

    #[test]
    fn fresh_probe_is_reused_unless_refresh_is_requested() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(tmp.path().to_path_buf());
        env.github_rate_limit_payload = Some(payload(4800));
        let ledger = BudgetLedger::at(&tmp.path().join("budget"));

        let mut out = String::new();
        run_with(&mut env, false, &ledger, at(0), &mut out).expect("first");
        run_with(&mut env, false, &ledger, at(60), &mut out).expect("second");
        assert_eq!(env.github_rate_limit_probe_count, 1);
        run_with(&mut env, true, &ledger, at(61), &mut out).expect("refresh");
        assert_eq!(env.github_rate_limit_probe_count, 2);
    }

    #[test]
    fn probe_failure_is_reported_not_fatal() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(tmp.path().to_path_buf());
        let ledger = BudgetLedger::at(&tmp.path().join("budget"));
        let mut out = String::new();
        let code = run_with(&mut env, false, &ledger, at(0), &mut out).expect("run");
        assert_eq!(code, 0);
        let value: serde_json::Value = serde_json::from_str(&out).expect("json");
        assert!(value["primary"].is_null(), "{out}");
        assert!(value["probe_error"].is_string(), "{out}");
        assert!(value["throttle"]["graphql"].is_null(), "{out}");
    }
}
