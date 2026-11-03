//! Fixed release-workflow policy. This never dispatches a workflow.
use std::{
    fs, io,
    path::{Path, PathBuf},
};

use gwt_config::{settings::ReleasePolicySnapshot, Settings};
use gwt_core::pm_authorization::{self as authority, Authorization, OperationIntent};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::cli::{CliEnv, CliParseError};

const WORKFLOW: &str = "prepare-release.yml";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleasePolicyContext {
    pub host_instance_id: String,
    pub session_id: String,
    pub capability_generation: u64,
    pub worktree_digest: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Input {
    request_id: String,
    expected_token: String,
    reason: String,
    added_by: Option<String>,
}

fn canonical_uuid(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|id| id.to_string() == value)
}

pub(crate) fn validate(intent: &OperationIntent) -> io::Result<()> {
    let p: Input = serde_json::from_value(intent.params.clone())
        .map_err(|_| invalid("invalid release policy parameters"))?;
    if !canonical_uuid(&p.request_id)
        || p.expected_token.len() != 64
        || !p
            .expected_token
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || p.reason.trim().is_empty()
        || p.reason.len() > 4096
        || p.reason.chars().any(char::is_control)
        || match intent.operation.as_str() {
            "actions.release_policy.add" => p.added_by.is_some(),
            "actions.release_policy.rollback" => p
                .added_by
                .as_deref()
                .is_none_or(|id| !canonical_uuid(id) || id == p.request_id),
            _ => true,
        }
    {
        return Err(invalid("release policy requires a UUID request_id, current expected_token and reason; rollback also requires the original added_by request_id"));
    }
    Ok(())
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
fn refused(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}
fn safe_config_error(_: impl std::fmt::Display) -> io::Error {
    io::Error::other("release policy configuration is unreadable or could not be persisted; inspect actions.release_policy.status before retrying")
}

/// Inspection PMs deliberately have capability generation zero. This proof
/// authenticates the existing issuer; it never mints or rebinds a capability.
pub(crate) fn context(repo: &Path) -> io::Result<ReleasePolicyContext> {
    let session_id = super::super::pm::ambient_session_id()
        .ok_or_else(|| refused("release policy requires a current registered PM Session"))?;
    gwt_agent::validate_session_id_path_component(&session_id)
        .map_err(|_| refused("invalid PM Session identity"))?;
    if !super::super::pm::caller_is_registered_pm(repo) {
        return Err(refused(
            "release policy requires the registered repository PM",
        ));
    }
    let session = gwt_agent::Session::load(
        &gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml")),
    )
    .map_err(|_| refused("PM Session is unavailable; inspect the current PM registration"))?;
    let worktree = dunce::canonicalize(gwt_core::paths::resolve_current_worktree_root(repo))?;
    let repository_hash = gwt_core::repo_hash::detect_repo_hash(&worktree)
        .ok_or_else(|| refused("current repository identity is unavailable"))?;
    if session.id != session_id
        || !matches!(
            session.status,
            gwt_agent::AgentStatus::Running
                | gwt_agent::AgentStatus::Idle
                | gwt_agent::AgentStatus::WaitingInput
        )
        || dunce::canonicalize(&session.worktree_path).ok().as_ref() != Some(&worktree)
        || session
            .repo_hash
            .as_ref()
            .is_some_and(|hash| hash != repository_hash.as_str())
        || session.project_state_root.as_deref().is_some_and(|root| {
            gwt_core::repo_hash::detect_repo_hash(root)
                != gwt_core::repo_hash::detect_repo_hash(&worktree)
        })
    {
        return Err(refused(
            "PM Session does not match the current live repository and worktree",
        ));
    }
    let target = crate::daemon_runtime::HookForwardTarget::from_env_strict()
        .map_err(|_| refused("PM Host bridge is invalid; inspect the current Session"))?
        .ok_or_else(|| {
            refused("PM Host bridge is unavailable; use the existing registered PM context")
        })?;
    let request = super::super::host_contract::request_for("release-policy");
    let probe = crate::daemon_runtime::fetch_host_contract_via_agent_bridge(&target, &request);
    let verdict =
        super::super::host_contract::classify_for_session(probe.clone(), &request, &session_id);
    if !verdict.is_compatible() {
        return Err(refused("PM Host identity or contract is unavailable; inspect execution.status in the current Session"));
    }
    let super::super::host_contract::HostContractProbe::Answered(receipt) = probe else {
        return Err(refused("PM Host proof is missing"));
    };
    Ok(ReleasePolicyContext {
        host_instance_id: receipt.host_instance_id,
        session_id,
        capability_generation: receipt.capability_generation,
        worktree_digest: authority::digest(worktree.as_os_str().as_encoded_bytes()),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    token: String,
    allowed: bool,
}
fn snapshot(repository: &str, value: &ReleasePolicySnapshot) -> Snapshot {
    let bytes = [repository.as_bytes(), &[0], value.content.as_bytes()].concat();
    Snapshot {
        token: authority::digest(&bytes),
        allowed: value.allowed,
    }
}

fn receipts_dir(repo: &Path) -> PathBuf {
    gwt_core::paths::gwt_project_dir_for_repo_path(repo).join("project-state/release-policy")
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    schema_version: u32,
    request_id: String,
    repository: String,
    scope: authority::OperationScope,
    authorization: Authorization,
    actor: ReleasePolicyContext,
    at: String,
    reason: String,
    before: Snapshot,
    after: Snapshot,
    outcome: String,
    attempt: Option<authority::Attempt>,
}

fn read_receipt(path: &Path) -> io::Result<Option<Receipt>> {
    match fs::read(path) {
        Ok(bytes) if bytes.len() <= 1024 * 1024 => {
            serde_json::from_slice(&bytes).map(Some).map_err(|_| {
                io::Error::other(
                    "release policy receipt is invalid; preserve it and inspect status",
                )
            })
        }
        Ok(_) => Err(io::Error::other(
            "release policy receipt is oversized; preserve it and inspect status",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(io::Error::other(
            "release policy receipt is unreadable; preserve it and inspect status",
        )),
    }
}
fn save_receipt(path: &Path, receipt: &Receipt) -> io::Result<()> {
    gwt_config::atomic::write_atomic(path, &serde_json::to_string(receipt)?)
        .map_err(|_| io::Error::other("release policy outcome is UNKNOWN: receipt persistence failed; inspect status without resending"))
}
fn public_receipt(receipt: &Receipt) -> Value {
    let succeeded = receipt.outcome == "succeeded";
    let outcome = if receipt.outcome == "pending" {
        "unknown"
    } else {
        &receipt.outcome
    };
    json!({"request_id":receipt.request_id,"repository":receipt.repository,"workflow":WORKFLOW,
        "outcome":outcome,"before":receipt.before,"after":succeeded.then_some(&receipt.after),
        "proposed_after":receipt.after,
        "attempt_id":receipt.attempt.as_ref().map(|attempt| &attempt.id),
        "rollback_added_by":(succeeded && receipt.scope.operation == "actions.release_policy.add" && !receipt.before.allowed && receipt.after.allowed).then_some(&receipt.request_id)})
}

fn record_failure(
    repo: &Path,
    path: &Path,
    receipt: &mut Receipt,
    current: Value,
) -> io::Result<()> {
    let audit_failed = receipt.attempt.as_ref().is_some_and(|attempt| {
        attempt
            .finish(
                repo,
                false,
                json!({"outcome":receipt.outcome,"current":current}),
            )
            .is_err()
    });
    if audit_failed {
        receipt.outcome = "unknown".into();
    }
    save_receipt(path, receipt)?;
    if audit_failed {
        return Err(io::Error::other("release policy outcome is UNKNOWN: final audit failed; inspect status without resending"));
    }
    Ok(())
}

pub(crate) fn status<E: CliEnv>(env: &mut E) -> io::Result<Value> {
    let (owner, name) = env.github_repository()?;
    let repository = format!("{owner}/{name}");
    let path = gwt_core::paths::gwt_config_path();
    let current = snapshot(
        &repository,
        &Settings::release_policy_snapshot(&path, &repository).map_err(safe_config_error)?,
    );
    let identity = env.release_policy_context();
    let mut receipts = Vec::new();
    match fs::read_dir(receipts_dir(env.repo_path())) {
        Ok(entries) => {
            for entry in entries {
                let entry = entry
                    .map_err(|_| io::Error::other("release policy receipts are unreadable"))?;
                if entry.path().extension().is_some_and(|ext| ext == "json") {
                    if let Some(receipt) = read_receipt(&entry.path())? {
                        receipts.push(public_receipt(&receipt));
                    }
                }
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(_) => return Err(io::Error::other("release policy receipts are unreadable")),
    }
    Ok(
        json!({"schema_version":1,"repository":repository,"workflow":WORKFLOW,
        "allowed":current.allowed,"token":current.token,"runtime_version":env!("CARGO_PKG_VERSION"),
        "build_commit":option_env!("GWT_BUILD_COMMIT"),"build_epoch":option_env!("GWT_BUILD_EPOCH"),
        "requirement":{"registered_pm":true,"permission_stage":"instruction_or_approval","exact_scope":true,"host_proof":"fresh_same_issuer_session_generation"},
        "current":{"registered_pm":super::super::pm::caller_is_registered_pm(env.repo_path()),"host_verified":identity.is_ok(),"context":identity.ok()},
        "recovery":{"authority":"pm.approval.request or a new scoped human instruction; existing QA scopes are immutable",
            "stale_token":"actions.release_policy.status, then use the fresh token and exact scoped authority",
            "unknown":"read the request receipt and current token; do not resend an unknown request",
            "runtime":"use the verified checkout CLI with the existing registered PM Host bridge; never create replacement credentials"},
        "requests":receipts}),
    )
}

pub(crate) fn parse_status(params: &Value) -> Result<(), CliParseError> {
    if params.as_object().is_some_and(|p| p.is_empty()) {
        Ok(())
    } else {
        Err(CliParseError::InvalidJson(
            "actions.release_policy.status accepts no parameters".into(),
        ))
    }
}

pub(crate) fn execute<E: CliEnv>(
    env: &mut E,
    repo: &Path,
    intent: &OperationIntent,
    authorization: &Authorization,
) -> io::Result<Value> {
    validate(intent)?;
    let input: Input = serde_json::from_value(intent.params.clone()).map_err(io::Error::other)?;
    let (owner, name) = env.github_repository()?;
    let repository = format!("{owner}/{name}");
    let initial_context = env.release_policy_context()?;
    let scope = intent.scope()?;
    let path = receipts_dir(repo).join(format!("{}.json", input.request_id));
    let mut receipt: Option<Receipt> = None;
    let mut replay = None;
    let mut refusal = None;
    let allow = intent.operation == "actions.release_policy.add";
    let result = Settings::with_locked_release_policy(
        &gwt_core::paths::gwt_config_path(),
        &repository,
        allow,
        |raw_before, raw_after| {
            let accepted = (|| -> io::Result<()> {
                // Acceptance is the final fresh proof under the Settings lock.
                // An already accepted operation is not cancelled by later rotation.
                let current_context = env.release_policy_context()?;
                if current_context != initial_context
                    || !super::super::pm::caller_is_registered_pm(repo)
                {
                    return Err(refused("PM Host, Session, generation or registration changed before release policy acceptance"));
                }
                let before = snapshot(&repository, raw_before);
                let after = snapshot(&repository, raw_after);
                if let Some(previous) = read_receipt(&path)? {
                    if previous.schema_version != 1
                        || previous.scope != scope
                        || previous.repository != repository
                        || previous.request_id != input.request_id
                    {
                        return Err(refused(
                            "release policy request_id already belongs to a different request",
                        ));
                    }
                    if previous.outcome != "succeeded" {
                        return Err(io::Error::other("release policy outcome is UNKNOWN or failed; inspect the existing request receipt without resending"));
                    }
                    let attempt = previous
                        .attempt
                        .as_ref()
                        .filter(|a| a.scope == previous.scope)
                        .ok_or_else(|| {
                            refused("release policy receipt has no accepted authority")
                        })?;
                    authority::authorize_replay(repo, intent, authorization, attempt)?;
                    replay = Some(public_receipt(&previous));
                    // Abort the writer even if later settings changed: replay must
                    // never reapply a historical request's effect.
                    return Err(io::Error::other("release policy request already completed"));
                }
                if before.token != input.expected_token {
                    return Err(refused("stale release policy expected_token; read actions.release_policy.status and obtain authority for the fresh request"));
                }
                if let Some(original_id) = &input.added_by {
                    let original = read_receipt(&receipts_dir(repo).join(format!("{original_id}.json")))?
                        .filter(|r| r.schema_version == 1 && r.request_id == *original_id
                            && r.repository == repository && r.outcome == "succeeded"
                            && r.scope.operation == "actions.release_policy.add"
                            && !r.before.allowed && r.after.allowed
                            && r.after.token == before.token
                            && r.attempt.as_ref().is_some_and(|a| a.scope == r.scope))
                        .ok_or_else(|| refused("rollback requires this repository's successful added_by receipt and its unchanged post-state; preexisting permission cannot be removed"))?;
                    if !before.allowed || original.after.token != input.expected_token {
                        return Err(refused("release policy rollback post-state changed"));
                    }
                }
                let normalized = authority::check_authorization(repo, intent, authorization)?;
                let pending = Receipt {
                    schema_version: 1,
                    request_id: input.request_id.clone(),
                    repository: repository.clone(),
                    scope: scope.clone(),
                    authorization: normalized.clone(),
                    actor: current_context,
                    at: chrono::Utc::now().to_rfc3339(),
                    reason: gwt_core::error_ledger::redact_credentials(&input.reason),
                    before,
                    after,
                    outcome: "pending".into(),
                    attempt: None,
                };
                // A durable pending request precedes consumption. Crashes on
                // either side of the effect remain observable and never resend.
                save_receipt(&path, &pending)?;
                receipt = Some(pending);
                let pending = receipt.as_mut().expect("pending receipt");
                pending.attempt = Some(authority::authorize(
                    repo,
                    intent,
                    &normalized,
                    Some(&initial_context.session_id),
                    json!({
                        "request_id":input.request_id,"repository":repository,"workflow":WORKFLOW,
                        "reason":pending.reason,"policy":pending.before,"actor":pending.actor,
                    }),
                )?);
                save_receipt(&path, pending)?;
                Ok(())
            })();
            accepted.map_err(|error| {
                refusal = Some(error);
                gwt_config::ConfigError::WriteError {
                    reason: "release policy request did not reach Settings persistence".into(),
                }
            })
        },
    );
    if let Some(replayed) = replay {
        return Ok(replayed);
    }
    if let Some(error) = refusal {
        if let Some(pending) = receipt.as_mut() {
            pending.outcome = "failed".into();
            record_failure(repo, &path, pending, json!({"effect":"none"}))?;
        }
        return Err(error);
    }
    let Some(mut receipt) = receipt else {
        return Err(safe_config_error("no receipt"));
    };
    if result.is_err() {
        let readback =
            Settings::release_policy_snapshot(&gwt_core::paths::gwt_config_path(), &repository)
                .ok()
                .map(|raw| snapshot(&repository, &raw));
        // A later readback cannot prove that no rename happened: another
        // writer may already have restored the old bytes. Keep it diagnostic.
        receipt.outcome = "unknown".into();
        record_failure(repo, &path, &mut receipt, json!(readback))?;
        return Err(io::Error::other("release policy persistence did not confirm success; inspect actions.release_policy.status without resending this request"));
    }
    let attempt = receipt.attempt.as_ref().expect("authorized receipt");
    if attempt
        .finish(
            repo,
            true,
            json!({"request_id":receipt.request_id,"policy":receipt.after}),
        )
        .is_err()
    {
        receipt.outcome = "unknown".into();
        save_receipt(&path, &receipt)?;
        return Err(io::Error::other("release policy outcome is UNKNOWN: configuration saved but final audit failed; inspect status without resending"));
    }
    receipt.outcome = "succeeded".into();
    save_receipt(&path, &receipt)?;
    Ok(public_receipt(&receipt))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_core::test_support::{ScopedEnvVar, ScopedGwtHome};

    fn pm(repo: &Path) -> ScopedEnvVar {
        use crate::pm_registry::{
            pm_prefs_path_for_repo_path, save_pm_prefs, PmPrefs, PmRegistration,
        };
        save_pm_prefs(
            &pm_prefs_path_for_repo_path(repo),
            &PmPrefs {
                registration: Some(PmRegistration {
                    session_id: "release-policy-pm".into(),
                    agent_id: "codex".into(),
                    worktree_path: repo.display().to_string(),
                    created_at: None,
                    consecutive_crashes: 0,
                    next_not_before: None,
                }),
                ..Default::default()
            },
        )
        .unwrap();
        ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "release-policy-pm")
    }
    fn proof() -> ReleasePolicyContext {
        ReleasePolicyContext {
            host_instance_id: "fixture-host".into(),
            session_id: "release-policy-pm".into(),
            capability_generation: 0,
            worktree_digest: "fixture-worktree".into(),
        }
    }
    fn approve(repo: &Path, operation: &str, params: Value) -> (OperationIntent, Authorization) {
        let intent = OperationIntent {
            operation: operation.into(),
            params,
        };
        let approval = authority::record_approval(
            repo,
            &intent,
            "Fixture approval",
            "Fixture rollback",
            false,
        )
        .unwrap();
        (
            intent,
            Authorization {
                instruction_id: None,
                approval_id: Some(approval.id),
            },
        )
    }
    fn proofs(env: &mut crate::cli::TestEnv) {
        env.release_policy_contexts
            .extend([Ok(proof()), Ok(proof())]);
    }

    #[test]
    fn release_policy_add_replays_without_effect_and_rollback_preserves_preexisting_permission() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        let _session = pm(&repo);
        let mut env = crate::cli::TestEnv::new(repo.clone());
        let original_token = status(&mut env).unwrap()["token"]
            .as_str()
            .unwrap()
            .to_owned();
        let (intent, authorization) = approve(
            &repo,
            "actions.release_policy.add",
            json!({
                "request_id":"d7c21b88-9ed4-47ef-a86f-53bb8ec58b92", "expected_token":original_token,
                "reason":"Enable release"
            }),
        );
        proofs(&mut env);
        let result = execute(&mut env, &repo, &intent, &authorization).unwrap();
        assert_eq!(result["outcome"], "succeeded");
        let config = fs::read(gwt_core::paths::gwt_config_path()).unwrap();
        let audit = fs::read(authority::audit_path(&repo)).unwrap();
        proofs(&mut env);
        assert_eq!(
            execute(&mut env, &repo, &intent, &authorization).unwrap(),
            result
        );
        assert_eq!(
            fs::read(gwt_core::paths::gwt_config_path()).unwrap(),
            config
        );
        assert_eq!(fs::read(authority::audit_path(&repo)).unwrap(), audit);

        let (rollback, rollback_auth) = approve(
            &repo,
            "actions.release_policy.rollback",
            json!({
                "request_id":"81ba15f0-3889-4bb8-a49b-dfc587f1170d", "expected_token":result["after"]["token"],
                "reason":"Undo this add", "added_by":intent.params["request_id"]
            }),
        );
        proofs(&mut env);
        assert_eq!(
            execute(&mut env, &repo, &rollback, &rollback_auth).unwrap()["after"]["allowed"],
            false
        );

        // A completed add request stays historical after rollback: it must not
        // write permission back even though the current setting is different.
        proofs(&mut env);
        assert_eq!(
            execute(&mut env, &repo, &intent, &authorization).unwrap(),
            result
        );
        assert_eq!(status(&mut env).unwrap()["allowed"], false);

        let current = status(&mut env).unwrap();
        let (add, add_auth) = approve(
            &repo,
            "actions.release_policy.add",
            json!({
                "request_id":"d8426bdb-2c0f-4fa7-b191-9d58a3600f61", "expected_token":current["token"],
                "reason":"Enable again"
            }),
        );
        proofs(&mut env);
        let first = execute(&mut env, &repo, &add, &add_auth).unwrap();
        let (noop, noop_auth) = approve(
            &repo,
            "actions.release_policy.add",
            json!({
                "request_id":"0c7c7873-3f0c-4ad7-a303-ffbc25d05d62", "expected_token":first["after"]["token"],
                "reason":"Already allowed"
            }),
        );
        proofs(&mut env);
        let unchanged = execute(&mut env, &repo, &noop, &noop_auth).unwrap();
        let (invalid_undo, invalid_auth) = approve(
            &repo,
            "actions.release_policy.rollback",
            json!({
                "request_id":"680fbdc6-c215-4642-a3b3-48e53f78d777", "expected_token":unchanged["after"]["token"],
                "reason":"Must not remove preexisting permission", "added_by":noop.params["request_id"]
            }),
        );
        proofs(&mut env);
        assert!(execute(&mut env, &repo, &invalid_undo, &invalid_auth).is_err());
        assert_eq!(status(&mut env).unwrap()["allowed"], true);
    }

    #[test]
    fn release_policy_changed_issuer_and_stale_cas_refuse_before_consumption() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        let _session = pm(&repo);
        let mut env = crate::cli::TestEnv::new(repo.clone());
        let current = status(&mut env).unwrap();
        let (intent, authorization) = approve(
            &repo,
            "actions.release_policy.add",
            json!({
                "request_id":"f33c3165-33f2-48ae-9ffc-13d052d60f57", "expected_token":current["token"],
                "reason":"Controlled refusal"
            }),
        );
        let mut changed = proof();
        changed.host_instance_id = "replacement-host".into();
        env.release_policy_contexts
            .extend([Ok(proof()), Ok(changed)]);
        assert!(execute(&mut env, &repo, &intent, &authorization).is_err());
        assert!(!gwt_core::paths::gwt_config_path().exists());
        assert!(!receipts_dir(&repo).exists());
        assert!(!authority::approvals(&repo).unwrap()[0].consumed);

        Settings::update_global(|settings| {
            settings.debug = true;
            Ok(())
        })
        .unwrap();
        let before = fs::read(gwt_core::paths::gwt_config_path()).unwrap();
        proofs(&mut env);
        assert!(execute(&mut env, &repo, &intent, &authorization).is_err());
        assert_eq!(
            fs::read(gwt_core::paths::gwt_config_path()).unwrap(),
            before
        );
        assert!(!authority::approvals(&repo).unwrap()[0].consumed);
        assert!(!receipts_dir(&repo).exists());
    }

    #[test]
    fn release_policy_persistence_failure_is_unknown_without_resend() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        let _session = pm(&repo);
        Settings::update_global(|settings| {
            settings.debug = true;
            Ok(())
        })
        .unwrap();
        let config = gwt_core::paths::gwt_config_path();
        let backup = config.with_extension("fixture-backup");
        let mut env = crate::cli::TestEnv::new(repo.clone());
        let current = status(&mut env).unwrap();
        let (intent, authorization) = approve(
            &repo,
            "actions.release_policy.add",
            json!({
                "request_id":"966b0e85-afc9-4500-b5b3-cf967f790464", "expected_token":current["token"],
                "reason":"Controlled persistence failure"
            }),
        );
        let target = config.clone();
        let saved = backup.clone();
        let mut calls = 0;
        env.release_policy_context_hook = Some(Box::new(move || {
            calls += 1;
            if calls == 2 {
                fs::rename(&target, &saved).unwrap();
                fs::create_dir(&target).unwrap();
            }
            Ok(proof())
        }));
        let result = execute(&mut env, &repo, &intent, &authorization);
        fs::remove_dir(&config).unwrap();
        fs::rename(&backup, &config).unwrap();
        env.release_policy_context_hook = None;
        assert!(result.is_err());
        assert_eq!(
            status(&mut env).unwrap()["requests"][0]["outcome"],
            "unknown"
        );
        assert_eq!(status(&mut env).unwrap()["allowed"], false);
        proofs(&mut env);
        assert!(execute(&mut env, &repo, &intent, &authorization).is_err());
        assert_eq!(status(&mut env).unwrap()["allowed"], false);
    }

    #[test]
    fn release_policy_audit_failure_preserves_unknown_and_pending_never_reapplies() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        let _session = pm(&repo);
        let mut env = crate::cli::TestEnv::new(repo.clone());
        let current = status(&mut env).unwrap();
        let (intent, authorization) = approve(
            &repo,
            "actions.release_policy.add",
            json!({
                "request_id":"e2c04065-05c3-4fc4-842c-bff38dbf380e", "expected_token":current["token"],
                "reason":"Crash and final audit failure fixture"
            }),
        );
        let attempt = authority::authorize(
            &repo,
            &intent,
            &authorization,
            Some("release-policy-pm"),
            json!({}),
        )
        .unwrap();
        let path = receipts_dir(&repo).join(format!(
            "{}.json",
            intent.params["request_id"].as_str().unwrap()
        ));
        let raw =
            Settings::release_policy_snapshot(&gwt_core::paths::gwt_config_path(), "owner/repo")
                .unwrap();
        let mut receipt = Receipt {
            schema_version: 1,
            request_id: intent.params["request_id"].as_str().unwrap().into(),
            repository: "owner/repo".into(),
            scope: intent.scope().unwrap(),
            authorization: authorization.clone(),
            actor: proof(),
            at: chrono::Utc::now().to_rfc3339(),
            reason: "Fixture".into(),
            before: snapshot("owner/repo", &raw),
            after: Snapshot {
                token: "a".repeat(64),
                allowed: true,
            },
            outcome: "pending".into(),
            attempt: Some(attempt),
        };
        save_receipt(&path, &receipt).unwrap();
        assert_eq!(
            status(&mut env).unwrap()["requests"][0]["outcome"],
            "unknown"
        );
        proofs(&mut env);
        assert!(execute(&mut env, &repo, &intent, &authorization).is_err());
        assert!(!gwt_core::paths::gwt_config_path().exists());
        fs::remove_file(authority::audit_path(&repo)).unwrap();
        fs::create_dir(authority::audit_path(&repo)).unwrap();
        receipt.outcome = "failed".into();
        assert!(record_failure(&repo, &path, &mut receipt, json!({"effect":"none"})).is_err());
        assert_eq!(read_receipt(&path).unwrap().unwrap().outcome, "unknown");
        assert_eq!(public_receipt(&receipt)["after"], Value::Null);
    }
}
