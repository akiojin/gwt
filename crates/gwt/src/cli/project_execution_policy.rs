//! Project-fixed policy diagnostics and explicitly authorized choice delivery.
use std::{
    io,
    path::{Path, PathBuf},
};

use gwt_core::{
    pm_authorization::{self as authority, Authorization, OperationIntent},
    project_execution_policy::{self as store, Choice, ChoiceRequest},
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::project_runtime::{client::ProjectRuntimeClient, events::OwnerCommand};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ChoiceInput {
    operation_id: String,
    expected_revision: u64,
    expected_generation: u64,
    choice: Choice,
}

pub(super) fn validate(params: &Value) -> io::Result<ChoiceInput> {
    let input: ChoiceInput = serde_json::from_value(params.clone())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    if !uuid::Uuid::parse_str(&input.operation_id)
        .is_ok_and(|id| id.to_string() == input.operation_id)
        || input.expected_generation == 0
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "operation_id must be a canonical lowercase UUID and expected_generation must be a live Project generation",
        ));
    }
    Ok(input)
}

fn runtime() -> io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
}

/// PM worktrees share a repository but have a different runtime worktree key.
/// Resolve only live owners in this project's canonical store, and refuse an
/// ambiguous match rather than sending a choice to whichever endpoint sorts first.
async fn project_client(repo: &Path) -> io::Result<(PathBuf, ProjectRuntimeClient)> {
    let home = gwt_core::paths::gwt_home();
    if let Some(observation) =
        crate::project_execution_policy::observation(repo).map_err(io::Error::other)?
    {
        if gwt_core::repo_hash::same_repository(repo, &observation.project_root)
            || dunce::canonicalize(repo)
                .ok()
                .zip(dunce::canonicalize(&observation.project_root).ok())
                .is_some_and(|(a, b)| a == b)
        {
            let client =
                ProjectRuntimeClient::from_current_project(&observation.project_root, &home)
                    .await
                    .map_err(io::Error::other)?;
            return Ok((observation.project_root, client));
        }
        return Err(io::Error::other(
            "Project observation belongs to another repository",
        ));
    }
    let project_dir = gwt_core::pm_peer::project_dir(repo);
    let expected_key = project_dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::other("Project identity unavailable"))?;
    let endpoints = match std::fs::read_dir(project_dir.join("runtime/daemon")) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(io::Error::other("Project execution owner is not ready"))
        }
        Err(error) => return Err(error),
    };
    let mut live = Vec::new();
    for entry in endpoints {
        let entry = entry?;
        if entry
            .path()
            .extension()
            .is_none_or(|extension| extension != "json")
        {
            continue;
        }
        let endpoint: gwt_core::daemon::DaemonEndpoint = match std::fs::read(entry.path())
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        {
            Some(endpoint) => endpoint,
            None => continue,
        };
        if endpoint.scope.repo_hash != expected_key
            || endpoint.scope.target != gwt_core::daemon::RuntimeTarget::Host
            || !(gwt_core::repo_hash::same_repository(repo, &endpoint.scope.project_root)
                || dunce::canonicalize(repo)
                    .ok()
                    .zip(dunce::canonicalize(&endpoint.scope.project_root).ok())
                    .is_some_and(|(a, b)| a == b))
        {
            continue;
        }
        if let Ok(client) =
            ProjectRuntimeClient::from_current_project(&endpoint.scope.project_root, &home).await
        {
            live.push((endpoint.scope.project_root, client));
        }
    }
    if live.len() == 1 {
        Ok(live.remove(0))
    } else {
        Err(io::Error::other(
            "Project execution owner is unavailable or ambiguous; inspect project.execution.status",
        ))
    }
}

/// Discovery is read-only: status must never start an owner or a cold Project.
pub(super) fn status(repo: &Path, operation_id: Option<&str>) -> io::Result<Value> {
    runtime()?.block_on(async {
        match project_client(repo).await {
            Ok((root, client)) => match client
                .command(OwnerCommand::ProjectExecutionStatus {
                    operation_id: operation_id.map(str::to_owned),
                })
                .await
            {
                Ok(status) => Ok(status),
                Err(error) => degraded_status(&root, operation_id, &error),
            },
            Err(error) => degraded_status(repo, operation_id, &error.to_string()),
        }
    })
}

fn degraded_status(repo: &Path, operation_id: Option<&str>, reason: &str) -> io::Result<Value> {
    let mut status = serde_json::to_value(crate::project_execution_policy::local_status(
        repo,
        operation_id,
    ))?;
    status["effective_state"] = json!("unknown");
    status["start_reason"] = json!(format!("runtime_unavailable: {reason}"));
    status["degraded"] = json!(true);
    status["recovery_operation"] = json!("project.execution.status");
    Ok(status)
}

