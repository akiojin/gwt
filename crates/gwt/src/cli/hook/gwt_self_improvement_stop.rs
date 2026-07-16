//! gwt-repository-only self-improvement Stop hook.
//!
//! This hook is intentionally separate from the shared gwt Managed Hooks
//! dispatcher. It belongs to the `akiojin/gwt` repository's own development
//! loop, not to arbitrary projects managed by gwt.

use std::{path::Path, time::Duration};

use gwt_github::client::ResolutionDeadline;

use super::{envelope::stop_hook_active_from, HookOutput};
use crate::cli::{
    improvement::{
        is_contract_artifact, BlockedReason, CandidateState, CaptureBudgetProfile, FailureSubcode,
        ImprovementCandidate,
    },
    improvement_owner::{
        owner_resolution_failure_from_error, repair_source_success_snapshots,
        resolve_candidate_owner_with_operation_deadline, retry_pending_owner_status_with_deadline,
        OwnerResolutionFailure,
    },
    improvement_store, CliEnv,
};

const DIRECT_STOP_SETTLEMENT_RESERVE: Duration = Duration::from_millis(750);

pub fn handle_with_input<E: CliEnv>(env: &mut E, input: &str) -> HookOutput {
    let deadline = CaptureBudgetProfile::StrictStop.resolution_deadline();
    let worktree_root = env.repo_path().to_path_buf();
    // SPEC-3248 (hooks v2 P3): whether this Stop gate fires is a lane policy,
    // resolved from the worktree lane file (source of truth) via the shared
    // HookContext. A lane whose profile disables `self_improvement_stop`
    // (intake today) suppresses the gate. Replaces the SPEC-3247 ad-hoc
    // `SessionKind::from_env().is_intake()` branch.
    let suppress = !super::context::HookContext::for_worktree(&worktree_root)
        .lane
        .policy_flags
        .self_improvement_stop;
    evaluate_with_deadline(env, stop_hook_active_from(input), suppress, &deadline)
}

/// Decide the self-improvement Stop block.
///
/// SPEC-3247 FR-003 / AS-4 → SPEC-3248 (hooks v2): this is a producing-work Stop
/// gate. A lane whose profile disables `self_improvement_stop` (intake today)
/// must never be forced to handle improvement candidates before stopping, so
/// `suppressed_by_lane` short-circuits to [`HookOutput::Silent`] alongside the
/// existing `stop_hook_active` / non-gwt-repo guards.
pub fn evaluate_with_env<E: CliEnv>(
    env: &mut E,
    stop_hook_active: bool,
    suppressed_by_lane: bool,
) -> HookOutput {
    let deadline = CaptureBudgetProfile::StrictStop.resolution_deadline();
    evaluate_with_deadline(env, stop_hook_active, suppressed_by_lane, &deadline)
}

