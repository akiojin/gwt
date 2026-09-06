//! Issue #3490: contention on the Codex state directory shared by every
//! concurrently launched Codex process.
//!
//! Without a Backend Override profile, `build_codex_args` leaves `CODEX_HOME`
//! unset, so every Codex process gwt starts points at the same `~/.codex`.
//! Codex initializes `state_*.sqlite` / `logs_*.sqlite` there while it boots,
//! and SQLite refuses the second writer with `database is locked` when two
//! Codex processes reach that window together. gwt starts Codex agents in
//! parallel (Issue Monitor fan-out plus manual launches), so the mitigation
//! belongs here.
//!
//! Two halves, matching the two acceptance criteria that survive together:
//!
//! - [`pace_shared_codex_spawn`] spaces consecutive shared-`~/.codex` spawns
//!   apart so the initialization windows stop overlapping.
//! - [`is_codex_shared_state_lock_failure`] and
//!   [`codex_shared_state_lock_detail`] turn a contention that still slips
//!   through into an explained, automatically retried failure instead of a raw
//!   Codex stack trace that spends the Issue's retry budget.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::prepare::TRANSIENT_LAUNCH_RETRY_HINT;
use crate::types::{AgentId, LaunchRuntimeTarget};

/// Minimum spacing between two Codex spawns that share one `~/.codex`.
///
/// The window being protected is Codex's own SQLite initialization, which is
/// short but stretches under the host load that makes the collision likely in
/// the first place. Three quarters of a second clears it with margin while
/// costing a fully saturated fan-out only a few seconds in total.
pub const CODEX_SHARED_STATE_SPAWN_GAP: Duration = Duration::from_millis(750);

/// Serializes spawns that share one Codex state directory and keeps
/// consecutive grants at least `gap` apart.
pub struct CodexSpawnPacer {
    gap: Duration,
    last_grant: Mutex<Option<Instant>>,
}

impl CodexSpawnPacer {
    pub const fn new(gap: Duration) -> Self {
        Self {
            gap,
            last_grant: Mutex::new(None),
        }
    }

    /// Block until this caller may spawn, and report how long it waited.
    ///
    /// The lock is deliberately held across the sleep: releasing it first would
    /// let every waiting launch compute the same delay and then spawn together,
    /// which is the behavior this pacer exists to prevent.
    pub fn wait_for_turn(&self) -> Duration {
        let mut last_grant = self
            .last_grant
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let waited = stagger_delay(*last_grant, Instant::now(), self.gap);
        if !waited.is_zero() {
            std::thread::sleep(waited);
        }
        *last_grant = Some(Instant::now());
        waited
    }
}

/// How long a spawn must wait given the previous grant.
fn stagger_delay(last_grant: Option<Instant>, now: Instant, gap: Duration) -> Duration {
    match last_grant {
        Some(last) => gap.saturating_sub(now.saturating_duration_since(last)),
        None => Duration::ZERO,
    }
}

/// Whether a resolved launch will contend for the user's `~/.codex`.
///
/// Only a Host Codex launch that never got a `CODEX_HOME` override does: a
/// Backend Override profile materializes a worktree-local state directory with
/// no other writer, and a Docker agent initializes state inside its container.
pub fn shares_user_codex_state(
    agent_id: &AgentId,
    runtime_target: LaunchRuntimeTarget,
    env: &HashMap<String, String>,
) -> bool {
    *agent_id == AgentId::Codex
        && runtime_target == LaunchRuntimeTarget::Host
        && !env.contains_key("CODEX_HOME")
}

/// Process-wide pacer for launches that share the user's `~/.codex`.
pub fn pace_shared_codex_spawn() -> Duration {
    static PACER: OnceLock<CodexSpawnPacer> = OnceLock::new();
    PACER
        .get_or_init(|| CodexSpawnPacer::new(CODEX_SHARED_STATE_SPAWN_GAP))
        .wait_for_turn()
}

/// SQLite's refusal, as Codex prints it.
const SQLITE_LOCK_MARKER: &str = "database is locked";
/// The Codex startup step that opens the shared state directory.
const CODEX_STATE_RUNTIME_MARKER: &str = "failed to initialize state runtime";

/// Whether an agent's failure output is Codex losing the race for the shared
/// state directory.
///
/// Both a SQLite lock refusal and a Codex-owned context are required: a bare
/// `database is locked` from some other tool the agent ran is not this failure,
/// and mis-classifying it would hide a real problem behind an automatic retry.
pub fn is_codex_shared_state_lock_failure(output: &str) -> bool {
    let output = output.to_ascii_lowercase();
    output.contains(SQLITE_LOCK_MARKER)
        && (output.contains(CODEX_STATE_RUNTIME_MARKER) || output.contains(".codex"))
}

