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
/// With the contract's 300s subscribe this is roughly an hour of unattended
/// residency after the last conversation.
const PM_LOOP_MAX_CONSECUTIVE: u32 = 12;

/// UserPromptSubmit entry: real user contact re-arms the loop budget.
pub fn handle_user_prompt_submit(worktree: &Path) {
    if !super::is_resident_pm_worktree(worktree) {
        return;
    }
    let Some(state_path) = pm_registry::pm_loop_state_path_for_pm_worktree(worktree) else {
        return;
    };
    let _ = pm_registry::save_pm_loop_state(&state_path, &PmLoopState::default());
}

pub fn handle_with_input(worktree: &Path, _input: &str) -> HookOutput {
    handle_at(
        worktree,
        &chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    )
}

fn handle_at(worktree: &Path, now: &str) -> HookOutput {
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
    let monitor_enabled = crate::load_issue_monitor_prefs(&prefs_path)
        .map(|prefs| prefs.enabled)
        .unwrap_or(false);
    if !monitor_enabled {
        return HookOutput::Silent;
    }
    let interval_secs = pm_registry::load_pm_prefs(&project_state.join("pm.json"))
        .map(|prefs| prefs.settings.loop_interval_secs_clamped())
        .unwrap_or(60);

    let mut state = pm_registry::load_pm_loop_state(&state_path).unwrap_or_default();
    if state.consecutive_continuations >= PM_LOOP_MAX_CONSECUTIVE {
        return HookOutput::Silent;
    }
    if let Some(last) = state.last_continued_at.as_deref() {
        if let (Ok(now_t), Ok(last_t)) = (
            chrono::DateTime::parse_from_rfc3339(now),
            chrono::DateTime::parse_from_rfc3339(last),
        ) {
            if (now_t - last_t).num_seconds() < interval_secs as i64 {
                return HookOutput::Silent;
            }
        }
    }
    state.consecutive_continuations = state.consecutive_continuations.saturating_add(1);
    state.last_continued_at = Some(now.to_string());
    let _ = pm_registry::save_pm_loop_state(&state_path, &state);
    HookOutput::stop_block(format!(
        "Resident PM loop: run one cycle before stopping. Run JSON operation `daemon.subscribe` \
         on the `issue_monitor` channel with `params.timeout_seconds:{interval_secs}`, then \
         reconcile a fresh `issue.monitor.status` snapshot: triage new issues, re-evaluate \
         order, check the running agents' `last_activity_at`, and report milestones to the user \
         as a digest. If the snapshot shows nothing actionable, stop again — the loop parks on \
         its own after repeated empty cycles."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
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

        let output = handle_at(&worktree, "2026-08-08T00:00:00Z");

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
            handle_at(&worktree, "2026-08-08T00:00:00Z"),
            HookOutput::StopBlock { .. }
        ));
        // Inside the floor: silent, and the budget is not consumed.
        assert_eq!(
            handle_at(&worktree, "2026-08-08T00:00:30Z"),
            HookOutput::Silent
        );
        // Exhaust the budget past the floor each time.
        let mut minute = 2;
        loop {
            let now = format!("2026-08-08T00:{minute:02}:00Z");
            match handle_at(&worktree, &now) {
                HookOutput::StopBlock { .. } => minute += 2,
                _ => break,
            }
            assert!(minute < 60, "the cap must bound the loop");
        }
        // A user prompt re-arms.
        handle_user_prompt_submit(&worktree);
        assert!(matches!(
            handle_at(&worktree, "2026-08-08T02:00:00Z"),
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
            handle_at(&worktree, "2026-08-08T00:00:00Z"),
            HookOutput::Silent
        );

        let ordinary = tempfile::tempdir().expect("ordinary");
        assert_eq!(
            handle_at(ordinary.path(), "2026-08-08T00:00:00Z"),
            HookOutput::Silent
        );
    }
}
