//! Public-safe, read-only RecoveryStore projection for the Recovery Center.

use std::path::Path;

use gwt_core::recovery::{RecoveryRecord, RecoveryState, RecoveryStore};

use super::{
    same_worktree_path, AppRuntime, BackendEvent, OutboundEvent, ProjectTabRuntime,
    RecoveryCenterAction,
};

const SUMMARY_LIMIT: usize = 240;
const TITLE_LIMIT: usize = 96;

impl AppRuntime {
    pub(super) fn load_recovery_center_events(
        &mut self,
        client_id: &str,
        request_id: &str,
    ) -> Vec<OutboundEvent> {
        let generation = self.next_recovery_center_generation();
        self.recovery_center_handles.clear();

        let tab = self
            .active_tab_id
            .as_ref()
            .and_then(|active_id| self.tabs.iter().find(|tab| tab.id == *active_id).cloned());
        let Some(tab) = tab else {
            return vec![recovery_center_state_event(
                client_id,
                request_id,
                generation,
                gwt::RecoveryCenterLoadStatus::Error,
                Vec::new(),
            )];
        };

        let loaded = load_project_recovery_records(&self.sessions_dir, &tab);
        let Ok(records) = loaded else {
            return vec![recovery_center_state_event(
                client_id,
                request_id,
                generation,
                gwt::RecoveryCenterLoadStatus::Error,
                Vec::new(),
            )];
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
            self.recovery_center_handles.insert(
                handle.clone(),
                RecoveryCenterAction {
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
        client_id: &str,
        request_id: &str,
        generation: u64,
        action_handle: &str,
    ) -> Vec<OutboundEvent> {
        let board_entry_id = self
            .recovery_center_handles
            .get(action_handle)
            .filter(|action| action.generation == generation)
            .and_then(|action| action.board_entry_id.clone());
        vec![OutboundEvent::reply(
            client_id,
            BackendEvent::RecoveryCenterBoardEntry {
                request_id: request_id.to_string(),
                generation,
                board_entry_id,
            },
        )]
    }

    fn next_recovery_center_generation(&mut self) -> u64 {
        self.recovery_center_generation = self.recovery_center_generation.wrapping_add(1).max(1);
        self.recovery_center_generation
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
) -> Result<Vec<RecoveryRecord>, ()> {
    let entries = std::fs::read_dir(sessions_dir).map_err(|_| ())?;
    let mut records = Vec::new();
    for entry in entries {
        let path = entry.map_err(|_| ())?.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("toml") {
            continue;
        }
        let session = gwt_agent::Session::load(&path).map_err(|_| ())?;
        if !session_belongs_to_project(&session, tab) {
            continue;
        }

        // `RecoveryStore::for_repo` creates its private directory hierarchy.
        // Avoid calling it for Sessions that have never owned a recovery so
        // this read surface cannot materialize an empty Store as a side effect.
        let intents_root = gwt_core::paths::gwt_project_dir_for_repo_path(&session.worktree_path)
            .join("recovery")
            .join("intents");
        if !intents_root.is_dir() {
            continue;
        }
        let store = RecoveryStore::for_repo(&session.worktree_path, &session.id).map_err(|_| ())?;
        records.extend(store.list().map_err(|_| ())?);
    }
    records.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| left.intent.entry.id.cmp(&right.intent.entry.id))
    });
    Ok(records)
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
