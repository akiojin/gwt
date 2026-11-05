//! Repository-scoped PM instructions, explicit native approvals and mutation audits.
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// No generic command, credential, arbitrary-repository or destructive API grant.
pub const OPERATIONS: &[&str] = &[
    "discussion.create",
    "board.config.set",
    "repository.required_checks.update",
    "update.settings.set",
    "actions.release_policy.add",
    "actions.release_policy.rollback",
    "actions.diagnostic.policy.add",
    "actions.diagnostic.policy.rollback",
    "pr.close",
    "worktree.gc_build_artifacts",
    "pm.message.send",
    "pr.reopen",
    "pr.review.submit",
    "pr.auto_merge.enable",
    "release.metadata.set",
    "repository.settings.set",
    "project.execution.choose",
];

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OperationIntent {
    pub operation: String,
    pub params: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct OperationScope {
    pub operation: String,
    pub params_sha256: String,
}

impl OperationIntent {
    pub fn scope(&self) -> io::Result<OperationScope> {
        if !OPERATIONS.contains(&self.operation.as_str()) || !self.params.is_object() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unsupported authorization scope; use a named PM operation and object params",
            ));
        }
        if self.params.as_object().is_some_and(|params| {
            params.keys().any(|key| {
                matches!(
                    key.as_str(),
                    "instruction_id" | "approval_id" | "project_root" | "repo" | "owner"
                )
            })
        }) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "authorization scope must contain effect parameters only; repository is fixed",
            ));
        }
        Ok(OperationScope {
            operation: self.operation.clone(),
            params_sha256: digest(&serde_json::to_vec(&canonical_value(&self.params))?),
        })
    }
}

fn canonical_value(value: &Value) -> Value {
    match value {
        Value::Object(fields) => {
            let mut entries: Vec<_> = fields.iter().collect();
            entries.sort_unstable_by(|left, right| left.0.cmp(right.0));
            Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key.clone(), canonical_value(value)))
                    .collect(),
            )
        }
        Value::Array(values) => Value::Array(values.iter().map(canonical_value).collect()),
        other => other.clone(),
    }
}

pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Authorization {
    pub instruction_id: Option<String>,
    pub approval_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Approval {
    pub id: String,
    pub scope: OperationScope,
    pub reason: String,
    pub rollback: String,
    pub standing: bool,
    pub revoked: bool,
    pub consumed: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Default, Serialize, Deserialize)]
struct Store {
    approvals: Vec<Approval>,
    consumed_instructions: Vec<String>,
    #[serde(default)]
    project_choice_attempts: Vec<Attempt>,
}

fn store_path(repo: &Path) -> PathBuf {
    crate::paths::gwt_project_dir_for_repo_path(repo).join("project-state/pm-authorizations.json")
}

pub fn audit_path(repo: &Path) -> PathBuf {
    store_path(repo).with_file_name("pm-operation-audit.jsonl")
}

fn load(repo: &Path) -> io::Result<Store> {
    match fs::read(store_path(repo)) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::other),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Store::default()),
        Err(error) => Err(error),
    }
}

fn locked<T>(repo: &Path, action: impl FnOnce(&mut Store) -> io::Result<T>) -> io::Result<T> {
    let path = store_path(repo);
    fs::create_dir_all(path.parent().expect("authorization parent"))?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path.with_extension("lock"))?;
    lock.lock_exclusive()?;
    let mut store = load(repo)?;
    let result = action(&mut store)?;
    crate::workspace_projection::write_atomic(&path, &serde_json::to_vec(&store)?)
        .map_err(io::Error::other)?;
    Ok(result)
}

fn audit(repo: &Path, value: Value) -> io::Result<()> {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(audit_path(repo))?;
    let mut bytes = serde_json::to_vec(&crate::user_instruction::redact_json(value))?;
    bytes.push(b'\n');
    file.write_all(&bytes)?;
    file.sync_all()
}

