//! Bounded PM settings operations; caller-supplied approval booleans never authorize.
use super::{CliEnv, CliParseError};
use gwt_core::pm_authorization::{self as authority, Authorization, OperationIntent};
use gwt_github::SpecOpsError;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{io, path::Path};

mod delegated;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PmAuthorizationCommand {
    Execute {
        intent: OperationIntent,
        authorization: Authorization,
    },
    Request {
        intent: OperationIntent,
        reason: String,
        rollback: String,
    },
    Status,
    Revoke {
        id: String,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DiscussionInput {
    title: String,
    body: String,
    category: String,
    #[serde(default)]
    sync_board: bool,
    board_provider: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BindingInput {
    #[serde(default)]
    github_discussion: u64,
    category: Option<String>,
    provider: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChecksInput {
    branch: String,
    #[serde(default)]
    add: Vec<String>,
    #[serde(default)]
    remove: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateInput {
    auto_apply_updates: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AutoMergeInput {
    number: u64,
    reviewed_head_sha: String,
}

fn decode<T: serde::de::DeserializeOwned>(params: &Value) -> io::Result<T> {
    serde_json::from_value(params.clone())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))
}

fn validate(intent: &OperationIntent) -> io::Result<()> {
    intent.scope()?;
    let invalid = || {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid effect parameters for the bounded PM operation",
        )
    };
    match intent.operation.as_str() {
        "discussion.create" => {
            let p: DiscussionInput = decode(&intent.params)?;
            if [&p.title, &p.body, &p.category]
                .iter()
                .any(|value| value.trim().is_empty())
            {
                return Err(invalid());
            }
            if p.board_provider
                .as_deref()
                .is_some_and(|value| value != "local")
                || (p.board_provider.is_some() && !p.sync_board)
            {
                return Err(invalid());
            }
        }
        "board.config.set" => {
            let p: BindingInput = decode(&intent.params)?;
            if p.category
                .as_deref()
                .is_some_and(|value| value.trim().is_empty())
                || p.provider.as_deref().is_some_and(|value| value != "local")
            {
                return Err(invalid());
            }
        }
        "repository.required_checks.update" => {
            let p: ChecksInput = decode(&intent.params)?;
            if !matches!(p.branch.as_str(), "develop" | "main")
                || (p.add.is_empty() && p.remove.is_empty())
                || p.add
                    .iter()
                    .chain(&p.remove)
                    .any(|name| name.trim().is_empty() || name.chars().any(char::is_control))
                || p.add.iter().any(|name| p.remove.contains(name))
            {
                return Err(invalid());
            }
        }
        "update.settings.set" => {
            let _: UpdateInput = decode(&intent.params)?;
        }
        "pr.auto_merge.enable" => {
            let p: AutoMergeInput = decode(&intent.params)?;
            if p.number == 0
                || p.reviewed_head_sha.len() != 40
                || !p
                    .reviewed_head_sha
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(invalid());
            }
        }
        "pr.reopen" | "pr.review.submit" | "release.metadata.set" | "repository.settings.set" => {
            gwt_github::client::http::validate_pm_settings(&intent.operation, &intent.params)
                .map_err(io::Error::other)?;
        }
        operation if delegated::supports(operation) => delegated::validate(intent)?,
        _ => return Err(invalid()),
    }
    Ok(())
}

pub(super) fn parse(
    operation: &str,
    params: &Value,
) -> Result<PmAuthorizationCommand, CliParseError> {
    if operation == "pm.operation.execute" {
        return delegated::parse(params);
    }
    let parse_error = |error: io::Error| CliParseError::InvalidJson(error.to_string());
    if operation == "pm.approval.request" {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Request {
            operation: String,
            params: Value,
            reason: String,
            rollback: String,
        }
        let p: Request = decode(params).map_err(parse_error)?;
        let intent = OperationIntent {
            operation: p.operation,
            params: p.params,
        };
        validate(&intent).map_err(parse_error)?;
        if p.reason.trim().is_empty() || p.rollback.trim().is_empty() {
            return Err(CliParseError::InvalidJson(
                "reason and rollback must be nonempty".into(),
            ));
        }
        return Ok(PmAuthorizationCommand::Request {
            intent,
            reason: p.reason,
            rollback: p.rollback,
        });
    }
    if operation == "pm.approval.status" {
        if !params.as_object().is_some_and(|params| params.is_empty()) {
            return Err(CliParseError::InvalidJson(
                "pm.approval.status accepts no parameters".into(),
            ));
        }
        return Ok(PmAuthorizationCommand::Status);
    }
    if operation == "pm.approval.revoke" {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Revoke {
            id: String,
        }
        let p: Revoke = decode(params).map_err(parse_error)?;
        return Ok(PmAuthorizationCommand::Revoke { id: p.id });
    }
    let mut effects = params
        .as_object()
        .cloned()
        .ok_or_else(|| CliParseError::InvalidJson("params must be an object".into()))?;
    let mut take_id = |key| -> Result<Option<String>, CliParseError> {
        match effects.remove(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(id)) if !id.trim().is_empty() => Ok(Some(id)),
            _ => Err(CliParseError::InvalidJson(format!(
                "{key} must be a nonempty string"
            ))),
        }
    };
    let authorization = Authorization {
        instruction_id: take_id("instruction_id")?,
        approval_id: take_id("approval_id")?,
    };
    let intent = OperationIntent {
        operation: operation.into(),
        params: Value::Object(effects),
    };
    validate(&intent).map_err(parse_error)?;
    Ok(PmAuthorizationCommand::Execute {
        intent,
        authorization,
    })
}

pub(super) fn run<E: CliEnv>(
    env: &mut E,
    command: PmAuthorizationCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let repo = gwt_core::paths::resolve_current_worktree_root(env.repo_path());
    // Missing identity never authorizes use of an instruction or stored grant.
    // Unmanaged revocation remains available to remove permission if the PM stops.
    if (matches!(&command, PmAuthorizationCommand::Execute { .. })
        || super::pm::ambient_session_id().is_some())
        && !super::pm::caller_is_registered_pm(&repo)
    {
        return Err(super::io_as_api_error(io::Error::new(io::ErrorKind::PermissionDenied,
            "permission_stage=instruction_or_approval: this operation requires the registered repository PM; implementation agents cannot exercise PM authority")));
    }
    let result = match command {
        PmAuthorizationCommand::Request {
            intent,
            reason,
            rollback,
        } => {
            json!({"approval":request_with(&repo,&intent,&reason,&rollback,native_confirm).map_err(super::io_as_api_error)?})
        }
        PmAuthorizationCommand::Status => {
            json!({"permission_stage":"instruction_or_approval","approvals":authority::approvals(&repo).map_err(super::io_as_api_error)?,"audit_path":authority::audit_path(&repo)})
        }
        PmAuthorizationCommand::Revoke { id } => {
            authority::revoke(&repo, &id).map_err(super::io_as_api_error)?;
            json!({"revoked":id})
        }
        PmAuthorizationCommand::Execute {
            intent,
            authorization,
        } => {
            if delegated::supports(&intent.operation) {
                return delegated::run(env, &repo, &intent, &authorization, out);
            }
            execute(env, &repo, &intent, &authorization).map_err(super::io_as_api_error)?
        }
    };
    out.push_str(&serde_json::to_string(&result).map_err(super::serde_as_api_error)?);
    out.push('\n');
    Ok(0)
}

fn request_with(
    repo: &Path,
    intent: &OperationIntent,
    reason: &str,
    rollback: &str,
    confirm: impl FnOnce(&str) -> Result<Option<bool>, String>,
) -> io::Result<authority::Approval> {
    validate(intent)?;
    if reason.trim().is_empty() || rollback.trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "reason and rollback are required",
        ));
    }
    let effect = match intent.operation.as_str() {
        "update.settings.set" => "This preference affects automatic updates for the entire app.",
        "board.config.set" | "discussion.create" => "Board synchronization can publish this project's Board entries to its repository Discussion.",
        "repository.required_checks.update" => "This changes required check contexts on the selected develop/main branch.",
        "pr.auto_merge.enable" => "Enables GitHub auto-merge for this reviewed external fork head. It can merge immediately when GitHub's gates pass. Remembered approval remains fixed to this PR and reviewed SHA.",
        _ => "The existing operation's ownership and safety checks still apply.",
    };
    let approval_scope = if intent.operation == "pr.auto_merge.enable" {
        "Permission applies only to this PR number and reviewed head SHA, including remembered approval. A changed head requires another review and authorization."
    } else {
        "After approval, you can optionally allow future parameters for this same operation in this repository."
    };
    let prompt=format!("Repository: {}\nOperation: {}\nParameters: {}\nWhy: {reason}\nUndo: {rollback}\n\n{effect}\n\n{approval_scope} Revoke with pm.approval.revoke.",repo.display(),intent.operation,intent.params);
    let standing = confirm(&prompt).map_err(io::Error::other)?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            "permission_stage=approval: native approval declined; no permission recorded",
        )
    })?;
    authority::record_approval(repo, intent, reason, rollback, standing)
}

