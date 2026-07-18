//! Intake completion hard gate (SPEC-3248 P7A, FR-014/FR-016/FR-017).
//!
//! Intake (Curate) sessions must settle every curation prompt with a durable
//! Issue/SPEC outcome or an explicit, reasoned No Action before Stop. Two
//! handlers implement that contract:
//!
//! - [`handle_user_prompt_submit`] marks the artifact requirement dirty
//!   (`required_since`) for the current session on each curation/producing
//!   prompt (T-079). Pure status/question prompts are exempt per FR-168's
//!   coarse rule until the typed FR-138..FR-150 obligations land.
//! - [`handle_with_input`] blocks Stop for `completion_gate` lanes when the
//!   current session holds no valid outcome recorded at or after the latest
//!   dirty marker (T-074/T-080). Board posts and prose answers never write
//!   outcome state, so they can never satisfy the gate (FR-017, T-081).
//!
//! On a missing/stale-outcome block the gate auto-captures one
//! `issue-spec-workflow` self-improvement candidate with a stable dedupe key
//! (T-084, FR-019); a capture failure is surfaced inside the StopBlock reason
//! with a manual `improvement.capture` fallback instead of silently passing
//! (T-085).
//!
//! Existing Stop contracts are preserved: `stop_hook_active` short-circuits
//! (one forced continuation per cycle, FR-014o), parse/IO errors fail open
//! (FR-014u), and execution lanes / worktrees without a lane file never see
//! the gate (FR-015).

use std::path::Path;

use chrono::Utc;

use super::{context::HookContext, envelope::stop_hook_active_from, HookOutput};
use crate::cli::{improvement, intake_outcome};

/// UserPromptSubmit handler: persist the FR-016 dirty marker. Fail-open state
/// writer — it never contributes to the hook output.
///
/// Payloads that carry no `prompt` string (provider bridges such as
/// OpenCode/OpenClaw/Hermes do not normalize one yet) mark the requirement
/// dirty unconditionally: an unknown prompt is enforced, not exempted, so the
/// gate cannot go permanently dormant on those providers. Prompt-text
/// normalization for provider bridges is a dependent follow-up.
pub fn handle_user_prompt_submit(worktree: &Path, input: &str) {
    let resolved = gwt_core::paths::resolve_current_worktree_root(worktree);
    let lane = HookContext::for_worktree(&resolved).lane;
    if !lane.policy_flags.completion_gate {
        return;
    }
    let Some(session_id) = current_session_from_env() else {
        return;
    };
    if let Some(prompt) = prompt_from_input(input) {
        if is_pure_status_question(&prompt) {
            return;
        }
    }
    if let Err(error) = intake_outcome::mark_required_since(&resolved, &session_id, Utc::now()) {
        tracing::warn!(?error, "intake required_since marking failed");
    }
}

