//! Optional Discussion mirroring. Local Board operations never wait for GitHub.
use std::{collections::BTreeSet, fs::OpenOptions, io::Write, path::Path};

use chrono::{DateTime, Utc};
use fs2::FileExt;
use gwt_config::{board_config::GitHubDiscussionConfig, BoardProviderKind, ProjectBoardConfig};
use gwt_core::coordination::{self, BoardEntry};
use gwt_github::{
    client::http::{DiscussionCommentsPage, HttpIssueClient, HttpTransport},
    ApiError,
};
use serde::{Deserialize, Serialize};

const MARKER: &str = "<!-- gwt-board-sync:v1\n";
const STATE_FILE: &str = "discussion-sync.json";
const POLL_SECONDS: i64 = 60;
const RETRY_SECONDS: i64 = 300;
const MAX_COMMENT_BYTES: usize = 60_000;

trait DiscussionTransport {
    fn read(&self, number: u64, cursor: Option<&str>) -> Result<DiscussionCommentsPage, ApiError>;
    fn post(&self, discussion_id: &str, body: &str) -> Result<(), ApiError>;
}

impl<T: HttpTransport> DiscussionTransport for HttpIssueClient<T> {
    fn read(&self, number: u64, cursor: Option<&str>) -> Result<DiscussionCommentsPage, ApiError> {
        self.discussion_comments(number, cursor)
    }
    fn post(&self, discussion_id: &str, body: &str) -> Result<(), ApiError> {
        self.add_discussion_comment(discussion_id, body)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct SyncState {
    binding: GitHubDiscussionConfig,
    cursor: Option<String>,
    delivered: BTreeSet<String>,
    next_attempt: DateTime<Utc>,
    last_success: Option<DateTime<Utc>>,
    last_error: Option<String>,
    sent: u64,
    received: u64,
}

impl SyncState {
    fn new(binding: GitHubDiscussionConfig) -> Self {
        Self {
            binding,
            cursor: None,
            delivered: BTreeSet::new(),
            next_attempt: DateTime::UNIX_EPOCH,
            last_success: None,
            last_error: None,
            sent: 0,
            received: 0,
        }
    }
}

/// Machine-local delivery diagnostics; remote failures never become local Board errors.
#[derive(Debug, Serialize)]
pub struct SyncReport {
    status: &'static str,
    last_error: Option<String>,
    last_success: Option<DateTime<Utc>>,
    next_attempt: Option<DateTime<Utc>>,
    sent: u64,
    received: u64,
}

impl SyncReport {
    fn from_state(status: &'static str, state: Option<&SyncState>) -> Self {
        Self {
            status,
            last_error: state.and_then(|s| s.last_error.clone()),
            last_success: state.and_then(|s| s.last_success),
            next_attempt: state.map(|s| s.next_attempt),
            sent: state.map_or(0, |s| s.sent),
            received: state.map_or(0, |s| s.received),
        }
    }
}

fn binding_for(root: &Path) -> Option<GitHubDiscussionConfig> {
    let config =
        ProjectBoardConfig::load_from_work_dir(&gwt_core::paths::gwt_repo_local_work_dir(root));
    if config
        .provider
        .unwrap_or_else(crate::board_provider::current_kind)
        != BoardProviderKind::Local
    {
        return None;
    }
    config.github_discussion
}

fn load_state(root: &Path, binding: &GitHubDiscussionConfig) -> Result<SyncState, String> {
    let path = coordination::coordination_dir(root).join(STATE_FILE);
    match std::fs::read(path) {
        Ok(bytes) => {
            let state: SyncState = serde_json::from_slice(&bytes)
                .map_err(|e| format!("invalid Discussion sync state: {e}"))?;
            Ok(if &state.binding == binding {
                state
            } else {
                SyncState::new(binding.clone())
            })
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(SyncState::new(binding.clone())),
        Err(e) => Err(e.to_string()),
    }
}

fn save_state(root: &Path, state: &SyncState) -> Result<(), String> {
    let dir = coordination::coordination_dir(root);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut file = tempfile::NamedTempFile::new_in(&dir).map_err(|e| e.to_string())?;
    serde_json::to_writer(&mut file, state).map_err(|e| e.to_string())?;
    file.flush().map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist(dir.join(STATE_FILE))
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn encode_entry(entry: &BoardEntry) -> Result<String, String> {
    let mut entry = entry.clone();
    entry.body_html = None;
    let json = serde_json::to_string(&entry).map_err(|e| e.to_string())?;
    let body = format!(
        "### {} — {}\n\n{}\n\n{MARKER}{json}\n-->",
        entry.kind.as_str(),
        entry.author,
        entry.body
    );
    if body.len() > MAX_COMMENT_BYTES {
        return Err(format!(
            "Board entry {} exceeds Discussion payload limit",
            entry.id
        ));
    }
    Ok(body)
}

fn decode_entry(body: &str) -> Option<BoardEntry> {
    if body.len() > MAX_COMMENT_BYTES {
        return None;
    }
    let (_, payload) = body.rsplit_once(MARKER)?;
    let entry: BoardEntry = serde_json::from_str(payload.strip_suffix("\n-->")?).ok()?;
    if !entry.kind.is_remote_shareable()
        || entry.id.trim().is_empty()
        || entry.body.trim().is_empty()
    {
        return None;
    }
    Some(entry)
}

/// Pull before posting: an earlier mutation may have committed before its response was lost.
/// Checkpoint each imported/sent entry so crashes replay through the exact append contract.
fn synchronize(
    root: &Path,
    state: &mut SyncState,
    remote: &impl DiscussionTransport,
) -> Result<bool, String> {
    let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
        gwt_core::operation_deadline::now() + std::time::Duration::from_secs(30),
    );
    state.last_error = None;
    let page = remote
        .read(state.binding.number, state.cursor.as_deref())
        .map_err(|e| e.to_string())?;
    for body in &page.bodies {
        let Some(entry) = decode_entry(body) else {
            continue;
        };
        let receipt =
            coordination::post_entry_exact(root, entry.clone()).map_err(|e| e.to_string())?;
        if receipt.disposition == coordination::BoardExactAppendDisposition::Appended {
            state.received += 1;
        }
        state.delivered.insert(entry.id);
    }
    state.cursor = page.cursor.or_else(|| state.cursor.clone());
    let snapshot = coordination::load_snapshot(root).map_err(|e| e.to_string())?;
    // Bound bookkeeping and delivery to the existing 500-entry hot window.
    state
        .delivered
        .retain(|id| snapshot.board.entries.iter().any(|e| &e.id == id));
    save_state(root, state)?;
    if page.has_next_page {
        return Ok(false);
    }
    for entry in snapshot
        .board
        .entries
        .iter()
        .filter(|e| e.kind.is_remote_shareable())
    {
        if state.delivered.contains(&entry.id) {
            continue;
        }
        let body = match encode_entry(entry) {
            Ok(body) => body,
            Err(error) => {
                state.last_error = Some(error);
                continue;
            }
        };
        remote
            .post(&page.discussion_id, &body)
            .map_err(|e| e.to_string())?;
        state.delivered.insert(entry.id.clone());
        state.sent += 1;
        save_state(root, state)?;
    }
    Ok(true)
}

/// Run one due sync, exposing remote failures as diagnostic state rather than local failure.
pub fn sync_now(root: &Path) -> Result<SyncReport, String> {
    let Some(binding) = binding_for(root) else {
        return Ok(SyncReport::from_state("disabled", None));
    };
    if binding.owner.trim().is_empty() || binding.repo.trim().is_empty() || binding.number == 0 {
        return Err("github_discussion requires owner, repo, and a positive number".into());
    }
    let dir = coordination::coordination_dir(root);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("discussion-sync.lock"))
        .map_err(|e| e.to_string())?;
    if let Err(e) = lock.try_lock_exclusive() {
        if e.kind() == std::io::ErrorKind::WouldBlock
            || e.raw_os_error() == fs2::lock_contended_error().raw_os_error()
        {
            return Ok(SyncReport::from_state("busy", None));
        }
        return Err(e.to_string());
    }
    let mut state = load_state(root, &binding)?;
    let now = Utc::now();
    if now < state.next_attempt {
        return Ok(SyncReport::from_state("backoff", Some(&state)));
    }
    state.next_attempt = now + chrono::Duration::seconds(POLL_SECONDS);
    save_state(root, &state)?;
    let result = HttpIssueClient::from_gh_auth_with_deadline(
        &binding.owner,
        &binding.repo,
        &gwt_github::client::ResolutionDeadline::new(
            std::time::Duration::from_secs(5),
            std::time::Duration::from_secs(30),
        ),
    )
    .map_err(|e| e.to_string())
    .and_then(|remote| synchronize(root, &mut state, &remote));
    let status = match result {
        Ok(caught_up) => {
            state.last_success = Some(Utc::now());
            if state.last_error.is_some() {
                "partial"
            } else if caught_up {
                "synced"
            } else {
                "catching_up"
            }
        }
        Err(error) => {
            state.last_error = Some(error);
            state.next_attempt = Utc::now() + chrono::Duration::seconds(RETRY_SECONDS);
            "error"
        }
    };
    save_state(root, &state)?;
    Ok(SyncReport::from_state(status, Some(&state)))
}

/// No GitHub work occurs in the caller. The short-lived child owns its own lock and timeouts.
#[cfg(not(test))]
pub(crate) fn schedule(root: &Path) {
    let Some(binding) = binding_for(root) else {
        return;
    };
    if binding.owner.trim().is_empty() || binding.repo.trim().is_empty() || binding.number == 0 {
        return;
    }
    let Ok(state) = load_state(root, &binding) else {
        return;
    };
    if Utc::now() < state.next_attempt {
        return;
    }
    let Some(bin) = std::env::current_exe()
        .ok()
        .map(|p| crate::cli::gwtd_resolver::gwtd_companion_path(&p))
        .filter(|p| p.is_file())
        .or_else(crate::cli::gwtd_resolver::resolve_gwtd_path)
    else {
        return;
    };
    use std::process::Stdio;
    let mut command = gwt_core::process::hidden_command(bin);
    let child = command
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    if let Ok(mut child) = child {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin
                .write_all(b"{\"schema_version\":1,\"operation\":\"board.sync\",\"params\":{}}\n");
        }
        // Start the child before returning from a short-lived CLI process.
        // Only reaping is delegated to a thread; it never holds up the caller.
        let _ = std::thread::Builder::new()
            .name("gwt-board-discussion-reap".into())
            .spawn(move || {
                let _ = child.wait();
            });
    }
}

#[cfg(test)]
pub(crate) fn schedule(_root: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_core::coordination::{self, AuthorKind, BoardEntryKind};
    use std::cell::RefCell;

    #[derive(Default)]
    struct Discussion {
        comments: RefCell<Vec<String>>,
        fail_post_after_commit: std::cell::Cell<bool>,
        fail_read: std::cell::Cell<bool>,
    }

    impl DiscussionTransport for Discussion {
        fn read(
            &self,
            _number: u64,
            cursor: Option<&str>,
        ) -> Result<DiscussionCommentsPage, ApiError> {
            assert!(
                gwt_core::operation_deadline::current().is_some(),
                "Discussion HTTP needs a finite deadline"
            );
            if self.fail_read.get() {
                return Err(ApiError::RateLimited {
                    retry_after: Some(300),
                });
            }
            let start = cursor.unwrap_or("0").parse::<usize>().unwrap();
            let comments = self.comments.borrow();
            Ok(DiscussionCommentsPage {
                discussion_id: "D_1".into(),
                bodies: comments[start..].to_vec(),
                cursor: (start < comments.len()).then(|| comments.len().to_string()),
                has_next_page: false,
            })
        }
        fn post(&self, _id: &str, body: &str) -> Result<(), ApiError> {
            self.comments.borrow_mut().push(body.to_string());
            if self.fail_post_after_commit.replace(false) {
                return Err(ApiError::Network("response lost".into()));
            }
            Ok(())
        }
    }

    fn post(root: &Path, kind: BoardEntryKind, body: &str) -> BoardEntry {
        let entry = BoardEntry::new(
            AuthorKind::Agent,
            "agent",
            kind,
            body,
            None,
            None,
            vec![],
            vec![],
        );
        coordination::post_entry(root, entry.clone()).unwrap();
        entry
    }

    fn state() -> SyncState {
        SyncState::new(GitHubDiscussionConfig {
            owner: "example".into(),
            repo: "project".into(),
            number: 42,
        })
    }

    #[test]
    fn two_machines_share_only_fixed_kinds_without_echoing_or_losing_metadata() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let remote = Discussion::default();
        let mut sa = state();
        let mut sb = state();
        let decision = post(a.path(), BoardEntryKind::Decision, "裁定");
        post(a.path(), BoardEntryKind::Status, "local status");
        post(a.path(), BoardEntryKind::Claim, "local claim");
        synchronize(a.path(), &mut sa, &remote).unwrap();
        synchronize(b.path(), &mut sb, &remote).unwrap();
        assert_eq!(
            coordination::load_snapshot(b.path()).unwrap().board.entries,
            [decision]
        );
        post(b.path(), BoardEntryKind::Handoff, "引き継ぎ");
        post(b.path(), BoardEntryKind::Blocked, "blocked");
        synchronize(b.path(), &mut sb, &remote).unwrap();
        synchronize(a.path(), &mut sa, &remote).unwrap();
        synchronize(a.path(), &mut sa, &remote).unwrap();
        assert_eq!(remote.comments.borrow().len(), 3);
        assert_eq!(
            coordination::load_snapshot(a.path())
                .unwrap()
                .board
                .entries
                .len(),
            5
        );
    }