fn evaluate_with_deadline<E: CliEnv>(
    env: &mut E,
    stop_hook_active: bool,
    suppressed_by_lane: bool,
    deadline: &ResolutionDeadline,
) -> HookOutput {
    let worktree_root = env.repo_path().to_path_buf();
    if stop_hook_active || suppressed_by_lane {
        return HookOutput::Silent;
    }
    match is_gwt_repository_with_deadline(&worktree_root, deadline) {
        Ok(true) => {}
        Ok(false) => return HookOutput::Silent,
        Err(failure) => return repository_probe_failure_block(failure),
    }
    let _operation_deadline =
        gwt_core::operation_deadline::ScopedOperationDeadline::enter(deadline.expires_at());

    let candidates = match load_stop_candidates(&worktree_root) {
        Ok(candidates) => candidates,
        Err(failure) => return evaluation_failure_block(deadline, failure),
    };
    let mut attempt_failure = None;
    if let Some(candidate_id) = select_pending_owner_status_candidate(&candidates) {
        let resolution_deadline = deadline.reserving(DIRECT_STOP_SETTLEMENT_RESERVE);
        let _ = retry_pending_owner_status_with_deadline(env, &candidate_id, &resolution_deadline);
    } else if let Some(candidate_id) = select_attempt_candidate(&candidates) {
        let resolution_deadline = deadline.reserving(DIRECT_STOP_SETTLEMENT_RESERVE);
        if let Err(error) = resolve_candidate_owner_with_operation_deadline(
            env,
            &candidate_id,
            CaptureBudgetProfile::StrictStop,
            &resolution_deadline,
            deadline.expires_at(),
        ) {
            let failure = if resolution_deadline
                .remaining("strict Stop Owner Resolution")
                .is_err()
            {
                OwnerResolutionFailure {
                    reason: BlockedReason::Timeout,
                    failure_subcode: None,
                    remediation: "RETRY_WITHIN_BUDGET",
                }
            } else {
                owner_resolution_failure_from_error(&error)
            };
            attempt_failure = Some((candidate_id, failure));
        }
    }

    match load_stop_candidates(&worktree_root) {
        Ok(mut candidates) => {
            if let Some((candidate_id, failure)) = attempt_failure {
                apply_attempt_failure(&mut candidates, &candidate_id, failure);
            }
            render_stop_result(&candidates)
        }
        Err(failure) => evaluation_failure_block(deadline, failure),
    }
}

#[derive(Debug, Clone)]
struct StopCandidate {
    id: String,
    updated_at: String,
    target_artifact: String,
    state: CandidateState,
    blocked_reason: Option<BlockedReason>,
    failure_subcode: Option<FailureSubcode>,
    remediation: Option<String>,
    active_attempt: bool,
    remote_mutation_seen: bool,
    pending_owner_status: bool,
}

const fn is_attemptable(state: CandidateState) -> bool {
    matches!(
        state,
        CandidateState::OwnerResolving
            | CandidateState::Blocked
            | CandidateState::RemoteOutcomeUnknown
            | CandidateState::Recurrent
    )
}

const fn blocks_stop(state: CandidateState) -> bool {
    matches!(
        state,
        CandidateState::Pending
            | CandidateState::OwnerResolving
            | CandidateState::Blocked
            | CandidateState::RemoteOutcomeUnknown
            | CandidateState::Recurrent
    )
}

fn candidate_blocks_stop(candidate: &StopCandidate) -> bool {
    has_actionable_pending_owner_status(candidate) || blocks_stop(candidate.state)
}

