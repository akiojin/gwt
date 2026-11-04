//! Durable request tracking, separate from instruction and execution authority.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

/// These identities must be resolved by a trusted adapter, never a wire field.
/// This domain does not authenticate callers or grant execution authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Actor {
    pub kind: ActorKind,
    pub id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorKind {
    Intake,
    Pm,
    User,
}

/// Capture only after the adapter has authenticated this exact human turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntakeSource {
    pub id: String,
    pub project_key: String,
    pub original: String,
    pub authority: crate::user_instruction::InstructionAuthority,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskDraft {
    pub id: String,
    pub title: String,
    pub purpose: String,
    pub completion_conditions: Vec<String>,
    pub issue_numbers: Vec<u64>,
    pub goal_ids: Vec<String>,
    pub assignee: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    InProgress,
    Waiting,
    Completed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Completion {
    pub condition_revision: u64,
    pub review_generation: u64,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub draft: TaskDraft,
    pub source_id: String,
    pub parent_id: Option<String>,
    pub status: TaskStatus,
    pub condition_revision: u64,
    pub review_generation: u64,
    pub review_required: bool,
    pub next_action: Option<String>,
    pub wait_reason: Option<String>,
    pub completion: Option<Completion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub operation_id: String,
    pub expected_revision: u64,
    pub reason: String,
    pub action: Action,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Capture {
        source: IntakeSource,
    },
    ClassifyNonTask {
        source_id: String,
    },
    Decompose {
        source_id: String,
        root: TaskDraft,
        children: Vec<TaskDraft>,
    },
    AddChild {
        parent_id: String,
        child: TaskDraft,
    },
    Update {
        task: TaskDraft,
    },
    Progress {
        task_id: String,
        status: TaskStatus,
        wait_reason: Option<String>,
        next_action: String,
    },
    Complete {
        task_id: String,
        condition_revision: u64,
        review_generation: u64,
        evidence: Vec<String>,
    },
    Reopen {
        task_id: String,
    },
    Reassess {
        task_id: String,
        condition_revision: u64,
        review_generation: u64,
        next_action: String,
    },
    Cancel {
        task_id: String,
    },
}

/// A receipt is also the immutable audit entry, including the resulting tasks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    pub operation_id: String,
    pub action: String,
    pub request_digest: String,
    pub revision: u64,
    pub actor: Actor,
    pub at: DateTime<Utc>,
    pub reason: String,
    pub source_id: Option<String>,
    pub tasks: Vec<Task>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    schema_version: u32,
    pub project_key: String,
    pub revision: u64,
    pub sources: BTreeMap<String, IntakeSource>,
    pub pending: BTreeSet<String>,
    pub tasks: BTreeMap<String, Task>,
    pub receipts: Vec<Receipt>,
}

#[derive(Debug, thiserror::Error)]
pub enum TaskError {
    #[error("foreign_project: read the current project's task store")]
    ForeignProject,
    #[error("unauthorized: resolve the actor through the authenticated adapter")]
    Unauthorized,
    #[error("conflict: read revision {current_revision} before retrying")]
    Conflict { current_revision: u64 },
    #[error("operation_conflict: reconcile the original operation id")]
    OperationConflict,
    #[error("stale_review: read the current conditions and review generation")]
    StaleReview,
    #[error("reassessment_required: record a fresh PM assessment first")]
    ReassessmentRequired,
    #[error("unfinished_children: verify all children before the parent")]
    UnfinishedChildren,
    #[error("invalid_request: {0}")]
    Invalid(String),
    #[error("missing_source: read or recover the captured human turn")]
    MissingSource,
    #[error("missing_task: read the current task snapshot")]
    MissingTask,
    #[error("unreadable_store: {0}; recover the store before retrying")]
    Unreadable(String),
    #[error("persistence_unconfirmed: {0}; read and retry the exact operation id")]
    PersistenceUnconfirmed(String),
}

pub struct TaskStore {
    path: PathBuf,
    project_key: String,
}

impl TaskStore {
    pub fn for_repo(repo: &Path) -> Self {
        Self {
            path: crate::paths::gwt_project_dir_for_repo_path(repo)
                .join("project-state/request-tasks/state.json"),
            project_key: crate::paths::project_scope_hash(repo).to_string(),
        }
    }