pub(super) fn choose(
    repo: &Path,
    intent: &OperationIntent,
    authorization: &Authorization,
) -> io::Result<Value> {
    let input = validate(&intent.params)?;
    let actor = super::pm::ambient_session_id().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Project choice requires the current registered PM Session",
        )
    })?;
    runtime()?.block_on(async {
    let (owner_root, client) = project_client(repo).await?;
    let repo = owner_root.as_path();
    let policy = store::load(repo).ok();
    // The first call may have persisted before losing its ACK. Reconcile the
    // exact public intent without consuming one-off human authority twice.
    if let Some(receipt) = policy
        .as_ref()
        .and_then(|policy| policy.operation(&input.operation_id))
    {
        if receipt.actor != actor
            || receipt.generation != input.expected_generation
            || receipt.before_revision != input.expected_revision
            || receipt.choice != input.choice
            || authorization
                .instruction_id
                .as_ref()
                .or(authorization.approval_id.as_ref())
                .is_some_and(|id| id != &receipt.consent_source_id)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "operation_id already records a different intent; inspect project.execution.status",
            ));
        }
        return match client.command(OwnerCommand::ProjectExecutionStatus {
            operation_id: Some(input.operation_id.clone()),
        }).await {
            Ok(status) => Ok(status),
            Err(error) => degraded_status(repo, Some(&input.operation_id), &error),
        };
    }
        let mut before = client.command(OwnerCommand::ProjectExecutionStatus { operation_id: None })
            .await.map_err(io::Error::other)?;
        if before["generation"].as_u64() != Some(input.expected_generation)
            || before["revision"].as_u64() != Some(input.expected_revision) {
            return Err(io::Error::new(io::ErrorKind::WouldBlock,
                "Project generation or revision changed; inspect project.execution.status"));
        }
        let identity = client.identity().ok_or_else(|| io::Error::other("Project owner identity is unconfirmed"))?;
        let execution_owner = json!({"pid":identity.pid,"host_started_at":identity.host_started_at,"host_instance_id":identity.host_instance_id});
        let host_observation = crate::project_execution_policy::observation(repo).map_err(io::Error::other)?
            .map(|observed| json!({"host_pid":observed.host_pid,"host_started_at":observed.host_started_at,
                "generation":observed.generation,"view_set_revision":observed.view_set_revision}));
        if host_observation.as_ref().is_some_and(|observed|
            observed["generation"] != before["generation"] || observed["view_set_revision"] != before["view_set_revision"]) {
            return Err(io::Error::new(io::ErrorKind::WouldBlock,
                "Project views changed during preflight; inspect project.execution.status before using the human authorization"));
        }
        before["execution_owner"] = execution_owner.clone();
        before["host_observation"] = json!(host_observation);
        let attempt = authority::authorize(repo, intent, authorization, Some(&actor), before)?;
        if attempt.before["execution_owner"] != execution_owner
            || attempt.before["host_observation"] != json!(host_observation) {
            return Err(io::Error::new(io::ErrorKind::WouldBlock,
                "The authorized Project incarnation changed; reconcile the original operation before making a new explicit choice"));
        }
        let request = ChoiceRequest {
            operation_id: input.operation_id.clone(), expected_revision: input.expected_revision,
            generation: input.expected_generation, choice: input.choice,
            consent_source_id: attempt.authority_id.clone(), actor: actor.clone(),
            // The physical owner derives this from admitted work; JSON cannot grant lineage.
            authorized_work_lineage: Vec::new(),
        };
        let result = client.command(OwnerCommand::ProjectExecutionChoose {
            request, caller_session_id: Some(actor), expected_view_set_revision: None,
        }).await;
        let after = match &result {
            Ok(value) => value.clone(),
            Err(error) => json!({"operation_id":input.operation_id,"outcome":"unknown","error":error,
                "recovery_operation":"project.execution.status"}),
        };
        let finished = attempt.finish(repo, result.is_ok(), after);
        finish_choice_result(result, finished)
    })
}