fn has_actionable_pending_owner_status(candidate: &StopCandidate) -> bool {
    candidate.pending_owner_status
        && matches!(
            candidate.state,
            CandidateState::Linked | CandidateState::Created
        )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StopEvaluationFailure {
    OwnerProjection,
    CandidateStore,
}

fn load_stop_candidates(worktree_root: &Path) -> Result<Vec<StopCandidate>, StopEvaluationFailure> {
    let store = load_candidate_store_after_projection_repair_with(
        worktree_root,
        repair_source_success_snapshots,
    )?;
    let mut candidates = store
        .candidates
        .into_iter()
        .filter(is_high_confidence_gwt_contract_candidate)
        .map(|candidate| {
            let active_attempt = candidate
                .attempt
                .as_ref()
                .is_some_and(|attempt| attempt.expires_at > chrono::Utc::now());
            let remote_mutation_seen = candidate
                .attempt
                .as_ref()
                .is_some_and(|attempt| attempt.remote_mutation_seen);
            let pending_owner_status =
                candidate.owner_status_delivered_generation < candidate.owner_status_generation;
            StopCandidate {
                id: candidate.id,
                updated_at: candidate.updated_at,
                target_artifact: candidate.target_artifact,
                state: candidate.state,
                blocked_reason: candidate.blocked_reason,
                failure_subcode: candidate.failure_subcode,
                remediation: candidate.retry.map(|retry| retry.remediation),
                active_attempt,
                remote_mutation_seen,
                pending_owner_status,
            }
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(candidates)
}

fn load_candidate_store_after_projection_repair_with<F>(
    worktree_root: &Path,
    repair: F,
) -> Result<crate::cli::improvement::CandidateStore, StopEvaluationFailure>
where
    F: FnOnce(&Path) -> Result<bool, gwt_github::SpecOpsError>,
{
    improvement_store::load_and_repair(worktree_root)
        .map_err(|_| StopEvaluationFailure::CandidateStore)?;
    repair(worktree_root).map_err(|_| StopEvaluationFailure::OwnerProjection)?;
    improvement_store::load_and_repair(worktree_root)
        .map_err(|_| StopEvaluationFailure::CandidateStore)
}

fn select_pending_owner_status_candidate(candidates: &[StopCandidate]) -> Option<String> {
    candidates
        .iter()
        .filter(|candidate| has_actionable_pending_owner_status(candidate))
        .min_by(|left, right| {
            left.updated_at
                .cmp(&right.updated_at)
                .then_with(|| left.id.cmp(&right.id))
        })
        .map(|candidate| candidate.id.clone())
}

fn select_attempt_candidate(candidates: &[StopCandidate]) -> Option<String> {
    candidates
        .iter()
        .filter(|candidate| is_attemptable(candidate.state) && !candidate.active_attempt)
        .min_by(|left, right| {
            left.updated_at
                .cmp(&right.updated_at)
                .then_with(|| left.id.cmp(&right.id))
        })
        .map(|candidate| candidate.id.clone())
}

fn is_high_confidence_gwt_contract_candidate(candidate: &ImprovementCandidate) -> bool {
    candidate.classification == "gwt-caused"
        && candidate.confidence == "high"
        && is_contract_artifact(&candidate.target_artifact)
}

fn render_stop_result(candidates: &[StopCandidate]) -> HookOutput {
    let unresolved = candidates
        .iter()
        .filter(|candidate| candidate_blocks_stop(candidate))
        .collect::<Vec<_>>();
    if unresolved.is_empty() {
        return HookOutput::Silent;
    }

    let mut reason = String::from(
        "High-confidence gwt self-improvement Owner Resolution remains unresolved.\n\n",
    );
    reason.push_str("Unresolved candidates:\n");
    for candidate in unresolved {
        let blocked_reason = if has_actionable_pending_owner_status(candidate) {
            "status-delivery"
        } else {
            candidate
                .blocked_reason
                .map(blocked_reason_token)
                .unwrap_or_else(|| unresolved_state_reason(candidate.state))
        };
        let failure_subcode = candidate
            .failure_subcode
            .map(failure_subcode_token)
            .unwrap_or("none");
        let remediation = if has_actionable_pending_owner_status(candidate) {
            "RETRY_OWNER_STATUS"
        } else {
            candidate
                .remediation
                .as_deref()
                .unwrap_or_else(|| unresolved_state_remediation(candidate.state))
        };
        reason.push_str(&format!(
            "- {} [{}]: state={} reason={} subcode={} remediation={}\n",
            candidate.id,
            candidate.target_artifact,
            candidate.state.as_str(),
            blocked_reason,
            failure_subcode,
            remediation,
        ));
    }
    reason.push_str(
        "\nRetry action: apply the listed remediation, then rerun the direct Stop hook. Use the Improvement Inbox for ambiguity selection or evidence remediation.",
    );
    HookOutput::StopBlock { reason }
}

fn evaluation_failure_block(
    deadline: &ResolutionDeadline,
    failure: StopEvaluationFailure,
) -> HookOutput {
    if deadline.remaining("direct Stop state evaluation").is_err() {
        HookOutput::StopBlock {
            reason: "High-confidence gwt self-improvement state could not be evaluated.\n\n- state-evaluation: state=blocked reason=timeout subcode=none remediation=RETRY_WITHIN_BUDGET\n\nRetry action: release the contended resource, then rerun the direct Stop hook."
                .to_string(),
        }
    } else {
        let (artifact, reason, remediation, retry_action) = match failure {
            StopEvaluationFailure::OwnerProjection => (
                "owner-projection",
                "local-commit",
                "REPAIR_OWNER_PROJECTION",
                "repair the owner projection",
            ),
            StopEvaluationFailure::CandidateStore => (
                "candidate-store",
                "store",
                "REPAIR_CANDIDATE_STORE",
                "repair the candidate store",
            ),
        };
        HookOutput::StopBlock {
            reason: format!(
                "High-confidence gwt self-improvement state could not be evaluated.\n\n- {artifact}: state=blocked reason={reason} subcode=none remediation={remediation}\n\nRetry action: {retry_action}, then rerun the direct Stop hook."
            ),
        }
    }
}

fn apply_attempt_failure(
    candidates: &mut [StopCandidate],
    candidate_id: &str,
    failure: OwnerResolutionFailure,
) {
    let Some(candidate) = candidates
        .iter_mut()
        .find(|candidate| candidate.id == candidate_id && blocks_stop(candidate.state))
    else {
        return;
    };
    if candidate.state != CandidateState::OwnerResolving
        || candidate.blocked_reason.is_some()
        || candidate.active_attempt
    {
        return;
    }
    if candidate.remote_mutation_seen {
        candidate.state = CandidateState::RemoteOutcomeUnknown;
        candidate.failure_subcode = None;
        candidate.remediation = Some("REFRESH_OWNER_CORPUS".to_string());
        return;
    }
    candidate.state = CandidateState::Blocked;
    candidate.blocked_reason = Some(failure.reason);
    candidate.failure_subcode = failure.failure_subcode;
    candidate.remediation = Some(failure.remediation.to_string());
}

pub fn is_gwt_repository(worktree_root: &Path) -> bool {
    let deadline = CaptureBudgetProfile::StrictStop.resolution_deadline();
    is_gwt_repository_with_deadline(worktree_root, &deadline).unwrap_or(false)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RepositoryProbeFailure {
    Timeout,
    Routing,
}

fn is_gwt_repository_with_deadline(
    worktree_root: &Path,
    deadline: &ResolutionDeadline,
) -> Result<bool, RepositoryProbeFailure> {
    Ok(origin_remote_url(worktree_root, deadline)?
        .and_then(|url| github_slug_from_remote_url(&url))
        .is_some_and(|slug| slug == "akiojin/gwt"))
}

fn origin_remote_url(
    worktree_root: &Path,
    deadline: &ResolutionDeadline,
) -> Result<Option<String>, RepositoryProbeFailure> {
    let root = gwt_core::paths::resolve_current_worktree_root(worktree_root);
    let root = root.to_str().ok_or(RepositoryProbeFailure::Routing)?;
    let hub = gwt_core::process_console::global();
    let output = gwt_core::process_console::spawn_logged_blocking_with_deadline(
        &hub,
        gwt_core::process_console::ProcessKind::Git,
        "git",
        &["-C", root, "config", "--get", "remote.origin.url"],
        gwt_core::process_console::SpawnOptions::new("git remote origin for self-improvement Stop")
            .forward_output(false),
        deadline.expires_at(),
    )
    .map_err(|error| {
        if error.kind() == std::io::ErrorKind::TimedOut {
            RepositoryProbeFailure::Timeout
        } else {
            RepositoryProbeFailure::Routing
        }
    })?;
    if !output.success() {
        return match output.exit_code {
            Some(1) => Ok(None),
            _ => Err(RepositoryProbeFailure::Routing),
        };
    }
    let value = output.stdout.trim().to_string();
    Ok((!value.is_empty()).then_some(value))
}

fn repository_probe_failure_block(failure: RepositoryProbeFailure) -> HookOutput {
    let reason = match failure {
        RepositoryProbeFailure::Timeout => "timeout",
        RepositoryProbeFailure::Routing => "routing",
    };
    HookOutput::StopBlock {
        reason: format!(
            "High-confidence gwt self-improvement state could not be evaluated.\n\n- repository-probe: state=blocked reason={reason} subcode=none remediation=RETRY_REPOSITORY_PROBE\n\nRetry action: restore repository access, then rerun the direct Stop hook."
        ),
    }
}

fn blocked_reason_token(reason: BlockedReason) -> &'static str {
    match reason {
        BlockedReason::Store => "store",
        BlockedReason::Search => "search",
        BlockedReason::Auth => "auth",
        BlockedReason::Privacy => "privacy",
        BlockedReason::Ambiguity => "ambiguity",
        BlockedReason::Routing => "routing",
        BlockedReason::Create => "create",
        BlockedReason::Update => "update",
        BlockedReason::Readback => "readback",
        BlockedReason::LocalCommit => "local-commit",
        BlockedReason::Timeout => "timeout",
        BlockedReason::RateLimit => "rate-limit",
        BlockedReason::Network => "network",
        BlockedReason::Parse => "parse",
        BlockedReason::Reconciliation => "reconciliation",
    }
}

fn failure_subcode_token(subcode: FailureSubcode) -> &'static str {
    match subcode {
        FailureSubcode::EmptyCorpus => "empty-corpus",
        FailureSubcode::PartialPage => "partial-page",
    }
}

fn unresolved_state_reason(state: CandidateState) -> &'static str {
    match state {
        CandidateState::RemoteOutcomeUnknown => "remote-outcome-unknown",
        CandidateState::OwnerResolving => "owner-resolving",
        CandidateState::Recurrent => "recurrent",
        CandidateState::Pending | CandidateState::Blocked => "store",
        _ => "none",
    }
}

fn unresolved_state_remediation(state: CandidateState) -> &'static str {
    match state {
        CandidateState::Pending => "REPAIR_CANDIDATE_STORE",
        CandidateState::RemoteOutcomeUnknown => "REFRESH_OWNER_CORPUS",
        CandidateState::OwnerResolving | CandidateState::Blocked | CandidateState::Recurrent => {
            "RETRY_OWNER_RESOLUTION"
        }
        _ => "NONE",
    }
}