    /// `None` means neither a snapshot nor initialization evidence exists.
    /// A partial store requires recovery, never automatic reinitialization.
    pub fn read(&self) -> Result<Option<Snapshot>, TaskError> {
        let initialized = match std::fs::read(self.path.with_extension("initialized")) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(TaskError::Unreadable(error.to_string())),
        };
        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return if initialized.is_none() {
                    Ok(None)
                } else {
                    Err(TaskError::Unreadable(
                        "initialized snapshot missing; restore the original snapshot before retrying"
                            .into(),
                    ))
                };
            }
            Err(error) => return Err(TaskError::Unreadable(error.to_string())),
        };
        let snapshot: Snapshot = serde_json::from_slice(&bytes)
            .map_err(|error| TaskError::Unreadable(error.to_string()))?;
        if snapshot.project_key != self.project_key {
            return Err(TaskError::ForeignProject);
        }
        if initialized.as_deref() != Some(self.project_key.as_bytes()) {
            return Err(TaskError::Unreadable(
                "initialization evidence missing or invalid; restore the original store before retrying"
                    .into(),
            ));
        }
        if snapshot.schema_version != 1
            || snapshot.revision != snapshot.receipts.len() as u64
            || snapshot
                .receipts
                .iter()
                .enumerate()
                .any(|(index, receipt)| receipt.revision != index as u64 + 1)
            || snapshot
                .pending
                .iter()
                .any(|id| !snapshot.sources.contains_key(id))
            || snapshot
                .sources
                .iter()
                .any(|(id, source)| id != &source.id || source.project_key != self.project_key)
            || snapshot.tasks.iter().any(|(id, task)| {
                id != &task.draft.id
                    || !snapshot.sources.contains_key(&task.source_id)
                    || task.parent_id.as_ref().is_some_and(|parent| {
                        snapshot
                            .tasks
                            .get(parent)
                            .is_none_or(|parent| parent.source_id != task.source_id)
                    })
            })
        {
            return Err(TaskError::Unreadable(
                "invalid request task snapshot".into(),
            ));
        }
        Ok(Some(snapshot))
    }

    pub fn apply(&self, actor: &Actor, request: &Request) -> Result<Receipt, TaskError> {
        self.apply_with(actor, request, |path, bytes| {
            let text = std::str::from_utf8(bytes).map_err(|error| error.to_string())?;
            gwt_config::atomic::write_atomic(path, text).map_err(|error| error.to_string())
        })
    }

    fn apply_with(
        &self,
        actor: &Actor,
        request: &Request,
        publish: impl FnOnce(&Path, &[u8]) -> Result<(), String>,
    ) -> Result<Receipt, TaskError> {
        let capture = matches!(request.action, Action::Capture { .. });
        if actor.id.trim().is_empty()
            || (actor.kind == ActorKind::Intake) != capture
            || (actor.kind == ActorKind::User
                && matches!(
                    request.action,
                    Action::Reassess { .. }
                        | Action::Decompose { .. }
                        | Action::ClassifyNonTask { .. }
                        | Action::AddChild { .. }
                ))
        {
            return Err(TaskError::Unauthorized);
        }
        require_text(&request.operation_id)?;
        require_text(&request.reason)?;
        if let Action::Capture { source } = &request.action {
            if source.project_key != self.project_key {
                return Err(TaskError::ForeignProject);
            }
            for value in [
                &source.id,
                &source.original,
                &source.authority.session_id,
                &source.authority.turn_id,
            ] {
                require_text(value)?;
            }
            if source.authority.prompt_sha256
                != crate::pm_authorization::digest(source.original.as_bytes())
            {
                return Err(TaskError::Invalid(
                    "source does not match the captured prompt hash".into(),
                ));
            }
        }
        let parent = self
            .path
            .parent()
            .ok_or_else(|| TaskError::Invalid("store parent missing".into()))?;
        gwt_config::atomic::create_dir_all_durable(parent)
            .map_err(|error| TaskError::PersistenceUnconfirmed(error.to_string()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Protect the raw original even while the publisher's temporary
            // file is being filled. This directory belongs only to TaskStore.
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
                .map_err(|error| TaskError::PersistenceUnconfirmed(error.to_string()))?;
            gwt_config::atomic::create_dir_all_durable(parent)
                .map_err(|error| TaskError::PersistenceUnconfirmed(error.to_string()))?;
        }
        let _lock = crate::operation_deadline::NamedFileLock::acquire(
            &self.path.with_extension("lock"),
            "pm.request_tasks",
        )
        .map_err(|error| TaskError::PersistenceUnconfirmed(error.to_string()))?;
        let existing = self.read()?;
        let initialize = existing.is_none();
        let mut state = existing.unwrap_or_else(|| Snapshot {
            schema_version: 1,
            project_key: self.project_key.clone(),
            revision: 0,
            sources: BTreeMap::new(),
            pending: BTreeSet::new(),
            tasks: BTreeMap::new(),
            receipts: Vec::new(),
        });
        let digest = crate::pm_authorization::digest(
            &serde_json::to_vec(&(actor, request))
                .map_err(|error| TaskError::Invalid(error.to_string()))?,
        );
        let receipt = if let Some(previous) = state
            .receipts
            .iter()
            .find(|receipt| receipt.operation_id == request.operation_id)
        {
            if previous.request_digest != digest {
                return Err(TaskError::OperationConflict);
            }
            previous.clone()
        } else {
            if state.revision != request.expected_revision {
                return Err(TaskError::Conflict {
                    current_revision: state.revision,
                });
            }
            let previous = state.tasks.clone();
            let (action, source_id) = state.transition(&request.action)?;
            state.revision = state
                .revision
                .checked_add(1)
                .ok_or_else(|| TaskError::Invalid("revision exhausted".into()))?;
            let receipt = Receipt {
                operation_id: request.operation_id.clone(),
                action: action.into(),
                request_digest: digest,
                revision: state.revision,
                actor: actor.clone(),
                at: Utc::now(),
                reason: request.reason.clone(),
                source_id,
                tasks: state
                    .tasks
                    .iter()
                    .filter(|(id, task)| previous.get(*id) != Some(*task))
                    .map(|(_, task)| task.clone())
                    .collect(),
            };
            state.receipts.push(receipt.clone());
            receipt
        };
        // An earlier call may have renamed successfully but failed directory sync.
        // Re-publish even on exact replay, under the same stable lock, before ACK.
        let bytes = serde_json::to_vec(&state)
            .map_err(|error| TaskError::PersistenceUnconfirmed(error.to_string()))?;
        if initialize {
            // Persist evidence before the first snapshot can become visible.
            // An interrupted first publication must remain unavailable too.
            gwt_config::atomic::write_atomic(
                &self.path.with_extension("initialized"),
                &self.project_key,
            )
            .map_err(|error| TaskError::PersistenceUnconfirmed(error.to_string()))?;
        }
        publish(&self.path, &bytes).map_err(TaskError::PersistenceUnconfirmed)?;
        let saved = std::fs::read(&self.path)
            .map_err(|error| TaskError::PersistenceUnconfirmed(error.to_string()))?;
        if saved != bytes {
            return Err(TaskError::PersistenceUnconfirmed(
                "publication readback differs".into(),
            ));
        }
        Ok(receipt)
    }
}