#[cfg(not(test))]
fn native_confirm(prompt: &str) -> Result<Option<bool>, String> {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        let show = |first: &str, second: &str, third: &str, description: &str| {
            rfd::MessageDialog::new()
                .set_title("GWT — PM operation approval")
                .set_description(description)
                .set_level(rfd::MessageLevel::Warning)
                .set_buttons(rfd::MessageButtons::YesNoCancelCustom(
                    first.into(),
                    second.into(),
                    third.into(),
                ))
                .show()
        };
        if !matches!(show("Cancel", "Approve once", "Close", prompt),rfd::MessageDialogResult::Custom(label)if label=="Approve once")
        {
            return Ok(None);
        }
        // The first dialog defaults to decline. After explicit consent, the
        // final default keeps it one-time; Escape still withdraws consent.
        let remember=format!("{prompt}\n\nChoose one-time approval, remember permission for this operation kind in this repository, or cancel without recording permission.");
        Ok(final_native_decision(show(
            "Approve once only",
            "Remember permission",
            "Cancel",
            &remember,
        )))
    }
    #[cfg(target_os = "linux")]
    {
        use gtk::prelude::*;
        gtk::init().map_err(|_| "Native approval requires an available desktop display")?;
        let dialog = gtk::MessageDialog::builder()
            .title("GWT — PM operation approval")
            .text("Approve this PM operation?")
            .secondary_text(prompt)
            .message_type(gtk::MessageType::Warning)
            .modal(true)
            .build();
        dialog.add_button("Cancel", gtk::ResponseType::Cancel);
        dialog.add_button("Approve once", gtk::ResponseType::Accept);
        dialog.add_button("Remember permission", gtk::ResponseType::Other(1));
        dialog.set_default_response(gtk::ResponseType::Cancel);
        let response = dialog.run();
        dialog.close();
        Ok(match response {
            gtk::ResponseType::Accept => Some(false),
            gtk::ResponseType::Other(1) => Some(true),
            _ => None,
        })
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = prompt;
        Err("Native PM approval is unavailable on this platform".into())
    }
}
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn final_native_decision(result: rfd::MessageDialogResult) -> Option<bool> {
    match result {
        rfd::MessageDialogResult::Custom(label) if label == "Approve once only" => Some(false),
        rfd::MessageDialogResult::Custom(label) if label == "Remember permission" => Some(true),
        _ => None,
    }
}
#[cfg(test)]
fn native_confirm(_prompt: &str) -> Result<Option<bool>, String> {
    panic!("tests must inject a native decision")
}

