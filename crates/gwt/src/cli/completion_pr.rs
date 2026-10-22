//! Autonomous implementation completion requires a prepared owner PR (#5034).

use super::{execution_state as execution, verification_record as verification};
use crate::cli::{CliEnv, PrCommand};
use gwt_git::pr_status::PrState;

/// Repository-filtered discovery data; readiness is read fresh after selection.
#[derive(Debug, Clone)]
pub struct CompletionPrCandidate {
    pub number: u64,
    pub url: String,
    pub head_ref_name: String,
    pub head_sha: String,
}

/// Admit the autonomous owner's completion before any terminal state is
/// written, and pin the verification snapshot through the subsequent commit.
/// Manual and unrelated lifecycle operations retain their existing contracts.
pub(super) fn admit<E: CliEnv>(
    env: &mut E,
    session_id: &str,
    expected_owner: Option<u64>,
    explicit_pr: Option<u64>,
    out: &mut String,
) -> Result<Option<String>, String> {
    if execution::session_launch_route(Some(session_id)) != Some(gwt_agent::LaunchRoute::Autonomous)
    {
        return Ok(None);
    }
    let worktree = gwt_core::paths::resolve_current_worktree_root(env.repo_path());
    let owner = execution::load(&worktree).map_err(|error| error.to_string())?
        .ok_or("autonomous PR completion has no owning Execution record. Run execution.status and restore the owning execution before retrying completion")?;
    if owner.primary_session_id != session_id
        || expected_owner.is_some_and(|number| owner.owner_number != number)
    {
        return Err("autonomous PR completion owner/Session does not match. Run execution.status and restore the owning execution before retrying completion".into());
    }
    if owner.status != execution::ExecutionControlStatus::Active {
        return Ok(None);
    }
    let evidence = verification::evaluate_evidence(&worktree, session_id, Some(owner.owner_number));
    if !matches!(
        evidence,
        verification::EvidenceStatus::Fresh | verification::EvidenceStatus::FreshWithQuarantine
    ) {
        return Err(format!("PR completion requires current verification: {}. Run verify.plan / verify.run and retry completion.", evidence.describe()));
    }
    let verified = verification::load(&worktree)
        .map_err(|error| error.to_string())?
        .ok_or("PR completion verification record disappeared")?;
    let verified_head = verified
        .verified_head
        .as_deref()
        .ok_or("PR completion verified HEAD is unknown; rerun verify.run")?;
    let branch = gwt_git::Repository::open(&worktree)
        .and_then(|repo| repo.current_branch())
        .map_err(|error| error.to_string())?
        .ok_or("PR completion requires the owning branch")?;
    let candidates = env
        .fetch_completion_pr_candidates(explicit_pr)
        .map_err(|error| {
            format!("owner PR read failed: {error}. Restore the read and retry completion.")
        })?;
    let candidates: Vec<_> = candidates
        .iter()
        .filter(|pr| {
            pr.head_ref_name == branch && explicit_pr.is_none_or(|number| pr.number == number)
        })
        .collect();
    let selector = format!(
        "build.complete with params.spec:{} and params.pr:<number>",
        owner.owner_number
    );
    let pr = if let Some(number) = explicit_pr {
        candidates.first().copied().ok_or_else(|| format!("explicit PR #{number} does not belong to the execution branch/repository. Check the target and retry completion."))?
    } else {
        if candidates.is_empty() {
            return Err(format!("owner branch {branch} has no PR. Create its Ready PR through pr.create, finish CI and review repair, then retry completion."));
        }
        let mut matching: Vec<_> = candidates
            .iter()
            .copied()
            .filter(|pr| pr.head_sha == verified_head)
            .collect();
        if matching.is_empty() {
            matching = candidates
                .iter()
                .copied()
                .filter(|pr| {
                    verification::preserves_verified_head(&worktree, verified_head, &pr.head_sha)
                })
                .collect();
        }
        if matching.is_empty() {
            matching = candidates
                .iter()
                .copied()
                .filter(|pr| {
                    verification::preserves_verified_head(&worktree, &pr.head_sha, verified_head)
                })
                .collect();
        }
        if matching.len() != 1 {
            let numbers = candidates
                .iter()
                .map(|pr| format!("PR #{}", pr.number))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!("cannot uniquely select a completion PR for verified HEAD {verified_head}. Candidates: {numbers}. Repair an unmatched head or specify {selector}, then retry completion."));
        }
        matching[0]
    };
    let snapshot = env.fetch_completion_pr(pr.number).map_err(|error| {
        format!(
            "PR #{} completion read failed: {error}. Restore the read and retry completion.",
            pr.number
        )
    })?;
    let fields = &snapshot.inventory;
    let refuse = |reason: &str| {
        format!("PR #{} completion refused: {reason}. The owning implementation agent must repair the PR, rerun verification when needed, and retry completion.", pr.number)
    };
    if snapshot.head_sha != pr.head_sha {
        return Err(refuse("PR head changed after candidate selection"));
    }
    let validate = |snapshot: &gwt_git::pr_status::PrCompletionSnapshot| -> Result<(), String> {
        let fields = &snapshot.inventory;
        if fields.number != pr.number || fields.url != pr.url || fields.head_ref_name != branch {
            return Err(refuse("PR does not belong to the execution branch"));
        }
        // A fresh run may include the merged PR's later Work receipt.
        let preserves_head =
            verification::preserves_verified_head(&worktree, verified_head, &snapshot.head_sha)
                || (snapshot.state == PrState::Merged
                    && verification::preserves_verified_head(
                        &worktree,
                        &snapshot.head_sha,
                        verified_head,
                    ));
        if !preserves_head {
            return Err(refuse("PR head differs from the verified HEAD"));
        }
        if snapshot.state == PrState::Closed {
            return Err(refuse("PR is closed without merging"));
        }
        if snapshot.state == PrState::Open {
            if let Some(blocker) = fields.completion_blocker() {
                return Err(refuse(blocker));
            }
            let ready_record =
                super::pr::ready_verification(&worktree, Some(session_id), &fields.body, true)
                    .map_err(|reason| refuse(&reason))?
                    .ok_or_else(|| refuse("autonomous Ready evidence is missing"))?;
            if ready_record.content_hash != verified.content_hash {
                return Err(refuse("verification evidence changed during PR validation"));
            }
        }
        Ok(())
    };
    validate(&snapshot)?;
    // Re-read the remote head after the readiness probes. Unknown is a
    // refusal, including a PR whose branch moved after the first snapshot.
    if env
        .fetch_pr_head_sha(pr.number)
        .map_err(|error| refuse(&error.to_string()))?
        .as_deref()
        != Some(snapshot.head_sha.as_str())
    {
        return Err(refuse(
            "PR head changed or became unreadable during validation",
        ));
    }
    if snapshot.state == PrState::Open && fields.is_draft {
        let code = super::pr::run(env, PrCommand::Ready { number: pr.number }, out)
            .map_err(|error| refuse(&error.to_string()))?;
        if code != 0 {
            return Err(refuse("canonical pr.ready gate refused the handoff"));
        }
        let ready = env
            .fetch_completion_pr(pr.number)
            .map_err(|error| refuse(&format!("post-Ready PR read failed: {error}")))?;
        validate(&ready)?;
        if ready.head_sha != snapshot.head_sha {
            return Err(refuse("PR head changed during pr.ready"));
        }
        if ready.state == PrState::Open && ready.inventory.is_draft {
            return Err(refuse("PR is still Draft after pr.ready"));
        }
        if env
            .fetch_pr_head_sha(pr.number)
            .map_err(|error| refuse(&error.to_string()))?
            .as_deref()
            != Some(snapshot.head_sha.as_str())
        {
            return Err(refuse(
                "PR head changed or became unreadable during pr.ready",
            ));
        }
    }
    Ok(Some(verified.content_hash.clone()))
}