fn require_text(value: &str) -> Result<(), TaskError> {
    if value.trim().is_empty() {
        return Err(TaskError::Invalid("nonempty text is required".into()));
    }
    Ok(())
}

impl TaskDraft {
    fn validate(&self) -> Result<(), TaskError> {
        for value in [&self.id, &self.title, &self.purpose] {
            require_text(value)?;
        }
        if self.completion_conditions.is_empty() || self.issue_numbers.contains(&0) {
            return Err(TaskError::Invalid(
                "completion conditions and valid Issue links are required".into(),
            ));
        }
        for value in self.completion_conditions.iter().chain(&self.goal_ids) {
            require_text(value)?;
        }
        if let Some(assignee) = &self.assignee {
            require_text(assignee)?;
        }
        Ok(())
    }
}

impl Snapshot {
    fn task_mut(&mut self, id: &str) -> Result<&mut Task, TaskError> {
        self.tasks.get_mut(id).ok_or(TaskError::MissingTask)
    }

    fn insert_task(
        &mut self,
        draft: &TaskDraft,
        source_id: &str,
        parent_id: Option<String>,
    ) -> Result<(), TaskError> {
        draft.validate()?;
        if self.tasks.contains_key(&draft.id) {
            return Err(TaskError::Invalid("task id already exists".into()));
        }
        self.tasks.insert(
            draft.id.clone(),
            Task {
                draft: draft.clone(),
                source_id: source_id.into(),
                parent_id,
                status: TaskStatus::Pending,
                condition_revision: 1,
                review_generation: 1,
                review_required: false,
                next_action: None,
                wait_reason: None,
                completion: None,
            },
        );
        Ok(())
    }

    fn invalidate(&mut self, id: &str) -> Result<(), TaskError> {
        let mut current = Some(id.to_string());
        let mut visited = BTreeSet::new();
        while let Some(id) = current {
            if !visited.insert(id.clone()) {
                return Err(TaskError::Invalid("cyclic task ancestry".into()));
            }
            let task = self.task_mut(&id)?;
            if task.status != TaskStatus::Cancelled {
                task.status = TaskStatus::Pending;
            }
            task.review_generation = task
                .review_generation
                .checked_add(1)
                .ok_or_else(|| TaskError::Invalid("review generation exhausted".into()))?;
            task.review_required = true;
            task.completion = None;
            task.next_action = None;
            task.wait_reason = None;
            current = task.parent_id.clone();
        }
        Ok(())
    }