fn read_board(repo: &Path) -> io::Result<gwt_config::ProjectBoardConfig> {
    let path = gwt_core::paths::gwt_repo_local_work_dir(repo)
        .join(gwt_config::board_config::PROJECT_BOARD_FILE);
    match std::fs::read_to_string(path) {
        Ok(raw) => toml::from_str(&raw).map_err(io::Error::other),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Default::default()),
        Err(error) => Err(error),
    }
}
fn board_snapshot(config: &gwt_config::ProjectBoardConfig) -> Value {
    json!({"provider":config.provider.unwrap_or_else(crate::board_provider::current_kind).as_str(),"github_discussion":config.github_discussion})
}

fn require_local_board(
    config: &gwt_config::ProjectBoardConfig,
    switch_to_local: bool,
) -> io::Result<()> {
    if !switch_to_local
        && config
            .provider
            .unwrap_or_else(crate::board_provider::current_kind)
            != gwt_config::board_config::BoardProviderKind::Local
    {
        return Err(io::Error::new(io::ErrorKind::InvalidInput,
            "GitHub Discussion sync requires Local Board provider; retry discussion.create with sync_board:true,board_provider:local or board.config.set with provider:local under matching scoped authority"));
    }
    Ok(())
}

fn bind_board(
    repo: &Path,
    owner: String,
    name: String,
    number: u64,
    category: Option<String>,
    switch_to_local: bool,
) -> io::Result<Value> {
    let work_dir = gwt_core::paths::gwt_repo_local_work_dir(repo);
    let lock_path = gwt_core::paths::gwt_project_dir_for_repo_path(repo)
        .join("project-state/board-config.lock");
    std::fs::create_dir_all(lock_path.parent().expect("Board lock parent"))?;
    std::fs::create_dir_all(&work_dir)?;
    let _lock = gwt_core::operation_deadline::NamedFileLock::acquire(&lock_path, "Board binding")?;
    let mut config = read_board(repo)?;
    require_local_board(&config, switch_to_local)?;
    if switch_to_local {
        config.provider = Some(gwt_config::board_config::BoardProviderKind::Local);
    }
    config.github_discussion = Some(gwt_config::board_config::GitHubDiscussionConfig {
        owner,
        repo: name,
        number,
        category: category.unwrap_or_else(|| "General".into()),
    });
    let path = work_dir.join(gwt_config::board_config::PROJECT_BOARD_FILE);
    gwt_github::cache::write_atomic(
        &path,
        toml::to_string_pretty(&config)
            .map_err(io::Error::other)?
            .as_bytes(),
    )?;
    Ok(board_snapshot(&config))
}

fn checks_projection(before: &Value) -> io::Result<Value> {
    let invalid = || io::Error::other("required status check snapshot is incomplete");
    let strict = before
        .get("strict")
        .and_then(Value::as_bool)
        .ok_or_else(invalid)?;
    let checks = before
        .get("checks")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    let checks = {
        checks
            .iter()
            .map(|check| {
                let context = check
                    .get("context")
                    .and_then(Value::as_str)
                    .filter(|name| !name.is_empty())
                    .ok_or_else(invalid)?;
                let app_id = match check.get("app_id") {
                    None => return Err(invalid()),
                    Some(Value::Null) => -1,
                    Some(value) => value.as_i64().filter(|id| *id >= -1).ok_or_else(invalid)?,
                };
                Ok(json!({"context":context,"app_id":app_id}))
            })
            .collect::<io::Result<Vec<_>>>()?
    };
    Ok(json!({"strict":strict,"checks":checks}))
}

fn validate_checks_response(desired: &Value, after: &Value) -> io::Result<()> {
    if after.get("strict") != desired.get("strict") {
        return Err(io::Error::other("required check update returned a different strict policy; inspect repository protection before retrying"));
    }
    let actual = checks_projection(after)?;
    let expected = desired["checks"]
        .as_array()
        .expect("validated desired checks");
    let actual = actual["checks"]
        .as_array()
        .expect("validated response checks");
    if expected.len() != actual.len()
        || expected.iter().any(|check| {
            let matches: Vec<_> = actual
                .iter()
                .filter(|found| found["context"] == check["context"])
                .collect();
            matches.len() != 1
                || check
                    .get("app_id")
                    .is_some_and(|id| matches[0].get("app_id") != Some(id))
        })
    {
        return Err(io::Error::other("required check update returned different contexts or app bindings; inspect repository protection before retrying"));
    }
    Ok(())
}