#[cfg(test)]
mod tests {
    use super::super::{execution_state as execution, verification_record as verification};
    use crate::cli::{CliEnv, SkillStateAction, TestEnv};
    use chrono::Utc;
    use gwt_core::test_support::{ScopedEnvVar, ScopedGwtHome};
    use gwt_git::pr_status::PrState;
    use std::path::Path;

    fn prepare(repo: &Path, session_id: &str) {
        crate::cli::trusted_store::init_git_repo_with_origin(repo);
        let branch = gwt_git::Repository::open(repo)
            .unwrap()
            .current_branch()
            .unwrap()
            .unwrap();
        assert!(gwt_core::process::resolved_command(
            gwt_core::process::ProcessPlanRequest::new("git")
                .args(["update-ref", "refs/remotes/origin/develop", "HEAD"])
                .current_dir(repo)
        )
        .unwrap()
        .status()
        .unwrap()
        .success());
        let mut session = gwt_agent::Session::new(repo, &branch, gwt_agent::AgentId::Codex);
        session.id = session_id.into();
        session.launch_route = gwt_agent::LaunchRoute::Autonomous;
        session.save(&gwt_core::paths::gwt_sessions_dir()).unwrap();
        execution::save(
            repo,
            &execution::ExecutionControlRecord {
                owner_kind: execution::ExecutionOwnerKind::Issue,
                owner_number: 5034,
                primary_session_id: session_id.into(),
                entrypoint: "gwt-execute".into(),
                bundled_required_owners: Vec::new(),
                status: execution::ExecutionControlStatus::Active,
                blocked_reason: None,
                missing_verification: None,
                launched_at: Utc::now(),
                settled_at: None,
                completion_evidence: None,
                transfers: Vec::new(),
                recoveries: Vec::new(),
                content_hash: String::new(),
                permission_decision: None,
            },
        )
        .unwrap();
        verification::save_plan(
            repo,
            &verification::VerificationPlanRecord::from(verification::VerificationPlanData {
                format_version: Some(1),
                session_id: session_id.into(),
                owner_number: Some(5034),
                execution_binding: None,
                commands: vec!["git --version".into()],
                derived: false,
                worktree_fingerprint: String::new(),
                surfaces: Vec::new(),
                generated_outputs: Vec::new(),
                quarantines: Vec::new(),
                created_at: Utc::now(),
                content_hash: String::new(),
            }),
        )
        .unwrap();
        verification::run_verification(repo, session_id, &["git --version".into()]).unwrap();
    }