/// Only call after a native human decision; no JSON caller supplies that decision.
pub fn record_approval(
    repo: &Path,
    intent: &OperationIntent,
    reason: &str,
    rollback: &str,
    standing: bool,
) -> io::Result<Approval> {
    let scope = intent.scope()?;
    if reason.trim().is_empty() || rollback.trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "reason and rollback are required",
        ));
    }
    let approval = Approval {
        id: uuid::Uuid::new_v4().to_string(),
        scope,
        reason: crate::user_instruction::redact(reason),
        rollback: crate::user_instruction::redact(rollback),
        standing,
        revoked: false,
        consumed: false,
        created_at: chrono::Utc::now(),
    };
    locked(repo, |store| {
        audit(
            repo,
            serde_json::json!({"event":"native_approval", "approval":approval}),
        )?;
        store.approvals.push(approval.clone());
        Ok(approval)
    })
}

pub fn approvals(repo: &Path) -> io::Result<Vec<Approval>> {
    Ok(load(repo)?.approvals)
}

pub fn revoke(repo: &Path, id: &str) -> io::Result<()> {
    locked(repo, |store| {
        let approval = store
            .approvals
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "approval not found in this repository",
                )
            })?;
        approval.revoked = true;
        audit(
            repo,
            serde_json::json!({"event":"revoked", "approval_id":id}),
        )
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attempt {
    pub id: String,
    pub scope: OperationScope,
    pub authority_id: String,
    pub permission_stage: String,
    pub caller_session: Option<String>,
    pub before: Value,
}

fn permission_denied(detail: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, format!(
        "permission_stage=instruction_or_approval: {detail}; pass an active scoped instruction_id or approval_id, or use pm.approval.request with operation, params, reason and rollback"))
}

struct Selection {
    id: String,
    stage: &'static str,
    consume_instruction: Option<String>,
    consume_approval: Option<usize>,
}

fn select_authority(
    repo: &Path,
    store: &Store,
    scope: &OperationScope,
    authorization: &Authorization,
    replay: Option<&Attempt>,
) -> io::Result<Selection> {
    if authorization.instruction_id.is_some() && authorization.approval_id.is_some() {
        return Err(permission_denied("choose one authority id"));
    }
    if replay.is_some_and(|attempt| &attempt.scope != scope) {
        return Err(permission_denied("replay differs from the accepted scope"));
    }
    let matches_attempt = |id: &str, stage: &str| {
        replay.is_none_or(|attempt| attempt.authority_id == id && attempt.permission_stage == stage)
    };
    if let Some(id) = &authorization.instruction_id {
        let instructions = crate::user_instruction::load(repo).map_err(io::Error::other)?;
        let instruction = instructions
            .iter()
            .find(|entry| &entry.id == id)
            .filter(|entry| {
                entry.superseded_by.is_none()
                    && entry.authority.is_some()
                    && entry.kind != crate::user_instruction::UserInstructionKind::Question
                    && entry.authorizations.contains(scope)
                    && matches_attempt(id, "instruction")
            })
            .ok_or_else(|| {
                permission_denied(
                    "instruction does not authorize this exact operation and parameters",
                )
            })?;
        let consumption = format!("{id}:{}:{}", scope.operation, scope.params_sha256);
        let one_off = instruction.kind == crate::user_instruction::UserInstructionKind::OneOff;
        if one_off && store.consumed_instructions.contains(&consumption) != replay.is_some() {
            return Err(permission_denied(if replay.is_some() {
                "replay lacks the accepted one-off instruction consumption"
            } else {
                "one-off instruction has already been consumed"
            }));
        }
        return Ok(Selection {
            id: id.clone(),
            stage: "instruction",
            consume_instruction: (one_off && replay.is_none()).then_some(consumption),
            consume_approval: None,
        });
    }
    // These remembered permissions remain fixed to the reviewed head or the
    // exact policy request/CAS instead of granting future, different effects.
    let exact_scope_required = matches!(
        scope.operation.as_str(),
        "pr.auto_merge.enable"
            | "project.execution.choose"
            | "actions.release_policy.add"
            | "actions.release_policy.rollback"
            | "actions.diagnostic.policy.add"
            | "actions.diagnostic.policy.rollback"
    );
    let (index, approval) = store
        .approvals
        .iter()
        .enumerate()
        .find(|(_, entry)| {
            !entry.revoked
                && if replay.is_some() && !entry.standing {
                    entry.consumed
                } else {
                    !entry.consumed
                }
                && entry.scope.operation == scope.operation
                && match &authorization.approval_id {
                    Some(id) => &entry.id == id && (entry.standing || &entry.scope == scope),
                    None => entry.standing,
                }
                && (!exact_scope_required || &entry.scope == scope)
                && matches_attempt(&entry.id, "approval")
        })
        .ok_or_else(|| permission_denied("no applicable human approval"))?;
    Ok(Selection {
        id: approval.id.clone(),
        stage: "approval",
        consume_instruction: None,
        consume_approval: (!approval.standing && replay.is_none()).then_some(index),
    })
}

