//! Live PM Board probe using the normal checkout library (Issue #5392 AC-3).
//! The probe owns an unsaved Session and only its own disposable cursor.

use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use chrono::Utc;
use gwt::cli::hook::{board_reminder::compute_plan, HookOutput};
use gwt_agent::{AgentId, Session};
use gwt_core::coordination::{reminders_dir, write_reminders_state_for_repo_hash};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let repo = PathBuf::from(std::env::args_os().nth(1).ok_or("repository required")?);
    let worktree = gwt::pm_registry::pm_worktree_path_for_repo_path(&repo);
    if !worktree.is_dir() {
        return Err(format!("live PM worktree missing: {}", worktree.display()).into());
    }
    let mut session = Session::new(&worktree, "", AgentId::Codex);
    session.display_name = "PM Board probe".into();
    let cursor = reminders_dir(&worktree).join(format!("{}.json", session.id));
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let mut durations = Vec::new();
        for turn in 0..100 {
            let started = Instant::now();
            let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
                started + Duration::from_millis(200),
            );
            let plan = compute_plan("UserPromptSubmit", &session, Utc::now())?
                .ok_or("missing prompt plan")?;
            durations.push(started.elapsed().as_secs_f64() * 1000.0);
            let HookOutput::HookSpecificAdditionalContext { text, .. } = plan.output else {
                return Err("missing PM prompt context".into());
            };
            if text.contains("Board context unavailable") {
                return Err(format!("Board read failed on prompt {turn}").into());
            }
            write_reminders_state_for_repo_hash(
                &worktree,
                session.repo_hash.as_deref(),
                &session.id,
                &plan.next_reminders,
            )?;
        }
        durations.sort_by(f64::total_cmp);
        println!(
            "live PM prompts=100 failures=0 p50_ms={:.3} p95_ms={:.3} max_ms={:.3}",
            durations[49], durations[94], durations[99]
        );
        Ok(())
    })();
    if cursor.exists() {
        std::fs::remove_file(&cursor)?;
    }
    result
}
