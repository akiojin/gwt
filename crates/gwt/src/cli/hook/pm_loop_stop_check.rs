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

use std::path::Path;

use super::HookOutput;
use crate::pm_registry::{self, PmLoopState};

// The floor between continuations and the subscribe timeout both come from
// `PmSettings::loop_interval_secs` (FR-035, default 60s): one knob, because a
// floor shorter than the wait would never fire and a longer one would skip
// cycles.

/// Consecutive continuations without user contact before the PM parks.
/// At the default 60s `loop_interval_secs` this is ~12 minutes of unattended
/// residency after the last conversation; the daemon wake path (T-093)
/// revives a parked PM when new monitor activity arrives.
const PM_LOOP_MAX_CONSECUTIVE: u32 = 12;

/// Finalize a protected PM delivery only when the exact target Session's
/// UserPromptSubmit hook observes the self-authenticating operation marker.
/// Invalid or unrelated prompts are ordinary user input and remain silent.
pub fn handle_delivery_acknowledgement(worktree: &Path, input: &str) {
    let Some(prompt) = serde_json::from_str::<serde_json::Value>(input)
        .ok()
        .and_then(|value| {
            value
                .get("prompt")
                .and_then(|prompt| prompt.as_str())
                .map(str::to_string)
        })
    else {
        return;
    };
    let Some((operation_id, body_sha256)) =
        pm_registry::parse_protected_pm_delivery_prompt(&prompt)
    else {
        return;
    };
    let Some(target_session_id) = std::env::var(gwt_agent::GWT_SESSION_ID_ENV)
        .ok()
        .filter(|session_id| !session_id.trim().is_empty())
    else {
        return;
    };
    let receipt_path = pm_registry::pm_delivery_receipts_path_for_repo_path(worktree);
    if let Err(error) = pm_registry::finish_pm_delivery_receipt(
        &receipt_path,
        &operation_id,
        &target_session_id,
        &body_sha256,
        pm_registry::PmDeliveryReceiptStatus::Verified,
        None,
    ) {
        tracing::warn!(%error, operation_id, "PM delivery acknowledgement was rejected");
    }
}

/// UserPromptSubmit entry: real user contact re-arms the loop budget and
/// stamps the conversation clock the T-093 wake path defers to.
pub fn handle_user_prompt_submit(worktree: &Path) {
    handle_user_prompt_submit_at(
        worktree,
        &chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    );
}