/// Read-only preflight; fixes an implicitly selected standing grant to its ID.
/// Call `authorize` again at acceptance to recheck and consume that authority.
pub fn check_authorization(
    repo: &Path,
    intent: &OperationIntent,
    authorization: &Authorization,
) -> io::Result<Authorization> {
    let selected = select_authority(repo, &load(repo)?, &intent.scope()?, authorization, None)?;
    Ok(if selected.stage == "instruction" {
        Authorization {
            instruction_id: Some(selected.id),
            approval_id: None,
        }
    } else {
        Authorization {
            instruction_id: None,
            approval_id: Some(selected.id),
        }
    })
}

/// Recheck authority for returning a completed request's recorded result only.
/// The caller authenticates the PM and loads `attempt` from its own completed
/// receipt; an untrusted supplied attempt is never proof of prior acceptance.
/// This does not consume a grant, write an audit, or authorize another effect.
pub fn authorize_replay(
    repo: &Path,
    intent: &OperationIntent,
    authorization: &Authorization,
    attempt: &Attempt,
) -> io::Result<()> {
    select_authority(
        repo,
        &load(repo)?,
        &intent.scope()?,
        authorization,
        Some(attempt),
    )?;
    Ok(())
}

/// Consumes one-off authority and writes the before audit before any effect.
pub fn authorize(
    repo: &Path,
    intent: &OperationIntent,
    authorization: &Authorization,
    caller: Option<&str>,
    before: Value,
) -> io::Result<Attempt> {
    let scope = intent.scope()?;
    if authorization.instruction_id.is_some() && authorization.approval_id.is_some() {
        return Err(permission_denied("choose one authority id"));
    }
    let project_choice = scope.operation == "project.execution.choose";
    if project_choice && caller.is_none_or(|caller| caller.trim().is_empty()) {
        return Err(permission_denied(
            "project choices require a PM Session identity",
        ));
    }
    locked(repo, |store| {
        if project_choice {
            if let Some(attempt) = store
                .project_choice_attempts
                .iter()
                .find(|attempt| attempt.scope == scope)
            {
                let same_source = match attempt.permission_stage.as_str() {
                    "instruction" => {
                        authorization.instruction_id.as_ref() == Some(&attempt.authority_id)
                    }
                    "approval" => {
                        authorization.approval_id.as_ref() == Some(&attempt.authority_id)
                            || (authorization.instruction_id.is_none()
                                && authorization.approval_id.is_none()
                                && store.approvals.iter().any(|approval| {
                                    approval.id == attempt.authority_id && approval.standing
                                }))
                    }
                    _ => false,
                };
                if attempt.caller_session.as_deref() != caller || !same_source {
                    return Err(permission_denied("project choice attempt belongs to a different PM Session or authority source"));
                }
                validate_project_choice_source(repo, store, attempt)?;
                return Ok(attempt.clone());
            }
        }
        let selected = select_authority(repo, store, &scope, authorization, None)?;
        if let Some(consumption) = selected.consume_instruction {
            store.consumed_instructions.push(consumption);
        }
        if let Some(index) = selected.consume_approval {
            store.approvals[index].consumed = true;
        }
        let attempt = Attempt {
            id: uuid::Uuid::new_v4().to_string(),
            scope,
            authority_id: selected.id,
            permission_stage: selected.stage.into(),
            caller_session: caller.map(str::to_owned),
            before: if project_choice {
                crate::user_instruction::redact_json(before)
            } else {
                before
            },
        };
        audit(
            repo,
            serde_json::json!({"event":"authorized", "at":chrono::Utc::now(), "attempt":attempt}),
        )?;
        if project_choice {
            store.project_choice_attempts.push(attempt.clone());
        }
        Ok(attempt)
    })
}

