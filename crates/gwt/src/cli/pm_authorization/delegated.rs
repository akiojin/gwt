//! Scoped authority for three existing operations; their full dispatcher stays authoritative.
use std::{io, path::Path};

use gwt_core::pm_authorization::{self as authority, Authorization, OperationIntent};
use gwt_github::SpecOpsError;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::cli::{CliEnv, CliParseError};

pub(super) fn supports(operation: &str) -> bool {
    matches!(
        operation,
        "pr.close" | "worktree.gc_build_artifacts" | "pm.message.send"
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    operation: String,
    params: Value,
    instruction_id: Option<String>,
    approval_id: Option<String>,
}

pub(super) fn parse(params: &Value) -> Result<super::PmAuthorizationCommand, CliParseError> {
    let request: Request = serde_json::from_value(params.clone())
        .map_err(|error| CliParseError::InvalidJson(error.to_string()))?;
    let intent = OperationIntent {
        operation: request.operation,
        params: request.params,
    };
    validate(&intent).map_err(|error| CliParseError::InvalidJson(error.to_string()))?;
    if [&request.instruction_id, &request.approval_id]
        .into_iter()
        .flatten()
        .any(|id| id.trim().is_empty())
    {
        return Err(CliParseError::InvalidJson(
            "authority ids must be nonempty".into(),
        ));
    }
    Ok(super::PmAuthorizationCommand::Execute {
        intent,
        authorization: Authorization {
            instruction_id: request.instruction_id,
            approval_id: request.approval_id,
        },
    })
}

pub(super) fn validate(intent: &OperationIntent) -> io::Result<()> {
    intent.scope()?;
    let allowed: &[&str] = match intent.operation.as_str() {
        "pr.close" => &["number", "comment"],
        "worktree.gc_build_artifacts" => &["dry_run", "base", "include_unmerged", "include_protected_workspaces"],
        "pm.message.send" => &["id", "text"],
        _ => return Err(io::Error::new(io::ErrorKind::InvalidInput, "pm.operation.execute only supports pr.close, worktree.gc_build_artifacts and pm.message.send")),
    };
    let params = intent
        .params
        .as_object()
        .expect("scope requires object params");
    if params.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "delegated operation accepts only its named effect parameters; repository and project_root are fixed"));
    }
    crate::cli::json_envelope::parse_operation(&intent.operation, &intent.params)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error.to_string()))?;
    if intent.operation == "pr.close"
        && !params
            .get("number")
            .and_then(Value::as_u64)
            .is_some_and(|number| number > 0)
        || intent.operation == "pm.message.send"
            && ["id", "text"].iter().any(|key| {
                !params
                    .get(*key)
                    .and_then(Value::as_str)
                    .is_some_and(|value| !value.trim().is_empty())
            })
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "delegated target and action must be explicit and nonempty",
        ));
    }
    Ok(())
}

fn before<E: CliEnv>(env: &mut E, intent: &OperationIntent) -> io::Result<Value> {
    match intent.operation.as_str() {
        "pr.close" => {
            env.github_repository()?;
            let number = intent.params["number"]
                .as_u64()
                .expect("validated PR number");
            let pr = env.fetch_pr(number)?;
            if pr.number != number {
                return Err(io::Error::other(
                    "PR snapshot does not match the authorized number",
                ));
            }
            Ok(json!({"number":pr.number,"state":pr.state.to_string(),"url":pr.url}))
        }
        "worktree.gc_build_artifacts" => {
            let mut params = intent.params.clone();
            params["dry_run"] = true.into();
            let command = crate::cli::json_envelope::parse_operation(&intent.operation, &params)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error.to_string()))?;
            let (code, output) = crate::cli::run_collect(env, command)
                .map_err(|error| io::Error::other(error.to_string()))?;
            if code != 0 {
                return Err(io::Error::other("cleanup dry-run preflight failed"));
            }
            serde_json::from_str(&output).map_err(io::Error::other)
        }
        "pm.message.send" => Ok(json!({
            "target_id":intent.params["id"],
            "text_sha256":authority::digest(intent.params["text"].as_str().expect("validated input").as_bytes()),
        })),
        _ => unreachable!("validated delegation"),
    }
}

pub(super) fn run<E: CliEnv>(
    env: &mut E,
    repo: &Path,
    intent: &OperationIntent,
    authorization: &Authorization,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    validate(intent).map_err(crate::cli::io_as_api_error)?;
    let before = before(env, intent).map_err(crate::cli::io_as_api_error)?;
    let attempt = authority::authorize(
        repo,
        intent,
        authorization,
        crate::cli::pm::ambient_session_id().as_deref(),
        before.clone(),
    )
    .map_err(crate::cli::io_as_api_error)?;
    let input =
        json!({"schema_version":1,"operation":intent.operation,"params":intent.params}).to_string();
    let mut captured = Vec::new();
    let code = crate::cli::json_envelope::dispatch_input(env, "gwtd", &input, Some(&mut captured));
    let after = serde_json::from_slice::<Value>(&captured).unwrap_or_else(|_| json!({
        "ok":false,"exit_code":code,"error":"inner operation response unavailable; inspect its state before retrying",
    }));
    let code = if code == 0 && after["ok"] != true {
        1
    } else {
        code
    };
    attempt.finish(repo, code == 0, after.clone()).map_err(|error| crate::cli::io_as_api_error(io::Error::other(
        format!("delegated operation exit_code={code}, but final audit failed: {error}; inspect its state before retrying")
    )))?;
    out.push_str(&json!({"attempt_id":attempt.id,"before":before,"after":after,"audit_path":authority::audit_path(repo)}).to_string());
    out.push('\n');
    Ok(code)
}