fn github_slug_from_remote_url(url: &str) -> Option<String> {
    let value = url.trim().trim_end_matches('/').trim_end_matches(".git");
    let rest = if let Some(rest) = ["https://", "http://", "ssh://"]
        .into_iter()
        .find_map(|scheme| value.strip_prefix(scheme))
    {
        let (authority, path) = rest.split_once('/')?;
        let host = authority.rsplit('@').next()?.split(':').next()?;
        if !host.eq_ignore_ascii_case("github.com") {
            return None;
        }
        path
    } else {
        value.strip_prefix("git@github.com:")?
    };
    let slug = rest
        .trim_matches('/')
        .trim_end_matches(".git")
        .to_ascii_lowercase();
    let mut parts = slug.split('/');
    let owner = parts.next()?.trim();
    let repo = parts.next()?.trim();
    if owner.is_empty() || repo.is_empty() || parts.next().is_some() {
        return None;
    }
    Some(format!("{owner}/{repo}"))
}

#[cfg(test)]
mod tests {
    use std::{fs::OpenOptions, process::Command, time::Duration};

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use fs2::FileExt;
    use gwt_core::test_support::ScopedGwtHome;

    use super::{
        apply_attempt_failure, evaluate_with_deadline, github_slug_from_remote_url,
        load_candidate_store_after_projection_repair_with, origin_remote_url, render_stop_result,
        select_attempt_candidate, select_pending_owner_status_candidate, BlockedReason,
        CandidateState, FailureSubcode, HookOutput, OwnerResolutionFailure, RepositoryProbeFailure,
        ResolutionDeadline, StopCandidate, StopEvaluationFailure,
    };
    use crate::cli::TestEnv;