fn handle_user_prompt_submit_at(worktree: &Path, now: &str) {
    if !super::is_resident_pm_worktree(worktree) {
        return;
    }
    let Some(state_path) = pm_registry::pm_loop_state_path_for_pm_worktree(worktree) else {
        return;
    };
    let _ = pm_registry::save_pm_loop_state(
        &state_path,
        &PmLoopState {
            last_user_prompt_at: Some(now.to_string()),
            ..PmLoopState::default()
        },
    );
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
    let mut state = pm_registry::load_pm_loop_state(&state_path).unwrap_or_default();
    // A `stop_hook_active` chain the loop did not start belongs to another
    // Stop gate — riding it would stack this loop's directive on top of that
    // gate's forced continuation. The loop's own chain carries the marker set
    // below and keeps flowing.
    if stop_hook_active && !state.pending_own_block {
        return HookOutput::Silent;
    }
    // Every Silent below ends the loop's own chain, so the marker is cleared
    // (persisted only when it changes) before returning.
    let end_own_chain = |state: &mut PmLoopState| {
        if state.pending_own_block {
            state.pending_own_block = false;
            let _ = pm_registry::save_pm_loop_state(&state_path, state);
        }
    };
    if !monitor_enabled {
        end_own_chain(&mut state);
        return HookOutput::Silent;
    }
    let interval_secs = pm_registry::load_pm_prefs(&project_state.join("pm.json"))
        .map(|prefs| prefs.settings.loop_interval_secs_clamped())
        .unwrap_or(pm_registry::PM_LOOP_INTERVAL_DEFAULT_SECS);
    if !has_unconsumed_observations && state.consecutive_continuations >= PM_LOOP_MAX_CONSECUTIVE {
        end_own_chain(&mut state);
        return HookOutput::Silent;
    }
    if let Some(last) = state.last_continued_at.as_deref() {
        if let (Ok(now_t), Ok(last_t)) = (
            chrono::DateTime::parse_from_rfc3339(now),
            chrono::DateTime::parse_from_rfc3339(last),
        ) {
            if (now_t - last_t).num_seconds() < i64::try_from(interval_secs).unwrap_or(i64::MAX) {
                end_own_chain(&mut state);
                return HookOutput::Silent;
            }
        }
    }
    // FR-110: only truly empty cycles spend the park budget; held cycles keep
    // the count as-is so the brake resumes once the observations are consumed.
    if !has_unconsumed_observations {
        state.consecutive_continuations = state.consecutive_continuations.saturating_add(1);
    }
    state.last_continued_at = Some(now.to_string());
    state.pending_own_block = true;
    let _ = pm_registry::save_pm_loop_state(&state_path, &state);
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
    use gwt_core::test_support::{ScopedEnvVar, ScopedGwtHome};

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

    #[test]
    fn exact_target_user_prompt_submit_verifies_the_delivery_receipt() {
        let home = tempfile::tempdir().expect("home");
        let repo = home.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo");
        let _gwt_home = ScopedGwtHome::set(home.path());
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "target-session");
        let operation_id = "72fc3cd4-ad49-43e3-bf3d-d791357643a3";
        let body = "report exact status";
        let body_sha256 = pm_registry::pm_delivery_prompt_sha256(body);
        let receipt_path = pm_registry::pm_delivery_receipts_path_for_repo_path(&repo);
        pm_registry::prepare_pm_delivery_receipt(
            &receipt_path,
            &pm_registry::PmDeliveryReceipt {
                operation_id: operation_id.to_string(),
                recorded_at: "2026-08-13T00:00:00Z".to_string(),
                status: pm_registry::PmDeliveryReceiptStatus::Prepared,
                principal_session_id: "pm-session".to_string(),
                target_window_id: "tab-1::agent-1".to_string(),
                target_session_id: "target-session".to_string(),
                body_sha256: body_sha256.clone(),
                reason: None,
            },
        )
        .expect("prepare receipt");
        let input = serde_json::json!({
            "prompt": format!("{body} [gwt-delivery:{operation_id}:{body_sha256}]")
        })
        .to_string();

        handle_delivery_acknowledgement(&repo, &input);

        assert!(pm_registry::load_pm_delivery_receipts(&receipt_path)
            .expect("load receipt")
            .iter()
            .any(|receipt| receipt.status == pm_registry::PmDeliveryReceiptStatus::Verified));
    }

    #[test]
    fn wrong_session_or_body_hash_cannot_verify_a_delivery_receipt() {
        let home = tempfile::tempdir().expect("home");
        let repo = home.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo");
        let _gwt_home = ScopedGwtHome::set(home.path());
        let operation_id = "72fc3cd4-ad49-43e3-bf3d-d791357643a4";
        let body = "report exact status";
        let body_sha256 = pm_registry::pm_delivery_prompt_sha256(body);
        let receipt_path = pm_registry::pm_delivery_receipts_path_for_repo_path(&repo);
        pm_registry::prepare_pm_delivery_receipt(
            &receipt_path,
            &pm_registry::PmDeliveryReceipt {
                operation_id: operation_id.to_string(),
                recorded_at: "2026-08-13T00:00:00Z".to_string(),
                status: pm_registry::PmDeliveryReceiptStatus::Prepared,
                principal_session_id: "pm-session".to_string(),
                target_window_id: "tab-1::agent-1".to_string(),
                target_session_id: "target-session".to_string(),
                body_sha256: body_sha256.clone(),
                reason: None,
            },
        )
        .expect("prepare receipt");
        let input = serde_json::json!({
            "prompt": format!("{body} [gwt-delivery:{operation_id}:{body_sha256}]")
        })
        .to_string();
        {
            let _wrong_session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "wrong-session");
            handle_delivery_acknowledgement(&repo, &input);
        }
        let _target_session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "target-session");
        let forged = serde_json::json!({
            "prompt": format!("tampered [gwt-delivery:{operation_id}:{body_sha256}]")
        })
        .to_string();
        handle_delivery_acknowledgement(&repo, &forged);

        assert!(!pm_registry::load_pm_delivery_receipts(&receipt_path)
            .expect("load receipt")
            .iter()
            .any(|receipt| receipt.status == pm_registry::PmDeliveryReceiptStatus::Verified));
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

    #[test]
    fn next_stop_cycle_reloads_the_updated_loop_interval() {
        let (home, _repo, worktree) = pm_fixture();
        let _guard = ScopedGwtHome::set(home.path());

        let first = handle_at(&worktree, "2026-08-08T00:00:00Z", false);
        let HookOutput::StopBlock { reason } = first else {
            panic!("expected initial cycle, got {first:?}");
        };
        assert!(reason.contains("timeout_seconds:60"));

        let prefs_path = pm_registry::pm_loop_state_path_for_pm_worktree(&worktree)
            .expect("loop state path")
            .parent()
            .expect("project state")
            .join("pm.json");
        pm_registry::mutate_pm_prefs(&prefs_path, |prefs| {
            prefs.settings.loop_interval_secs = 10;
        })
        .expect("update loop interval");

        let next = handle_at(&worktree, "2026-08-08T00:00:10Z", true);
        let HookOutput::StopBlock { reason } = next else {
            panic!("updated interval must apply to the next Stop cycle, got {next:?}");
        };
        assert!(reason.contains("timeout_seconds:10"));
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
        handle_user_prompt_submit_at(&worktree, "2026-08-08T00:00:30Z");

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