fn reviewed_external_pr<E: CliEnv>(env: &mut E, p: &AutoMergeInput) -> io::Result<Value> {
    let snapshot = env.fetch_auto_merge_pr(p.number)?;
    let pr = &snapshot.inventory;
    let refuse = |reason| {
        io::Error::new(io::ErrorKind::PermissionDenied, format!(
        "external fork auto-merge refused: {reason}; review the current PR before retrying. If this scoped route is unavailable, human merge required: repository automation may exclude external forks"
    ))
    };
    if pr.number != p.number
        || snapshot.state != gwt_git::pr_status::PrState::Open
        || pr.is_draft
        || pr.is_cross_repository != Some(true)
    {
        return Err(refuse("a current OPEN, Ready external fork PR is required"));
    }
    if snapshot.head_sha != p.reviewed_head_sha
        || pr.head_sha.as_deref() != Some(p.reviewed_head_sha.as_str())
    {
        return Err(refuse(
            "the PR head differs from the authorized reviewed SHA",
        ));
    }
    let merge_ready = pr.mergeable.eq_ignore_ascii_case("MERGEABLE")
        && (pr.merge_state_status.eq_ignore_ascii_case("CLEAN")
            || (pr.merge_state_status.eq_ignore_ascii_case("BEHIND")
                && pr.required_status_checks_strict == Some(false)));
    if !merge_ready
        || !pr.ci_status.eq_ignore_ascii_case("SUCCESS")
        || !pr.check_counts.is_some_and(|counts| {
            counts.success > 0 && counts.failure == 0 && counts.in_progress == 0
        })
    {
        return Err(refuse(
            "current mergeability and passing CI must be measured",
        ));
    }
    if !pr.review_status.eq_ignore_ascii_case("APPROVED") || pr.unresolved_review_threads != Some(0)
    {
        return Err(refuse(
            "current approval and zero unresolved review threads must be measured",
        ));
    }
    Ok(
        json!({"number":p.number,"reviewed_head_sha":snapshot.head_sha,"is_cross_repository":true,
        "state":"OPEN","ci_status":pr.ci_status,"review_status":pr.review_status,
        "auto_merge_enabled":pr.auto_merge_enabled}),
    )
}

