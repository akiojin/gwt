//! Bounded, read-only summaries. TaskStore remains the tracking authority.

use chrono::{DateTime, Utc};
use gwt_core::pm_request_tasks::{ActorKind, Snapshot, TaskStatus, TaskStore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PmTaskOverview {
    Absent,
    Ready {
        revision: u64,
        total: usize,
        pending_intake: usize,
        tasks: Vec<PmTaskSummary>,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PmTaskSummary {
    pub key: String,
    pub parent_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_key: Option<String>,
    pub title: String,
    pub status: TaskStatus,
    pub assignee: Option<String>,
    pub wait_reason: Option<String>,
    pub next_action: Option<String>,
    pub review_required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub criteria_preview: Option<PmTaskCriteriaPreview>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_progress: Option<PmTaskChildProgress>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PmTaskChildProgress {
    pub total: usize,
    pub completed: usize,
    pub cancelled: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PmTaskCriteriaPreview {
    pub purpose: String,
    pub completion_conditions: Vec<String>,
    pub completion_condition_total: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PmTaskBoundedText {
    pub text: String,
    pub total_chars: usize,
}

impl PmTaskBoundedText {
    fn new(text: &str, limit: usize) -> Self {
        Self {
            text: text.chars().take(limit).collect(),
            total_chars: text.chars().count(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PmTaskDetail {
    pub status: TaskStatus,
    pub original: Option<PmTaskBoundedText>,
    pub purpose: PmTaskBoundedText,
    pub completion_conditions: Vec<PmTaskBoundedText>,
    pub completion_condition_total: usize,
    pub evidence: Vec<PmTaskBoundedText>,
    pub evidence_total: usize,
    pub history: Vec<PmTaskHistoryEntry>,
    pub history_total: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PmTaskHistoryEntry {
    pub revision: u64,
    pub action: String,
    pub actor_kind: ActorKind,
    pub actor_id: PmTaskBoundedText,
    pub at: DateTime<Utc>,
    pub reason: PmTaskBoundedText,
}

fn key(id: &str) -> String {
    format!("{:x}", Sha256::digest(id.as_bytes()))
}

/// Inspect one selected task without exposing provenance or receipt snapshots.
pub fn task_detail(state: &Snapshot, selected_key: &str) -> Option<PmTaskDetail> {
    let task = state
        .tasks
        .values()
        .find(|task| key(&task.draft.id) == selected_key)?;
    let evidence = task
        .completion
        .as_ref()
        .map_or(&[][..], |completion| completion.evidence.as_slice());
    let mut history = Vec::new();
    let mut history_total = 0;
    for receipt in state.receipts.iter().rev().filter(|receipt| {
        receipt
            .tasks
            .iter()
            .any(|changed| changed.draft.id == task.draft.id)
    }) {
        history_total += 1;
        if history.len() < 20 {
            history.push(PmTaskHistoryEntry {
                revision: receipt.revision,
                action: receipt.action.clone(),
                actor_kind: receipt.actor.kind,
                actor_id: PmTaskBoundedText::new(&receipt.actor.id, 240),
                at: receipt.at,
                reason: PmTaskBoundedText::new(&receipt.reason, 1000),
            });
        }
    }
    Some(PmTaskDetail {
        status: task.status,
        original: state
            .sources
            .get(&task.source_id)
            .map(|source| PmTaskBoundedText::new(&source.original, 4000)),
        purpose: PmTaskBoundedText::new(&task.draft.purpose, 1000),
        completion_conditions: task
            .draft
            .completion_conditions
            .iter()
            .take(20)
            .map(|condition| PmTaskBoundedText::new(condition, 1000))
            .collect(),
        completion_condition_total: task.draft.completion_conditions.len(),
        evidence: evidence
            .iter()
            .take(10)
            .map(|reference| PmTaskBoundedText::new(reference, 1000))
            .collect(),
        evidence_total: evidence.len(),
        history,
        history_total,
    })
}

fn short(text: &str) -> String {
    let mut chars = text.chars();
    let mut value: String = chars.by_ref().take(240).collect();
    if chars.next().is_some() {
        value.push('…');
    }
    value
}

pub(super) fn load(repo: &Path) -> PmTaskOverview {
    let state = match TaskStore::for_repo(repo).read() {
        Ok(Some(state)) => state,
        Ok(None) => return PmTaskOverview::Absent,
        Err(_) => {
            return PmTaskOverview::Error {
                // Decode errors can echo arbitrary text from a corrupt store.
                message: "Request tasks could not be read. Restore this project's saved task store and try again.".into(),
            };
        }
    };
    let unfinished =
        |status: TaskStatus| !matches!(status, TaskStatus::Completed | TaskStatus::Cancelled);
    // Count direct children before the preview limit; cancellation does not
    // satisfy a parent's completion conditions and grandchildren are separate.
    let mut child_progress: HashMap<&str, PmTaskChildProgress> = HashMap::new();
    for task in state.tasks.values() {
        if let Some(parent) = task.parent_id.as_deref() {
            let progress = child_progress.entry(parent).or_default();
            progress.total += 1;
            progress.completed += usize::from(task.status == TaskStatus::Completed);
            progress.cancelled += usize::from(task.status == TaskStatus::Cancelled);
        }
    }
    // Preserve stable ID order in each group, without copying the source,
    // authority, full drafts or audit history into every Reports poll.
    let tasks = state
        .tasks
        .values()
        .filter(|task| unfinished(task.status))
        .chain(state.tasks.values().filter(|task| !unfinished(task.status)))
        .take(50)
        .map(|task| PmTaskSummary {
            key: key(&task.draft.id),
            parent_key: task.parent_id.as_deref().map(key),
            source_key: Some(key(&task.source_id)),
            title: short(&task.draft.title),
            status: task.status,
            assignee: task.draft.assignee.as_deref().map(short),
            wait_reason: task.wait_reason.as_deref().map(short),
            next_action: task.next_action.as_deref().map(short),
            review_required: task.review_required,
            criteria_preview: Some(PmTaskCriteriaPreview {
                purpose: short(&task.draft.purpose),
                completion_conditions: task
                    .draft
                    .completion_conditions
                    .iter()
                    .take(5)
                    .map(|condition| short(condition))
                    .collect(),
                completion_condition_total: task.draft.completion_conditions.len(),
            }),
            child_progress: child_progress.get(task.draft.id.as_str()).copied(),
        })
        .collect();
    PmTaskOverview::Ready {
        revision: state.revision,
        total: state.tasks.len(),
        pending_intake: state.pending.len(),
        tasks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_core::pm_request_tasks::Snapshot;
    use serde_json::json;

    #[test]
    fn selected_task_detail_is_bounded_and_excludes_unrelated_receipts_and_authority() {
        let task = json!({
            "draft":{"id":"private-task","title":"Fixture","purpose":"目".repeat(1001),
                "completion_conditions":(0..21).map(|i| format!("{i}:{}", "条".repeat(1001))).collect::<Vec<_>>(),
                "issue_numbers":[],"goal_ids":[],"assignee":null},
            "source_id":"private-source","parent_id":null,"status":"completed",
            "condition_revision":1,"review_generation":1,"review_required":false,
            "next_action":null,"wait_reason":null,
            "completion":{"condition_revision":1,"review_generation":1,
                "evidence":(0..11).map(|i| format!("e{i}:{}", "証".repeat(1001))).collect::<Vec<_>>()}
        });
        let mut unrelated = task.clone();
        unrelated["draft"]["id"] = json!("private-unrelated-task");
        let mut receipts = (1..=24).map(|revision| json!({
            "operation_id":format!("private-operation-{revision}"),"action":"progress",
            "request_digest":"PRIVATE-DIGEST","revision":revision,
            "actor":{"kind":"pm","id":"人".repeat(241)},
            "at":"2026-11-13T00:00:00Z","reason":"由".repeat(1001),
            "source_id":"private-source","tasks":[if revision == 1 { &unrelated } else { &task }]
        })).collect::<Vec<_>>();
        let mut noop = receipts.last().unwrap().clone();
        noop["revision"] = json!(25);
        noop["tasks"] = json!([]);
        receipts.push(noop);
        let mut snapshot: Snapshot = serde_json::from_value(json!({
            "schema_version":1,"project_key":"fixture","revision":25,
            "sources":{"private-source":{"id":"private-source","project_key":"fixture",
                "original":"原".repeat(4001),"authority":{"session_id":"PRIVATE-AUTHORITY",
                    "turn_id":"private-turn","prompt_sha256":"PRIVATE-DIGEST"}}},
            "pending":[],"tasks":{"private-task":task},"receipts":receipts
        }))
        .unwrap();
        let selected_key = key("private-task");
        let detail = task_detail(&snapshot, &selected_key).expect("selected saved task");
        assert_eq!(detail.original.as_ref().unwrap().text, "原".repeat(4000));
        assert_eq!(detail.original.as_ref().unwrap().total_chars, 4001);
        assert_eq!(detail.purpose.text, "目".repeat(1000));
        assert_eq!(detail.purpose.total_chars, 1001);
        assert_eq!(detail.completion_condition_total, 21);
        assert_eq!(detail.completion_conditions.len(), 20);
        assert_eq!(detail.completion_conditions[0].text.chars().count(), 1000);
        assert_eq!(detail.completion_conditions[0].total_chars, 1003);
        assert_eq!(detail.evidence_total, 11);
        assert_eq!(detail.evidence.len(), 10);
        assert_eq!(detail.evidence[0].text.chars().count(), 1000);
        assert_eq!(detail.history_total, 23);
        assert_eq!(detail.history.len(), 20);
        assert_eq!(detail.history[0].revision, 24);
        assert_eq!(detail.history[19].revision, 5);
        assert_eq!(detail.history[0].actor_id.text, "人".repeat(240));
        assert_eq!(detail.history[0].actor_id.total_chars, 241);
        assert_eq!(detail.history[0].reason.text, "由".repeat(1000));
        let serialized = serde_json::to_string(&detail).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&serialized).unwrap()["status"],
            "completed"
        );
        for excluded in [
            "PRIVATE-",
            "private-",
            "operation_id",
            "request_digest",
            "\"tasks\"",
            "\"authority\"",
        ] {
            assert!(
                !serialized.contains(excluded),
                "private metadata: {excluded}"
            );
        }
        snapshot.sources.clear();
        assert!(task_detail(&snapshot, &selected_key)
            .unwrap()
            .original
            .is_none());
        assert!(task_detail(&snapshot, &key("unknown")).is_none());
    }
}