/// The operator-facing replacement for the raw Codex stack trace.
///
/// It states the cause, says the work is intact, and names the one lever the
/// operator has. It also carries [`TRANSIENT_LAUNCH_RETRY_HINT`], which is what
/// keeps the Issue Monitor from spending an attempt on someone else's lock
/// contention.
pub fn codex_shared_state_lock_detail() -> String {
    format!(
        "Codex could not start: several Codex agents initialized the shared ~/.codex state \
         directory at the same time and SQLite refused the second writer. Nothing about this \
         work failed. gwt spaces Codex launches apart and {TRANSIENT_LAUNCH_RETRY_HINT}. If it \
         keeps happening, lower the Issue Monitor's max_active so fewer Codex agents start \
         together."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The raw pane output recorded on Issue #3490.
    const OBSERVED_CODEX_STACK: &str = "/Users/akiojin/.codex/state_5.sqlite: failed to initialize state runtime at /Users/akiojin/.codex: failed to open log DB at /Users/akiojin/.codex/logs_2.sqlite: error returned from database: (code: 5) database is locked: error returned from database: (code: 5) database is locked: (code: 5) database is locked";

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    /// Issue #3490 AC-1: only the launches that actually share `~/.codex` are
    /// paced. Pacing a worktree-local or containerized Codex would slow launches
    /// down for contention that cannot happen.
    #[test]
    fn only_host_codex_without_a_codex_home_override_shares_the_user_state() {
        assert!(shares_user_codex_state(
            &AgentId::Codex,
            LaunchRuntimeTarget::Host,
            &env(&[])
        ));
        assert!(
            !shares_user_codex_state(
                &AgentId::Codex,
                LaunchRuntimeTarget::Host,
                &env(&[("CODEX_HOME", "/repo/.gwt/codex")]),
            ),
            "a Backend Override profile already isolated the state directory"
        );
        assert!(
            !shares_user_codex_state(&AgentId::Codex, LaunchRuntimeTarget::Docker, &env(&[])),
            "a container initializes its own state directory"
        );
        assert!(
            !shares_user_codex_state(&AgentId::ClaudeCode, LaunchRuntimeTarget::Host, &env(&[])),
            "no other agent writes the Codex state directory"
        );
    }

    #[test]
    fn the_first_shared_codex_spawn_is_not_delayed() {
        let pacer = CodexSpawnPacer::new(Duration::from_millis(60));
        assert_eq!(
            pacer.wait_for_turn(),
            Duration::ZERO,
            "nothing is contending yet, so the first launch must not pay the gap"
        );
    }

    /// Issue #3490 AC-1/AC-4: the concurrent fan-out this Issue reported. Four
    /// launches asking for a turn at once must be handed out one at a time, each
    /// at least the gap after the previous one — no overlapping initialization
    /// windows, whatever order the threads arrive in.
    #[test]
    fn concurrent_shared_codex_spawns_are_paced_apart() {
        let gap = Duration::from_millis(60);
        let pacer = CodexSpawnPacer::new(gap);
        let mut grants = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..4)
                .map(|_| {
                    scope.spawn(|| {
                        pacer.wait_for_turn();
                        Instant::now()
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| handle.join().expect("pacer thread must not panic"))
                .collect::<Vec<_>>()
        });
        grants.sort();
        for pair in grants.windows(2) {
            let spacing = pair[1].saturating_duration_since(pair[0]);
            assert!(
                spacing >= gap,
                "consecutive Codex spawns must stay {gap:?} apart, got {spacing:?}"
            );
        }
    }

    #[test]
    fn the_observed_codex_stack_is_recognized_as_shared_state_contention() {
        assert!(is_codex_shared_state_lock_failure(OBSERVED_CODEX_STACK));
    }

    #[test]
    fn unrelated_failures_are_not_treated_as_shared_state_contention() {
        assert!(
            !is_codex_shared_state_lock_failure("Agent exited — last output: command not found"),
            "an ordinary agent failure must keep its own diagnosis"
        );
        assert!(
            !is_codex_shared_state_lock_failure(
                "sqlite3: error: database is locked while writing coverage.db"
            ),
            "a lock from some other tool the agent ran is not the Codex startup race"
        );
    }

    /// Issue #3490 AC-2/AC-3: the replacement text names the cause and the
    /// lever, and it classifies as a transient launch failure so the Issue
    /// Monitor requeues without spending an attempt.
    #[test]
    fn the_shared_state_lock_detail_explains_the_cause_and_retries_for_free() {
        let detail = codex_shared_state_lock_detail();
        assert!(detail.contains("~/.codex"), "got: {detail}");
        assert!(detail.contains("max_active"), "got: {detail}");
        assert!(
            !detail.contains("sqlite"),
            "the raw Codex stack must not survive into the pane detail: {detail}"
        );
        assert!(
            crate::prepare::is_transient_launch_failure(&detail),
            "the detail must not spend an Issue Monitor attempt: {detail}"
        );
    }
}
