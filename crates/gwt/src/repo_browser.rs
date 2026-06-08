use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    thread,
};

use crate::{AppEventProxy, OutboundEvent, UserEvent};
use gwt::{
    hydrate_branch_entries_with_active_sessions, list_branch_inventory, BackendEvent,
    BranchEntriesPhase, BranchListEntry, BranchResumeInfo, BranchScope,
};

pub fn spawn_branch_load_async(
    proxy: AppEventProxy,
    window_id: String,
    project_root: PathBuf,
    active_session_branches: HashSet<String>,
    sessions_dir: PathBuf,
) {
    thread::spawn(move || {
        // Load resume candidates fresh from disk (off the main thread) rather
        // than from the GUI's in-memory session cache, so branch Resume
        // availability reflects session TOMLs updated out-of-process by the
        // hook CLI after launch (#2995).
        let resume_sessions = gwt::launch_wizard::load_sessions(&sessions_dir);
        // Refresh the main-thread Launch Wizard cache from this same disk-fresh
        // load BEFORE the branch entries (and their enabled Resume buttons)
        // reach the client. The event queue is FIFO, so a later Resume click
        // resolves against fresh data via the in-memory cache without ever
        // scanning the session directory on the UI thread (#2995).
        proxy.send(UserEvent::RefreshLaunchWizardSessions(
            resume_sessions.clone(),
        ));
        dispatch_branch_load_progressive(
            &proxy,
            &window_id,
            &project_root,
            &active_session_branches,
            &resume_sessions,
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
    resume_sessions: &[gwt_agent::Session],
) {
    match list_branch_inventory(project_root) {
        Ok(mut entries) => {
            apply_branch_resume_availability(project_root, &mut entries, resume_sessions);
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
                Ok(mut entries) => {
                    apply_branch_resume_availability(project_root, &mut entries, resume_sessions);
                    dispatch_async_events(
                        proxy,
                        vec![OutboundEvent::broadcast(BackendEvent::BranchEntries {
                            id: window_id.to_string(),
                            phase: BranchEntriesPhase::Hydrated,
                            entries,
                        })],
                    )
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
        Err(error) => dispatch_async_events(
            proxy,
            vec![OutboundEvent::broadcast(BackendEvent::BranchError {
                id: window_id.to_string(),
                message: error.to_string(),
            })],
        ),
    }
}

fn apply_branch_resume_availability(
    project_root: &Path,
    entries: &mut [BranchListEntry],
    resume_sessions: &[gwt_agent::Session],
) {
    for entry in entries {
        let has_resumable_session = gwt::launch_wizard::quick_start_entries_from_sessions(
            project_root,
            &entry.name,
            resume_sessions,
        )
        .into_iter()
        .any(|quick_start| quick_start.resume_session_id.is_some());
        entry.resume = if has_resumable_session {
            BranchResumeInfo::available()
        } else {
            BranchResumeInfo::unavailable()
        };
    }
}

fn dispatch_async_events(proxy: &AppEventProxy, events: Vec<OutboundEvent>) {
    proxy.send(UserEvent::Dispatch(events));
}

#[cfg(test)]
mod tests {
    use gwt::{BranchCleanupInfo, BranchListEntry};
    use tempfile::tempdir;

    use super::{
        apply_branch_resume_availability, preferred_issue_launch_branch, BranchResumeInfo,
        BranchScope,
    };

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
            resume: BranchResumeInfo::unavailable(),
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
            resume: BranchResumeInfo::unavailable(),
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
    fn branch_resume_availability_marks_only_resumable_branch_sessions() {
        let repo = tempdir().expect("repo");
        let mut session = gwt_agent::Session::new(
            repo.path(),
            "feature/resumable",
            gwt_agent::AgentId::ClaudeCode,
        );
        session.agent_session_id = Some("claude-session-1".to_string());
        let sessions = vec![session];
        let mut entries = vec![
            local_branch("feature/resumable", false),
            local_branch("feature/no-session", false),
        ];

        apply_branch_resume_availability(repo.path(), &mut entries, &sessions);

        assert_eq!(entries[0].resume, BranchResumeInfo::available());
        assert_eq!(entries[1].resume, BranchResumeInfo::unavailable());
    }

    #[test]
    fn branch_resume_availability_uses_disk_fresh_loaded_sessions() {
        // #2995: the branch load reads sessions fresh from disk via
        // gwt::launch_wizard::load_sessions, so a session TOML persisted after
        // the in-memory cache was built is still marked resumable on the next
        // branch load (no process restart needed).
        let repo = tempfile::tempdir().expect("repo");
        let sessions_dir = tempfile::tempdir().expect("sessions");
        let mut session = gwt_agent::Session::new(
            repo.path(),
            "feature/disk-fresh",
            gwt_agent::AgentId::ClaudeCode,
        );
        session.agent_session_id = Some("claude-disk-1".to_string());
        session.save(sessions_dir.path()).expect("save session");

        let loaded = gwt::launch_wizard::load_sessions(sessions_dir.path());
        let mut entries = vec![local_branch("feature/disk-fresh", false)];
        apply_branch_resume_availability(repo.path(), &mut entries, &loaded);

        assert_eq!(entries[0].resume, BranchResumeInfo::available());
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
