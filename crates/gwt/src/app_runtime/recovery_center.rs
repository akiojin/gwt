//! Public-safe, read-only RecoveryStore projection for the Recovery Center.

use std::path::Path;

use gwt_core::recovery::{RecoveryRecord, RecoveryState, RecoveryStore};

use super::{
    same_worktree_path, AppRuntime, BackendEvent, OutboundEvent, ProjectTabRuntime,
    RecoveryCenterAction,
};

const SUMMARY_LIMIT: usize = 240;
const TITLE_LIMIT: usize = 96;

pub(super) fn rejected_action_events(
    client_id: &str,
    event: &gwt::FrontendEvent,
) -> Option<Vec<OutboundEvent>> {
    let gwt::FrontendEvent::OpenRecoveryCenterBoardEntry {
        request_id,
        generation,
        action_handle,
    } = event
    else {
        return None;
    };
    Some(vec![OutboundEvent::reply(client_id, BackendEvent::RecoveryCenterBoardEntry {
        request_id: request_id.clone(), generation: *generation, board_entry_id: None,
        error: Some(format!("Recovery action handle {action_handle} is invalid: project tab is closed or replaced")),
    })])
}

impl AppRuntime {
    pub(super) fn load_recovery_center_events(
        &mut self,
        context: &super::ProjectContext,
        client_id: &str,
        request_id: &str,
    ) -> Vec<OutboundEvent> {
        let Some(state) = self.project_state_mut(context) else {
            return Vec::new();
        };
        state.recovery_center_generation = state.recovery_center_generation.wrapping_add(1).max(1);
        let generation = state.recovery_center_generation;
        state
            .recovery_center_handles
            .retain(|_, action| action.client_id != client_id);

        let tab = self.tab(&context.tab_id).cloned();
        let Some(tab) = tab else {
            return vec![recovery_center_state_event(
                client_id,
                request_id,
                generation,
                gwt::RecoveryCenterLoadStatus::Error,
                Vec::new(),
            )];
        };

        let records = match load_project_recovery_records(&self.sessions_dir, &tab) {
            Ok(Some(records)) => records,
            result => {
                return vec![recovery_center_state_event(
                    client_id,
                    request_id,
                    generation,
                    if result.is_ok() {
                        gwt::RecoveryCenterLoadStatus::NotFound
                    } else {
                        gwt::RecoveryCenterLoadStatus::Error
                    },
                    Vec::new(),
                )]
            }
        };

        let mut items = Vec::with_capacity(records.len());
        for record in records {
            let handle = format!("rc-{}", uuid::Uuid::new_v4().simple());
            let board_entry_id = (record.state == RecoveryState::Acknowledged)
                .then(|| {
                    record
                        .acknowledgement
                        .as_ref()
                        .map(|acknowledgement| acknowledgement.entry_id.clone())
                })
                .flatten();
            self.project_state_mut(context)
                .expect("current recovery project")
                .recovery_center_handles
                .insert(
                    handle.clone(),
                    RecoveryCenterAction {
                        client_id: client_id.to_string(),
                        generation,
                        board_entry_id,
                    },
                );
            items.push(project_recovery_record(record, handle));
        }

        vec![recovery_center_state_event(
            client_id,
            request_id,
            generation,
            gwt::RecoveryCenterLoadStatus::Ready,
            items,
        )]
    }

    pub(super) fn open_recovery_center_board_entry_events(
        &self,
        context: &super::ProjectContext,
        client_id: &str,
        request_id: &str,
        generation: u64,
        action_handle: &str,
    ) -> Vec<OutboundEvent> {
        let action = self
            .project_state(context)
            .and_then(|state| state.recovery_center_handles.get(action_handle));
        let reason = if !self.project_context_is_current(context) {
            Some("project tab is closed or replaced")
        } else {
            match action {
                None => Some("unknown or expired handle for this client/project"),
                Some(action) if action.client_id != client_id => Some("client mismatch"),
                Some(action) if action.generation != generation => Some("generation mismatch"),
                _ => None,
            }
        };
        let board_entry_id = reason
            .is_none()
            .then(|| action.and_then(|action| action.board_entry_id.clone()))
            .flatten();
        vec![OutboundEvent::reply(
            client_id,
            BackendEvent::RecoveryCenterBoardEntry {
                request_id: request_id.to_string(),
                generation,
                board_entry_id,
                error: reason.map(|reason| {
                    format!("Recovery action handle {action_handle} is invalid: {reason}")
                }),
            },
        )]
    }
}