/// Stop handler: the hard gate.
pub fn handle_with_input(
    worktree: &Path,
    input: &str,
    current_session: Option<&str>,
) -> HookOutput {
    if stop_hook_active_from(input) {
        return HookOutput::Silent;
    }
    let resolved = gwt_core::paths::resolve_current_worktree_root(worktree);
    let lane = HookContext::for_worktree(&resolved).lane;
    if !lane.policy_flags.completion_gate {
        // FR-015: execution lanes, old worktrees, and missing lane files keep
        // their existing behavior.
        return HookOutput::Silent;
    }
    let state = match intake_outcome::load(&resolved) {
        Ok(Some(state)) => state,
        // No state: either no prompt was marked (pre-P7A hooks) or the
        // session never started — fail open (FR-015).
        Ok(None) => return HookOutput::Silent,
        // Malformed state fails open for hooks (FR-014u).
        Err(_) => return HookOutput::Silent,
    };
    if let Some(current) = current_session {
        if current != state.session_id {
            // Another session's state — stay silent (FR-014t convention).
            return HookOutput::Silent;
        }
    }
    // FR-016: the gate is armed only when a prompt marked the requirement
    // dirty. Outcome-only state (no marker) has nothing to enforce.
    if state.required_since.is_none() {
        return HookOutput::Silent;
    }
    if state.has_fresh_valid_outcome() {
        return HookOutput::Silent;
    }

    let situation = describe_gate_violation(&state);
    let capture_note = if lane.policy_flags.self_improvement_capture {
        match improvement::capture_intake_gate_violation(
            &resolved,
            "Intake session attempted to stop without a fresh Issue/SPEC outcome (intake artifact gate)",
            &format!(
                "session {session}: {situation}",
                session = state.session_id
            ),
        ) {
            Ok(summary) => format!(
                "Self-improvement candidate {id} {verb} (occurrences: {count}).",
                id = summary.id,
                verb = if summary.updated { "updated" } else { "captured" },
                count = summary.occurrences,
            ),
            // T-085: a capture failure must not silently pass (or silently
            // drop the bookkeeping) — surface it with a manual fallback.
            Err(error) => format!(
                "Self-improvement auto-capture failed ({error}). Record it manually: run JSON operation `improvement.capture` with `params.source:\"hook-runtime\"`, `params.target_artifact:\"issue-spec-workflow\"`, `params.classification:\"gwt-caused\"`, `params.confidence:\"medium\"`, `params.summary:\"intake artifact gate violation\"`, and `params.dedupe_key:\"{key}\"`.",
                key = improvement::INTAKE_GATE_DEDUPE_KEY,
            ),
        }
    } else {
        String::new()
    };

    let mut reason = format!(
        "Intake artifact gate: {situation} Board posts and prose answers do not count as outcomes (FR-017).\n\
         Settle the curation before stopping:\n\
         - create or update the owner Issue/SPEC via JSON operations `issue.create`, `issue.comment`, `issue.spec.create`, or `issue.spec.edit` (successful operations auto-record the outcome), or\n\
         - record an explicit decision: JSON operation `intake.outcome.record` with `params.kind:\"no_action\"` and a non-empty `params.reason` (use kind `issue_updated` / `spec_updated` with `params.number` when the durable update happened outside this session)."
    );
    if !capture_note.is_empty() {
        reason.push('\n');
        reason.push_str(&capture_note);
    }
    HookOutput::stop_block(reason)
}

/// Human-readable violation summary for the StopBlock reason.
fn describe_gate_violation(state: &intake_outcome::IntakeOutcomeState) -> String {
    match &state.outcome {
        None => {
            "no Issue/SPEC outcome has been recorded since the latest user prompt.".to_string()
        }
        Some(outcome) if !outcome.is_valid() => format!(
            "the recorded outcome '{}' is invalid ({}).",
            outcome.kind.as_str(),
            outcome
                .validate()
                .err()
                .unwrap_or_else(|| "validation failed".to_string()),
        ),
        Some(outcome) => format!(
            "the last outcome '{}' (recorded {}) predates the latest user prompt — record a fresh outcome for this prompt.",
            outcome.kind.as_str(),
            outcome.recorded_at.to_rfc3339(),
        ),
    }
}