    fn seed_pr(env: &mut TestEnv, draft: bool, state: gwt_git::pr_status::PrState) {
        let branch = gwt_git::Repository::open(env.repo_path())
            .unwrap()
            .current_branch()
            .unwrap()
            .unwrap();
        let value = serde_json::json!({
            "number": 9001, "title": "fix: owner completion", "url": "https://github.com/o/r/pull/9001",
            "state": state.to_string(), "isDraft": draft, "headRefName": branch,
            "baseRefName": "develop", "mergeable": "MERGEABLE", "mergeStateStatus": "CLEAN",
            "reviewDecision": "APPROVED", "statusCheckRollup": [{"status":"COMPLETED","conclusion":"SUCCESS"}],
            "body": "User Verification Result: n/a (autonomous)\nAgent Visual Check: n/a (no UI surface)\n"
        });
        let pr = gwt_git::pr_status::parse_pr_status_json(&value.to_string()).unwrap();
        let mut inventory =
            gwt_git::pr_status::parse_pr_inventory_json(&format!("[{value}]"), Utc::now())
                .unwrap()
                .remove(0);
        inventory.unresolved_review_threads = Some(0);
        inventory.coderabbit_review_complete = Some(true);
        env.pr_quarantine_contexts.insert(
            9001,
            super::super::pr::PrQuarantineContext {
                number: 9001,
                body: inventory.body.clone(),
                comments: Vec::new(),
            },
        );
        env.seed_current_pr(Some(pr.clone()));
        env.seed_pr(9001, pr);
        env.completion_prs.insert(
            9001,
            gwt_git::pr_status::PrCompletionSnapshot {
                state,
                head_sha: verification::load(env.repo_path())
                    .unwrap()
                    .unwrap()
                    .verified_head
                    .clone()
                    .unwrap(),
                inventory,
            },
        );
    }

    fn commit_fixture_change(repo: &Path, path: &str) {
        for args in [
            vec!["add", "--", path],
            vec!["commit", "-qm", "chore(work): record completion receipt"],
        ] {
            let output = gwt_core::process::hidden_command("git")
                .args(args)
                .current_dir(repo)
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
        }
    }

