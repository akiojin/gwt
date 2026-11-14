//! Resident-loop driver for the PM pane (SPEC-3431 FR-012 / FR-035).
//!
//! The PM's contract describes an unattended subscribe → reconcile loop, but a
//! contract cannot make an agent take another turn: after its bootstrap turn
//! the PM simply stopped, and nothing in gwt ever woke it again — observed
//! live as "the PM runs once and is done". This Stop gate is the driving
//! mechanism: while the Issue Monitor is enabled for the project, a stopping
//! PM is turned around into the next loop cycle.
//!
//! The brakes live here, in the mechanism, not in guidance an unattended LLM
//! may not follow (same policy as the Monitor's claim/backoff):
//! - a floor between continuations, so a misbehaving cycle cannot spin;
//! - a cap on consecutive continuations without any user contact, after which
//!   the PM parks silently (a user prompt resets the budget). Waking a parked
//!   PM on daemon events is the follow-up wake path.

use std::path::{Path, PathBuf};

use super::HookOutput;
use crate::pm_registry;

// The floor between continuations and the subscribe timeout both come from
// `PmSettings::loop_interval_secs` (FR-035, default 60s): one knob, because a
// floor shorter than the wait would never fire and a longer one would skip
// cycles.

/// Consecutive continuations without user contact before the PM parks.
/// At the default 60s `loop_interval_secs` this is ~12 minutes of unattended
/// residency after the last conversation; the daemon wake path (T-093)
/// revives a parked PM when new monitor activity arrives.
const PM_LOOP_MAX_CONSECUTIVE: u32 = 12;

/// UserPromptSubmit entry: real user contact re-arms the loop budget and
/// stamps the conversation clock the T-093 wake path defers to.
pub fn handle_user_prompt_submit(worktree: &Path) -> bool {
    handle_user_prompt_submit_with_input(worktree, "{}")
}

/// UserPromptSubmit entry with the hook envelope. The compatibility wrapper
/// above keeps existing callers source-compatible until the dispatcher passes
/// the input through; only this path can acknowledge a protected delivery.
pub fn handle_user_prompt_submit_with_input(worktree: &Path, input: &str) -> bool {
    handle_user_prompt_submit_at(
        worktree,
        input,
        &chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    )
}

fn handle_user_prompt_submit_at(worktree: &Path, input: &str, now: &str) -> bool {
    let delivery_evidence = delivery_evidence_from_input(input);
    let mut rejects_protected_delivery =
        matches!(delivery_evidence, Some(DeliveryEvidence::Rejected { .. }));
    if let Some(evidence) = delivery_evidence.clone() {
        let persisted =
            current_delivery_receipt_context(worktree).is_some_and(|(receipt_path, session_id)| {
                record_pm_delivery_evidence_at(&receipt_path, &session_id, evidence, now)
                    .is_ok_and(|status| status == pm_registry::PmDeliveryReceiptStatus::Verified)
            });
        if !persisted {
            rejects_protected_delivery = true;
            tracing::warn!("protected pane delivery acknowledgement was not durably accepted");
        }
    }
    if !super::is_resident_pm_worktree(worktree) {
        return rejects_protected_delivery;
    }
    let Some(state_path) = pm_registry::pm_loop_state_path_for_pm_worktree(worktree) else {
        return rejects_protected_delivery;
    };
    if let Err(error) = pm_registry::mutate_pm_loop_state(&state_path, |state| {
        state.consecutive_continuations = 0;
        state.last_continued_at = None;
        state.last_user_prompt_at = Some(now.to_string());
        state.pending_own_block = false;
        let current_session_id = std::env::var(gwt_agent::GWT_SESSION_ID_ENV).ok();
        match delivery_evidence {
            Some(DeliveryEvidence::Exact {
                operation_id,
                body_sha256,
            }) if !rejects_protected_delivery
                && state.pending_delivery_operation_id.as_deref()
                    == Some(operation_id.as_str())
                && state.pending_delivery_session_id.as_deref()
                    == current_session_id.as_deref()
                && state.pending_delivery_body_sha256.as_deref() == Some(body_sha256.as_str()) =>
            {
                state.last_delivery_operation_id = Some(operation_id);
                state.last_delivery_submitted_at = Some(now.to_string());
                state.last_wake_at = Some(now.to_string());
                state.last_delivery_rejected_operation_id = None;
            }
            Some(DeliveryEvidence::Exact { operation_id, .. })
            | Some(DeliveryEvidence::Rejected { operation_id, .. }) => {
                state.last_delivery_rejected_operation_id = Some(operation_id);
            }
            None => {}
        }
    }) {
        tracing::warn!(%error, "failed to persist resident PM UserPromptSubmit acknowledgement");
    }
    rejects_protected_delivery
}