    fn stop_candidate(
        state: CandidateState,
        blocked_reason: Option<BlockedReason>,
        failure_subcode: Option<FailureSubcode>,
    ) -> StopCandidate {
        StopCandidate {
            id: "impr-test".to_string(),
            updated_at: "2026-07-16T00:00:00Z".to_string(),
            target_artifact: "hook".to_string(),
            state,
            blocked_reason,
            failure_subcode,
            remediation: Some("RETRY_OWNER_RESOLUTION".to_string()),
            active_attempt: false,
            remote_mutation_seen: false,
            pending_owner_status: false,
        }
    }

    #[test]
    fn parses_github_remote_urls() {
        for url in [
            "https://github.com/akiojin/gwt.git",
            "https://github.com/akiojin/gwt",
            "https://x-access-token:github_pat_secret@github.com/akiojin/gwt.git",
            "git@github.com:akiojin/gwt.git",
            "ssh://git@github.com/akiojin/gwt.git",
        ] {
            assert_eq!(
                github_slug_from_remote_url(url).as_deref(),
                Some("akiojin/gwt"),
                "{url}"
            );
        }
    }

    #[test]
    fn rejects_non_github_urls() {
        assert_eq!(github_slug_from_remote_url("file:///tmp/gwt.git"), None);
        assert_eq!(
            github_slug_from_remote_url("https://example.com/akiojin/gwt.git"),
            None
        );
    }