    fn check_review(
        task: &Task,
        condition_revision: u64,
        review_generation: u64,
    ) -> Result<(), TaskError> {
        if task.condition_revision != condition_revision
            || task.review_generation != review_generation
        {
            return Err(TaskError::StaleReview);
        }
        Ok(())
    }

    fn transition(&mut self, action: &Action) -> Result<(&'static str, Option<String>), TaskError> {
        match action {
            Action::Capture { source } => {
                if let Some(existing) = self.sources.get(&source.id) {
                    if existing != source {
                        return Err(TaskError::Invalid("captured source is immutable".into()));
                    }
                } else {
                    if self.sources.values().any(|existing| {
                        existing.authority.session_id == source.authority.session_id
                            && existing.authority.turn_id == source.authority.turn_id
                    }) {
                        return Err(TaskError::Invalid(
                            "human turn already captured under another source id".into(),
                        ));
                    }
                    self.sources.insert(source.id.clone(), source.clone());
                    self.pending.insert(source.id.clone());
                }
                Ok(("capture", Some(source.id.clone())))
            }
            Action::ClassifyNonTask { source_id } => {
                if !self.sources.contains_key(source_id) {
                    return Err(TaskError::MissingSource);
                }
                if self.tasks.values().any(|task| &task.source_id == source_id) {
                    return Err(TaskError::Invalid(
                        "source already has tracked tasks; correct those tasks explicitly".into(),
                    ));
                }
                self.pending.remove(source_id);
                Ok(("classify_non_task", Some(source_id.clone())))
            }
            Action::Decompose {
                source_id,
                root,
                children,
            } => {
                if !self.sources.contains_key(source_id) {
                    return Err(TaskError::MissingSource);
                }
                if self.tasks.values().any(|task| &task.source_id == source_id) {
                    return Err(TaskError::Invalid("source already decomposed".into()));
                }
                self.insert_task(root, source_id, None)?;
                for child in children {
                    self.insert_task(child, source_id, Some(root.id.clone()))?;
                }
                self.pending.remove(source_id);
                Ok(("decompose", Some(source_id.clone())))
            }
            Action::AddChild { parent_id, child } => {
                let parent = self.tasks.get(parent_id).ok_or(TaskError::MissingTask)?;
                if parent.status == TaskStatus::Cancelled {
                    return Err(TaskError::Invalid(
                        "reopen a cancelled parent before adding children".into(),
                    ));
                }
                let source_id = parent.source_id.clone();
                self.insert_task(child, &source_id, Some(parent_id.clone()))?;
                self.invalidate(parent_id)?;
                Ok(("add_child", Some(source_id)))
            }
            Action::Update { task: draft } => {
                draft.validate()?;
                let task = self.task_mut(&draft.id)?;
                let changed_conditions = task.draft.completion_conditions
                    != draft.completion_conditions
                    || task.draft.purpose != draft.purpose;
                if changed_conditions {
                    task.condition_revision = task
                        .condition_revision
                        .checked_add(1)
                        .ok_or_else(|| TaskError::Invalid("condition revision exhausted".into()))?;
                }
                task.draft = draft.clone();
                let source_id = task.source_id.clone();
                if changed_conditions {
                    self.invalidate(&draft.id)?;
                }
                Ok(("update", Some(source_id)))
            }
            Action::Progress {
                task_id,
                status,
                wait_reason,
                next_action,
            } => {
                require_text(next_action)?;
                if matches!(status, TaskStatus::Completed | TaskStatus::Cancelled)
                    || (*status == TaskStatus::Waiting
                        && wait_reason
                            .as_ref()
                            .is_none_or(|reason| reason.trim().is_empty()))
                {
                    return Err(TaskError::Invalid(
                        "progress cannot complete/cancel a task and waiting requires a reason"
                            .into(),
                    ));
                }
                let task = self.task_mut(task_id)?;
                if matches!(task.status, TaskStatus::Completed | TaskStatus::Cancelled) {
                    return Err(TaskError::Invalid(
                        "reopen the task before changing its progress".into(),
                    ));
                }
                task.status = *status;
                task.wait_reason = if *status == TaskStatus::Waiting {
                    wait_reason.clone()
                } else {
                    None
                };
                task.next_action = Some(next_action.clone());
                Ok(("progress", Some(task.source_id.clone())))
            }
            Action::Complete {
                task_id,
                condition_revision,
                review_generation,
                evidence,
            } => {
                let task = self.tasks.get(task_id).ok_or(TaskError::MissingTask)?;
                Self::check_review(task, *condition_revision, *review_generation)?;
                if task.review_required {
                    return Err(TaskError::ReassessmentRequired);
                }
                if task.status == TaskStatus::Cancelled {
                    return Err(TaskError::Invalid("reopen the cancelled task first".into()));
                }
                if evidence.is_empty() {
                    return Err(TaskError::Invalid(
                        "completion requires original-outcome evidence".into(),
                    ));
                }
                for reference in evidence {
                    require_text(reference)?;
                }
                if self.tasks.values().any(|child| {
                    child.parent_id.as_deref() == Some(task_id)
                        && child.status != TaskStatus::Completed
                }) {
                    return Err(TaskError::UnfinishedChildren);
                }
                let task = self.task_mut(task_id)?;
                task.status = TaskStatus::Completed;
                task.completion = Some(Completion {
                    condition_revision: *condition_revision,
                    review_generation: *review_generation,
                    evidence: evidence.clone(),
                });
                task.wait_reason = None;
                task.next_action = None;
                Ok(("complete", Some(task.source_id.clone())))
            }
            Action::Reopen { task_id } => {
                self.task_mut(task_id)?.status = TaskStatus::Pending;
                self.invalidate(task_id)?;
                Ok(("reopen", Some(self.task_mut(task_id)?.source_id.clone())))
            }
            Action::Reassess {
                task_id,
                condition_revision,
                review_generation,
                next_action,
            } => {
                require_text(next_action)?;
                let task = self.task_mut(task_id)?;
                Self::check_review(task, *condition_revision, *review_generation)?;
                if task.status == TaskStatus::Cancelled {
                    return Err(TaskError::Invalid("reopen the cancelled task first".into()));
                }
                task.review_required = false;
                task.next_action = Some(next_action.clone());
                Ok(("reassess", Some(task.source_id.clone())))
            }
            Action::Cancel { task_id } => {
                self.invalidate(task_id)?;
                let task = self.task_mut(task_id)?;
                task.status = TaskStatus::Cancelled;
                Ok(("cancel", Some(task.source_id.clone())))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn store(dir: &tempfile::TempDir) -> TaskStore {
        TaskStore {
            path: dir.path().join("tasks.json"),
            project_key: "project-a".into(),
        }
    }

    fn actor(kind: ActorKind) -> Actor {
        Actor {
            kind,
            id: "authenticated-session".into(),
        }
    }

    fn apply(
        store: &TaskStore,
        id: &str,
        revision: u64,
        action: Value,
    ) -> Result<Receipt, TaskError> {
        let action: Action = serde_json::from_value(action).unwrap();
        let kind = if matches!(action, Action::Capture { .. }) {
            ActorKind::Intake
        } else {
            ActorKind::Pm
        };
        store.apply(
            &actor(kind),
            &Request {
                operation_id: id.into(),
                expected_revision: revision,
                reason: "verified request".into(),
                action,
            },
        )
    }

    fn source() -> Value {
        json!({"kind":"capture","source":{
            "id":"turn-1", "project_key":"project-a", "original":"Keep the original purpose.\nDo not replace it.",
            "authority":{"session_id":"human-session", "turn_id":"human-turn", "prompt_sha256":crate::pm_authorization::digest(b"Keep the original purpose.\nDo not replace it.")}
        }})
    }

    fn draft(id: &str) -> Value {
        json!({"id":id,"title":"Track request","purpose":"Keep the original purpose",
            "completion_conditions":["Original outcome verified"],"issue_numbers":[3431],"goal_ids":[],"assignee":null})
    }

    fn seed(store: &TaskStore) {
        apply(store, "capture", 0, source()).unwrap();
        apply(store, "decompose", 1, json!({"kind":"decompose","source_id":"turn-1","root":draft("parent"),"children":[draft("child")]})).unwrap();
    }

    fn complete(id: &str, condition: u64, generation: u64) -> Value {
        json!({"kind":"complete","task_id":id,"condition_revision":condition,"review_generation":generation,"evidence":["verified:original-outcome"]})
    }

    #[test]
    fn pending_source_survives_restart_and_cannot_be_replaced_or_decomposed_twice() {
        let dir = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let store = store(&dir);
        assert!(
            store.read().unwrap().is_none(),
            "missing is distinct from an empty snapshot"
        );
        let receipt = apply(&store, "capture", 0, source()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                std::fs::metadata(&store.path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let state = store.read().unwrap().unwrap();
        assert_eq!(state.revision, 1);
        assert!(state.pending.contains("turn-1"));
        assert!(state.tasks.is_empty());
        assert_eq!(apply(&store, "capture", 0, source()).unwrap(), receipt);
        let mut changed = source();
        changed["source"]["original"] = json!("replacement");
        changed["source"]["authority"]["prompt_sha256"] =
            json!(crate::pm_authorization::digest(b"replacement"));
        assert!(apply(&store, "changed", 1, changed).is_err());
        apply(&store, "decompose", 1, json!({"kind":"decompose","source_id":"turn-1","root":draft("parent"),"children":[draft("child")]})).unwrap();
        assert!(apply(
            &store,
            "second-decomposition",
            2,
            json!({"kind":"decompose","source_id":"turn-1","root":draft("duplicate"),"children":[]})
        )
        .is_err());
        let loaded = self::store(&dir).read().unwrap().unwrap();
        assert!(loaded.pending.is_empty());
        assert_eq!(
            loaded.sources["turn-1"].original,
            "Keep the original purpose.\nDo not replace it."
        );
        assert_eq!(loaded.tasks["child"].parent_id.as_deref(), Some("parent"));
        assert_eq!(loaded.receipts.len(), 2);
    }

    #[test]
    fn parent_needs_all_children_and_its_own_current_evidence() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        seed(&store);
        assert!(matches!(
            apply(&store, "early-parent", 2, complete("parent", 1, 1)),
            Err(TaskError::UnfinishedChildren)
        ));
        apply(&store, "complete-child", 2, complete("child", 1, 1)).unwrap();
        assert_eq!(
            store.read().unwrap().unwrap().tasks["parent"].status,
            TaskStatus::Pending
        );
        let mut missing = complete("parent", 1, 1);
        missing["evidence"] = json!([]);
        assert!(apply(&store, "no-evidence", 3, missing).is_err());
        apply(&store, "complete-parent", 3, complete("parent", 1, 1)).unwrap();
        apply(
            &store,
            "new-child",
            4,
            json!({"kind":"add_child","parent_id":"parent","child":draft("new-child")}),
        )
        .unwrap();
        let state = store.read().unwrap().unwrap();
        assert_eq!(state.tasks["parent"].status, TaskStatus::Pending);
        assert!(state.tasks["parent"].review_required);
        assert_eq!(state.receipts[3].tasks[0].status, TaskStatus::Completed);
    }

    #[test]
    fn user_uncheck_requires_fresh_pm_reassessment_and_preserves_audit() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        seed(&store);
        apply(&store, "complete-child", 2, complete("child", 1, 1)).unwrap();
        let reopen = Request {
            operation_id: "user-uncheck".into(),
            expected_revision: 3,
            reason: "The outcome is still missing".into(),
            action: Action::Reopen {
                task_id: "child".into(),
            },
        };
        let receipt = store.apply(&actor(ActorKind::User), &reopen).unwrap();
        assert_eq!(receipt.actor.kind, ActorKind::User);
        assert_eq!(receipt.reason, reopen.reason);
        assert!(matches!(
            apply(&store, "old-review", 4, complete("child", 1, 1)),
            Err(TaskError::StaleReview)
        ));
        assert!(matches!(
            apply(&store, "skip-reassessment", 4, complete("child", 1, 2)),
            Err(TaskError::ReassessmentRequired)
        ));
        let reassess = Request { operation_id: "user-assessment".into(), expected_revision: 4, reason: "Only the PM can reassess".into(), action: serde_json::from_value(json!({"kind":"reassess","task_id":"child","condition_revision":1,"review_generation":2,"next_action":"Verify again"})).unwrap() };
        assert!(matches!(
            store.apply(&actor(ActorKind::User), &reassess),
            Err(TaskError::Unauthorized)
        ));
        apply(&store, "reassess", 4, json!({"kind":"reassess","task_id":"child","condition_revision":1,"review_generation":2,"next_action":"Verify the original outcome again"})).unwrap();
        apply(&store, "fresh-review", 5, complete("child", 1, 2)).unwrap();
        let state = store.read().unwrap().unwrap();
        assert_eq!(state.receipts[2].actor.kind, ActorKind::Pm);
        assert_eq!(state.receipts[3], receipt);
        assert_eq!(state.tasks["child"].status, TaskStatus::Completed);
        let mut changed = draft("child");
        changed["completion_conditions"] = json!(["New outcome"]);
        let correction = Request {
            operation_id: "correct-condition".into(),
            expected_revision: 6,
            reason: "Correct the outcome".into(),
            action: serde_json::from_value(json!({"kind":"update","task":changed})).unwrap(),
        };
        store.apply(&actor(ActorKind::User), &correction).unwrap();
        assert!(matches!(
            apply(&store, "old-condition", 7, complete("child", 1, 2)),
            Err(TaskError::StaleReview)
        ));
        let cancel = Request {
            operation_id: "cancel".into(),
            expected_revision: 7,
            reason: "Withdraw this task".into(),
            action: Action::Cancel {
                task_id: "child".into(),
            },
        };
        store.apply(&actor(ActorKind::User), &cancel).unwrap();
        let state = store.read().unwrap().unwrap();
        assert_eq!(state.tasks["child"].status, TaskStatus::Cancelled);
        assert!(state.tasks["child"].completion.is_none());
    }

    #[test]
    fn reopening_a_descendant_invalidates_completed_ancestors() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        seed(&store);
        apply(
            &store,
            "grandchild",
            2,
            json!({"kind":"add_child","parent_id":"child","child":draft("grandchild")}),
        )
        .unwrap();
        apply(&store, "assess-child", 3, json!({"kind":"reassess","task_id":"child","condition_revision":1,"review_generation":2,"next_action":"Verify children"})).unwrap();
        apply(&store, "assess-parent", 4, json!({"kind":"reassess","task_id":"parent","condition_revision":1,"review_generation":2,"next_action":"Verify original outcome"})).unwrap();
        apply(
            &store,
            "complete-grandchild",
            5,
            complete("grandchild", 1, 1),
        )
        .unwrap();
        apply(&store, "complete-child", 6, complete("child", 1, 2)).unwrap();
        apply(&store, "complete-parent", 7, complete("parent", 1, 2)).unwrap();
        apply(
            &store,
            "reopen-grandchild",
            8,
            json!({"kind":"reopen","task_id":"grandchild"}),
        )
        .unwrap();
        let state = store.read().unwrap().unwrap();
        for id in ["parent", "child", "grandchild"] {
            assert_eq!(state.tasks[id].status, TaskStatus::Pending);
            assert!(state.tasks[id].review_required);
            assert!(state.tasks[id].completion.is_none());
        }
    }

    #[test]
    fn cas_and_operation_identity_never_overwrite_newer_state() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        seed(&store);
        let receipt = apply(&store, "finish", 2, complete("child", 1, 1)).unwrap();
        assert_eq!(
            apply(&store, "finish", 2, complete("child", 1, 1)).unwrap(),
            receipt
        );
        assert!(matches!(
            apply(&store, "finish", 2, complete("parent", 1, 1)),
            Err(TaskError::OperationConflict)
        ));
        assert!(matches!(
            apply(&store, "stale", 2, complete("parent", 1, 1)),
            Err(TaskError::Conflict {
                current_revision: 3
            })
        ));
        assert_eq!(store.read().unwrap().unwrap().revision, 3);
    }

    #[test]
    fn progress_keeps_wait_reason_and_cannot_bypass_completion_review() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        seed(&store);
        apply(&store, "wait", 2, json!({"kind":"progress","task_id":"child","status":"waiting","wait_reason":"Owner verification pending","next_action":"Read fresh owner evidence"})).unwrap();
        let state = store.read().unwrap().unwrap();
        assert_eq!(state.tasks["child"].status, TaskStatus::Waiting);
        assert_eq!(
            state.tasks["child"].wait_reason.as_deref(),
            Some("Owner verification pending")
        );
        assert!(apply(&store, "bypass", 3, json!({"kind":"progress","task_id":"child","status":"completed","wait_reason":null,"next_action":"Pretend success"})).is_err());
        apply(&store, "work", 3, json!({"kind":"progress","task_id":"child","status":"in_progress","wait_reason":null,"next_action":"Verify original outcome"})).unwrap();
        assert_eq!(
            store.read().unwrap().unwrap().tasks["child"].status,
            TaskStatus::InProgress
        );
    }