/// Validate the exact, already-authorized PM intent at the physical owner.
pub fn validate_project_choice_attempt(
    repo: &Path,
    intent: &OperationIntent,
    caller: &str,
    source_id: &str,
) -> io::Result<Attempt> {
    let scope = intent.scope()?;
    if scope.operation != "project.execution.choose"
        || caller.trim().is_empty()
        || source_id.trim().is_empty()
    {
        return Err(permission_denied(
            "an exact Project choice, PM Session and authority source are required",
        ));
    }
    let store = load(repo)?;
    let attempt = store.project_choice_attempts.iter().find(|attempt| {
        attempt.scope == scope && attempt.caller_session.as_deref() == Some(caller)
            && attempt.authority_id == source_id
    }).ok_or_else(|| permission_denied("no persisted authorization attempt matches this Project choice, PM Session and source"))?;
    validate_project_choice_source(repo, &store, attempt)?;
    Ok(attempt.clone())
}

fn validate_project_choice_source(repo: &Path, store: &Store, attempt: &Attempt) -> io::Result<()> {
    let active = match attempt.permission_stage.as_str() {
        "instruction" => crate::user_instruction::load(repo)
            .map_err(io::Error::other)?
            .iter()
            .any(|instruction| {
                instruction.id == attempt.authority_id
                    && instruction.superseded_by.is_none()
                    && instruction.authority.is_some()
                    && instruction.kind != crate::user_instruction::UserInstructionKind::Question
                    && instruction.authorizations.contains(&attempt.scope)
            }),
        "approval" => store.approvals.iter().any(|approval| {
            approval.id == attempt.authority_id
                && !approval.revoked
                && approval.scope == attempt.scope
        }),
        _ => false,
    };
    if active {
        Ok(())
    } else {
        Err(permission_denied("the Project choice attempt's exact human authority was revoked, superseded or no longer applies"))
    }
}