    #[test]
    fn verified_or_non_actionable_states_do_not_block_stop() {
        for state in [
            CandidateState::NeedsEvidence,
            CandidateState::Linked,
            CandidateState::Created,
            CandidateState::Parked,
            CandidateState::Dismissed,
        ] {
            assert_eq!(
                render_stop_result(&[stop_candidate(state, None, None)]),
                HookOutput::Silent,
                "{state:?}"
            );
        }
    }

    #[test]
    fn pending_owner_status_does_not_override_a_non_actionable_state() {
        for state in [CandidateState::Parked, CandidateState::Dismissed] {
            let mut candidate = stop_candidate(state, None, None);
            candidate.pending_owner_status = true;

            assert_eq!(
                select_pending_owner_status_candidate(&[candidate.clone()]),
                None,
                "{state:?}"
            );
            assert_eq!(
                render_stop_result(&[candidate]),
                HookOutput::Silent,
                "{state:?}"
            );
        }
    }

    #[test]
    fn pending_owner_status_blocks_with_a_typed_retry_action() {
        let mut candidate = stop_candidate(CandidateState::Linked, None, None);
        candidate.pending_owner_status = true;
        candidate.remediation = None;

        assert_eq!(
            select_pending_owner_status_candidate(&[candidate.clone()]).as_deref(),
            Some("impr-test")
        );
        let HookOutput::StopBlock { reason } = render_stop_result(&[candidate]) else {
            panic!("pending owner status must block Stop");
        };
        assert!(reason.contains("state=linked"), "{reason}");
        assert!(reason.contains("reason=status-delivery"), "{reason}");
        assert!(reason.contains("subcode=none"), "{reason}");
        assert!(
            reason.contains("remediation=RETRY_OWNER_STATUS"),
            "{reason}"
        );
    }

    #[test]
    fn every_typed_blocked_reason_is_rendered_exactly() {
        for (blocked_reason, token) in [
            (BlockedReason::Store, "store"),
            (BlockedReason::Search, "search"),
            (BlockedReason::Auth, "auth"),
            (BlockedReason::Privacy, "privacy"),
            (BlockedReason::Ambiguity, "ambiguity"),
            (BlockedReason::Routing, "routing"),
            (BlockedReason::Create, "create"),
            (BlockedReason::Update, "update"),
            (BlockedReason::Readback, "readback"),
            (BlockedReason::LocalCommit, "local-commit"),
            (BlockedReason::Timeout, "timeout"),
            (BlockedReason::RateLimit, "rate-limit"),
            (BlockedReason::Network, "network"),
            (BlockedReason::Parse, "parse"),
            (BlockedReason::Reconciliation, "reconciliation"),
        ] {
            let HookOutput::StopBlock { reason } = render_stop_result(&[stop_candidate(
                CandidateState::Blocked,
                Some(blocked_reason),
                None,
            )]) else {
                panic!("{blocked_reason:?} must block");
            };
            assert!(reason.contains(&format!("reason={token}")), "{reason}");
            assert!(reason.contains("remediation=RETRY_OWNER_RESOLUTION"));
        }
    }