const DELIVERY_MARKER_PREFIX: &str = "[gwt-delivery:";
const CANONICAL_UUID_LEN: usize = 36;
const SHA256_HEX_LEN: usize = 64;
const DELIVERY_MARKER_LEN: usize =
    DELIVERY_MARKER_PREFIX.len() + CANONICAL_UUID_LEN + 1 + SHA256_HEX_LEN + 1;

#[derive(Clone)]
enum DeliveryEvidence {
    Exact {
        operation_id: String,
        body_sha256: String,
    },
    Rejected {
        operation_id: String,
        body_sha256: String,
    },
}

fn current_delivery_receipt_context(worktree: &Path) -> Option<(PathBuf, String)> {
    let session_id = std::env::var(gwt_agent::GWT_SESSION_ID_ENV).ok()?;
    let sessions_dir = std::env::var_os(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV)
        .map(PathBuf::from)
        .and_then(|path| gwt_agent::sessions_dir_from_runtime_path(&path))
        .unwrap_or_else(gwt_core::paths::gwt_sessions_dir);
    let session =
        gwt_agent::Session::load_and_migrate(&sessions_dir.join(format!("{session_id}.toml")))
            .ok()?;
    if session.id != session_id {
        return None;
    }
    let project_root = session
        .project_state_root
        .or_else(|| gwt_git::worktree::main_worktree_root(worktree).ok())?;
    Some((
        pm_registry::pm_delivery_receipts_path_for_repo_path(&project_root),
        session_id,
    ))
}

fn record_pm_delivery_evidence_at(
    receipt_path: &Path,
    target_session_id: &str,
    evidence: DeliveryEvidence,
    _now: &str,
) -> std::io::Result<pm_registry::PmDeliveryReceiptStatus> {
    match evidence {
        DeliveryEvidence::Exact {
            operation_id,
            body_sha256,
        } => pm_registry::finish_pm_delivery_receipt(
            receipt_path,
            &operation_id,
            target_session_id,
            &body_sha256,
            pm_registry::PmDeliveryReceiptStatus::Verified,
            None,
        ),
        DeliveryEvidence::Rejected {
            operation_id,
            body_sha256,
        } => pm_registry::finish_pm_delivery_receipt(
            receipt_path,
            &operation_id,
            target_session_id,
            &body_sha256,
            pm_registry::PmDeliveryReceiptStatus::Ambiguous,
            Some("composer_hash_mismatch"),
        ),
    }
}

fn delivery_evidence_from_input(input: &str) -> Option<DeliveryEvidence> {
    let envelope: serde_json::Value = serde_json::from_str(input).ok()?;
    let prompt = envelope.get("prompt")?.as_str()?;
    delivery_evidence_from_prompt(prompt)
}

fn delivery_evidence_from_prompt(prompt: &str) -> Option<DeliveryEvidence> {
    // Inspect one fixed-size suffix only: never copy, persist, or scan the
    // untrusted prompt body looking for marker-like text.
    let marker = prompt.get(prompt.len().checked_sub(DELIVERY_MARKER_LEN)?..)?;
    let (operation_id, expected_sha256) = marker
        .strip_prefix(DELIVERY_MARKER_PREFIX)?
        .strip_suffix(']')?
        .split_once(':')?;
    let parsed = uuid::Uuid::parse_str(operation_id).ok()?;
    if parsed.hyphenated().to_string() != operation_id
        || expected_sha256.len() != SHA256_HEX_LEN
        || !expected_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return None;
    }
    let body = prompt
        .get(..prompt.len().checked_sub(DELIVERY_MARKER_LEN)?)?
        .strip_suffix(' ')?;
    if pm_registry::pm_delivery_prompt_sha256(body) == expected_sha256 {
        Some(DeliveryEvidence::Exact {
            operation_id: operation_id.to_string(),
            body_sha256: expected_sha256.to_string(),
        })
    } else {
        Some(DeliveryEvidence::Rejected {
            operation_id: operation_id.to_string(),
            body_sha256: expected_sha256.to_string(),
        })
    }
}

