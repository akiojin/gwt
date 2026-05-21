use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    thread,
};

use crate::{AppEventProxy, OutboundEvent, UserEvent};
use gwt::{
    hydrate_branch_entries_with_active_sessions, list_branch_inventory, BackendEvent,
    BranchEntriesPhase, BranchListEntry, BranchScope,
};

pub fn spawn_branch_load_async(
    proxy: AppEventProxy,
    window_id: String,
    project_root: PathBuf,
    active_session_branches: HashSet<String>,
) {
    thread::spawn(move || {
        dispatch_branch_load_progressive(
            &proxy,
            &window_id,
            &project_root,
            &active_session_branches,
        );
    });
}

pub fn preferred_issue_launch_branch(entries: &[BranchListEntry]) -> Option<String> {
    let mut locals = entries
        .iter()
        .filter(|entry| entry.scope == BranchScope::Local)
        .collect::<Vec<_>>();
    locals.sort_by(|left, right| left.name.cmp(&right.name));

    for preferred in ["develop", "main", "master"] {
        if let Some(entry) = locals.iter().find(|entry| entry.name == preferred) {
            return Some(entry.name.clone());
        }
    }
    if let Some(entry) = locals.iter().find(|entry| entry.is_head) {
        return Some(entry.name.clone());
    }
    locals.first().map(|entry| entry.name.clone())
}

fn dispatch_branch_load_progressive(
    proxy: &AppEventProxy,
    window_id: &str,
    project_root: &Path,
    active_session_branches: &HashSet<String>,
) {
    match list_branch_inventory(project_root) {
        Ok(entries) => {
            dispatch_async_events(
                proxy,
                vec![OutboundEvent::broadcast(BackendEvent::BranchEntries {
                    id: window_id.to_string(),
                    phase: BranchEntriesPhase::Inventory,
                    entries: entries.clone(),
                })],
            );
            match hydrate_branch_entries_with_active_sessions(
                project_root,
                entries,
                active_session_branches,
            ) {
                Ok(entries) => dispatch_async_events(
                    proxy,
                    vec![OutboundEvent::broadcast(BackendEvent::BranchEntries {
                        id: window_id.to_string(),
                        phase: BranchEntriesPhase::Hydrated,
                        entries,
                    })],
                ),
                Err(error) => dispatch_async_events(
                    proxy,
                    vec![OutboundEvent::broadcast(BackendEvent::BranchError {
                        id: window_id.to_string(),
                        message: error.to_string(),
                    })],
                ),
            }
        }
        Err(error) => dispatch_async_events(
            proxy,
            vec![OutboundEvent::broadcast(BackendEvent::BranchError {
                id: window_id.to_string(),
                message: error.to_string(),
            })],
        ),
    }
}

fn dispatch_async_events(proxy: &AppEventProxy, events: Vec<OutboundEvent>) {
    proxy.send(UserEvent::Dispatch(events));
}

#[cfg(test)]
mod tests {
    use gwt::{BranchCleanupInfo, BranchListEntry};

    use super::{preferred_issue_launch_branch, BranchScope};

    fn local_branch(name: &str, is_head: bool) -> BranchListEntry {
        BranchListEntry {
            name: name.to_string(),
            scope: BranchScope::Local,
            is_head,
            upstream: None,
            ahead: 0,
            behind: 0,
            last_commit_date: None,
            cleanup_ready: true,
            cleanup: BranchCleanupInfo::default(),
        }
    }

    fn remote_branch(name: &str) -> BranchListEntry {
        BranchListEntry {
            name: name.to_string(),
            scope: BranchScope::Remote,
            is_head: false,
            upstream: None,
            ahead: 0,
            behind: 0,
            last_commit_date: None,
            cleanup_ready: true,
            cleanup: BranchCleanupInfo::default(),
        }
    }

    #[test]
    fn preferred_issue_launch_branch_prefers_develop_then_head_then_first_local() {
        let entries = vec![
            local_branch("feature/demo", true),
            local_branch("develop", false),
        ];

        assert_eq!(
            preferred_issue_launch_branch(&entries),
            Some("develop".to_string())
        );

        let head_only = vec![local_branch("feature/demo", true)];
        assert_eq!(
            preferred_issue_launch_branch(&head_only),
            Some("feature/demo".to_string())
        );
    }

    #[test]
    fn preferred_issue_launch_branch_ignores_remote_only_entries() {
        let entries = vec![
            remote_branch("origin/develop"),
            remote_branch("origin/feature/demo"),
        ];

        assert_eq!(preferred_issue_launch_branch(&entries), None);
    }

    #[test]
    fn async_branch_load_results_are_broadcast_not_bound_to_a_stale_websocket_client() {
        let source = include_str!("repo_browser.rs");
        let production_source = source
            .split("#[cfg(test)]")
            .next()
            .expect("production source before tests");

        assert_async_branch_event_is_broadcast_only(production_source, "BranchEntries");
        assert_async_branch_event_is_broadcast_only(production_source, "BranchError");
    }

    fn assert_async_branch_event_is_broadcast_only(production_source: &str, event: &str) {
        let event_marker = format!("BackendEvent::{event}");
        let positions = production_source
            .match_indices(&event_marker)
            .map(|(index, _)| index)
            .collect::<Vec<_>>();

        assert!(
            !positions.is_empty(),
            "async branch {event} must still be dispatched"
        );

        for position in positions {
            let prefix = &production_source[..position];
            let last_broadcast = prefix.rfind("OutboundEvent::broadcast(");
            let last_reply = prefix.rfind("OutboundEvent::reply(");

            assert!(
                last_broadcast.is_some() && last_broadcast > last_reply,
                "async branch {event} must be broadcast by window id, not targeted to a transient websocket client id",
            );
        }
    }
}