    #[test]
    fn search_failure_subcodes_are_rendered_exactly() {
        for (subcode, token) in [
            (FailureSubcode::EmptyCorpus, "empty-corpus"),
            (FailureSubcode::PartialPage, "partial-page"),
        ] {
            let HookOutput::StopBlock { reason } = render_stop_result(&[stop_candidate(
                CandidateState::Blocked,
                Some(BlockedReason::Search),
                Some(subcode),
            )]) else {
                panic!("{subcode:?} must block");
            };
            assert!(reason.contains(&format!("subcode={token}")), "{reason}");
        }
    }

    #[test]
    fn attempt_selection_skips_active_leases_and_prefers_the_oldest_candidate() {
        let mut active = stop_candidate(CandidateState::OwnerResolving, None, None);
        active.id = "active".to_string();
        active.updated_at = "2026-07-14T00:00:00Z".to_string();
        active.active_attempt = true;
        let mut older = stop_candidate(CandidateState::Blocked, None, None);
        older.id = "older".to_string();
        older.updated_at = "2026-07-15T00:00:00Z".to_string();
        let mut newer = stop_candidate(CandidateState::Blocked, None, None);
        newer.id = "newer".to_string();
        newer.updated_at = "2026-07-16T00:00:00Z".to_string();

        assert_eq!(
            select_attempt_candidate(&[active, newer, older]).as_deref(),
            Some("older")
        );
    }

    #[test]
    fn resolver_failure_does_not_hide_remote_outcome_unknown_contract() {
        let mut candidate = stop_candidate(CandidateState::RemoteOutcomeUnknown, None, None);
        candidate.remediation = None;

        apply_attempt_failure(
            std::slice::from_mut(&mut candidate),
            "impr-test",
            OwnerResolutionFailure {
                reason: BlockedReason::Store,
                failure_subcode: None,
                remediation: "RELOAD_CANDIDATE_STORE",
            },
        );

        assert_eq!(candidate.blocked_reason, None);
        assert_eq!(candidate.remediation, None);
        let HookOutput::StopBlock { reason } = render_stop_result(&[candidate]) else {
            panic!("remote outcome unknown must block");
        };
        assert!(reason.contains("reason=remote-outcome-unknown"), "{reason}");
        assert!(
            reason.contains("remediation=REFRESH_OWNER_CORPUS"),
            "{reason}"
        );
    }

    #[test]
    fn submitted_owner_resolving_attempt_renders_as_remote_outcome_unknown() {
        let mut candidate = stop_candidate(CandidateState::OwnerResolving, None, None);
        candidate.remediation = None;
        candidate.remote_mutation_seen = true;

        apply_attempt_failure(
            std::slice::from_mut(&mut candidate),
            "impr-test",
            OwnerResolutionFailure {
                reason: BlockedReason::Store,
                failure_subcode: None,
                remediation: "RELOAD_CANDIDATE_STORE",
            },
        );

        assert_eq!(candidate.state, CandidateState::RemoteOutcomeUnknown);
        assert_eq!(candidate.blocked_reason, None);
        assert_eq!(
            candidate.remediation.as_deref(),
            Some("REFRESH_OWNER_CORPUS")
        );
    }

    #[test]
    fn owner_projection_failure_has_its_own_typed_remediation() {
        let deadline = ResolutionDeadline::new(Duration::from_secs(1), Duration::from_secs(5));

        let HookOutput::StopBlock { reason } =
            super::evaluation_failure_block(&deadline, StopEvaluationFailure::OwnerProjection)
        else {
            panic!("owner projection failure must block");
        };

        assert!(reason.contains("reason=local-commit"), "{reason}");
        assert!(
            reason.contains("remediation=REPAIR_OWNER_PROJECTION"),
            "{reason}"
        );
    }

