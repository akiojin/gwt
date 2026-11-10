//! Bounded, read-only summaries. TaskStore remains the tracking authority.

use gwt_core::pm_request_tasks::{TaskStatus, TaskStore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
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
    pub title: String,
    pub status: TaskStatus,
    pub assignee: Option<String>,
    pub wait_reason: Option<String>,
    pub next_action: Option<String>,
    pub review_required: bool,
}

fn key(id: &str) -> String {
    format!("{:x}", Sha256::digest(id.as_bytes()))
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
            title: short(&task.draft.title),
            status: task.status,
            assignee: task.draft.assignee.as_deref().map(short),
            wait_reason: task.wait_reason.as_deref().map(short),
            next_action: task.next_action.as_deref().map(short),
            review_required: task.review_required,
        })
        .collect();
    PmTaskOverview::Ready {
        revision: state.revision,
        total: state.tasks.len(),
        pending_intake: state.pending.len(),
        tasks,
    }
}
