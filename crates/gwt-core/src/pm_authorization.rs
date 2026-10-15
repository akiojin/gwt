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
    "pr.close",
    "worktree.gc_build_artifacts",
    "pm.message.send",
    "pr.reopen",
    "pr.review.submit",
    "release.metadata.set",
    "repository.settings.set",
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

#[derive(Debug, Serialize)]
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
    locked(repo, |store| {
        let (authority_id, stage) = if let Some(id) = &authorization.instruction_id {
            let instructions = crate::user_instruction::load(repo).map_err(io::Error::other)?;
            let instruction = instructions
                .iter()
                .find(|entry| &entry.id == id)
                .filter(|entry| {
                    entry.superseded_by.is_none()
                        && entry.authority.is_some()
                        && entry.kind != crate::user_instruction::UserInstructionKind::Question
                        && entry.authorizations.contains(&scope)
                })
                .ok_or_else(|| {
                    permission_denied(
                        "instruction does not authorize this exact operation and parameters",
                    )
                })?;
            let consumption = format!("{id}:{}:{}", scope.operation, scope.params_sha256);
            if instruction.kind == crate::user_instruction::UserInstructionKind::OneOff {
                if store.consumed_instructions.contains(&consumption) {
                    return Err(permission_denied(
                        "one-off instruction has already been consumed",
                    ));
                }
                store.consumed_instructions.push(consumption);
            }
            (id.clone(), "instruction")
        } else {
            let approval = store
                .approvals
                .iter_mut()
                .find(|entry| {
                    !entry.revoked
                        && !entry.consumed
                        && entry.scope.operation == scope.operation
                        && match &authorization.approval_id {
                            Some(id) => &entry.id == id && (entry.standing || entry.scope == scope),
                            None => entry.standing,
                        }
                })
                .ok_or_else(|| permission_denied("no applicable human approval"))?;
            if !approval.standing {
                approval.consumed = true;
            }
            (approval.id.clone(), "approval")
        };
        let attempt = Attempt {
            id: uuid::Uuid::new_v4().to_string(),
            scope,
            authority_id,
            permission_stage: stage.into(),
            caller_session: caller.map(str::to_owned),
            before,
        };
        audit(
            repo,
            serde_json::json!({"event":"authorized", "at":chrono::Utc::now(), "attempt":attempt}),
        )?;
        Ok(attempt)
    })
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