    #[test]
    fn candidate_store_is_reloaded_after_projection_repair_even_when_no_repair_was_needed() {
        let home = tempfile::tempdir().expect("home");
        let _home = ScopedGwtHome::set(home.path());
        let repo = tempfile::tempdir().expect("repository");
        crate::cli::improvement_store::load_and_repair(repo.path())
            .expect("initialize candidate store");
        let replacement_nonce = "a".repeat(64);

        let store = load_candidate_store_after_projection_repair_with(repo.path(), |_| {
            let path = gwt_core::paths::gwt_project_dir_for_repo_path(repo.path())
                .join("improvements")
                .join("candidates.json");
            let mut value: serde_json::Value = serde_json::from_slice(
                &std::fs::read(&path).expect("read initial candidate store"),
            )
            .expect("parse initial candidate store");
            value["source_scope_nonce"] = serde_json::Value::String(replacement_nonce.clone());
            std::fs::write(
                path,
                serde_json::to_vec_pretty(&value).expect("serialize replacement candidate store"),
            )
            .expect("replace candidate store during projection repair");
            Ok::<bool, gwt_github::SpecOpsError>(false)
        })
        .expect("load latest candidate store");

        assert_eq!(
            store.source_scope_nonce.as_deref(),
            Some(replacement_nonce.as_str())
        );
    }

    #[cfg(unix)]
    #[test]
    fn repository_probe_treats_git_exit_128_as_routing_failure() {
        let _env_lock = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let fake_bin = tempfile::tempdir().expect("fake bin");
        let fake_git = fake_bin.path().join("git");
        std::fs::write(&fake_git, "#!/bin/sh\nexit 128\n").expect("write fake git");
        std::fs::set_permissions(&fake_git, std::fs::Permissions::from_mode(0o755))
            .expect("make fake git executable");
        let _path = gwt_core::test_support::ScopedEnvVar::set("PATH", fake_bin.path());
        let repo = tempfile::tempdir().expect("repository");
        let deadline = ResolutionDeadline::new(Duration::from_millis(100), Duration::from_secs(5));

        let result = origin_remote_url(repo.path(), &deadline);

        assert_eq!(result, Err(RepositoryProbeFailure::Routing));
    }

    #[test]
    fn candidate_store_lock_contention_returns_a_timeout_block_within_the_deadline() {
        let _env_lock = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home");
        let _home = ScopedGwtHome::set(home.path());
        let repo = tempfile::tempdir().expect("repository");
        assert!(Command::new("git")
            .arg("init")
            .arg("-q")
            .arg(repo.path())
            .status()
            .expect("git init")
            .success());
        assert!(Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .args([
                "remote",
                "add",
                "origin",
                "https://github.com/akiojin/gwt.git",
            ])
            .status()
            .expect("git remote")
            .success());
        let improvements_dir =
            gwt_core::paths::gwt_project_dir_for_repo_path(repo.path()).join("improvements");
        std::fs::create_dir_all(&improvements_dir).expect("improvements dir");
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(improvements_dir.join(".lock"))
            .expect("candidate lock");
        lock.lock_exclusive().expect("hold candidate lock");
        let mut env = TestEnv::new(repo.path().join("cache"));
        env.repo_path = repo.path().to_path_buf();
        let deadline =
            ResolutionDeadline::new(Duration::from_millis(50), Duration::from_millis(250));
        let started = std::time::Instant::now();

        let output = evaluate_with_deadline(&mut env, false, false, &deadline);

        let HookOutput::StopBlock { reason } = output else {
            panic!("contended candidate store must block");
        };
        assert!(reason.contains("reason=timeout"), "{reason}");
        assert!(started.elapsed() < Duration::from_secs(1));
        FileExt::unlock(&lock).expect("unlock candidate store");
    }
}
