//! Exact GitHub CI evidence for Ready creation only (Issue #4214).
use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CiCreateRequest {
    pub base: String,
    pub head: String,
    pub head_sha: String,
    pub title: String,
    pub body: String,
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequiredCheck {
    pub context: String,
    pub app_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CiSnapshot {
    pub repository: String,
    pub repository_id: u64,
    pub base: String,
    pub base_sha: String,
    pub head: String,
    pub head_sha: String,
    pub policy: Value,
    pub required_checks: Vec<RequiredCheck>,
    pub check_runs: Vec<Value>,
    pub statuses: Vec<Value>,
    pub changed_files: Vec<String>,
    pub strict: bool,
    pub behind_by: u64,
}

fn valid_sha(sha: &str) -> bool {
    matches!(sha.len(), 40 | 64) && sha.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn refused(reason: impl std::fmt::Display) -> io::Error {
    io::Error::other(format!("CI Ready creation refused: {reason}"))
}

impl CiSnapshot {
    pub fn validate(&self, request: &CiCreateRequest) -> io::Result<()> {
        if self.repository_id == 0
            || self.repository.split('/').count() != 2
            || self.base != request.base
            || self.head != request.head
            || self.head_sha != request.head_sha
            || !valid_sha(&self.head_sha)
            || !valid_sha(&self.base_sha)
            || self.base_sha == self.head_sha
        {
            return Err(refused(
                "repository, refs or exact pushed head do not match",
            ));
        }
        if self.required_checks.is_empty() {
            return Err(refused("mandatory check policy is empty or unknown"));
        }
        if self.strict && self.behind_by != 0 {
            return Err(refused(
                "the required strict policy needs an up-to-date head",
            ));
        }
        for required in &self.required_checks {
            if required.context.trim().is_empty() {
                return Err(refused("empty required context"));
            }
            let app_id = required.app_id.filter(|id| *id >= 0);
            let mut latest_runs = BTreeMap::new();
            for run in self
                .check_runs
                .iter()
                .filter(|run| run["name"].as_str() == Some(&required.context))
            {
                let app = run["app"]["id"]
                    .as_i64()
                    .ok_or_else(|| refused("check app is unknown"))?;
                if app_id.is_some_and(|required_app| required_app != app) {
                    continue;
                }
                let id = run["id"]
                    .as_u64()
                    .ok_or_else(|| refused("check retry identity is unknown"))?;
                if latest_runs
                    .get(&app)
                    .is_none_or(|(previous, _)| id > *previous)
                {
                    latest_runs.insert(app, (id, run));
                }
            }
            let latest_status = self
                .statuses
                .iter()
                .filter(|status| status["context"].as_str() == Some(&required.context))
                .map(|status| {
                    status["id"]
                        .as_u64()
                        .map(|id| (id, status))
                        .ok_or_else(|| refused("status retry identity is unknown"))
                })
                .collect::<io::Result<Vec<_>>>()?
                .into_iter()
                .max_by_key(|(id, _)| *id);
            if latest_runs.is_empty() && (app_id.is_some() || latest_status.is_none()) {
                return Err(refused(format!(
                    "required check {} has not passed for its required app",
                    required.context
                )));
            }
            for (_, run) in latest_runs.values() {
                if run["head_sha"].as_str() != Some(&request.head_sha)
                    || run["status"].as_str() != Some("completed")
                    || run["conclusion"].as_str() != Some("success")
                {
                    return Err(refused(format!(
                        "required check {} is missing, pending or unsuccessful",
                        required.context
                    )));
                }
            }
            // GitHub requires BOTH when a check and a commit status share a name.
            if latest_status.is_some_and(|(_, status)| status["state"].as_str() != Some("success"))
            {
                return Err(refused(format!(
                    "required status {} is pending or unsuccessful",
                    required.context
                )));
            }
        }
        Ok(())
    }
}

fn api(worktree: &Path, endpoint: &str, paginate: bool) -> io::Result<Value> {
    let mut args = vec!["api", endpoint];
    if paginate {
        args.extend(["--paginate", "--slurp"]);
    }
    let result = super::gh::run_gh_in("gh api CI delivery evidence", Some(worktree), args)?;
    if !result.success() {
        return Err(refused(format!(
            "canonical GitHub read failed: {}",
            result.stderr.trim()
        )));
    }
    serde_json::from_str(&result.stdout).map_err(io::Error::other)
}

fn ref_segment(value: &str) -> String {
    value
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || b"-_.".contains(&byte) {
                char::from(byte).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

fn array(value: &Value) -> io::Result<&Vec<Value>> {
    value
        .as_array()
        .ok_or_else(|| refused("canonical GitHub response is incomplete"))
}

/// All facts come from the configured repository, never from operation params.
pub(crate) fn fetch(
    worktree: &Path,
    repository: &str,
    base: &str,
    head: &str,
) -> io::Result<CiSnapshot> {
    for branch in [base, head] {
        git(worktree, &["check-ref-format", "--branch", branch])?;
    }
    let repo = api(worktree, &format!("repos/{repository}"), false)?;
    if repo["full_name"].as_str() != Some(repository) {
        return Err(refused("canonical repository identity changed"));
    }
    let repository_id = repo["id"]
        .as_u64()
        .ok_or_else(|| refused("repository id is unknown"))?;
    let branch = |name: &str| {
        api(
            worktree,
            &format!("repos/{repository}/branches/{}", ref_segment(name)),
            false,
        )
    };
    let base_branch = branch(base)?;
    let head_branch = branch(head)?;
    let sha = |branch: &Value| {
        branch["commit"]["sha"]
            .as_str()
            .filter(|sha| valid_sha(sha))
            .map(str::to_string)
            .ok_or_else(|| refused("remote branch SHA is unreadable"))
    };
    let base_sha = sha(&base_branch)?;
    let head_sha = sha(&head_branch)?;
    let protection = api(
        worktree,
        &format!(
            "repos/{repository}/branches/{}/protection",
            ref_segment(base)
        ),
        false,
    )?;
    let rule_pages = api(
        worktree,
        &format!("repos/{repository}/rules/branches/{}", ref_segment(base)),
        true,
    )?;
    let mut rules = Vec::new();
    for page in array(&rule_pages)? {
        rules.extend(array(page)?.iter().cloned());
    }
    let mut required_checks = Vec::new();
    let mut strict = false;
    if let Some(policy) = protection
        .get("required_status_checks")
        .filter(|value| !value.is_null())
    {
        strict = policy["strict"]
            .as_bool()
            .ok_or_else(|| refused("strict check policy is unknown"))?;
        for check in array(&policy["checks"])? {
            required_checks.push(RequiredCheck {
                context: check["context"]
                    .as_str()
                    .ok_or_else(|| refused("required context is unreadable"))?
                    .into(),
                app_id: check["app_id"].as_i64().filter(|id| *id >= 0),
            });
        }
        for context in array(&policy["contexts"])? {
            let context = context
                .as_str()
                .ok_or_else(|| refused("required context is unreadable"))?;
            if !required_checks.iter().any(|check| check.context == context) {
                required_checks.push(RequiredCheck {
                    context: context.into(),
                    app_id: None,
                });
            }
        }
    }
    for rule in &rules {
        if rule["type"].as_str() != Some("required_status_checks") {
            continue;
        }
        let parameters = &rule["parameters"];
        strict |= parameters["strict_required_status_checks_policy"]
            .as_bool()
            .unwrap_or(true);
        for check in array(&parameters["required_status_checks"])? {
            let required = RequiredCheck {
                context: check["context"]
                    .as_str()
                    .ok_or_else(|| refused("ruleset context is unreadable"))?
                    .into(),
                app_id: check["integration_id"].as_i64().filter(|id| *id >= 0),
            };
            if !required_checks.contains(&required) {
                required_checks.push(required);
            }
        }
    }
    let check_pages = api(
        worktree,
        &format!("repos/{repository}/commits/{head_sha}/check-runs?filter=latest&per_page=100"),
        true,
    )?;
    let mut check_runs = Vec::new();
    for page in array(&check_pages)? {
        check_runs.extend(array(&page["check_runs"])?.iter().cloned());
    }
    let status_pages = api(
        worktree,
        &format!("repos/{repository}/commits/{head_sha}/statuses?per_page=100"),
        true,
    )?;
    let mut statuses = Vec::new();
    for page in array(&status_pages)? {
        statuses.extend(array(page)?.iter().cloned());
    }
    let comparison = api(
        worktree,
        &format!("repos/{repository}/compare/{base_sha}...{head_sha}"),
        false,
    )?;
    let files = array(&comparison["files"])?;
    // GitHub caps compare's file list at 300. At the cap, UI absence is unprovable.
    if files.len() >= 300 || !matches!(comparison["status"].as_str(), Some("ahead" | "diverged")) {
        return Err(refused(
            "complete deliverable remote source surface is unprovable",
        ));
    }
    let mut changed_files = Vec::new();
    for file in files {
        changed_files.push(
            file["filename"]
                .as_str()
                .ok_or_else(|| refused("remote filename is unreadable"))?
                .into(),
        );
        if let Some(previous) = file["previous_filename"].as_str() {
            changed_files.push(previous.into());
        }
    }
    Ok(CiSnapshot {
        repository: repository.into(),
        repository_id,
        base: base.into(),
        base_sha,
        head: head.into(),
        head_sha,
        policy: serde_json::json!({"protection":protection,"rules":rules}),
        required_checks,
        check_runs,
        statuses,
        changed_files,
        strict,
        behind_by: comparison["behind_by"]
            .as_u64()
            .ok_or_else(|| refused("base ancestry is unknown"))?,
    })
}

fn git(worktree: &Path, args: &[&str]) -> io::Result<String> {
    let output = gwt_core::process::hidden_command("git")
        .args(args)
        .current_dir(worktree)
        .output()?;
    if !output.status.success() {
        return Err(refused(String::from_utf8_lossy(&output.stderr)));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().into())
}

#[derive(Debug)]
enum Actor {
    Owner(gwt_agent::SessionExecutionBinding),
    Pm,
}

fn actor(worktree: &Path, session_id: &str, request: &CiCreateRequest) -> io::Result<Actor> {
    if crate::pm_registry::registered_pm_worktree_authority(worktree, session_id, worktree) {
        return Ok(Actor::Pm);
    }
    let binding = crate::cli::execution_state::snapshot_pr_mutation_execution_binding(
        worktree,
        Some(session_id),
    )?
    .ok_or_else(|| refused("exact durable owning generation or registered PM is required"))?;
    let record = crate::cli::execution_state::load(worktree)?
        .ok_or_else(|| refused("execution is missing"))?;
    if !matches!(
        record.status,
        crate::cli::execution_state::ExecutionControlStatus::Active
            | crate::cli::execution_state::ExecutionControlStatus::Blocked
    ) || crate::cli::delivered_owner::trusted_no_action_for_session(worktree, session_id)
        .is_some()
    {
        return Err(refused(
            "this terminal outcome carries no CI creation authority",
        ));
    }
    let session = gwt_agent::Session::load(
        &gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml")),
    )?;
    if session.branch != request.head
        || super::current_branch_name(worktree).as_deref() != Some(&request.head)
        || git(worktree, &["rev-parse", "HEAD"])? != request.head_sha
    {
        return Err(refused(
            "owning Session may nominate only its exact current pushed head",
        ));
    }
    if !crate::cli::verify_derivation::changed_source_paths_since(worktree, "HEAD")
        .map_err(refused)?
        .is_empty()
    {
        return Err(refused(
            "commit and push remaining source before CI delivery",
        ));
    }
    Ok(Actor::Owner(binding))
}

fn target_context(
    worktree: &Path,
    actor: &Actor,
    request: &CiCreateRequest,
) -> io::Result<(PathBuf, Option<String>, Option<u64>)> {
    let owner = match actor {
        Actor::Owner(binding) => Some(binding.owner_number),
        Actor::Pm => request
            .head
            .strip_prefix("work/issue-")
            .or_else(|| request.head.strip_prefix("issue-"))
            .and_then(|number| number.parse().ok()),
    };
    let target = if matches!(actor, Actor::Pm) {
        owner
            .and_then(|number| {
                crate::cli::execution_state::diagnose_owner(
                    worktree,
                    crate::cli::execution_state::ExecutionOwnerKey {
                        kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
                        number,
                    },
                )
                .holder_worktree
            })
            .map(PathBuf::from)
            .ok_or_else(|| {
                refused("PM nomination requires a provable managed Issue head and producing holder")
            })?
    } else {
        worktree.to_path_buf()
    };
    let record = crate::cli::execution_state::load(&target)?
        .ok_or_else(|| refused("producing execution is unknown"))?;
    if !crate::cli::execution_state::integrity_ok(&record) || Some(record.owner_number) != owner {
        return Err(refused("producing owner is inconsistent"));
    }
    let session = gwt_agent::Session::load(
        &gwt_core::paths::gwt_sessions_dir().join(format!("{}.toml", record.primary_session_id)),
    )?;
    let target_canonical = dunce::canonicalize(&target)?;
    if session.branch != request.head
        || dunce::canonicalize(&session.worktree_path).ok().as_ref() != Some(&target_canonical)
        || crate::cli::delivered_owner::trusted_no_action_for_session(
            &target,
            &record.primary_session_id,
        )
        .is_some()
    {
        return Err(refused(
            "nominated head does not match its producing Session",
        ));
    }
    let session = Some(record.primary_session_id);
    if super::current_branch_name(&target).as_deref() != Some(&request.head)
        || git(&target, &["rev-parse", "HEAD"])? != request.head_sha
        || !crate::cli::verify_derivation::changed_source_paths_since(&target, "HEAD")
            .map_err(refused)?
            .is_empty()
    {
        return Err(refused(
            "nominated producing worktree must match the complete pushed head",
        ));
    }
    Ok((target, session, owner))
}

fn unchanged_gates(
    worktree: &Path,
    actor: &Actor,
    request: &CiCreateRequest,
    snapshot: &CiSnapshot,
) -> io::Result<()> {
    let (target, target_session, owner) = target_context(worktree, actor, request)?;
    if let Some(refusal) =
        crate::cli::permission_readiness::settlement_refusal(&target).or_else(|| {
            owner.and_then(|owner| {
                crate::cli::permission_readiness::owner_settlement_refusal(worktree, owner)
            })
        })
    {
        return Err(refused(refusal));
    }
    for result in gwt_git::pr_status::user_verification_results(&request.body) {
        if let Some(reason) = crate::cli::verification_record::autonomous_confirmation_refusal(
            target_session.as_deref(),
            result,
        ) {
            return Err(refused(reason));
        }
    }
    if gwt_git::pr_status::body_defers_user_verification(&request.body) {
        return Err(refused(
            "human verification is still deferred; CI evidence does not settle that gate",
        ));
    }
    if let Some(session_id) = &target_session {
        if let Some(reason) = crate::cli::action_obligation::open_obligation_refusal(
            &target,
            session_id,
            &[crate::cli::action_obligation::ObligationKind::Pr],
        ) {
            return Err(refused(reason));
        }
    }
    if snapshot
        .changed_files
        .iter()
        .any(|path| crate::cli::verify_derivation::is_ui_surface_path(path))
    {
        let record = crate::cli::verification_record::load(&target)?.ok_or_else(|| {
            refused(
                "UI changes require existing exact-head measured headed dark/light E2E evidence",
            )
        })?;
        let session_id = target_session
            .as_deref()
            .ok_or_else(|| refused("UI producing Session is unknown"))?;
        if record.verified_head.as_deref() != Some(&request.head_sha)
            || !record.headed_e2e_passed()
            || crate::cli::verification_record::evaluate_evidence(&target, session_id, owner)
                != crate::cli::verification_record::EvidenceStatus::Fresh
        {
            return Err(refused(
                "UI changes require fresh exact-head measured headed dark/light E2E evidence",
            ));
        }
        let autonomous = crate::cli::execution_state::session_launch_route(Some(session_id))
            == Some(gwt_agent::LaunchRoute::Autonomous);
        if autonomous {
            if !request.body.lines().any(|line| {
                line.trim().trim_start_matches(['-', '*', ' ']) == "Agent Visual Check: pass"
            }) {
                return Err(refused(
                    "autonomous UI delivery requires Agent Visual Check: pass",
                ));
            }
        } else if gwt_git::pr_status::user_verification_result(&request.body).as_deref()
            != Some("confirmed")
        {
            return Err(refused(
                "manual UI delivery requires User Verification Result: confirmed",
            ));
        }
    }
    Ok(())
}

pub(super) fn run<E: crate::cli::CliEnv>(
    env: &mut E,
    request: CiCreateRequest,
    out: &mut String,
) -> Result<i32, gwt_github::SpecOpsError> {
    match create(env, &request) {
        Ok((pr, audit)) => {
            super::render_pr(out, &pr);
            out.push_str(&format!(
                "CI delivery audit: {audit}; execution and local verification unchanged\n"
            ));
            for warning in crate::cli::operation_warnings::snapshot() {
                out.push_str(&format!("warning: {}\n", warning.message));
            }
            Ok(0)
        }
        Err(error) => {
            out.push_str(&format!(
                "{error}. No Draft fallback or local PASS is authorized.\n"
            ));
            Ok(2)
        }
    }
}

fn create<E: crate::cli::CliEnv>(
    env: &mut E,
    request: &CiCreateRequest,
) -> io::Result<(gwt_git::PrStatus, String)> {
    let worktree = gwt_core::paths::resolve_current_worktree_root(env.repo_path());
    let session_id = std::env::var(gwt_agent::GWT_SESSION_ID_ENV).map_err(refused)?;
    let authority = actor(&worktree, &session_id, request)?;
    let snapshot = env.fetch_ci_delivery_snapshot(&request.base, &request.head)?;
    snapshot.validate(request)?;
    unchanged_gates(&worktree, &authority, request, &snapshot)?;
    let (_, producing_session_id, _) = target_context(&worktree, &authority, request)?;
    let producing_session = producing_session_id
        .as_ref()
        .map(|id| {
            gwt_agent::Session::load(
                &gwt_core::paths::gwt_sessions_dir().join(format!("{id}.toml")),
            )
        })
        .transpose()?;
    // Never acquire the worktree-global write lease beneath owner/Session
    // leases: activation acquires global -> owner. Audit outside that boundary.
    let audit_id = uuid::Uuid::new_v4().to_string();
    let file = format!("pr-ci-delivery-{audit_id}.json");
    let audit = serde_json::json!({"schema_version":1,"audit_id":audit_id,
        "created_at":chrono::Utc::now(),"session_id":session_id,"actor":format!("{authority:?}"),
        "evidence":snapshot,"execution_hash":crate::cli::execution_state::load(&worktree)?.map(|record| record.content_hash),
        "local_verification_hash":crate::cli::verification_record::load(&worktree)?.map(|record| record.content_hash.clone())});
    crate::cli::trusted_store::write_with_readback(
        &worktree,
        &file,
        &serde_json::to_vec_pretty(&audit)?,
    )?;
    let body = format!("{}\n\n<!-- gwt-ci-delivery:{audit_id} head:{} -->\nCI delivery evidence: mandatory GitHub checks passed for `{}` at `{}`. This admits Ready creation only; it does not claim local verification PASS or reopen the execution.\n", request.body, request.head_sha, request.head, request.head_sha);
    let body = crate::session_launch::with_named_pr_author(&body, producing_session.as_ref());
    let mut dispatch = || -> io::Result<gwt_git::PrStatus> {
        actor(&worktree, &session_id, request)?;
        let fresh = env.fetch_ci_delivery_snapshot(&request.base, &request.head)?;
        fresh.validate(request)?;
        if fresh != snapshot {
            return Err(refused(
                "repository, refs, policy or check evidence changed; fetch fresh CI evidence",
            ));
        }
        unchanged_gates(&worktree, &authority, request, &fresh)?;
        env.create_pr(
            &request.base,
            Some(&request.head),
            &request.title,
            &body,
            &request.labels,
            false,
        )
    };
    let result = match &authority {
        Actor::Owner(binding) => {
            crate::cli::execution_state::with_current_ci_pr_creation_binding_lease(
                &gwt_core::paths::gwt_sessions_dir(),
                binding,
                dispatch,
            )
            .and_then(|result| {
                result.ok_or_else(|| refused("owning generation changed before dispatch"))
            })
            .and_then(|result| result)
        }
        Actor::Pm => dispatch(),
    };
    let observed_head = match &result {
        Ok(pr) => env.fetch_pr_head_sha(pr.number),
        Err(_) => Ok(None),
    };
    let outcome = serde_json::json!({"audit_id":audit_id,"created_at":chrono::Utc::now(),
            "pr":result.as_ref().ok(),"error":result.as_ref().err().map(ToString::to_string),
            "observed_head":observed_head.as_ref().ok(),"readback_error":observed_head.as_ref().err().map(ToString::to_string)});
    crate::cli::trusted_store::write_with_readback(
        &worktree,
        &format!("pr-ci-delivery-{audit_id}-result.json"),
        &serde_json::to_vec_pretty(&outcome)?,
    )?;
    let pr = result?;
    if observed_head?.as_deref() != Some(&request.head_sha) || pr.head_ref_name != request.head {
        return Err(refused(format!("PR {} was created but its exact head could not be confirmed; inspect it before delivery", pr.url)));
    }
    if let Some(session) = producing_session.as_ref() {
        if let Err(error) = crate::session_launch::record_named_issue_activity(
            env.client(),
            session,
            crate::session_launch::NamedIssueActivity::PrCreated { number: pr.number },
        ) {
            crate::cli::operation_warnings::push(
                "named_issue_history_recording_skipped",
                format!(
                    "PR #{} was created, but its named Issue history could not be recorded: {error}",
                    pr.number
                ),
            );
        }
    }
    if matches!(authority, Actor::Owner(_)) {
        crate::cli::action_obligation::settle_kinds_best_effort(
            &worktree,
            &session_id,
            &[crate::cli::action_obligation::ObligationKind::Pr],
            "exact GitHub CI Ready creation",
        );
    }
    Ok((pr, audit_id))
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use serde_json::json;

    pub(in crate::cli::pr) fn request() -> CiCreateRequest {
        CiCreateRequest {
            base: "develop".into(), head: "work/issue-4214".into(),
            head_sha: "a".repeat(40), title: "fix: delivery".into(),
            body: "User Verification Result: n/a (autonomous)\nAgent Visual Check: n/a (no UI surface)".into(),
            labels: vec![],
        }
    }

    pub(in crate::cli::pr) fn snapshot() -> CiSnapshot {
        CiSnapshot {
            repository: "akiojin/gwt".into(),
            repository_id: 1,
            base: "develop".into(),
            base_sha: "b".repeat(40),
            head: "work/issue-4214".into(),
            head_sha: "a".repeat(40),
            policy: json!({"required_status_checks":{"strict":false}}),
            required_checks: vec![RequiredCheck {
                context: "Test".into(),
                app_id: Some(15368),
            }],
            check_runs: vec![json!({"id":1,"name":"Test","app":{"id":15368},
                "head_sha":"a".repeat(40),"status":"completed","conclusion":"success"})],
            statuses: vec![],
            changed_files: vec!["crates/gwt/src/cli/pr/mod.rs".into()],
            strict: false,
            behind_by: 0,
        }
    }

    #[test]
    fn exact_required_ci_pass_is_accepted_without_local_verification() {
        snapshot()
            .validate(&request())
            .expect("canonical exact-head required CI PASS");
    }

    #[test]
    fn ci_delivery_rejects_missing_pending_failed_skipped_or_foreign_checks() {
        let request = request();
        let mut missing = snapshot();
        missing.check_runs.clear();
        assert!(missing.validate(&request).is_err());
        for conclusion in ["failure", "skipped", "neutral", "cancelled"] {
            let mut failed = snapshot();
            failed.check_runs[0]["conclusion"] = json!(conclusion);
            assert!(failed.validate(&request).is_err());
        }
        let mut pending = snapshot();
        pending.check_runs[0]["status"] = json!("in_progress");
        assert!(pending.validate(&request).is_err());
        let mut foreign = snapshot();
        foreign.check_runs[0]["app"]["id"] = json!(99);
        assert!(foreign.validate(&request).is_err());
        foreign = snapshot();
        foreign.check_runs[0]["head_sha"] = json!("c".repeat(40));
        assert!(foreign.validate(&request).is_err());
    }

    #[test]
    fn ci_delivery_rejects_moved_head_and_latest_failed_retry() {
        let request = request();
        let mut moved = snapshot();
        moved.head_sha = "c".repeat(40);
        assert!(moved.validate(&request).is_err());
        let mut retry = snapshot();
        let mut newer = retry.check_runs[0].clone();
        newer["id"] = json!(2);
        newer["conclusion"] = json!("failure");
        retry.check_runs.push(newer);
        assert!(retry.validate(&request).is_err());
    }

    #[test]
    fn ci_delivery_checks_both_same_named_statuses_and_strict_policy() {
        let request = request();
        let mut snapshot = snapshot();
        snapshot
            .statuses
            .push(json!({"id":1,"context":"Test","state":"pending"}));
        assert!(snapshot.validate(&request).is_err());
        snapshot.statuses[0]["state"] = json!("success");
        snapshot.required_checks[0].app_id = Some(-1);
        snapshot.validate(&request).unwrap();
        snapshot.behind_by = 1;
        snapshot
            .validate(&request)
            .expect("strict=false BEHIND does not require synchronization");
        snapshot.strict = true;
        assert!(snapshot.validate(&request).is_err());
        snapshot.required_checks.clear();
        assert!(snapshot.validate(&request).is_err());
    }
}