    #[test]
    fn concurrent_cas_has_one_winner_across_independent_store_handles() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        seed(&store);
        let barrier = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            let handles: Vec<_> = ["writer-a", "writer-b"]
                .into_iter()
                .map(|id| {
                    let barrier = &barrier;
                    let store = &store;
                    scope.spawn(move || {
                        let independent = TaskStore {
                            path: store.path.clone(),
                            project_key: store.project_key.clone(),
                        };
                        barrier.wait();
                        apply(&independent, id, 2, complete("child", 1, 1))
                    })
                })
                .collect();
            let results: Vec<_> = handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect();
            assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
            assert_eq!(
                results
                    .iter()
                    .filter(|result| matches!(
                        result,
                        Err(TaskError::Conflict {
                            current_revision: 3
                        })
                    ))
                    .count(),
                1
            );
        });
        assert_eq!(store.read().unwrap().unwrap().receipts.len(), 3);
    }

    #[test]
    fn sources_and_mutations_stay_in_the_project_and_actor_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let mut foreign = source();
        foreign["source"]["project_key"] = json!("foreign");
        assert!(matches!(
            apply(&store, "foreign", 0, foreign),
            Err(TaskError::ForeignProject)
        ));
        apply(&store, "capture", 0, source()).unwrap();
        apply(
            &store,
            "question",
            1,
            json!({"kind":"classify_non_task","source_id":"turn-1"}),
        )
        .unwrap();
        let state = store.read().unwrap().unwrap();
        assert_eq!(state.receipts[0].action, "capture");
        assert_eq!(state.receipts[1].action, "classify_non_task");
        assert!(store.read().unwrap().unwrap().tasks.is_empty());
        assert!(store.read().unwrap().unwrap().pending.is_empty());
        let request = Request {
            operation_id: "fake-intake-action".into(),
            expected_revision: 2,
            reason: "test".into(),
            action: Action::Cancel {
                task_id: "anything".into(),
            },
        };
        assert!(matches!(
            store.apply(&actor(ActorKind::Intake), &request),
            Err(TaskError::Unauthorized)
        ));
        let foreign_store = TaskStore {
            path: store.path.clone(),
            project_key: "foreign".into(),
        };
        assert!(matches!(
            foreign_store.read(),
            Err(TaskError::ForeignProject)
        ));
    }

    #[test]
    fn initialized_snapshot_loss_cannot_reset_task_or_receipt_history() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        assert!(matches!(
            apply(
                &store,
                "invalid-first",
                0,
                json!({"kind":"reopen","task_id":"missing"})
            ),
            Err(TaskError::MissingTask)
        ));
        assert!(store.read().unwrap().is_none());
        seed(&store);
        let before = std::fs::read(&store.path).unwrap();
        let receipt = store.read().unwrap().unwrap().receipts[0].clone();
        std::fs::remove_file(&store.path).unwrap();
        assert!(matches!(store.read(), Err(TaskError::Unreadable(_))));
        assert!(matches!(
            apply(&store, "capture", 0, source()),
            Err(TaskError::Unreadable(_))
        ));
        assert!(!store.path.exists(), "replay must not recreate lost state");

        // Restoring the original snapshot permits exact replay without reset.
        std::fs::write(&store.path, &before).unwrap();
        assert_eq!(apply(&store, "capture", 0, source()).unwrap(), receipt);
        assert_eq!(std::fs::read(&store.path).unwrap(), before);
        std::fs::remove_file(store.path.with_extension("initialized")).unwrap();
        assert!(matches!(store.read(), Err(TaskError::Unreadable(_))));
        assert!(matches!(
            apply(&store, "capture", 0, source()),
            Err(TaskError::Unreadable(_))
        ));
        assert_eq!(std::fs::read(&store.path).unwrap(), before);
    }

    #[test]
    fn interrupted_first_publication_is_unavailable_instead_of_fresh() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let request = Request {
            operation_id: "capture".into(),
            expected_revision: 0,
            reason: "authenticated human turn".into(),
            action: serde_json::from_value(source()).unwrap(),
        };
        let failed = store.apply_with(&actor(ActorKind::Intake), &request, |_, _| {
            assert!(matches!(store.read(), Err(TaskError::Unreadable(_))));
            Err("first snapshot publication failed".into())
        });
        assert!(matches!(failed, Err(TaskError::PersistenceUnconfirmed(_))));
        assert!(matches!(store.read(), Err(TaskError::Unreadable(_))));
        assert!(matches!(
            store.apply(&actor(ActorKind::Intake), &request),
            Err(TaskError::Unreadable(_))
        ));
        assert!(!store.path.exists());
    }

    #[test]
    fn failed_and_uncertain_publications_reconcile_using_the_same_receipt() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        seed(&store);
        let before = std::fs::read(&store.path).unwrap();
        let request = Request {
            operation_id: "write".into(),
            expected_revision: 2,
            reason: "reviewed".into(),
            action: serde_json::from_value(complete("child", 1, 1)).unwrap(),
        };
        let failed = store.apply_with(&actor(ActorKind::Pm), &request, |_, _| {
            Err("write refused".into())
        });
        assert!(matches!(failed, Err(TaskError::PersistenceUnconfirmed(_))));
        assert_eq!(std::fs::read(&store.path).unwrap(), before);
        let uncertain = store.apply_with(&actor(ActorKind::Pm), &request, |path, bytes| {
            crate::workspace_projection::write_atomic(path, bytes).unwrap();
            Err("response lost after publication".into())
        });
        assert!(matches!(
            uncertain,
            Err(TaskError::PersistenceUnconfirmed(_))
        ));
        let retry = store.apply_with(&actor(ActorKind::Pm), &request, |_, _| {
            Err("directory sync still unavailable".into())
        });
        assert!(
            matches!(retry, Err(TaskError::PersistenceUnconfirmed(_))),
            "reading a receipt cannot confirm durability"
        );
        let receipt = store.apply(&actor(ActorKind::Pm), &request).unwrap();
        assert_eq!(receipt.revision, 3);
        assert_eq!(store.read().unwrap().unwrap().receipts.len(), 3);
        std::fs::write(&store.path, b"incomplete").unwrap();
        assert!(
            store.read().is_err(),
            "unreadable state must not become zero tasks"
        );
    }
}