impl Attempt {
    pub fn finish(&self, repo: &Path, succeeded: bool, after: Value) -> io::Result<()> {
        locked(repo, |_| {
            audit(
                repo,
                serde_json::json!({
                    "event":if succeeded {"succeeded"} else {"failed"}, "at":chrono::Utc::now(),
                    "attempt_id":self.id, "operation":self.scope.operation,
                    "authority_id":self.authority_id, "before":self.before, "after":after,
                }),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ScopedGwtHome;

    fn authority_bytes(repo: &Path) -> [Option<Vec<u8>>; 3] {
        [
            fs::read(store_path(repo)).ok(),
            fs::read(audit_path(repo)).ok(),
            fs::read(crate::user_instruction::instructions_path(repo)).ok(),
        ]
    }

    #[test]
    fn one_off_approval_preflight_and_completed_replay_are_read_only() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        let intent = OperationIntent {
            operation: "update.settings.set".into(),
            params: serde_json::json!({"auto_apply_updates":true}),
        };
        let before = authority_bytes(&repo);
        assert!(check_authorization(&repo, &intent, &Authorization::default()).is_err());
        assert_eq!(before, authority_bytes(&repo));
        let approval =
            record_approval(&repo, &intent, "Enable updates", "Disable updates", false).unwrap();
        let authorization = Authorization {
            instruction_id: None,
            approval_id: Some(approval.id.clone()),
        };
        let before = authority_bytes(&repo);
        assert_eq!(
            check_authorization(&repo, &intent, &authorization).unwrap(),
            authorization
        );
        assert_eq!(before, authority_bytes(&repo));
        let attempt = authorize(&repo, &intent, &authorization, Some("pm"), Value::Null).unwrap();
        attempt.finish(&repo, true, Value::Null).unwrap();
        let saved: Attempt =
            serde_json::from_value(serde_json::to_value(attempt).unwrap()).unwrap();
        let before = authority_bytes(&repo);
        assert!(check_authorization(&repo, &intent, &authorization).is_err());
        authorize_replay(&repo, &intent, &authorization, &saved).unwrap();
        let mut changed = intent.clone();
        changed.params["auto_apply_updates"] = false.into();
        assert!(authorize_replay(&repo, &changed, &authorization, &saved).is_err());
        assert!(authorize_replay(
            &home.path().join("foreign"),
            &intent,
            &authorization,
            &saved
        )
        .is_err());
        assert!(authorize_replay(&repo, &intent, &Authorization::default(), &saved).is_err());
        assert!(authorize_replay(
            &repo,
            &intent,
            &Authorization {
                instruction_id: None,
                approval_id: Some("another-approval".into()),
            },
            &saved
        )
        .is_err());
        assert_eq!(before, authority_bytes(&repo));
        revoke(&repo, &approval.id).unwrap();
        let before = authority_bytes(&repo);
        assert!(authorize_replay(&repo, &intent, &authorization, &saved).is_err());
        assert_eq!(before, authority_bytes(&repo));
    }

    fn scoped_instruction(
        repo: &Path,
        intent: &OperationIntent,
        kind: crate::user_instruction::UserInstructionKind,
        turn: &str,
        supersedes: Vec<String>,
    ) -> crate::user_instruction::UserInstruction {
        crate::user_instruction::record_from_prompt(
            repo,
            crate::user_instruction::UserInstructionInput {
                original: format!("Enable updates for test turn {turn}"),
                summary: "Enable updates".into(),
                answer: "Use the scoped settings operation".into(),
                actions: vec!["Enable updates".into()],
                issue_numbers: vec![4249],
                kind,
                supersedes,
                authorizations: vec![intent.clone()],
            },
            "pm",
            turn,
        )
        .unwrap()
    }

    #[test]
    fn instruction_replay_requires_the_current_exact_human_authority() {
        use crate::user_instruction::UserInstructionKind;
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        let intent = OperationIntent {
            operation: "update.settings.set".into(),
            params: serde_json::json!({"auto_apply_updates":true}),
        };
        let instruction = scoped_instruction(
            &repo,
            &intent,
            UserInstructionKind::OneOff,
            "one-off",
            vec![],
        );
        let authorization = Authorization {
            instruction_id: Some(instruction.id),
            approval_id: None,
        };
        let attempt = authorize(&repo, &intent, &authorization, Some("pm"), Value::Null).unwrap();
        attempt.finish(&repo, true, Value::Null).unwrap();
        let before = authority_bytes(&repo);
        assert!(check_authorization(&repo, &intent, &authorization).is_err());
        authorize_replay(&repo, &intent, &authorization, &attempt).unwrap();
        assert_eq!(before, authority_bytes(&repo));

        let instruction = scoped_instruction(
            &repo,
            &intent,
            UserInstructionKind::Standing,
            "standing",
            vec![],
        );
        let authorization = Authorization {
            instruction_id: Some(instruction.id.clone()),
            approval_id: None,
        };
        let attempt = authorize(&repo, &intent, &authorization, Some("pm"), Value::Null).unwrap();
        attempt.finish(&repo, true, Value::Null).unwrap();
        let before = authority_bytes(&repo);
        authorize_replay(&repo, &intent, &authorization, &attempt).unwrap();
        let mut changed = intent.clone();
        changed.params["auto_apply_updates"] = false.into();
        assert!(authorize_replay(&repo, &changed, &authorization, &attempt).is_err());
        assert_eq!(before, authority_bytes(&repo));
        scoped_instruction(
            &repo,
            &intent,
            UserInstructionKind::Standing,
            "superseding",
            vec![instruction.id],
        );
        let before = authority_bytes(&repo);
        assert!(authorize_replay(&repo, &intent, &authorization, &attempt).is_err());
        assert_eq!(before, authority_bytes(&repo));
    }

    #[test]
    fn release_policy_scopes_require_exact_parameters_even_with_standing_approval() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        for operation in [
            "actions.release_policy.add",
            "actions.release_policy.rollback",
            "actions.diagnostic.policy.add",
            "actions.diagnostic.policy.rollback",
        ] {
            let intent = OperationIntent {
                operation: operation.into(),
                params: serde_json::json!({
                    "request_id":"b4ae91f1-98f3-4fa6-929c-8ce5e3510597",
                    "expected_token":"a".repeat(64),
                    "reason":"Permit the release workflow"
                }),
            };
            assert!(intent.scope().is_ok(), "missing bounded scope: {operation}");
            let approval = record_approval(
                &repo,
                &intent,
                "Permit the release workflow",
                "Roll back this addition",
                true,
            )
            .unwrap();
            let authorization = Authorization {
                instruction_id: None,
                approval_id: Some(approval.id),
            };
            let before = authority_bytes(&repo);
            assert_eq!(
                check_authorization(&repo, &intent, &Authorization::default()).unwrap(),
                authorization
            );
            assert_eq!(before, authority_bytes(&repo));
            assert!(authorize(&repo, &intent, &authorization, Some("pm"), Value::Null).is_ok());
            let before = (
                fs::read(store_path(&repo)).unwrap(),
                fs::read(audit_path(&repo)).unwrap(),
            );
            let mut changed = intent.clone();
            changed.params["expected_token"] = "b".repeat(64).into();
            assert!(authorize(&repo, &changed, &authorization, Some("pm"), Value::Null).is_err());
            changed = intent.clone();
            changed.params["request_id"] = "cbd15e96-bc2b-478c-a354-302baf307de4".into();
            assert!(authorize(
                &repo,
                &changed,
                &Authorization::default(),
                Some("pm"),
                Value::Null
            )
            .is_err());
            assert_eq!(
                before,
                (
                    fs::read(store_path(&repo)).unwrap(),
                    fs::read(audit_path(&repo)).unwrap()
                )
            );
        }
    }

    #[test]
    fn authorization_scope_ignores_nested_object_key_order() {
        let first = OperationIntent {
            operation: "repository.required_checks.update".into(),
            params: serde_json::from_str(
                r#"{"branch":"develop","checks":[{"context":"CI","app_id":1234}],"strict":false}"#,
            )
            .unwrap(),
        };
        let second = OperationIntent {
            operation: first.operation.clone(),
            params: serde_json::from_str(
                r#"{"strict":false,"checks":[{"app_id":1234,"context":"CI"}],"branch":"develop"}"#,
            )
            .unwrap(),
        };
        assert_eq!(first.scope().unwrap(), second.scope().unwrap());
    }

    #[test]
    fn project_execution_approval_stays_bound_to_exact_choice_and_generation() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        let intent = OperationIntent {
            operation: "project.execution.choose".into(),
            params: serde_json::json!({"operation_id":"00000000-0000-4000-8000-000000000001","expected_revision":0,"expected_generation":1,"choice":"continue"}),
        };
        let approval = record_approval(
            &repo,
            &intent,
            "Continue this project",
            "Stop this project",
            true,
        )
        .unwrap();
        let authorization = Authorization {
            instruction_id: None,
            approval_id: Some(approval.id),
        };
        let first = authorize(&repo, &intent, &authorization, Some("pm"), Value::Null).unwrap();
        assert_eq!(
            authorize(
                &repo,
                &intent,
                &Authorization::default(),
                Some("pm"),
                Value::Null
            )
            .unwrap()
            .id,
            first.id
        );
        assert!(authorize(
            &repo,
            &intent,
            &authorization,
            Some("foreign-pm"),
            Value::Null
        )
        .is_err());
        for (field, value) in [
            ("choice", serde_json::json!("stop")),
            ("expected_generation", serde_json::json!(2)),
            (
                "operation_id",
                serde_json::json!("00000000-0000-4000-8000-000000000002"),
            ),
        ] {
            let mut changed = intent.clone();
            changed.params[field] = value;
            assert!(authorize(&repo, &changed, &authorization, Some("pm"), Value::Null).is_err());
        }
    }

    fn project_choice_intent() -> OperationIntent {
        OperationIntent {
            operation: "project.execution.choose".into(),
            params: serde_json::json!({"operation_id":"00000000-0000-4000-8000-000000000001","expected_revision":0,"expected_generation":7,"choice":"continue"}),
        }
    }

    fn project_choice_instruction(
        repo: &Path,
        intent: &OperationIntent,
        kind: crate::user_instruction::UserInstructionKind,
        turn: &str,
        supersedes: Vec<String>,
    ) -> crate::user_instruction::UserInstruction {
        crate::user_instruction::record_from_prompt(
            repo,
            crate::user_instruction::UserInstructionInput {
                original: "Continue this project".into(),
                summary: "Continue project".into(),
                answer: "Continue the admitted work".into(),
                actions: vec![],
                issue_numbers: vec![],
                kind,
                supersedes,
                authorizations: vec![intent.clone()],
            },
            "pm",
            turn,
        )
        .unwrap()
    }

    #[test]
    fn project_choice_one_off_instruction_retries_only_its_exact_attempt() {
        use crate::user_instruction::UserInstructionKind;
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        let intent = project_choice_intent();
        let instruction = project_choice_instruction(
            &repo,
            &intent,
            UserInstructionKind::OneOff,
            "turn-1",
            vec![],
        );
        let authorization = Authorization {
            instruction_id: Some(instruction.id.clone()),
            approval_id: None,
        };
        let first = authorize(
            &repo,
            &intent,
            &authorization,
            Some("pm"),
            serde_json::json!({"revision":0,"access_token":"private-before-value"}),
        )
        .unwrap();
        assert!(
            !crate::project_execution_policy::policy_path(&repo).exists(),
            "authorization precedes a failed or missing runtime dispatch"
        );
        let retry = authorize(
            &repo,
            &intent,
            &authorization,
            Some("pm"),
            serde_json::json!({"revision":99}),
        )
        .expect("retry the exact consumed one-off after a missing ACK");
        assert_eq!(retry.id, first.id);
        assert_eq!(retry.before, first.before);
        assert!(!fs::read_to_string(store_path(&repo))
            .unwrap()
            .contains("private-before-value"));
        assert_eq!(
            validate_project_choice_attempt(&repo, &intent, "pm", &instruction.id)
                .unwrap()
                .id,
            first.id
        );
        assert!(authorize(
            &repo,
            &intent,
            &authorization,
            Some("foreign-pm"),
            Value::Null
        )
        .is_err());
        assert!(
            validate_project_choice_attempt(&repo, &intent, "foreign-pm", &instruction.id).is_err()
        );
        let mut changed = intent.clone();
        changed.params["operation_id"] = "00000000-0000-4000-8000-000000000002".into();
        assert!(authorize(&repo, &changed, &authorization, Some("pm"), Value::Null).is_err());
        assert!(validate_project_choice_attempt(&repo, &changed, "pm", &instruction.id).is_err());
    }

    #[test]
    fn project_choice_attempt_rejects_a_superseded_standing_instruction() {
        use crate::user_instruction::UserInstructionKind;
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        let intent = project_choice_intent();
        let instruction = project_choice_instruction(
            &repo,
            &intent,
            UserInstructionKind::Standing,
            "turn-1",
            vec![],
        );
        let authorization = Authorization {
            instruction_id: Some(instruction.id.clone()),
            approval_id: None,
        };
        authorize(&repo, &intent, &authorization, Some("pm"), Value::Null).unwrap();
        assert!(validate_project_choice_attempt(&repo, &intent, "pm", &instruction.id).is_ok());
        project_choice_instruction(
            &repo,
            &intent,
            UserInstructionKind::Standing,
            "turn-2",
            vec![instruction.id.clone()],
        );
        assert!(authorize(&repo, &intent, &authorization, Some("pm"), Value::Null).is_err());
        assert!(validate_project_choice_attempt(&repo, &intent, "pm", &instruction.id).is_err());
    }

    #[test]
    fn project_choice_one_off_approval_retries_without_regranting_or_ignoring_revocation() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        let intent = project_choice_intent();
        let approval =
            record_approval(&repo, &intent, "Continue this project", "Stop it", false).unwrap();
        let authorization = Authorization {
            instruction_id: None,
            approval_id: Some(approval.id.clone()),
        };
        let first = authorize(&repo, &intent, &authorization, Some("pm"), Value::Null).unwrap();
        let retry = authorize(&repo, &intent, &authorization, Some("pm"), Value::Null)
            .expect("one-off approval resumes its saved exact attempt");
        assert_eq!(retry.id, first.id);
        assert!(approvals(&repo).unwrap()[0].consumed);
        assert!(authorize(
            &repo,
            &intent,
            &Authorization::default(),
            Some("pm"),
            Value::Null
        )
        .is_err());
        assert_eq!(
            validate_project_choice_attempt(&repo, &intent, "pm", &approval.id)
                .unwrap()
                .id,
            first.id
        );
        assert!(validate_project_choice_attempt(&repo, &intent, "pm", "foreign-source").is_err());
        assert!(authorize(
            &repo,
            &intent,
            &authorization,
            Some("foreign-pm"),
            Value::Null
        )
        .is_err());
        let mut changed = intent.clone();
        changed.params["choice"] = "stop".into();
        assert!(authorize(&repo, &changed, &authorization, Some("pm"), Value::Null).is_err());
        revoke(&repo, &approval.id).unwrap();
        assert!(authorize(&repo, &intent, &authorization, Some("pm"), Value::Null).is_err());
        assert!(validate_project_choice_attempt(&repo, &intent, "pm", &approval.id).is_err());
    }