fn current_session_from_env() -> Option<String> {
    std::env::var(gwt_agent::GWT_SESSION_ID_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Extract the `prompt` field from the UserPromptSubmit payload. Permissive:
/// any parse failure or missing field means "nothing to classify".
fn prompt_from_input(input: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(input).ok()?;
    value
        .get("prompt")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

/// FR-168 coarse classification, in force until the typed FR-138..FR-150
/// prompt obligations land: short question-shaped prompts are pure
/// status/questions and skip the dirty marker UNLESS they carry an explicit
/// request form ("〜して", "please …", "can you …"). Everything that is not
/// question-shaped counts as curation/producing work — the gate errs toward
/// enforcement on ambiguity, but noun mentions of work ("any updates?",
/// "登録済みですか？") must not arm the gate (FR-168/AS-150).
fn is_pure_status_question(prompt: &str) -> bool {
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        return true;
    }
    let ends_with_question = trimmed.ends_with('?') || trimmed.ends_with('？');
    if !ends_with_question || trimmed.chars().count() > 200 {
        return false;
    }
    // A question that phrases a work request is not "pure". Match request
    // *forms* (imperative/te-form suffixes, polite request prefixes) rather
    // than bare work nouns, so noun-form status questions stay exempt.
    const REQUEST_FORMS: &[&str] = &[
        "登録して",
        "作成して",
        "更新して",
        "修正して",
        "実装して",
        "追加して",
        "記録して",
        "やって",
        "進めて",
        "お願い",
        "ください",
        "してもらえ",
        "してくれ",
        "can you ",
        "could you ",
        "would you ",
        "please ",
    ];
    let lower = trimmed.to_lowercase();
    !REQUEST_FORMS.iter().any(|form| lower.contains(form))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use gwt_core::test_support::ScopedEnvVar;
    use gwt_skills::{write_lane_file, EXECUTION_PROFILE, INTAKE_PROFILE};

    fn mk_worktree(profile: Option<&gwt_skills::LaneProfile>) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".gwt")).unwrap();
        if let Some(profile) = profile {
            write_lane_file(dir.path(), profile).unwrap();
        }
        dir
    }

    fn record_valid_outcome(worktree: &Path, session: &str, recorded_at: chrono::DateTime<Utc>) {
        intake_outcome::record_outcome(
            worktree,
            session,
            intake_outcome::IntakeOutcome {
                kind: intake_outcome::IntakeOutcomeKind::IssueUpdated,
                number: Some(3248),
                reason: None,
                source_operation: "issue.comment".to_string(),
                recorded_at,
            },
        )
        .unwrap();
    }

    fn candidate_values(worktree: &Path) -> Vec<serde_json::Value> {
        improvement::candidate_public_values(worktree)
    }

    // AS-8 / T-081: a dirty marker with no outcome blocks Stop — Board posts
    // and prose answers never write outcome state, so "Board-only handoff"
    // is exactly this shape.
    #[test]
    fn blocks_when_dirty_and_no_outcome_recorded() {
        let dir = mk_worktree(Some(&INTAKE_PROFILE));
        intake_outcome::mark_required_since(dir.path(), "sess-1", Utc::now()).unwrap();

        let output = handle_with_input(dir.path(), "{}", Some("sess-1"));
        let HookOutput::StopBlock { reason } = output else {
            panic!("expected StopBlock, got {output:?}");
        };
        assert!(reason.contains("Intake artifact gate"), "{reason}");
        assert!(reason.contains("intake.outcome.record"), "{reason}");
        assert!(reason.contains("no_action"), "{reason}");
        assert!(reason.contains("issue.spec.edit"), "{reason}");
    }

    // AS-9: a fresh valid Issue/SPEC outcome passes Stop.
    #[test]
    fn passes_with_fresh_valid_outcome() {
        let dir = mk_worktree(Some(&INTAKE_PROFILE));
        intake_outcome::mark_required_since(dir.path(), "sess-1", Utc::now()).unwrap();
        record_valid_outcome(dir.path(), "sess-1", Utc::now() + Duration::seconds(1));

        assert_eq!(
            handle_with_input(dir.path(), "{}", Some("sess-1")),
            HookOutput::Silent
        );
    }

    // AS-11 / T-080: an outcome recorded before the latest prompt is stale.
    #[test]
    fn blocks_stale_outcome_after_later_prompt() {
        let dir = mk_worktree(Some(&INTAKE_PROFILE));
        record_valid_outcome(dir.path(), "sess-1", Utc::now() - Duration::minutes(5));
        intake_outcome::mark_required_since(dir.path(), "sess-1", Utc::now()).unwrap();

        let output = handle_with_input(dir.path(), "{}", Some("sess-1"));
        let HookOutput::StopBlock { reason } = output else {
            panic!("expected StopBlock for stale outcome, got {output:?}");
        };
        assert!(
            reason.contains("predates the latest user prompt"),
            "{reason}"
        );
    }

    // AS-10: an invalid persisted no_action (empty reason) cannot pass.
    #[test]
    fn blocks_invalid_no_action_outcome() {
        let dir = mk_worktree(Some(&INTAKE_PROFILE));
        let now = Utc::now();
        intake_outcome::mark_required_since(dir.path(), "sess-1", now).unwrap();
        // Bypass strict validation by writing the state directly — the gate
        // must still refuse to accept the invalid outcome.
        intake_outcome::save(
            dir.path(),
            &intake_outcome::IntakeOutcomeState {
                session_id: "sess-1".to_string(),
                required_since: Some(now),
                outcome: Some(intake_outcome::IntakeOutcome {
                    kind: intake_outcome::IntakeOutcomeKind::NoAction,
                    number: None,
                    reason: Some("   ".to_string()),
                    source_operation: "intake.outcome.record".to_string(),
                    recorded_at: now + Duration::seconds(1),
                }),
            },
        )
        .unwrap();

        let output = handle_with_input(dir.path(), "{}", Some("sess-1"));
        let HookOutput::StopBlock { reason } = output else {
            panic!("expected StopBlock for invalid outcome, got {output:?}");
        };
        assert!(reason.contains("invalid"), "{reason}");
    }

    // FR-015: execution lane and missing lane file never fire the gate.
    #[test]
    fn execution_lane_and_missing_lane_file_stay_silent() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _kind = ScopedEnvVar::unset(gwt_skills::GWT_SESSION_KIND_ENV);

        for profile in [Some(&EXECUTION_PROFILE), None] {
            let dir = mk_worktree(profile);
            intake_outcome::mark_required_since(dir.path(), "sess-1", Utc::now()).unwrap();
            assert_eq!(
                handle_with_input(dir.path(), "{}", Some("sess-1")),
                HookOutput::Silent,
                "profile {profile:?} must not fire the intake gate"
            );
        }
    }

    // FR-014o: stop_hook_active short-circuits (one forced continuation).
    #[test]
    fn stop_hook_active_short_circuits() {
        let dir = mk_worktree(Some(&INTAKE_PROFILE));
        intake_outcome::mark_required_since(dir.path(), "sess-1", Utc::now()).unwrap();
        assert_eq!(
            handle_with_input(dir.path(), r#"{"stop_hook_active":true}"#, Some("sess-1")),
            HookOutput::Silent
        );
    }

    // FR-014t convention: another session's state stays silent.
    #[test]
    fn session_mismatch_stays_silent() {
        let dir = mk_worktree(Some(&INTAKE_PROFILE));
        intake_outcome::mark_required_since(dir.path(), "sess-1", Utc::now()).unwrap();
        assert_eq!(
            handle_with_input(dir.path(), "{}", Some("other-session")),
            HookOutput::Silent
        );
    }

    // FR-014u: malformed state and missing state fail open.
    #[test]
    fn malformed_or_missing_state_fails_open() {
        let dir = mk_worktree(Some(&INTAKE_PROFILE));
        assert_eq!(
            handle_with_input(dir.path(), "{}", Some("sess-1")),
            HookOutput::Silent,
            "missing state must fail open"
        );
        let path = intake_outcome::state_path(dir.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{not json").unwrap();
        assert_eq!(
            handle_with_input(dir.path(), "{}", Some("sess-1")),
            HookOutput::Silent,
            "malformed state must fail open"
        );
    }

    // Outcome-only state (no dirty marker) has nothing to enforce.
    #[test]
    fn outcome_without_marker_stays_silent() {
        let dir = mk_worktree(Some(&INTAKE_PROFILE));
        record_valid_outcome(dir.path(), "sess-1", Utc::now());
        assert_eq!(
            handle_with_input(dir.path(), "{}", Some("sess-1")),
            HookOutput::Silent
        );
    }

    // T-084 / AS-13 / AS-14: blocking auto-captures one legacy-compatible
    // issue-spec-workflow candidate with the stable dedupe key. Repeats update
    // only the aggregate legacy count and never invent typed occurrences.
    #[test]
    fn block_auto_captures_single_candidate_with_stable_dedupe() {
        let dir = mk_worktree(Some(&INTAKE_PROFILE));
        intake_outcome::mark_required_since(dir.path(), "sess-1", Utc::now()).unwrap();

        for expected_occurrences in [1u64, 2u64] {
            let output = handle_with_input(dir.path(), "{}", Some("sess-1"));
            assert!(matches!(output, HookOutput::StopBlock { .. }));
            let candidates = candidate_values(dir.path());
            assert_eq!(candidates.len(), 1, "exactly one candidate expected");
            let candidate = &candidates[0];
            assert_eq!(
                candidate.get("target_artifact").and_then(|v| v.as_str()),
                Some("issue-spec-workflow")
            );
            assert_eq!(
                candidate.get("occurrences").and_then(|v| v.as_u64()),
                Some(0),
                "untyped intake captures must not count as distinct evidence"
            );
            assert_eq!(
                candidate
                    .get("legacy_occurrence_count")
                    .and_then(|v| v.as_u64()),
                Some(expected_occurrences)
            );
        }
    }

    // T-085: a capture failure surfaces in the StopBlock with a manual
    // fallback — the gate still blocks, never silently passes.
    #[test]
    fn capture_failure_surfaces_in_stop_block() {
        let dir = mk_worktree(Some(&INTAKE_PROFILE));
        intake_outcome::mark_required_since(dir.path(), "sess-1", Utc::now()).unwrap();
        // Make the candidate store unwritable by planting a directory at the
        // store file path.
        let store_path = crate::cli::improvement_store::candidate_store_path(dir.path());
        std::fs::create_dir_all(&store_path).unwrap();

        let output = handle_with_input(dir.path(), "{}", Some("sess-1"));
        let HookOutput::StopBlock { reason } = output else {
            panic!("expected StopBlock despite capture failure, got {output:?}");
        };
        assert!(reason.contains("auto-capture failed"), "{reason}");
        assert!(reason.contains("improvement.capture"), "{reason}");
        assert!(
            reason.contains(improvement::INTAKE_GATE_DEDUPE_KEY),
            "{reason}"
        );
    }

    // T-079 + FR-168: producing prompts mark the requirement dirty; pure
    // status/question prompts do not.
    #[test]
    fn user_prompt_submit_marks_dirty_for_producing_prompts_only() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "sess-1");

        let dir = mk_worktree(Some(&INTAKE_PROFILE));
        // Pure status question — no dirty marker.
        handle_user_prompt_submit(
            dir.path(),
            r#"{"prompt":"いまの進捗はどうなっていますか？"}"#,
        );
        assert_eq!(intake_outcome::load(dir.path()).unwrap(), None);

        // Producing prompt — dirty marker stored for the session.
        handle_user_prompt_submit(dir.path(), r#"{"prompt":"このバグを Issue に登録して"}"#);
        let state = intake_outcome::load(dir.path()).unwrap().unwrap();
        assert_eq!(state.session_id, "sess-1");
        assert!(state.required_since.is_some());
    }

    #[test]
    fn user_prompt_submit_ignores_execution_lane_and_arms_on_missing_prompt() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "sess-1");

        let execution = mk_worktree(Some(&EXECUTION_PROFILE));
        handle_user_prompt_submit(execution.path(), r#"{"prompt":"登録して"}"#);
        assert_eq!(intake_outcome::load(execution.path()).unwrap(), None);

        // Provider bridges do not normalize a `prompt` field yet — an
        // unknown prompt is enforced (marked dirty), never exempted, so the
        // gate cannot go dormant on those providers.
        let intake = mk_worktree(Some(&INTAKE_PROFILE));
        handle_user_prompt_submit(intake.path(), "{}");
        let state = intake_outcome::load(intake.path()).unwrap().unwrap();
        assert!(state.required_since.is_some());
    }

    #[test]
    fn pure_status_question_classifier_is_conservative() {
        assert!(is_pure_status_question(""));
        assert!(is_pure_status_question("進捗は？"));
        assert!(is_pure_status_question("What is the current status?"));
        // FR-168/AS-150: noun mentions of work inside a status question must
        // stay exempt — only request *forms* arm the gate.
        assert!(is_pure_status_question("any updates?"));
        assert!(is_pure_status_question("何か更新はありますか？"));
        assert!(is_pure_status_question("登録済みですか？"));
        assert!(is_pure_status_question("is the fix released?"));
        assert!(is_pure_status_question("Issue はもう作成されましたか？"));
        // Requests phrased as questions still count as producing work.
        assert!(!is_pure_status_question("can you create the issue?"));
        assert!(!is_pure_status_question(
            "この内容で Issue を登録してもらえますか？"
        ));
        assert!(!is_pure_status_question("please register this bug?"));
        // No question mark → producing by default (conservative).
        assert!(!is_pure_status_question("SPEC を確認"));
        assert!(!is_pure_status_question("進めて"));
        assert!(!is_pure_status_question("このバグを Issue に登録して"));
    }
}