fn recovery_center_state_event(
    client_id: &str,
    request_id: &str,
    generation: u64,
    status: gwt::RecoveryCenterLoadStatus,
    items: Vec<gwt::RecoveryCenterItemView>,
) -> OutboundEvent {
    OutboundEvent::reply(
        client_id,
        BackendEvent::RecoveryCenterState {
            request_id: request_id.to_string(),
            generation,
            status,
            items,
        },
    )
}

fn load_project_recovery_records(
    sessions_dir: &Path,
    tab: &ProjectTabRuntime,
) -> Result<Option<Vec<RecoveryRecord>>, ()> {
    let sessions = gwt_agent::session_ledger::load_sessions(sessions_dir).map_err(|_| ())?;
    let mut records = Vec::new();
    let mut found_store = false;
    let project_hash = gwt_core::paths::project_scope_hash(&tab.project_root);
    for session in sessions {
        if !session_belongs_to_project(&session, tab) {
            continue;
        }

        // Saved identity survives worktree deletion. Never replace a saved key
        // with path fallback, which could silently select another Store.
        let saved_hash = match session.repo_hash.as_deref() {
            Some(hash) => gwt_core::repo_hash::RepoHash::parse(hash).map_err(|_| ())?,
            None if session.worktree_path.exists() => {
                gwt_core::paths::project_scope_hash(&session.worktree_path)
            }
            None => continue,
        };
        if saved_hash != project_hash {
            return Err(());
        }
        let Some(store) =
            RecoveryStore::open_for_project_if_exists(&saved_hash, &session.id).map_err(|_| ())?
        else {
            continue;
        };
        found_store = true;
        records.extend(store.list_read_only().map_err(|_| ())?);
    }
    records.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| left.intent.entry.id.cmp(&right.intent.entry.id))
    });
    Ok(found_store.then_some(records))
}

fn session_belongs_to_project(session: &gwt_agent::Session, tab: &ProjectTabRuntime) -> bool {
    if tab.kind != gwt::ProjectKind::Git {
        return false;
    }
    if same_worktree_path(&tab.project_root, &session.worktree_path) {
        return true;
    }
    if session.project_state_root.as_ref().is_some_and(|root| {
        same_worktree_path(root, &tab.project_root)
            || same_worktree_path(root, &tab.main_worktree_root())
    }) {
        return true;
    }
    if session.repo_hash.as_deref()
        == Some(gwt_core::paths::project_scope_hash(&tab.project_root).as_str())
    {
        return true;
    }
    gwt_git::worktree::main_worktree_root(&session.worktree_path)
        .ok()
        .is_some_and(|root| same_worktree_path(&root, &tab.main_worktree_root()))
}

fn project_recovery_record(
    record: RecoveryRecord,
    action_handle: String,
) -> gwt::RecoveryCenterItemView {
    let state = match record.state {
        RecoveryState::Pending => gwt::RecoveryCenterItemState::Pending,
        RecoveryState::Acknowledged => gwt::RecoveryCenterItemState::Acknowledged,
        RecoveryState::Conflicted => gwt::RecoveryCenterItemState::Conflicted,
    };
    let entry = record.intent.entry;
    let title = entry
        .title
        .as_deref()
        .or(entry.title_summary.as_deref())
        .map(|value| public_preview(value, TITLE_LIMIT))
        .filter(|value| !value.is_empty());
    let summary_source = entry
        .title_summary
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(entry.body.as_str());
    let summary = public_preview(summary_source, SUMMARY_LIMIT);
    gwt::RecoveryCenterItemView {
        action_handle,
        state,
        worktree_form: record.intent.worktree_form,
        title,
        summary: if summary.is_empty() {
            "Recovery delivery".to_string()
        } else {
            summary
        },
        updated_at: record.updated_at.to_rfc3339(),
    }
}

fn public_preview(value: &str, limit: usize) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(limit)
        .collect()
}