    #[test]
    fn remembered_external_auto_merge_permission_stays_bound_to_pr_and_sha() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        let intent = OperationIntent {
            operation: "pr.auto_merge.enable".into(),
            params: serde_json::json!({"number":17,"reviewed_head_sha":"a".repeat(40)}),
        };
        let approval = record_approval(
            &repo,
            &intent,
            "Review external contribution",
            "Disable auto-merge before merge",
            true,
        )
        .unwrap();
        let authorization = Authorization {
            approval_id: Some(approval.id),
            instruction_id: None,
        };
        assert!(authorize(&repo, &intent, &authorization, Some("pm"), Value::Null).is_ok());
        let mut changed = intent.clone();
        changed.params["reviewed_head_sha"] = "b".repeat(40).into();
        assert!(authorize(&repo, &changed, &authorization, Some("pm"), Value::Null).is_err());
        changed.params = serde_json::json!({"number":18,"reviewed_head_sha":"a".repeat(40)});
        assert!(authorize(
            &repo,
            &changed,
            &Authorization::default(),
            Some("pm"),
            Value::Null
        )
        .is_err());
    }

    #[test]
    fn authorization_audit_redacts_nested_secrets_before_persistence() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        let intent = OperationIntent {
            operation: "board.config.set".into(),
            params: serde_json::json!({"enabled":true}),
        };
        let approval =
            record_approval(&repo, &intent, "Enable Board", "Disable Board", false).unwrap();
        let attempt = authorize(
            &repo,
            &intent,
            &Authorization {
                instruction_id: None,
                approval_id: Some(approval.id),
            },
            None,
            serde_json::json!({
                "nested":{"api_key":"before-private-value"},
                "items":["password=inline-private-value", {"access_token":"nested-private-value"}],
                "safe":"retain this state"
            }),
        )
        .unwrap();
        attempt
            .finish(
                &repo,
                true,
                serde_json::json!({
                    "authorization":{"Bearer":"after-private-value"},
                    "safe":"retain this result"
                }),
            )
            .unwrap();
        let audit = fs::read_to_string(audit_path(&repo)).unwrap();
        for secret in [
            "before-private-value",
            "inline-private-value",
            "nested-private-value",
            "after-private-value",
        ] {
            assert!(!audit.contains(secret), "audit exposed {secret}");
        }
        for line in audit.lines() {
            serde_json::from_str::<Value>(line).unwrap();
        }
        assert!(audit.contains("retain this state") && audit.contains("retain this result"));
    }
}