pub fn handle_with_input(worktree: &Path, input: &str) -> HookOutput {
    handle_at(
        worktree,
        &chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        super::envelope::stop_hook_active_from(input),
    )
}

fn handle_at(worktree: &Path, now: &str, stop_hook_active: bool) -> HookOutput {
    if !super::is_resident_pm_worktree(worktree) {
        return HookOutput::Silent;
    }
    let Some(state_path) = pm_registry::pm_loop_state_path_for_pm_worktree(worktree) else {
        return HookOutput::Silent;
    };
    // The loop only runs while the Monitor is on: with it off there is nothing
    // for the PM to reconcile against, and the user has explicitly parked
    // autonomous work.
    let Some(project_state) = state_path.parent() else {
        return HookOutput::Silent;
    };
    let prefs_path = project_state.join("issue-monitor.json");
    let monitor_prefs = crate::load_issue_monitor_prefs(&prefs_path).ok();
    let monitor_enabled = monitor_prefs
        .as_ref()
        .map(|prefs| prefs.enabled)
        .unwrap_or(false);
    // FR-110 (T-204): a cycle is only "empty" when the durable monitor state
    // holds nothing a supervisor still has to look at. While launches run,
    // escalations wait, or failures sit undigested, the park counter holds —
    // matching the Stop text's "repeated empty cycles" instead of retiring a
    // PM that plainly has supervision work.
    let has_unconsumed_observations = monitor_prefs
        .as_ref()
        .map(|prefs| {
            let monitor = crate::IssueMonitorState::with_prefs(
                crate::IssueMonitorConfig::default(),
                prefs.clone(),
            );
            let status = monitor.agent_status();
            !status.active_launches.is_empty()
                || !status.needs_human.is_empty()
                || status.inbox.iter().any(|row| row.error_message.is_some())
        })
        .unwrap_or(false);
    let interval_secs = pm_registry::load_pm_prefs(&project_state.join("pm.json"))
        .map(|prefs| prefs.settings.loop_interval_secs_clamped())
        .unwrap_or(60);
    let should_continue = pm_registry::mutate_pm_loop_state(&state_path, |state| {
        // A `stop_hook_active` chain the loop did not start belongs to another
        // Stop gate — riding it would stack this loop's directive on top of
        // that gate's forced continuation.
        if stop_hook_active && !state.pending_own_block {
            return false;
        }
        // Every other Silent outcome ends this loop's own chain.
        if !monitor_enabled
            || (!has_unconsumed_observations
                && state.consecutive_continuations >= PM_LOOP_MAX_CONSECUTIVE)
        {
            state.pending_own_block = false;
            return false;
        }
        if let Some(last) = state.last_continued_at.as_deref() {
            if let (Ok(now_t), Ok(last_t)) = (
                chrono::DateTime::parse_from_rfc3339(now),
                chrono::DateTime::parse_from_rfc3339(last),
            ) {
                if (now_t - last_t).num_seconds() < interval_secs as i64 {
                    state.pending_own_block = false;
                    return false;
                }
            }
        }
        // FR-110: only truly empty cycles spend the park budget; held cycles
        // keep the count so the brake resumes after observations are consumed.
        if !has_unconsumed_observations {
            state.consecutive_continuations = state.consecutive_continuations.saturating_add(1);
        }
        state.last_continued_at = Some(now.to_string());
        state.pending_own_block = true;
        true
    })
    .map(|(_, should_continue)| should_continue)
    .unwrap_or(false);
    if !should_continue {
        return HookOutput::Silent;
    }
    HookOutput::stop_block(format!(
        "Resident PM loop: run one cycle before stopping. Try JSON operation `daemon.subscribe` \
         on the `issue_monitor` channel with `params.timeout_seconds:{interval_secs}`; if the \
         subscribe fails (e.g. no daemon endpoint), continue the same cycle in degraded polling \
         mode instead of treating it as a failure (FR-109). Either way, reconcile a fresh \
         `issue.monitor.status` snapshot: triage new issues, re-evaluate order, check the \
         running agents' `last_activity_at`, and report milestones to the user as a digest. \
         If the snapshot shows nothing actionable, stop again — the loop parks on its own \
         after repeated empty cycles (cycles with running launches, escalations, or undigested \
         failures do not count as empty)."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pm_registry::PmLoopState;
    use gwt_core::test_support::ScopedGwtHome;

    fn pm_fixture() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let home = tempfile::tempdir().expect("home");
        let repo = home.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo");
        let worktree = {
            let _guard = ScopedGwtHome::set(home.path());
            let worktree = crate::pm_registry::pm_worktree_path_for_repo_path(&repo);
            std::fs::create_dir_all(&worktree).expect("pm worktree");
            let prefs = crate::IssueMonitorPrefs {
                enabled: true,
                ..crate::IssueMonitorPrefs::default()
            };
            let prefs_path = worktree
                .parent()
                .and_then(Path::parent)
                .expect("gwt project dir")
                .join("project-state/issue-monitor.json");
            std::fs::create_dir_all(prefs_path.parent().expect("parent")).expect("state dir");
            crate::save_issue_monitor_prefs(&prefs_path, &prefs).expect("seed prefs");
            worktree
        };
        (home, repo, worktree)
    }

    /// FR-012: a stopping PM is turned around into the next cycle while the
    /// Monitor is enabled — this is what converts one-shot into resident.
    #[test]
    fn pm_stop_continues_the_resident_loop() {
        let (home, _repo, worktree) = pm_fixture();
        let _guard = ScopedGwtHome::set(home.path());

        let output = handle_at(&worktree, "2026-08-08T00:00:00Z", false);

        let HookOutput::StopBlock { reason } = output else {
            panic!("expected the loop to continue, got {output:?}");
        };
        assert!(reason.contains("daemon.subscribe"));
        assert!(reason.contains("issue.monitor.status"));
    }

    /// The brakes: a floor between continuations and a cap without user
    /// contact; user contact re-arms.
    #[test]
    fn pm_loop_floor_cap_and_user_reset_bound_the_continuations() {
        let (home, _repo, worktree) = pm_fixture();
        let _guard = ScopedGwtHome::set(home.path());

        assert!(matches!(
            handle_at(&worktree, "2026-08-08T00:00:00Z", false),
            HookOutput::StopBlock { .. }
        ));
        // Inside the floor: silent, and the budget is not consumed.
        assert_eq!(
            handle_at(&worktree, "2026-08-08T00:00:30Z", false),
            HookOutput::Silent
        );
        // Exhaust the budget past the floor each time.
        let mut minute = 2;
        loop {
            let now = format!("2026-08-08T00:{minute:02}:00Z");
            match handle_at(&worktree, &now, false) {
                HookOutput::StopBlock { .. } => minute += 2,
                _ => break,
            }
            assert!(minute < 60, "the cap must bound the loop");
        }
        // A user prompt re-arms.
        handle_user_prompt_submit(&worktree);
        assert!(matches!(
            handle_at(&worktree, "2026-08-08T02:00:00Z", false),
            HookOutput::StopBlock { .. }
        ));
    }

    /// Monitor off = parked project; and no other worktree is ever driven.
    #[test]
    fn disabled_monitor_and_non_pm_worktrees_stay_silent() {
        let (home, _repo, worktree) = pm_fixture();
        let _guard = ScopedGwtHome::set(home.path());
        let prefs_path = worktree
            .parent()
            .and_then(Path::parent)
            .expect("dir")
            .join("project-state/issue-monitor.json");
        let mut prefs = crate::load_issue_monitor_prefs(&prefs_path).expect("prefs");
        prefs.enabled = false;
        crate::save_issue_monitor_prefs(&prefs_path, &prefs).expect("save");
        assert_eq!(
            handle_at(&worktree, "2026-08-08T00:00:00Z", false),
            HookOutput::Silent
        );

        let ordinary = tempfile::tempdir().expect("ordinary");
        assert_eq!(
            handle_at(ordinary.path(), "2026-08-08T00:00:00Z", false),
            HookOutput::Silent
        );
    }

    /// Review fix: the loop must not ride a `stop_hook_active` chain another
    /// Stop gate started — only its own chain (marker set by its own block)
    /// keeps flowing across `stop_hook_active` stops.
    #[test]
    fn foreign_forced_continuations_are_not_ridden_and_own_chain_flows() {
        let (home, _repo, worktree) = pm_fixture();
        let _guard = ScopedGwtHome::set(home.path());

        // A foreign chain: some other gate forced the previous continuation.
        assert_eq!(
            handle_at(&worktree, "2026-08-08T00:00:00Z", true),
            HookOutput::Silent,
            "the loop must not stack onto another gate's forced continuation"
        );

        // Its own chain: block once, then keep flowing across the
        // stop_hook_active stops of that same chain.
        assert!(matches!(
            handle_at(&worktree, "2026-08-08T00:10:00Z", false),
            HookOutput::StopBlock { .. }
        ));
        assert!(matches!(
            handle_at(&worktree, "2026-08-08T00:12:00Z", true),
            HookOutput::StopBlock { .. }
        ));

        // A within-floor stop ends the loop's own chain (marker cleared), so
        // a later stop_hook_active stop is foreign again.
        assert_eq!(
            handle_at(&worktree, "2026-08-08T00:12:30Z", true),
            HookOutput::Silent,
            "the floor ends the own chain"
        );
        assert_eq!(
            handle_at(&worktree, "2026-08-08T00:20:00Z", true),
            HookOutput::Silent,
            "after the own chain ended, stop_hook_active stops are foreign"
        );
        // ...while a fresh chain start (no stop_hook_active) still drives.
        assert!(matches!(
            handle_at(&worktree, "2026-08-08T00:21:00Z", false),
            HookOutput::StopBlock { .. }
        ));
    }

    /// Review fix: real user contact stamps the conversation clock (the wake
    /// path defers to it) and re-arms the budget.
    #[test]
    fn user_prompt_submit_stamps_the_conversation_clock() {
        let (home, _repo, worktree) = pm_fixture();
        let _guard = ScopedGwtHome::set(home.path());

        assert!(matches!(
            handle_at(&worktree, "2026-08-08T00:00:00Z", false),
            HookOutput::StopBlock { .. }
        ));
        handle_user_prompt_submit_at(&worktree, "{}", "2026-08-08T00:00:30Z");

        let state_path =
            pm_registry::pm_loop_state_path_for_pm_worktree(&worktree).expect("pm loop state path");
        let state = pm_registry::load_pm_loop_state(&state_path).expect("state");
        assert_eq!(
            state.last_user_prompt_at.as_deref(),
            Some("2026-08-08T00:00:30Z")
        );
        assert_eq!(state.consecutive_continuations, 0);
        assert!(!state.pending_own_block);
        assert_eq!(state.last_continued_at, None);
    }

    /// FR-108c follow-up: the PM hook acknowledges the exact wake operation
    /// whose prompt reached UserPromptSubmit. A generic prompt cannot satisfy
    /// another delivery's verification probe.
    #[test]
    fn user_prompt_submit_records_the_exact_delivery_marker() {
        let (home, repo, worktree) = pm_fixture();
        let _guard = ScopedGwtHome::set(home.path());
        let sessions_dir = home.path().join("sessions");
        let mut session =
            gwt_agent::Session::new(worktree.clone(), "pm", gwt_agent::AgentId::ClaudeCode);
        session.project_state_root = Some(repo.clone());
        session.save(&sessions_dir).expect("save PM Session");
        let runtime_path = sessions_dir.join("runtime").join("12345").join("hook.json");
        std::fs::create_dir_all(runtime_path.parent().expect("runtime parent"))
            .expect("runtime dir");
        let _session_id =
            gwt_core::test_support::ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, &session.id);
        let _runtime_path = gwt_core::test_support::ScopedEnvVar::set(
            gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
            runtime_path.to_string_lossy().as_ref(),
        );
        let operation_id = "72fc3cd4-ad49-43e3-bf3d-d791357643a3";
        let body = "reconcile now";
        let prompt = format!(
            "{body} [gwt-delivery:{operation_id}:{}]",
            pm_registry::pm_delivery_prompt_sha256(body)
        );
        let state_path =
            pm_registry::pm_loop_state_path_for_pm_worktree(&worktree).expect("pm loop state path");
        pm_registry::prepare_pm_delivery_receipt(
            &pm_registry::pm_delivery_receipts_path_for_repo_path(&repo),
            &pm_registry::PmDeliveryReceipt {
                operation_id: operation_id.to_string(),
                recorded_at: "2026-08-08T00:00:00Z".to_string(),
                status: pm_registry::PmDeliveryReceiptStatus::Prepared,
                principal_session_id: "issuer".to_string(),
                target_window_id: "tab::pm".to_string(),
                target_session_id: session.id.clone(),
                body_sha256: pm_registry::pm_delivery_prompt_sha256(body),
                error_code: None,
            },
        )
        .expect("prepare wake delivery receipt");
        pm_registry::prepare_pm_wake_delivery(
            &state_path,
            operation_id,
            "tab::pm",
            &session.id,
            &pm_registry::pm_delivery_prompt_sha256(body),
        )
        .expect("prepare wake delivery fence");
        let input = serde_json::json!({
            "prompt": prompt
        })
        .to_string();

        handle_user_prompt_submit_at(&worktree, &input, "2026-08-08T00:00:30Z");

        let state = pm_registry::load_pm_loop_state(&state_path).expect("state");
        assert_eq!(
            state.last_delivery_operation_id.as_deref(),
            Some(operation_id)
        );
        assert_eq!(
            state.last_delivery_submitted_at.as_deref(),
            Some("2026-08-08T00:00:30Z")
        );
        assert_eq!(state.last_wake_at.as_deref(), Some("2026-08-08T00:00:30Z"));
        let raw = std::fs::read_to_string(&state_path).expect("raw loop state");
        assert!(
            !raw.contains("reconcile now"),
            "the hook must never persist the prompt body"
        );
    }

    #[test]
    fn protected_delivery_rejects_a_marker_with_unreserved_composer_text() {
        let (home, _repo, worktree) = pm_fixture();
        let _guard = ScopedGwtHome::set(home.path());
        let operation_id = "72fc3cd4-ad49-43e3-bf3d-d791357643a3";
        let body = "reconcile now";
        let expected = format!(
            "{body} [gwt-delivery:{operation_id}:{}]",
            pm_registry::pm_delivery_prompt_sha256(body)
        );
        let mixed = format!("unsent human text {expected}");
        let state_path =
            pm_registry::pm_loop_state_path_for_pm_worktree(&worktree).expect("pm loop state path");

        assert!(
            handle_user_prompt_submit_at(
                &worktree,
                &serde_json::json!({ "prompt": mixed }).to_string(),
                "2026-08-08T00:00:30Z",
            ),
            "a mixed protected delivery must block before the prompt reaches the agent"
        );

        let state = pm_registry::load_pm_loop_state(&state_path).expect("state");
        assert_eq!(state.last_delivery_operation_id, None);
        assert_eq!(state.last_delivery_submitted_at, None);
        assert_eq!(state.last_wake_at, None);
        assert_eq!(
            state.last_delivery_rejected_operation_id.as_deref(),
            Some(operation_id)
        );
    }

    #[test]
    fn ordinary_agent_hook_finishes_only_its_exact_prepared_delivery() {
        let home = tempfile::tempdir().expect("home");
        let _guard = ScopedGwtHome::set(home.path());
        let receipt_path = home.path().join("project-state/pm-delivery-receipts.jsonl");
        let operation_id = "72fc3cd4-ad49-43e3-bf3d-d791357643a5";
        pm_registry::prepare_pm_delivery_receipt(
            &receipt_path,
            &pm_registry::PmDeliveryReceipt {
                operation_id: operation_id.to_string(),
                recorded_at: "2026-08-11T00:00:00Z".to_string(),
                status: pm_registry::PmDeliveryReceiptStatus::Prepared,
                principal_session_id: "pm-session".to_string(),
                target_window_id: "tab::agent".to_string(),
                target_session_id: "agent-session".to_string(),
                body_sha256: pm_registry::pm_delivery_prompt_sha256("review the current handoff"),
                error_code: None,
            },
        )
        .expect("prepare delivery");
        let body = "review the current handoff";
        let prompt = format!(
            "{body} [gwt-delivery:{operation_id}:{}]",
            pm_registry::pm_delivery_prompt_sha256(body)
        );
        let evidence = delivery_evidence_from_prompt(&prompt).expect("exact evidence");

        record_pm_delivery_evidence_at(
            &receipt_path,
            "agent-session",
            evidence,
            "2026-08-11T00:00:01Z",
        )
        .expect("finish delivery");

        let receipts = pm_registry::load_pm_delivery_receipts(&receipt_path).expect("receipts");
        assert_eq!(
            receipts.last().map(|receipt| receipt.status),
            Some(pm_registry::PmDeliveryReceiptStatus::Verified)
        );
        assert!(record_pm_delivery_evidence_at(
            &receipt_path,
            "foreign-session",
            DeliveryEvidence::Exact {
                operation_id: operation_id.to_string(),
                body_sha256: pm_registry::pm_delivery_prompt_sha256(body),
            },
            "2026-08-11T00:00:02Z",
        )
        .is_err());
    }

    #[test]
    fn protected_delivery_requires_its_prepared_body_and_target_session() {
        let home = tempfile::tempdir().expect("home");
        let _guard = ScopedGwtHome::set(home.path());
        let receipt_path = home.path().join("project-state/pm-delivery-receipts.jsonl");
        let operation_id = "72fc3cd4-ad49-43e3-bf3d-d791357643a6";
        let expected_body = "review the prepared handoff";
        pm_registry::prepare_pm_delivery_receipt(
            &receipt_path,
            &pm_registry::PmDeliveryReceipt {
                operation_id: operation_id.to_string(),
                recorded_at: "2026-08-11T00:00:00Z".to_string(),
                status: pm_registry::PmDeliveryReceiptStatus::Prepared,
                principal_session_id: "pm-session".to_string(),
                target_window_id: "tab::agent".to_string(),
                target_session_id: "agent-session".to_string(),
                body_sha256: pm_registry::pm_delivery_prompt_sha256(expected_body),
                error_code: None,
            },
        )
        .expect("prepare delivery");

        let foreign_body = "different message";
        let foreign_evidence = delivery_evidence_from_prompt(&format!(
            "{foreign_body} [gwt-delivery:{operation_id}:{}]",
            pm_registry::pm_delivery_prompt_sha256(foreign_body)
        ))
        .expect("syntactically exact foreign body evidence");
        assert!(record_pm_delivery_evidence_at(
            &receipt_path,
            "agent-session",
            foreign_evidence,
            "2026-08-11T00:00:01Z",
        )
        .is_err());

        let expected_evidence = delivery_evidence_from_prompt(&format!(
            "{expected_body} [gwt-delivery:{operation_id}:{}]",
            pm_registry::pm_delivery_prompt_sha256(expected_body)
        ))
        .expect("expected evidence");
        assert!(record_pm_delivery_evidence_at(
            &receipt_path,
            "foreign-session",
            expected_evidence,
            "2026-08-11T00:00:02Z",
        )
        .is_err());
        assert_eq!(
            pm_registry::load_pm_delivery_receipts(&receipt_path)
                .expect("receipts")
                .iter()
                .map(|receipt| receipt.status)
                .collect::<Vec<_>>(),
            vec![pm_registry::PmDeliveryReceiptStatus::Prepared]
        );
    }

    #[test]
    fn protected_delivery_without_a_durable_receipt_is_rejected() {
        let worktree = tempfile::tempdir().expect("worktree");
        let operation_id = "72fc3cd4-ad49-43e3-bf3d-d791357643a7";
        let body = "unreserved protected message";
        let input = serde_json::json!({
            "prompt": format!(
                "{body} [gwt-delivery:{operation_id}:{}]",
                pm_registry::pm_delivery_prompt_sha256(body)
            )
        })
        .to_string();

        assert!(handle_user_prompt_submit_at(
            worktree.path(),
            &input,
            "2026-08-11T00:00:00Z",
        ));
    }

    #[test]
    fn user_prompt_submit_requires_a_canonical_uuid_at_the_exact_prompt_suffix() {
        let (home, _repo, worktree) = pm_fixture();
        let _guard = ScopedGwtHome::set(home.path());
        let state_path =
            pm_registry::pm_loop_state_path_for_pm_worktree(&worktree).expect("pm loop state path");
        pm_registry::save_pm_loop_state(
            &state_path,
            &PmLoopState {
                last_delivery_operation_id: Some(
                    "72fc3cd4-ad49-43e3-bf3d-d791357643a3".to_string(),
                ),
                last_delivery_submitted_at: Some("2026-08-08T00:00:00Z".to_string()),
                ..PmLoopState::default()
            },
        )
        .expect("seed prior delivery receipt");

        for (index, prompt) in [
            "ordinary user prompt".to_string(),
            "[gwt-delivery:72fc3cd4-ad49-43e3-bf3d-d791357643a3] trailing".to_string(),
            "[gwt-delivery:72FC3CD4-AD49-43E3-BF3D-D791357643A3]".to_string(),
            "[gwt-delivery:not-a-uuid]".to_string(),
        ]
        .into_iter()
        .enumerate()
        {
            let input = serde_json::json!({ "prompt": prompt }).to_string();
            handle_user_prompt_submit_at(
                &worktree,
                &input,
                &format!("2026-08-08T00:00:{:02}Z", index + 1),
            );
        }

        let state = pm_registry::load_pm_loop_state(&state_path).expect("state");
        assert_eq!(
            state.last_delivery_operation_id.as_deref(),
            Some("72fc3cd4-ad49-43e3-bf3d-d791357643a3"),
            "generic or malformed prompts must not replace a prior exact receipt"
        );
        assert_eq!(
            state.last_delivery_submitted_at.as_deref(),
            Some("2026-08-08T00:00:00Z")
        );
    }

    /// FR-110 (T-204): unconsumed observations hold the park — while the
    /// durable monitor state still shows work a supervisor must look at
    /// (running launches, failures, needs-human), empty-cycle counting must
    /// not retire the PM.
    #[test]
    fn park_counting_holds_while_unconsumed_observations_remain() {
        let (home, _repo, worktree) = pm_fixture();
        let _guard = ScopedGwtHome::set(home.path());
        let state_path =
            pm_registry::pm_loop_state_path_for_pm_worktree(&worktree).expect("pm loop state path");
        let prefs_path = state_path
            .parent()
            .expect("project state dir")
            .join("issue-monitor.json");

        // A running launch is durable, unconsumed supervision work.
        let mut monitor = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            crate::load_issue_monitor_prefs(&prefs_path).expect("prefs"),
        );
        crate::scan_issue_monitor_candidates(
            &mut monitor,
            &[crate::IssueMonitorIssue {
                number: 42,
                title: "Issue 42".to_string(),
                labels: vec!["auto-merge".to_string()],
                state: crate::IssueMonitorIssueState::Open,
                body: None,
                url: None,
                readiness: crate::IssueMonitorReadiness::NotApplicable,
            }],
            "2026-08-10T00:00:00Z",
        );
        monitor.complete_active_launch(42, "tab-1::agent-1");
        crate::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).expect("save prefs");

        // Far past the cap: with a live launch the loop must keep driving.
        let mut minute = 0;
        for _ in 0..20 {
            let now = format!("2026-08-10T01:{minute:02}:00Z");
            assert!(
                matches!(
                    handle_at(&worktree, &now, false),
                    HookOutput::StopBlock { .. }
                ),
                "a supervising PM must not park while a launch is live (cycle at {now})"
            );
            minute += 2;
        }
        let state = pm_registry::load_pm_loop_state(&state_path).expect("state");
        assert_eq!(
            state.consecutive_continuations, 0,
            "cycles with unconsumed observations are not empty cycles"
        );
    }

    /// FR-110 (T-204): with nothing unconsumed the cap still parks the PM —
    /// the empty-cycle brake is unchanged.
    #[test]
    fn park_counting_still_caps_truly_empty_cycles() {
        let (home, _repo, worktree) = pm_fixture();
        let _guard = ScopedGwtHome::set(home.path());

        let mut minute = 0;
        let mut blocks = 0;
        for _ in 0..20 {
            let now = format!("2026-08-10T02:{minute:02}:00Z");
            if matches!(
                handle_at(&worktree, &now, false),
                HookOutput::StopBlock { .. }
            ) {
                blocks += 1;
            }
            minute += 2;
        }
        assert_eq!(blocks, 12, "truly empty cycles must still park at the cap");
    }
}