fn finish_choice_result(result: Result<Value, String>, audit: io::Result<()>) -> io::Result<Value> {
    let value = result.map_err(|error| io::Error::other(format!(
        "Project choice outcome requires status reconciliation: {error}; use project.execution.status with the same operation_id")))?;
    audit.map_err(|error| io::Error::other(format!(
        "Project choice was recorded but final audit failed: {error}; use project.execution.status with the same operation_id")))?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_failure_preserves_project_choice_outcome_and_reconciliation() {
        let unknown = finish_choice_result(
            Err("owner ACK lost".into()),
            Err(io::Error::other("audit unavailable")),
        )
        .unwrap_err()
        .to_string();
        assert!(unknown.contains("owner ACK lost") && unknown.contains("project.execution.status"));
        let accepted = finish_choice_result(
            Ok(json!({"accepted":true})),
            Err(io::Error::other("audit unavailable")),
        )
        .unwrap_err()
        .to_string();
        assert!(accepted.contains("recorded") && accepted.contains("project.execution.status"));
    }

    #[test]
    fn originless_pm_choice_uses_the_physical_owner_project_store() {
        let _env = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = gwt_core::test_support::socket_tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let _session = gwt_core::test_support::ScopedEnvVar::set("GWT_SESSION_ID", "fixture-pm");
        let repo = home.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        for args in [
            vec!["init", "--quiet", "--initial-branch=main"],
            vec![
                "-c",
                "user.name=fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "--allow-empty",
                "-qm",
                "fixture",
            ],
        ] {
            assert!(gwt_core::process::hidden_command("git")
                .args(args)
                .current_dir(&repo)
                .status()
                .unwrap()
                .success());
        }
        let pm = crate::pm_registry::pm_worktree_path_for_repo_path(&repo);
        assert!(gwt_core::process::hidden_command("git")
            .args(["worktree", "add", "--quiet", "-b", "fixture-pm"])
            .arg(&pm)
            .current_dir(&repo)
            .status()
            .unwrap()
            .success());
        assert_ne!(
            gwt_core::paths::resolve_project_scope(&pm).hash,
            gwt_core::paths::resolve_project_scope(&repo).hash
        );
        crate::pm_registry::save_pm_prefs(
            &crate::pm_registry::pm_prefs_path_for_repo_path(&repo),
            &crate::pm_registry::PmPrefs {
                registration: Some(crate::pm_registry::PmRegistration {
                    session_id: "fixture-pm".into(),
                    agent_id: "codex".into(),
                    worktree_path: pm.display().to_string(),
                    created_at: None,
                    consecutive_crashes: 0,
                    next_not_before: None,
                }),
                ..Default::default()
            },
        )
        .unwrap();
        let executor = runtime().unwrap();
        let owner = executor
            .block_on(crate::project_runtime::test_support::ProjectOwnerHarness::start(&repo))
            .unwrap();
        let intent = OperationIntent {
            operation: "project.execution.choose".into(),
            params: json!({"operation_id":uuid::Uuid::new_v4().to_string(),
                "expected_revision":0,"expected_generation":1,"choice":"continue"}),
        };
        let approval =
            authority::record_approval(&repo, &intent, "Continue this project", "Stop it", false)
                .unwrap();
        let authorization = Authorization {
            instruction_id: None,
            approval_id: Some(approval.id),
        };
        let result = choose(&pm, &intent, &authorization).and_then(|result| {
            let replay = choose(&pm, &intent, &authorization)?;
            assert_eq!(replay["choice"], "continue");
            Ok(result)
        });
        executor.block_on(owner.shutdown());
        let result =
            result.expect("the PM checkout must deliver its exact approval to the owner store");
        assert_eq!(result["status"]["choice"], "continue");
        assert_eq!(store::load(&repo).unwrap().revision, 1);
        assert_eq!(store::policy_path(&pm), store::policy_path(&repo));
        assert!(authority::approvals(&repo).unwrap()[0].consumed);
        let offline = status(&pm, intent.params["operation_id"].as_str()).unwrap();
        assert_eq!(offline["choice"], "continue");
        assert_eq!(
            offline["operation_outcome"]["operation_id"],
            intent.params["operation_id"]
        );
        assert_eq!(offline["effective_state"], "unknown");
        assert_eq!(offline["degraded"], true);
        let scope = gwt_core::daemon::RuntimeScope::from_project_root(
            &repo,
            gwt_core::daemon::RuntimeTarget::Host,
        )
        .unwrap();
        assert!(
            !scope.endpoint_path(&gwt_core::paths::gwt_home()).exists(),
            "read-only diagnosis never starts an owner"
        );
    }

    #[test]
    fn status_without_an_owner_is_unknown_and_does_not_create_a_policy() {
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let result = status(&repo, None).unwrap();
        assert_eq!(result["effective_state"], "unknown");
        assert_eq!(result["degraded"], true);
        assert!(result["start_reason"]
            .as_str()
            .unwrap()
            .starts_with("runtime_unavailable:"));
        assert!(!store::policy_path(&repo).exists());
    }
}