fn execute<E: CliEnv>(
    env: &mut E,
    repo: &Path,
    intent: &OperationIntent,
    authorization: &Authorization,
) -> io::Result<Value> {
    validate(intent)?;
    // Read-only preflight precedes one-off consumption. No request may select
    // another repository; production env verifies the project's origin identity.
    let coordinates = if intent.operation == "update.settings.set" {
        None
    } else {
        Some(env.github_repository()?)
    };
    let mut before = match intent.operation.as_str() {
        "update.settings.set" => {
            serde_json::to_value(crate::update_settings::status(repo)?).map_err(io::Error::other)?
        }
        "repository.required_checks.update" => {
            let p: ChecksInput = decode(&intent.params)?;
            checks_projection(&env.required_status_checks(&p.branch)?)?
        }
        "pr.auto_merge.enable" => reviewed_external_pr(env, &decode(&intent.params)?)?,
        "pr.reopen" | "pr.review.submit" | "release.metadata.set" | "repository.settings.set" => {
            env.pm_settings_snapshot(&intent.operation, &intent.params)?
        }
        "discussion.create" | "board.config.set" => {
            let config = read_board(repo)?;
            if intent.operation == "board.config.set" {
                require_local_board(
                    &config,
                    decode::<BindingInput>(&intent.params)?.provider.is_some(),
                )?;
            } else {
                let p: DiscussionInput = decode(&intent.params)?;
                if p.sync_board {
                    require_local_board(&config, p.board_provider.is_some())?;
                }
            }
            board_snapshot(&config)
        }
        _ => unreachable!(),
    };
    if intent.operation == "pr.auto_merge.enable" {
        let (owner, name) = coordinates.as_ref().expect("repo preflight");
        before["repository"] = format!("{owner}/{name}").into();
    }
    let attempt = authority::authorize(
        repo,
        intent,
        authorization,
        super::pm::ambient_session_id().as_deref(),
        before.clone(),
    )?;
    let result = (|| match intent.operation.as_str() {
        "update.settings.set" => {
            let p: UpdateInput = decode(&intent.params)?;
            crate::update_settings::set(Some(p.auto_apply_updates))?;
            serde_json::to_value(crate::update_settings::status(repo)?).map_err(io::Error::other)
        }
        "board.config.set" => {
            let p: BindingInput = decode(&intent.params)?;
            let (owner, name) = coordinates.clone().expect("repo preflight");
            bind_board(
                repo,
                owner,
                name,
                p.github_discussion,
                p.category,
                p.provider.is_some(),
            )
        }
        "discussion.create" => {
            let p: DiscussionInput = decode(&intent.params)?;
            let discussion = env.create_github_discussion(&p.category, &p.title, &p.body)?;
            let identity =
                json!({"id":discussion.id,"number":discussion.number,"url":discussion.url});
            let binding = if p.sync_board {
                let (owner, name) = coordinates.clone().expect("repo preflight");
                match bind_board(
                    repo,
                    owner,
                    name,
                    discussion.number,
                    Some(p.category),
                    p.board_provider.is_some(),
                ) {
                    Ok(binding) => json!({"status":"bound","binding":binding}),
                    Err(error) => {
                        json!({"status":"failed","reason":error.to_string(),"recovery_operation":"board.config.set","github_discussion":discussion.number})
                    }
                }
            } else {
                json!({"status":"unchanged"})
            };
            Ok(json!({"discussion":identity,"board_binding":binding}))
        }
        "repository.required_checks.update" => {
            let p: ChecksInput = decode(&intent.params)?;
            let mut desired = before.clone();
            let checks = desired["checks"].as_array_mut().expect("validated checks");
            checks.retain(|check| {
                !p.remove
                    .iter()
                    .any(|name| check["context"].as_str() == Some(name))
            });
            for name in &p.add {
                if !checks
                    .iter()
                    .any(|check| check["context"].as_str() == Some(name))
                {
                    checks.push(json!({"context":name}));
                }
            }
            let after =
                env.update_required_status_checks(&p.branch, &json!({"checks":desired["checks"]}))?;
            validate_checks_response(&desired, &after)?;
            Ok(after)
        }
        "pr.auto_merge.enable" => {
            let p: AutoMergeInput = decode(&intent.params)?;
            env.enable_pr_auto_merge(p.number, &p.reviewed_head_sha)
        }
        "pr.reopen" | "pr.review.submit" | "release.metadata.set" | "repository.settings.set" => {
            env.pm_settings_mutate(&intent.operation, &intent.params, &before)
        }
        _ => unreachable!(),
    })();
    match result {
        Ok(after) => {
            attempt.finish(repo,true,after.clone()).map_err(|error|io::Error::other(format!("operation succeeded but final audit failed: {error}; inspect status before retrying")))?;
            Ok(
                json!({"attempt_id":attempt.id,"before":before,"after":after,"audit_path":authority::audit_path(repo)}),
            )
        }
        Err(error) => {
            attempt.finish(repo, false, json!({"error":error.to_string()}))?;
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use gwt_core::{
        pm_authorization::{self, OperationIntent},
        test_support::{ScopedEnvVar, ScopedGwtHome},
    };
    use serde_json::{json, Value};

    fn registered_pm(repo: &std::path::Path) -> ScopedEnvVar {
        use crate::pm_registry::{self, PmPrefs, PmRegistration};
        pm_registry::save_pm_prefs(
            &pm_registry::pm_prefs_path_for_repo_path(repo),
            &PmPrefs {
                registration: Some(PmRegistration {
                    session_id: "pm-settings-test".into(),
                    agent_id: "codex".into(),
                    worktree_path: repo.display().to_string(),
                    created_at: None,
                    consecutive_crashes: 0,
                    next_not_before: None,
                }),
                ..PmPrefs::default()
            },
        )
        .unwrap();
        ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "pm-settings-test")
    }

    #[test]
    fn auto_merge_enable_accepts_only_an_explicit_pr_and_reviewed_sha() {
        let params = json!({"number":17,"reviewed_head_sha":"a".repeat(40)});
        assert!(super::parse("pr.auto_merge.enable", &params).is_ok());
        for invalid in [
            json!({"number":0,"reviewed_head_sha":"a".repeat(40)}),
            json!({"number":17,"reviewed_head_sha":""}),
            json!({"number":17,"reviewed_head_sha":"HEAD"}),
            json!({"number":17,"reviewed_head_sha":"z".repeat(40)}),
            json!({"number":17,"reviewed_head_sha":"a".repeat(40),"admin":true}),
            json!({"number":17,"reviewed_head_sha":"a".repeat(40),"repo":"other/repo"}),
        ] {
            assert!(
                super::parse("pr.auto_merge.enable", &invalid).is_err(),
                "{invalid}"
            );
        }
    }

    #[test]
    fn external_auto_merge_requires_fresh_gates_and_audits_exact_permission() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let repo = home.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let _session = registered_pm(&repo);
        let sha = "a".repeat(40);
        let intent = OperationIntent {
            operation: "pr.auto_merge.enable".into(),
            params: json!({"number":17,"reviewed_head_sha":sha}),
        };
        let mut inventory = gwt_git::pr_status::parse_pr_inventory_json(
            &json!([{
                "number":17,"state":"OPEN","isDraft":false,"isCrossRepository":true,
                "headRefName":"contribution","headRefOid":sha,"mergeable":"MERGEABLE",
                "mergeStateStatus":"CLEAN","reviewDecision":"APPROVED",
                "statusCheckRollup":[{"status":"COMPLETED","conclusion":"SUCCESS"}],
            }])
            .to_string(),
            chrono::Utc::now(),
        )
        .unwrap()
        .remove(0);
        inventory.unresolved_review_threads = Some(0);
        let snapshot = gwt_git::pr_status::PrCompletionSnapshot {
            state: gwt_git::pr_status::PrState::Open,
            head_sha: sha.clone(),
            inventory,
        };
        let mut env = crate::cli::TestEnv::new(repo.clone());
        env.completion_prs.insert(17, snapshot.clone());
        let envelope = |params| {
            json!({"schema_version":1,"operation":intent.operation,"params":params}).to_string()
        };
        env.stdin = envelope(intent.params.clone());
        assert_ne!(
            crate::cli::json_envelope::dispatch(&mut env, "gwtd"),
            0,
            "no authority"
        );
        assert!(env.pr_auto_merge_call_log.is_empty());
        let approval = pm_authorization::record_approval(
            &repo,
            &intent,
            "Reviewed external PR",
            "Disable auto-merge before merge",
            false,
        )
        .unwrap();
        let mut params = intent.params.clone();
        params["approval_id"] = approval.id.clone().into();
        for blocker in [
            "head_changed",
            "draft",
            "same_repository",
            "ci_unknown",
            "checks_unknown",
            "review_unknown",
            "review_threads_unknown",
        ] {
            let mut invalid = snapshot.clone();
            match blocker {
                "head_changed" => invalid.head_sha = "b".repeat(40),
                "draft" => invalid.inventory.is_draft = true,
                "same_repository" => invalid.inventory.is_cross_repository = Some(false),
                "ci_unknown" => invalid.inventory.ci_status = "UNKNOWN".into(),
                "checks_unknown" => invalid.inventory.check_counts = None,
                "review_unknown" => invalid.inventory.review_status = "UNKNOWN".into(),
                "review_threads_unknown" => invalid.inventory.unresolved_review_threads = None,
                _ => unreachable!(),
            }
            env.completion_prs.insert(17, invalid);
            env.stdin = envelope(params.clone());
            assert_ne!(
                crate::cli::json_envelope::dispatch(&mut env, "gwtd"),
                0,
                "{blocker}"
            );
            assert!(env.pr_auto_merge_call_log.is_empty(), "{blocker}");
        }
        assert!(!pm_authorization::approvals(&repo).unwrap()[0].consumed);
        env.completion_prs.insert(17, snapshot);
        env.stdin = envelope(params);
        env.stdout.clear();
        env.stderr.clear();
        assert_eq!(
            crate::cli::json_envelope::dispatch(&mut env, "gwtd"),
            0,
            "{}",
            String::from_utf8_lossy(&env.stderr)
        );
        assert_eq!(env.pr_auto_merge_call_log, vec![(17, sha.clone())]);
        let audit = std::fs::read_to_string(pm_authorization::audit_path(&repo)).unwrap();
        assert!(
            audit.contains(&sha) && audit.contains(&approval.id) && audit.contains("succeeded")
        );
        let reply: Value = serde_json::from_slice(&env.stdout).unwrap();
        let output: Value = serde_json::from_str(reply["output"].as_str().unwrap()).unwrap();
        assert_eq!(output["before"]["number"], 17);
        assert_eq!(output["before"]["repository"], "owner/repo");
        assert_eq!(output["after"]["reviewed_head_sha"], sha);
        assert_eq!(output["after"]["auto_merge_enabled"], true);
    }

    #[test]
    fn standing_permission_requires_a_registered_executor_but_can_be_revoked_without_a_pm() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let repo = home.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let _pm = registered_pm(&repo);
        let intent = OperationIntent {
            operation: "update.settings.set".into(),
            params: json!({"auto_apply_updates":true}),
        };
        let approval = pm_authorization::record_approval(
            &repo,
            &intent,
            "Enable updates",
            "Disable updates",
            true,
        )
        .unwrap();
        let audit_path = pm_authorization::audit_path(&repo);
        let audit_before = std::fs::read(&audit_path).ok();
        let approvals_before =
            serde_json::to_value(pm_authorization::approvals(&repo).unwrap()).unwrap();
        for caller in [None, Some("  "), Some("implementation-agent")] {
            let _session = match caller {
                Some(id) => ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, id),
                None => ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV),
            };
            let mut env = crate::cli::TestEnv::new(repo.clone());
            env.stdin = json!({"operation":intent.operation,"params":intent.params}).to_string();
            assert_ne!(
                crate::cli::json_envelope::dispatch(&mut env, "gwtd"),
                0,
                "{caller:?}"
            );
            assert!(String::from_utf8_lossy(&env.stderr).contains("registered repository PM"));
            assert!(!gwt_core::paths::gwt_config_path().exists());
            assert_eq!(
                serde_json::to_value(pm_authorization::approvals(&repo).unwrap()).unwrap(),
                approvals_before
            );
            assert_eq!(std::fs::read(&audit_path).ok(), audit_before);
        }
        // Removing authority remains available when no PM is running.
        let _session = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
        let mut env = crate::cli::TestEnv::new(repo.clone());
        env.stdin =
            json!({"operation":"pm.approval.revoke","params":{"id":approval.id}}).to_string();
        assert_eq!(crate::cli::json_envelope::dispatch(&mut env, "gwtd"), 0);
        assert!(pm_authorization::approvals(&repo).unwrap()[0].revoked);
        assert!(!gwt_core::paths::gwt_config_path().exists());
    }

    #[test]
    fn additional_repository_operations_accept_only_bounded_effect_parameters() {
        for (operation, params) in [
            ("pr.reopen", json!({"number":17})),
            (
                "pr.review.submit",
                json!({"number":17,"event":"COMMENT","body":"Review notes"}),
            ),
            (
                "release.metadata.set",
                json!({"tag":"v1.0.0","body":"Corrected notes"}),
            ),
            (
                "repository.settings.set",
                json!({"description":"Coordination","has_discussions":true}),
            ),
        ] {
            assert!(super::parse(operation, &params).is_ok(), "{operation}");
            let mut foreign = params.clone();
            foreign["owner"] = "elsewhere".into();
            assert!(super::parse(operation, &foreign).is_err());
        }
        for (operation, params) in [
            ("pr.reopen", json!({"number":0})),
            (
                "pr.review.submit",
                json!({"number":17,"event":"REQUEST_CHANGES"}),
            ),
            (
                "release.metadata.set",
                json!({"tag":"v1.0.0","draft":false}),
            ),
            ("repository.settings.set", json!({"visibility":"public"})),
            (
                "repository.settings.set",
                json!({"required_status_checks":{}}),
            ),
        ] {
            assert!(super::parse(operation, &params).is_err(), "{operation}");
        }
    }

    #[test]
    fn required_checks_reject_an_incomplete_snapshot_instead_of_losing_app_bindings() {
        for snapshot in [
            json!({"strict":false,"contexts":["Rust"],"checks":"malformed"}),
            json!({"strict":false,"contexts":["Rust"]}),
            json!({"strict":false,"checks":[{"context":"Rust"}]}),
        ] {
            assert!(super::checks_projection(&snapshot).is_err());
        }
    }

    #[test]
    fn required_checks_verify_the_returned_contexts_and_retained_app_id() {
        let desired =
            json!({"strict":false,"checks":[{"context":"Rust","app_id":7},{"context":"Frontend"}]});
        for after in [
            json!({"strict":false,"checks":[{"context":"Rust","app_id":null},{"context":"Frontend","app_id":8}]}),
            json!({"strict":false,"checks":[{"context":"Rust","app_id":7}]}),
        ] {
            assert!(super::validate_checks_response(&desired, &after).is_err());
        }
        assert!(super::validate_checks_response(&desired, &json!({"strict":false,"checks":[{"context":"Frontend","app_id":8},{"context":"Rust","app_id":7}]})).is_ok());
    }

    #[test]
    fn board_sync_requires_local_provider_before_creating_a_discussion() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let repo = home.path().join("repo");
        let work = gwt_core::paths::gwt_repo_local_work_dir(&repo);
        std::fs::create_dir_all(&work).unwrap();
        let _session = registered_pm(&repo);
        std::fs::write(
            work.join("board.toml"),
            "provider = 'teams'\nchannel = 'existing'\n",
        )
        .unwrap();
        let mut env = crate::cli::TestEnv::new(repo.clone());
        let mut intent = OperationIntent {
            operation: "discussion.create".into(),
            params: json!({"title":"Coordination","body":"Board updates","category":"General","sync_board":true}),
        };
        let approval =
            pm_authorization::record_approval(&repo, &intent, "Create Board", "Unbind", false)
                .unwrap();
        env.stdin = json!({"operation":intent.operation,"params":{
            "title":"Coordination","body":"Board updates","category":"General","sync_board":true,"approval_id":approval.id
        }}).to_string();
        assert_ne!(crate::cli::json_envelope::dispatch(&mut env, "gwtd"), 0);
        assert!(env.discussion_create_call_log.is_empty());
        intent.params["board_provider"] = "local".into();
        let approval = pm_authorization::record_approval(
            &repo,
            &intent,
            "Switch project Board to Local",
            "Restore Teams",
            false,
        )
        .unwrap();
        let mut params = intent.params.clone();
        params["approval_id"] = approval.id.into();
        env.stdin = json!({"operation":intent.operation,"params":params}).to_string();
        assert_eq!(
            crate::cli::json_envelope::dispatch(&mut env, "gwtd"),
            0,
            "{}",
            String::from_utf8_lossy(&env.stderr)
        );
        let config = gwt_config::ProjectBoardConfig::load_from_work_dir(&work);
        assert_eq!(
            config.provider,
            Some(gwt_config::board_config::BoardProviderKind::Local)
        );
        assert_eq!(config.channel.as_deref(), Some("existing"));
        assert_eq!(config.github_discussion.unwrap().number, 17);
        assert_eq!(env.discussion_create_call_log.len(), 1);
    }

    #[test]
    fn board_topics_can_be_enabled_with_category_without_creating_a_discussion() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let repo = home.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let _session = registered_pm(&repo);
        let mut env = crate::cli::TestEnv::new(repo.clone());
        let intent = OperationIntent {
            operation: "board.config.set".into(),
            params: json!({"category":"Announcements"}),
        };
        let approval = pm_authorization::record_approval(
            &repo,
            &intent,
            "Enable Board topics",
            "Remove opt-in",
            false,
        )
        .unwrap();
        let mut params = intent.params.clone();
        params["approval_id"] = approval.id.into();
        env.stdin = json!({"operation":intent.operation,"params":params}).to_string();
        assert_eq!(
            crate::cli::json_envelope::dispatch(&mut env, "gwtd"),
            0,
            "{}",
            String::from_utf8_lossy(&env.stderr)
        );
        let config = gwt_config::ProjectBoardConfig::load_from_work_dir(
            &gwt_core::paths::gwt_repo_local_work_dir(&repo),
        );
        let serialized = serde_json::to_value(config.github_discussion.unwrap()).unwrap();
        assert_eq!(serialized["category"], "Announcements");
        assert!(env.discussion_create_call_log.is_empty());
    }

    #[test]
    fn discussion_operation_returns_number_and_binds_board_without_another_creation() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let repo = home.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let _session = registered_pm(&repo);
        let mut env = crate::cli::TestEnv::new(repo.clone());
        let intent = OperationIntent {
            operation: "discussion.create".into(),
            params: json!({"title":"Coordination","body":"Board updates","category":"General","sync_board":true}),
        };
        let approval = pm_authorization::record_approval(
            &repo,
            &intent,
            "Create project Board",
            "Unbind Board",
            false,
        )
        .unwrap();
        let mut params = intent.params.clone();
        params["approval_id"] = approval.id.into();
        env.stdin = json!({"operation":intent.operation,"params":params}).to_string();
        assert_eq!(
            crate::cli::json_envelope::dispatch(&mut env, "gwtd"),
            0,
            "{}",
            String::from_utf8_lossy(&env.stderr)
        );
        assert_eq!(env.discussion_create_call_log.len(), 1);
        let config = gwt_config::ProjectBoardConfig::load_from_work_dir(
            &gwt_core::paths::gwt_repo_local_work_dir(&repo),
        );
        let binding = config.github_discussion.unwrap();
        assert_eq!(
            (
                binding.owner.as_str(),
                binding.repo.as_str(),
                binding.number
            ),
            ("owner", "repo", 17)
        );
        let reply: Value = serde_json::from_slice(&env.stdout).unwrap();
        let output: Value = serde_json::from_str(reply["output"].as_str().unwrap()).unwrap();
        assert_eq!(output["after"]["discussion"]["number"], 17);
    }

    #[test]
    fn required_checks_operation_preserves_strict_and_app_bindings() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let repo = home.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let _session = registered_pm(&repo);
        let mut env = crate::cli::TestEnv::new(repo.clone());
        env.required_checks = json!({"strict":false,"checks":[{"context":"Rust","app_id":7},{"context":"Old","app_id":null}]});
        let intent = OperationIntent {
            operation: "repository.required_checks.update".into(),
            params: json!({"branch":"develop","add":["Test (Frontend + WebView E2E)"],"remove":["Old"]}),
        };
        let approval = pm_authorization::record_approval(
            &repo,
            &intent,
            "Require frontend check",
            "Remove new context",
            false,
        )
        .unwrap();
        let mut params = intent.params.clone();
        params["approval_id"] = approval.id.into();
        env.stdin = json!({"operation":intent.operation,"params":params}).to_string();
        assert_eq!(
            crate::cli::json_envelope::dispatch(&mut env, "gwtd"),
            0,
            "{}",
            String::from_utf8_lossy(&env.stderr)
        );
        assert_eq!(env.required_checks_updates.len(), 1);
        assert!(
            env.required_checks_updates[0].1.get("strict").is_none(),
            "contexts authority must not write the strict policy"
        );
        assert_eq!(env.required_checks["strict"], false);
        assert_eq!(
            env.required_checks["checks"][0],
            json!({"context":"Rust","app_id":7})
        );
        assert_eq!(
            env.required_checks["checks"][1]["context"],
            "Test (Frontend + WebView E2E)"
        );
        let audit = std::fs::read_to_string(pm_authorization::audit_path(&repo)).unwrap();
        assert!(audit.contains("Old") && audit.contains("Frontend"));
        env.stdin =
            json!({"operation":intent.operation,"params":{"branch":"feature","add":["Other"]}})
                .to_string();
        assert_ne!(crate::cli::json_envelope::dispatch(&mut env, "gwtd"), 0);
        assert_eq!(env.required_checks_updates.len(), 1);
    }

    #[test]
    fn approval_request_records_only_an_explicit_native_decision() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        let intent = OperationIntent {
            operation: "update.settings.set".into(),
            params: json!({"auto_apply_updates":false}),
        };
        assert!(super::request_with(
            &repo,
            &intent,
            "Disable updates",
            "Enable updates",
            |prompt| {
                assert!(prompt.contains("Disable updates") && prompt.contains("Enable updates"));
                Ok(None)
            }
        )
        .is_err());
        assert!(pm_authorization::approvals(&repo).unwrap().is_empty());
        let approval =
            super::request_with(&repo, &intent, "Disable updates", "Enable updates", |_| {
                Ok(Some(true))
            })
            .unwrap();
        assert!(approval.standing);
        assert_eq!(
            pm_authorization::approvals(&repo).unwrap()[0].id,
            approval.id
        );
    }

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[test]
    fn final_native_cancellation_records_no_authority() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        let intent = OperationIntent {
            operation: "update.settings.set".into(),
            params: json!({"auto_apply_updates":false}),
        };
        for result in [
            rfd::MessageDialogResult::Custom("Cancel".into()),
            rfd::MessageDialogResult::Cancel,
        ] {
            assert!(super::request_with(
                &repo,
                &intent,
                "Disable updates",
                "Enable updates",
                |_| Ok(super::final_native_decision(result)),
            )
            .is_err());
            assert!(pm_authorization::approvals(&repo).unwrap().is_empty());
        }
        for (label, standing) in [("Approve once only", false), ("Remember permission", true)] {
            let approval =
                super::request_with(&repo, &intent, "Disable updates", "Enable updates", |_| {
                    Ok(super::final_native_decision(
                        rfd::MessageDialogResult::Custom(label.into()),
                    ))
                })
                .unwrap();
            assert_eq!(approval.standing, standing);
            assert_eq!(
                pm_authorization::approvals(&repo)
                    .unwrap()
                    .last()
                    .unwrap()
                    .id,
                approval.id
            );
        }
    }

    #[test]
    fn update_setting_operation_requires_authority_and_writes_the_system_preference() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let repo = home.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let _session = registered_pm(&repo);
        let intent = OperationIntent {
            operation: "update.settings.set".into(),
            params: json!({"auto_apply_updates":true}),
        };
        let mut env = crate::cli::TestEnv::new(repo.clone());
        let envelope = |params| {
            json!({"schema_version":1,"operation":"update.settings.set","params":params})
                .to_string()
        };
        env.stdin = envelope(intent.params.clone());
        assert_eq!(crate::cli::json_envelope::dispatch(&mut env, "gwtd"), 1);
        assert!(!gwt_core::paths::gwt_config_path().exists());
        let approval = pm_authorization::record_approval(
            &repo,
            &intent,
            "Enable updates",
            "Disable updates",
            false,
        )
        .unwrap();
        env.stdin = envelope(json!({"auto_apply_updates":true,"approval_id":approval.id}));
        env.stdout.clear();
        env.stderr.clear();
        assert_eq!(
            crate::cli::json_envelope::dispatch(&mut env, "gwtd"),
            0,
            "{}",
            String::from_utf8_lossy(&env.stderr)
        );
        assert_eq!(crate::update_settings::configured().unwrap(), Some(true));
        let reply: Value = serde_json::from_slice(&env.stdout).unwrap();
        let output: Value = serde_json::from_str(reply["output"].as_str().unwrap()).unwrap();
        assert_eq!(output["after"]["auto_apply_updates_effective"], true);
        let audit = std::fs::read_to_string(pm_authorization::audit_path(&repo)).unwrap();
        assert!(audit.contains("succeeded") && audit.contains(&approval.id));
    }
}
