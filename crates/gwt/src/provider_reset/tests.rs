use std::{cell::Cell, collections::VecDeque};

use super::*;

fn snapshot(available: bool, allowed: Option<bool>) -> ResetSnapshot {
    ResetSnapshot {
        account_id: "account-1".into(),
        available_count: u64::from(available),
        credits: if available {
            vec![codex::ResetCredit {
                id: "free-credit-1".into(),
                reset_type: "codexRateLimits".into(),
                status: "available".into(),
            }]
        } else {
            vec![]
        },
        ordinary_usage_allowed: allowed,
    }
}

struct FakeProvider {
    reads: VecDeque<ResetSnapshot>,
    outcome: Result<ResetOutcome, String>,
    calls: usize,
}

impl FakeProvider {
    fn ready() -> Self {
        Self {
            reads: VecDeque::from([
                snapshot(true, Some(false)),
                snapshot(true, Some(false)),
                snapshot(false, Some(true)),
            ]),
            outcome: Ok(ResetOutcome::Reset),
            calls: 0,
        }
    }
}

impl ResetBackend for FakeProvider {
    fn read(&mut self) -> Result<ResetSnapshot, String> {
        self.reads.pop_front().ok_or("unexpected read".into())
    }

    fn consume(&mut self, key: &str, credit: &str) -> Result<ResetOutcome, String> {
        assert_eq!(key, "attempt-1");
        assert_eq!(credit, "free-credit-1");
        self.calls += 1;
        std::mem::replace(&mut self.outcome, Err("attempt reused".into()))
    }
}

fn auth_root() -> gwt_agent::CodexAuthRoot {
    gwt_agent::CodexAuthRoot {
        path: "/proven/codex-home".into(),
        origin: gwt_agent::CodexAuthRootOrigin::Profile,
    }
}

fn request(provider: &str) -> ResetRequest {
    ResetRequest {
        id: "attempt-1".into(),
        provider: provider.into(),
        window_id: "tab-1::agent-2".into(),
    }
}

#[test]
fn unsupported_provider_and_no_free_credit_never_prompt_or_consume() {
    for provider in ["claude", "unknown", "codex"] {
        let mut backend = FakeProvider::ready();
        backend.reads = VecDeque::from([snapshot(false, Some(false))]);
        let result = execute(
            &request(provider),
            &auth_root(),
            &mut backend,
            |_| panic!("must not prompt"),
            || Ok(()),
            || panic!("must keep hold"),
            &mut |_, _| Ok(()),
        );
        assert!(result.is_err());
        assert_eq!(backend.calls, 0);
    }
}

#[test]
fn cancel_preserves_hold_even_when_the_caller_is_autonomous() {
    let mut backend = FakeProvider::ready();
    let mut events = vec![];
    let result = execute(
        &request("codex"),
        &auth_root(),
        &mut backend,
        |prompt| {
            assert!(prompt.contains("tab-1::agent-2"));
            assert!(prompt.contains("無料"));
            assert!(prompt.contains("/proven/codex-home"));
            assert!(prompt.contains("PROFILE"));
            assert!(prompt.contains(CLAUDE_NOTICE));
            Ok(false)
        },
        || Ok(()),
        || panic!("must keep hold"),
        &mut |phase, _| {
            events.push(phase.to_owned());
            Ok(())
        },
    );
    assert!(result.unwrap_err().contains("declined"));
    assert_eq!(backend.calls, 0);
    assert!(events.iter().any(|event| event == "declined"));
}