    #[test]
    fn unknown_post_outcome_is_reconciled_by_pull_before_retry() {
        let root = tempfile::tempdir().unwrap();
        let remote = Discussion::default();
        let mut state = state();
        post(root.path(), BoardEntryKind::Decision, "裁定");
        remote.fail_post_after_commit.set(true);
        assert!(synchronize(root.path(), &mut state, &remote).is_err());
        synchronize(root.path(), &mut state, &remote).unwrap();
        assert_eq!(remote.comments.borrow().len(), 1);
        assert_eq!(
            coordination::load_snapshot(root.path())
                .unwrap()
                .board
                .entries
                .len(),
            1
        );
    }

    #[test]
    fn an_empty_page_keeps_the_last_received_cursor() {
        let root = tempfile::tempdir().unwrap();
        let remote = Discussion::default();
        let mut state = state();
        post(root.path(), BoardEntryKind::Decision, "裁定");
        synchronize(root.path(), &mut state, &remote).unwrap();
        synchronize(root.path(), &mut state, &remote).unwrap();
        let cursor = state.cursor.clone();
        synchronize(root.path(), &mut state, &remote).unwrap();
        assert_eq!(state.cursor, cursor);
    }

    #[test]
    fn an_oversized_post_does_not_starve_later_milestones() {
        let root = tempfile::tempdir().unwrap();
        let remote = Discussion::default();
        let mut state = state();
        post(
            root.path(),
            BoardEntryKind::Decision,
            &"x".repeat(MAX_COMMENT_BYTES),
        );
        post(root.path(), BoardEntryKind::Handoff, "deliver this");
        synchronize(root.path(), &mut state, &remote).unwrap();
        assert_eq!(remote.comments.borrow().len(), 1);
        assert!(state.last_error.as_ref().unwrap().contains("payload limit"));
    }

    #[test]
    fn rate_limit_does_not_change_cursor_or_prevent_local_board_operations() {
        let root = tempfile::tempdir().unwrap();
        let remote = Discussion::default();
        let mut state = state();
        remote.fail_read.set(true);
        assert!(synchronize(root.path(), &mut state, &remote).is_err());
        assert!(state.cursor.is_none());
        post(root.path(), BoardEntryKind::Decision, "offline");
        assert_eq!(
            coordination::load_snapshot(root.path())
                .unwrap()
                .board
                .entries
                .len(),
            1
        );
        remote.fail_read.set(false);
        synchronize(root.path(), &mut state, &remote).unwrap();
        assert_eq!(remote.comments.borrow().len(), 1);
    }
}