    fn commit_and_reverify(repo: &Path, session_id: &str, path: &str) {
        commit_fixture_change(repo, path);
        let mut plan = verification::load_plan(repo).unwrap().unwrap();
        plan.worktree_fingerprint.clear();
        verification::save_plan(repo, &plan).unwrap();
        verification::run_verification(repo, session_id, &["git --version".into()]).unwrap();
        assert_eq!(
            verification::evaluate_evidence(repo, session_id, Some(5034)),
            verification::EvidenceStatus::Fresh
        );
    }

    #[test]
    fn autonomous_completion_accepts_reverified_receipt_only_for_merged_pr() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let session_id = "completion-pr-reverified-receipt";
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, session_id);
        let repo = tempfile::tempdir().unwrap();
        prepare(repo.path(), session_id);
        let mut env = TestEnv::new(repo.path().into());
        seed_pr(&mut env, false, PrState::Open);
        let event = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Pr,
            "work-completion-receipt",
            Utc::now(),
        );
        gwt_core::workspace_projection::record_workspace_work_event(repo.path(), event).unwrap();
        commit_and_reverify(repo.path(), session_id, ".gwt/work/events");
        let mut out = String::new();
        assert!(
            super::admit(&mut env, session_id, Some(5034), Some(9001), &mut out)
                .unwrap_err()
                .contains("PR head differs")
        );

        env.completion_prs.get_mut(&9001).unwrap().state = PrState::Merged;
        for explicit_pr in [Some(9001), None] {
            assert!(
                super::admit(&mut env, session_id, Some(5034), explicit_pr, &mut out)
                    .unwrap_or_else(|error| panic!("{explicit_pr:?}: {error}"))
                    .is_some()
            );
        }

        // A forward-preserving candidate must retain priority over this old PR.
        let event = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Pr,
            "work-completion-receipt",
            Utc::now(),
        );
        gwt_core::workspace_projection::record_workspace_work_event(repo.path(), event).unwrap();
        commit_fixture_change(repo.path(), ".gwt/work/events");
        let head = gwt_core::process::hidden_command("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(repo.path())
            .output()
            .unwrap();
        assert!(head.status.success());
        let mut newer = env.completion_prs[&9001].clone();
        newer.head_sha = String::from_utf8(head.stdout).unwrap().trim().to_string();
        newer.inventory.number = 9002;
        newer.inventory.url = "https://github.com/o/r/pull/9002".into();
        env.completion_prs.insert(9002, newer);
        assert!(
            super::admit(&mut env, session_id, Some(5034), None, &mut out)
                .unwrap_or_else(|error| panic!("{error}"))
                .is_some()
        );
        assert_eq!(env.completion_pr_call_log.last(), Some(&9002));
    }

    #[test]
    fn autonomous_completion_rejects_reverified_product_after_merged_pr() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let session_id = "completion-pr-reverified-product";
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, session_id);
        let repo = tempfile::tempdir().unwrap();
        prepare(repo.path(), session_id);
        let mut env = TestEnv::new(repo.path().into());
        seed_pr(&mut env, false, PrState::Merged);
        std::fs::write(
            repo.path().join("product.rs"),
            "additional product change\n",
        )
        .unwrap();
        commit_and_reverify(repo.path(), session_id, "product.rs");
        let mut out = String::new();
        let error =
            super::admit(&mut env, session_id, Some(5034), Some(9001), &mut out).unwrap_err();
        assert!(error.contains("PR head differs"), "{error}");
        assert_eq!(
            execution::load(repo.path()).unwrap().unwrap().status,
            execution::ExecutionControlStatus::Active
        );
    }

    #[test]
    fn autonomous_completion_selects_verified_merged_pr_before_later_open_slice() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "completion-pr-slices");
        let repo = tempfile::tempdir().unwrap();
        prepare(repo.path(), "completion-pr-slices");
        let mut env = TestEnv::new(repo.path().into());
        seed_pr(&mut env, false, gwt_git::pr_status::PrState::Merged);
        let mut later = env.completion_prs[&9001].clone();
        later.inventory.number = 9002;
        later.inventory.url = "https://github.com/o/r/pull/9002".into();
        later.head_sha = "later-unverified-slice".into();
        later.state = gwt_git::pr_status::PrState::Open;
        let mut pr = env.fetch_current_pr().unwrap().unwrap();
        pr.number = 9002;
        pr.url = later.inventory.url.clone();
        pr.state = later.state;
        env.seed_current_pr(Some(pr.clone()));
        env.seed_pr(9002, pr);
        env.completion_prs.insert(9002, later);
        let mut out = String::new();
        assert_eq!(
            execution::run(&mut env, execution::ExecutionCommand::Complete, &mut out).unwrap(),
            0,
            "{out}"
        );
        assert_eq!(env.completion_pr_call_log, vec![9001]);
        assert!(env.pr_ready_call_log.is_empty());
    }

    #[test]
    fn autonomous_completion_requires_explicit_pr_for_ambiguous_verified_head() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "completion-pr-ambiguous");
        let repo = tempfile::tempdir().unwrap();
        prepare(repo.path(), "completion-pr-ambiguous");
        let mut env = TestEnv::new(repo.path().into());
        seed_pr(&mut env, false, gwt_git::pr_status::PrState::Merged);
        let mut duplicate = env.completion_prs[&9001].clone();
        duplicate.inventory.number = 9002;
        duplicate.inventory.url = "https://github.com/o/r/pull/9002".into();
        env.completion_prs.insert(9002, duplicate);
        let mut out = String::new();
        let error = super::admit(
            &mut env,
            "completion-pr-ambiguous",
            Some(5034),
            None,
            &mut out,
        )
        .unwrap_err();
        assert!(
            error.contains("PR #9001") && error.contains("PR #9002") && error.contains("params.pr"),
            "{error}"
        );
        assert!(super::admit(
            &mut env,
            "completion-pr-ambiguous",
            Some(5034),
            Some(9001),
            &mut out
        )
        .unwrap()
        .is_some());
        env.completion_prs.get_mut(&9002).unwrap().head_sha = "unverified-head".into();
        let error = super::admit(
            &mut env,
            "completion-pr-ambiguous",
            Some(5034),
            Some(9002),
            &mut out,
        )
        .unwrap_err();
        assert!(error.contains("PR head differs"), "{error}");
        env.completion_prs
            .get_mut(&9002)
            .unwrap()
            .inventory
            .head_ref_name = "other-branch".into();
        let error = super::admit(
            &mut env,
            "completion-pr-ambiguous",
            Some(5034),
            Some(9002),
            &mut out,
        )
        .unwrap_err();
        assert!(error.contains("does not belong"), "{error}");
        assert_eq!(
            execution::load(repo.path()).unwrap().unwrap().status,
            execution::ExecutionControlStatus::Active
        );
        assert!(env.pr_ready_call_log.is_empty());
    }

    #[test]
    fn autonomous_completion_accepts_draft_ready_and_merged_with_only_ready_mutation() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "completion-pr-positive");
        for (draft, state) in [
            (true, gwt_git::pr_status::PrState::Open),
            (false, gwt_git::pr_status::PrState::Open),
            (false, gwt_git::pr_status::PrState::Merged),
        ] {
            let repo = tempfile::tempdir().unwrap();
            prepare(repo.path(), "completion-pr-positive");
            let mut env = TestEnv::new(repo.path().into());
            seed_pr(&mut env, draft, state);
            let mut out = String::new();
            assert_eq!(
                execution::run(&mut env, execution::ExecutionCommand::Complete, &mut out).unwrap(),
                0,
                "{draft} {state}: {out}"
            );
            assert_eq!(
                execution::load(repo.path()).unwrap().unwrap().status,
                execution::ExecutionControlStatus::Completed
            );
            assert_eq!(
                env.pr_ready_call_log,
                if draft { vec![9001] } else { vec![] }
            );
            assert!(env.pr_create_call_log.is_empty());
            assert!(env.pr_edit_call_log.is_empty());
            assert!(env.pr_draft_call_log.is_empty());
            assert!(env.pr_update_branch_call_log.is_empty());
        }
    }

    #[test]
    fn autonomous_completion_rejects_remaining_owner_repairs_and_unmeasured_ui() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "completion-pr-repairs");
        let repo = tempfile::tempdir().unwrap();
        prepare(repo.path(), "completion-pr-repairs");
        for condition in [
            "red_ci",
            "conflict",
            "unresolved_review",
            "missing_visual",
            "head_drift",
            "unknown_review",
            "ui_without_headed",
        ] {
            let mut env = TestEnv::new(repo.path().into());
            seed_pr(&mut env, true, gwt_git::pr_status::PrState::Open);
            let pr = env.completion_prs.get_mut(&9001).unwrap();
            match condition {
                "red_ci" => pr.inventory.ci_status = "FAILURE".into(),
                "conflict" => pr.inventory.mergeable = "CONFLICTING".into(),
                "unresolved_review" => pr.inventory.unresolved_review_threads = Some(1),
                "missing_visual" => {
                    pr.inventory.body = "User Verification Result: n/a (autonomous)".into()
                }
                "head_drift" => pr.head_sha = "different-head".into(),
                "unknown_review" => pr.inventory.unresolved_review_threads = None,
                "ui_without_headed" => {
                    pr.inventory.body =
                        "User Verification Result: n/a (autonomous)\nAgent Visual Check: pass\n"
                            .into()
                }
                _ => unreachable!(),
            }
            let mut out = String::new();
            assert_eq!(
                execution::run(&mut env, execution::ExecutionCommand::Complete, &mut out).unwrap(),
                2,
                "{condition}: {out}"
            );
            assert!(
                out.contains("PR #9001") && out.contains("retry completion"),
                "{condition}: {out}"
            );
            assert_eq!(
                execution::load(repo.path()).unwrap().unwrap().status,
                execution::ExecutionControlStatus::Active
            );
            assert!(env.pr_ready_call_log.is_empty(), "{condition}");
        }
    }

    #[test]
    fn manual_completion_keeps_existing_no_pr_contract() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "completion-pr-manual");
        let repo = tempfile::tempdir().unwrap();
        prepare(repo.path(), "completion-pr-manual");
        let path = gwt_core::paths::gwt_sessions_dir().join("completion-pr-manual.toml");
        let mut session = gwt_agent::Session::load(&path).unwrap();
        session.launch_route = gwt_agent::LaunchRoute::Manual;
        session.save(&gwt_core::paths::gwt_sessions_dir()).unwrap();
        let mut env = TestEnv::new(repo.path().into());
        let mut out = String::new();
        assert_eq!(
            execution::run(&mut env, execution::ExecutionCommand::Complete, &mut out).unwrap(),
            0,
            "{out}"
        );
        assert_eq!(env.pr_current_call_count, 0);
    }

    #[test]
    fn autonomous_completion_rejects_head_drift_during_ready_handoff() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "completion-pr-race");
        let repo = tempfile::tempdir().unwrap();
        prepare(repo.path(), "completion-pr-race");
        let mut env = TestEnv::new(repo.path().into());
        seed_pr(&mut env, true, gwt_git::pr_status::PrState::Open);
        let mut changed = env.completion_prs[&9001].clone();
        changed.head_sha = "pushed-during-ready".into();
        env.completion_prs_after_ready.insert(9001, changed);
        let mut out = String::new();
        assert_eq!(
            execution::run(&mut env, execution::ExecutionCommand::Complete, &mut out).unwrap(),
            2,
            "{out}"
        );
        assert_eq!(
            execution::load(repo.path()).unwrap().unwrap().status,
            execution::ExecutionControlStatus::Active
        );
    }

    #[test]
    fn autonomous_build_without_execution_record_stays_active() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let _session =
            ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "completion-pr-legacy-build");
        let _forward_url = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
        let _forward_token = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);
        let _runtime = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV);
        let repo = tempfile::tempdir().unwrap();
        prepare(repo.path(), "completion-pr-legacy-build");
        std::fs::remove_file(execution::state_path(repo.path())).unwrap();
        std::fs::remove_file(
            super::super::trusted_store::trusted_dir_for_worktree(repo.path())
                .unwrap()
                .join("execution-control.json"),
        )
        .unwrap();
        gwt_core::skill_state::save(
            repo.path(),
            "build-spec",
            &gwt_core::skill_state::SkillState {
                start_evidence: None,
                active: true,
                owner_spec: Some(5034),
                started_at: Utc::now(),
                phase: None,
                session_id: "completion-pr-legacy-build".into(),
            },
        )
        .unwrap();
        let mut env = TestEnv::new(repo.path().into());
        let mut out = String::new();
        assert_eq!(
            super::super::build::run(
                &mut env,
                SkillStateAction::Complete {
                    spec: 5034,
                    pr: None
                },
                &mut out
            )
            .unwrap(),
            2,
            "{out}"
        );
        assert!(
            gwt_core::skill_state::load(repo.path(), "build-spec")
                .unwrap()
                .unwrap()
                .active
        );
    }

    #[test]
    fn autonomous_completion_rechecks_ready_state_and_gates_at_unchanged_head() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let _session = ScopedEnvVar::set(
            gwt_agent::GWT_SESSION_ID_ENV,
            "completion-pr-ready-state-race",
        );
        for condition in [
            "closed",
            "red_ci",
            "unresolved_review",
            "missing_visual",
            "still_draft",
        ] {
            let repo = tempfile::tempdir().unwrap();
            prepare(repo.path(), "completion-pr-ready-state-race");
            let mut env = TestEnv::new(repo.path().into());
            seed_pr(&mut env, true, gwt_git::pr_status::PrState::Open);
            let mut changed = env.completion_prs[&9001].clone();
            changed.inventory.is_draft = false;
            match condition {
                "closed" => changed.state = gwt_git::pr_status::PrState::Closed,
                "red_ci" => changed.inventory.ci_status = "FAILURE".into(),
                "unresolved_review" => changed.inventory.unresolved_review_threads = Some(1),
                "missing_visual" => changed.inventory.body.clear(),
                "still_draft" => changed.inventory.is_draft = true,
                _ => unreachable!(),
            }
            env.completion_prs_after_ready.insert(9001, changed);
            let mut out = String::new();
            assert_eq!(
                execution::run(&mut env, execution::ExecutionCommand::Complete, &mut out).unwrap(),
                2,
                "{condition}: {out}"
            );
            assert_eq!(
                execution::load(repo.path()).unwrap().unwrap().status,
                execution::ExecutionControlStatus::Active
            );
        }
    }

    #[test]
    fn autonomous_execution_without_pr_stays_active() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "completion-pr-execution");
        let repo = tempfile::tempdir().unwrap();
        prepare(repo.path(), "completion-pr-execution");
        let mut env = TestEnv::new(repo.path().into());
        let mut out = String::new();
        let code =
            execution::run(&mut env, execution::ExecutionCommand::Complete, &mut out).unwrap();
        assert_eq!(code, 2, "{out}");
        assert!(out.contains("PR"), "{out}");
        assert_eq!(
            execution::load(repo.path()).unwrap().unwrap().status,
            execution::ExecutionControlStatus::Active
        );
        assert!(env.pr_ready_call_log.is_empty());
    }

    #[test]
    fn autonomous_build_without_pr_keeps_lifecycle_active() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "completion-pr-build");
        let repo = tempfile::tempdir().unwrap();
        prepare(repo.path(), "completion-pr-build");
        let state = gwt_core::skill_state::SkillState {
            start_evidence: None,
            active: true,
            owner_spec: Some(5034),
            started_at: Utc::now(),
            phase: None,
            session_id: "completion-pr-build".into(),
        };
        gwt_core::skill_state::save(repo.path(), "build-spec", &state).unwrap();
        let mut env = TestEnv::new(repo.path().into());
        let mut out = String::new();
        let code = super::super::build::run(
            &mut env,
            SkillStateAction::Complete {
                spec: 5034,
                pr: None,
            },
            &mut out,
        )
        .unwrap();
        assert_eq!(code, 2, "{out}");
        assert!(out.contains("PR"), "{out}");
        assert!(
            gwt_core::skill_state::load(repo.path(), "build-spec")
                .unwrap()
                .unwrap()
                .active
        );
        assert_eq!(
            execution::load(env.repo_path()).unwrap().unwrap().status,
            execution::ExecutionControlStatus::Active
        );
    }
}