#[test]
fn explicit_approval_consumes_once_and_only_verified_recovery_releases_hold() {
    let mut backend = FakeProvider::ready();
    let released = Cell::new(false);
    let mut events = vec![];
    execute(
        &request("codex"),
        &auth_root(),
        &mut backend,
        |_| Ok(true),
        || Ok(()),
        || {
            released.set(true);
            Ok(())
        },
        &mut |phase, _| {
            events.push(phase.to_owned());
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(backend.calls, 1);
    assert!(released.get());
    assert_eq!(
        events,
        [
            "proposed",
            "approved",
            "executing",
            "provider_result",
            "completed"
        ]
    );

    // A new operation must get its own consent; an audit row is not an approval token.
    let mut next = FakeProvider::ready();
    assert!(execute(
        &request("codex"),
        &auth_root(),
        &mut next,
        |_| Ok(false),
        || Ok(()),
        || panic!("old approval reused"),
        &mut |_, _| Ok(())
    )
    .is_err());
    assert_eq!(next.calls, 0);
}

#[test]
fn provider_failure_or_unconfirmed_recovery_never_clears_hold() {
    for outcome in [
        Ok(ResetOutcome::NoCredit),
        Ok(ResetOutcome::NothingToReset),
        Err("transport failed".into()),
        Ok(ResetOutcome::Reset),
    ] {
        let mut backend = FakeProvider::ready();
        backend.outcome = outcome;
        backend.reads[2].ordinary_usage_allowed = None;
        let mut events = vec![];
        assert!(execute(
            &request("codex"),
            &auth_root(),
            &mut backend,
            |_| Ok(true),
            || Ok(()),
            || panic!("must keep hold"),
            &mut |phase, _| {
                events.push(phase.to_owned());
                Ok(())
            }
        )
        .is_err());
        assert_eq!(events.last().map(String::as_str), Some("failed"));
    }
}

#[test]
fn stale_target_account_or_audit_failure_prevents_consume() {
    for failure in ["target", "account", "audit"] {
        let mut backend = FakeProvider::ready();
        if failure == "account" {
            backend.reads[1].account_id = "different-account".into();
        }
        assert!(execute(
            &request("codex"),
            &auth_root(),
            &mut backend,
            |_| Ok(true),
            || if failure == "target" {
                Err("target changed".into())
            } else {
                Ok(())
            },
            || panic!("must keep hold"),
            &mut |phase, _| if failure == "audit" && phase == "approved" {
                Err("audit failed".into())
            } else {
                Ok(())
            }
        )
        .is_err());
        assert_eq!(backend.calls, 0);
    }
}

#[test]
fn final_audit_failure_does_not_reverse_a_completed_reset() {
    let mut backend = FakeProvider::ready();
    let released = Cell::new(false);
    let mut events = vec![];
    let result = execute(
        &request("codex"),
        &auth_root(),
        &mut backend,
        |_| Ok(true),
        || Ok(()),
        || {
            released.set(true);
            Ok(())
        },
        &mut |phase, _| {
            events.push(phase.to_owned());
            if phase == "completed" {
                Err("disk full".into())
            } else {
                Ok(())
            }
        },
    );
    assert!(
        result.is_ok(),
        "reset and release already succeeded: {result:?}"
    );
    assert_eq!(result.unwrap().audit_warning.as_deref(), Some("disk full"));
    assert!(released.get());
    assert_eq!(backend.calls, 1);
    assert!(!events.iter().any(|phase| phase == "failed"));
}

#[test]
fn proposals_use_configurable_wait_and_never_offer_claude_paid_usage() {
    let now = chrono::DateTime::parse_from_rfc3339("2026-10-01T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let holds = std::collections::BTreeMap::from([
        ("codex".into(), "2026-10-02T00:00:00Z".into()),
        ("claude".into(), "2026-10-02T00:00:00Z".into()),
    ]);
    let rows = proposals(&holds, now, 3600);
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows.iter().find(|r| r.provider == "codex").unwrap().action,
        "check_free_reset"
    );
    let claude = rows.iter().find(|r| r.provider == "claude").unwrap();
    assert_eq!(claude.action, "switch_provider");
    assert!(claude.message.contains(CLAUDE_NOTICE));
    assert!(!proposals(&holds, now, 172800)
        .iter()
        .any(|r| r.provider == "codex"));
}
