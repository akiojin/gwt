//! `gwt hook coordination-event <event>` — append meaningful coordination
//! summaries to the shared Board timeline.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

use gwt_agent::{Session, GWT_SESSION_ID_ENV};
use gwt_core::{
    coordination::{post_entry, AuthorKind, BoardEntry, BoardEntryKind},
    paths::gwt_cache_dir,
};
use gwt_github::{Cache, IssueNumber};
use serde::{Deserialize, Serialize};

use super::HookError;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct IssueBranchLinkStore {
    branches: HashMap<String, u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IssueContext {
    number: u64,
    title: String,
    is_spec: bool,
}

pub fn handle(event: &str) -> Result<(), HookError> {
    let sessions_dir = gwt_core::paths::gwt_sessions_dir();
    let Some(session) = current_session_from_env(&sessions_dir)? else {
        return Ok(());
    };

    let cache_root =
        crate::issue_cache::issue_cache_root_for_repo_path_or_detached(&session.worktree_path);
    let linkage_store_path = default_issue_linkage_store_path(&session.worktree_path);
    sync_coordination_for_session_with_paths(
        &session,
        event,
        &cache_root,
        linkage_store_path.as_deref(),
    )
}

fn current_session_from_env(sessions_dir: &Path) -> io::Result<Option<Session>> {
    let Some(session_id) = std::env::var_os(GWT_SESSION_ID_ENV) else {
        return Ok(None);
    };
    let path = sessions_dir.join(format!("{}.toml", session_id.to_string_lossy()));
    if !path.exists() {
        return Ok(None);
    }
    Session::load(&path).map(Some)
}

fn sync_coordination_for_session_with_paths(
    session: &Session,
    event: &str,
    cache_root: &Path,
    linkage_store_path: Option<&Path>,
) -> Result<(), HookError> {
    let Some(entry) =
        board_entry_for_event_with_paths(session, event, cache_root, linkage_store_path)?
    else {
        return Ok(());
    };

    post_entry(&session.worktree_path, entry).map_err(coordination_as_hook_error)?;
    Ok(())
}

fn board_entry_for_event_with_paths(
    session: &Session,
    event: &str,
    cache_root: &Path,
    linkage_store_path: Option<&Path>,
) -> Result<Option<BoardEntry>, HookError> {
    let Some(context) = resolve_issue_context_with_paths(session, cache_root, linkage_store_path)
    else {
        return Ok(None);
    };

    let owner_label = issue_context_label(&context);
    let branch_suffix = if session.branch.trim().is_empty() {
        String::new()
    } else {
        format!(" ({})", session.branch)
    };
    let (state, body) = match event {
        "SessionStart" => (
            Some("started".to_string()),
            format!(
                "{} started work on {}{}",
                session.display_name, owner_label, branch_suffix
            ),
        ),
        "Stop" => (
            Some("ready".to_string()),
            format!(
                "{} is ready for the next instruction on {}{}",
                session.display_name, owner_label, branch_suffix
            ),
        ),
        _ => return Ok(None),
    };

    Ok(Some(BoardEntry::new(
        AuthorKind::Agent,
        session.display_name.clone(),
        BoardEntryKind::Status,
        body,
        state,
        None,
        Vec::new(),
        vec![context.number.to_string()],
    )))
}

fn resolve_issue_context_with_paths(
    session: &Session,
    cache_root: &Path,
    linkage_store_path: Option<&Path>,
) -> Option<IssueContext> {
    let issue_number = session
        .linked_issue_number
        .or_else(|| linked_issue_number_for_branch(session, linkage_store_path))?;

    let cache = Cache::new(cache_root.to_path_buf());
    let entry = cache.load_entry(IssueNumber(issue_number));
    let title = entry
        .as_ref()
        .map(|entry| entry.snapshot.title.trim())
        .filter(|title| !title.is_empty())
        .map(str::to_string)
        .unwrap_or_default();
    let is_spec = entry.as_ref().is_some_and(|entry| {
        entry
            .snapshot
            .labels
            .iter()
            .any(|label| label == "gwt-spec")
    });

    Some(IssueContext {
        number: issue_number,
        title,
        is_spec,
    })
}

fn linked_issue_number_for_branch(
    session: &Session,
    linkage_store_path: Option<&Path>,
) -> Option<u64> {
    let branch = session.branch.trim();
    if branch.is_empty() {
        return None;
    }
    let path = linkage_store_path?;
    let bytes = std::fs::read(path).ok()?;
    let store: IssueBranchLinkStore = serde_json::from_slice(&bytes).ok()?;
    store.branches.get(branch).copied()
}

fn issue_context_label(context: &IssueContext) -> String {
    let prefix = if context.is_spec {
        format!("SPEC #{}", context.number)
    } else {
        format!("Issue #{}", context.number)
    };

    if context.title.is_empty() {
        prefix
    } else {
        format!("{prefix} {}", context.title)
    }
}

fn default_issue_linkage_store_path(repo_path: &Path) -> Option<PathBuf> {
    let repo_hash = crate::index_worker::detect_repo_hash(repo_path)?;
    Some(
        gwt_cache_dir()
            .join("issue-links")
            .join(format!("{}.json", repo_hash.as_str())),
    )
}

fn coordination_as_hook_error(err: gwt_core::GwtError) -> HookError {
    HookError::Io(io::Error::other(err.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use gwt_agent::{AgentId, Session};
    use gwt_core::coordination::{
        coordination_events_path, load_snapshot, BoardEntry, BoardEntryKind,
    };
    use gwt_github::{CommentSnapshot, IssueSnapshot, IssueState, UpdatedAt};

    fn sample_issue_snapshot(number: u64, title: &str, labels: Vec<&str>) -> IssueSnapshot {
        IssueSnapshot {
            number: IssueNumber(number),
            title: title.to_string(),
            body: String::new(),
            labels: labels.into_iter().map(str::to_string).collect(),
            state: IssueState::Open,
            updated_at: UpdatedAt::new("2026-04-14T00:00:00Z".to_string()),
            comments: Vec::<CommentSnapshot>::new(),
        }
    }

    #[test]
    fn sync_coordination_for_session_appends_spec_summary_when_issue_context_exists() {
        let dir = tempfile::tempdir().unwrap();
        let cache_root = dir.path().join("issue-cache");
        let cache = Cache::new(cache_root.clone());
        cache
            .write_snapshot(&sample_issue_snapshot(
                1974,
                "Coordination Domain — Shared Board",
                vec!["gwt-spec"],
            ))
            .unwrap();

        let mut session = Session::new(dir.path(), "feature/spec-1974", AgentId::Codex);
        session.linked_issue_number = Some(1974);

        sync_coordination_for_session_with_paths(&session, "SessionStart", &cache_root, None)
            .unwrap();

        let snapshot = load_snapshot(dir.path()).unwrap();
        assert_eq!(snapshot.board.entries.len(), 1);
        let entry = &snapshot.board.entries[0];
        assert_eq!(entry.kind, BoardEntryKind::Status);
        assert_eq!(entry.author, "Codex");
        assert_eq!(entry.state.as_deref(), Some("started"));
        assert!(entry.body.contains("SPEC #1974"));
        assert!(entry.body.contains("Coordination Domain"));
        assert!(entry.body.contains("feature/spec-1974"));
        assert_eq!(entry.related_owners, vec!["1974".to_string()]);
    }

    #[test]
    fn sync_coordination_for_session_uses_branch_linkage_store_when_session_link_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let cache_root = dir.path().join("issue-cache");
        let cache = Cache::new(cache_root.clone());
        cache
            .write_snapshot(&sample_issue_snapshot(
                1776,
                "Launch Agent issue linkage",
                vec!["ux"],
            ))
            .unwrap();
        let linkage_store_path = dir.path().join("issue-links.json");
        std::fs::write(
            &linkage_store_path,
            serde_json::to_vec_pretty(&IssueBranchLinkStore {
                branches: HashMap::from([("feature/issue-link".to_string(), 1776)]),
            })
            .unwrap(),
        )
        .unwrap();

        let session = Session::new(dir.path(), "feature/issue-link", AgentId::Codex);

        sync_coordination_for_session_with_paths(
            &session,
            "Stop",
            &cache_root,
            Some(linkage_store_path.as_path()),
        )
        .unwrap();

        let snapshot = load_snapshot(dir.path()).unwrap();
        assert_eq!(snapshot.board.entries.len(), 1);
        let entry = &snapshot.board.entries[0];
        assert_eq!(entry.state.as_deref(), Some("ready"));
        assert!(entry.body.contains("Issue #1776"));
        assert!(entry.body.contains("Launch Agent issue linkage"));
    }

    #[test]
    fn sync_coordination_for_session_uses_issue_number_only_when_cache_entry_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let cache_root = dir.path().join("issue-cache");
        let mut session = Session::new(dir.path(), "feature/missing-cache", AgentId::Codex);
        session.linked_issue_number = Some(2042);

        sync_coordination_for_session_with_paths(&session, "Stop", &cache_root, None).unwrap();

        let snapshot = load_snapshot(dir.path()).unwrap();
        assert_eq!(snapshot.board.entries.len(), 1);
        let entry = &snapshot.board.entries[0];
        assert!(entry.body.contains("Issue #2042"));
        assert!(!entry.body.contains("Issue #2042 Issue #2042"));
    }

    #[test]
    fn sync_coordination_for_session_skips_lifecycle_summary_without_issue_context() {
        let dir = tempfile::tempdir().unwrap();
        let cache_root = dir.path().join("issue-cache");
        let session = Session::new(dir.path(), "feature/demo", AgentId::Codex);

        sync_coordination_for_session_with_paths(&session, "SessionStart", &cache_root, None)
            .unwrap();

        let snapshot = load_snapshot(dir.path()).unwrap();
        assert!(snapshot.board.entries.is_empty());
    }

    #[test]
    fn sync_coordination_for_session_accepts_legacy_board_post_entries() {
        let dir = tempfile::tempdir().unwrap();
        let cache_root = dir.path().join("issue-cache");
        let cache = Cache::new(cache_root.clone());
        cache
            .write_snapshot(&sample_issue_snapshot(
                1989,
                "Legacy coordination migration",
                vec![],
            ))
            .unwrap();

        let mut legacy_entry = BoardEntry::new(
            AuthorKind::Agent,
            "Codex",
            BoardEntryKind::Status,
            "legacy waiting",
            Some("waiting_input".into()),
            None,
            vec![],
            vec![],
        );
        legacy_entry.created_at = chrono::Utc.with_ymd_and_hms(2026, 4, 14, 0, 0, 0).unwrap();
        legacy_entry.updated_at = legacy_entry.created_at;
        std::fs::create_dir_all(dir.path().join(".gwt/coordination")).unwrap();
        std::fs::write(
            coordination_events_path(dir.path()),
            format!(
                "{}\n",
                serde_json::json!({
                    "type": "board_post",
                    "entry": legacy_entry,
                })
            ),
        )
        .unwrap();

        let mut session = Session::new(dir.path(), "bugfix/issue-1989", AgentId::Codex);
        session.linked_issue_number = Some(1989);

        sync_coordination_for_session_with_paths(&session, "SessionStart", &cache_root, None)
            .unwrap();

        let snapshot = load_snapshot(dir.path()).unwrap();
        assert_eq!(snapshot.board.entries.len(), 2);
        assert_eq!(snapshot.board.entries[0].body, "legacy waiting");
        assert!(snapshot.board.entries[1].body.contains("Issue #1989"));
    }

    #[test]
    fn sync_coordination_for_stop_accepts_legacy_board_post_entries() {
        let dir = tempfile::tempdir().unwrap();
        let cache_root = dir.path().join("issue-cache");
        let cache = Cache::new(cache_root.clone());
        cache
            .write_snapshot(&sample_issue_snapshot(
                1989,
                "Legacy coordination migration",
                vec![],
            ))
            .unwrap();

        let mut legacy_entry = BoardEntry::new(
            AuthorKind::Agent,
            "Codex",
            BoardEntryKind::Status,
            "legacy waiting",
            Some("waiting_input".into()),
            None,
            vec![],
            vec![],
        );
        legacy_entry.created_at = chrono::Utc.with_ymd_and_hms(2026, 4, 14, 0, 0, 0).unwrap();
        legacy_entry.updated_at = legacy_entry.created_at;
        std::fs::create_dir_all(dir.path().join(".gwt/coordination")).unwrap();
        std::fs::write(
            coordination_events_path(dir.path()),
            format!(
                "{}\n",
                serde_json::json!({
                    "type": "board_post",
                    "entry": legacy_entry,
                })
            ),
        )
        .unwrap();

        let mut session = Session::new(dir.path(), "bugfix/issue-1989", AgentId::Codex);
        session.linked_issue_number = Some(1989);

        sync_coordination_for_session_with_paths(&session, "Stop", &cache_root, None).unwrap();

        let snapshot = load_snapshot(dir.path()).unwrap();
        assert_eq!(snapshot.board.entries.len(), 2);
        assert_eq!(snapshot.board.entries[0].body, "legacy waiting");
        assert_eq!(snapshot.board.entries[1].state.as_deref(), Some("ready"));
        assert!(snapshot.board.entries[1].body.contains("Issue #1989"));
        assert!(snapshot.board.entries[1].body.contains("next instruction"));
    }
}
