//! Optional Discussion mirroring. Local Board operations never wait for GitHub.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::OpenOptions,
    io::Write,
    path::Path,
};

use chrono::{DateTime, Utc};
use fs2::FileExt;
use gwt_config::{board_config::GitHubDiscussionConfig, BoardProviderKind, ProjectBoardConfig};
use gwt_core::coordination::{self, BoardEntry};
use gwt_github::{
    client::http::{
        BoardDiscussionsPage, CreatedDiscussion, DiscussionComment, DiscussionCommentsPage,
        HttpIssueClient, HttpTransport,
    },
    ApiError,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MARKER: &str = "<!-- gwt-board-sync:v1\n";
const ROOT_MARKER: &str = "<!-- gwt-board-topic:v1\n";
const REPLY_MARKER: &str = "<!-- gwt-board-reply:v1\n";
const STATE_FILE: &str = "discussion-topics-sync.json";
const MAX_TOPICS: usize = 50;
const MAX_TOPIC_READS: usize = 5;
const MAX_MUTATIONS: usize = 5;
const POLL_SECONDS: i64 = 60;
const RETRY_SECONDS: i64 = 300;
const MAX_COMMENT_BYTES: usize = 60_000;

fn origin_path(root: &Path, id: &str) -> std::path::PathBuf {
    coordination::coordination_dir(root)
        .join("discussion-topic-origins")
        .join(format!("{:x}", Sha256::digest(id.as_bytes())))
}

fn record_local_root(root: &Path, entry: &BoardEntry) -> std::io::Result<()> {
    if entry.parent_id.is_some() || !entry.kind.is_remote_shareable() {
        return Ok(());
    }
    let path = origin_path(root, &entry.id);
    std::fs::create_dir_all(path.parent().expect("origin directory"))?;
    gwt_github::cache::write_atomic(&path, entry.id.as_bytes())
}

/// Record only locally authored posts; remote exact imports never call this.
/// No sync lock/network, and failure cannot change the local post result.
pub(crate) fn local_post_committed(root: &Path, entry: &BoardEntry) {
    if binding_for(root).is_some() {
        if let Err(error) = record_local_root(root, entry) {
            tracing::warn!(%error, "cannot record local Discussion topic origin");
        }
    }
}

fn can_create_root(root: &Path, entry: &BoardEntry) -> bool {
    if std::fs::read_to_string(origin_path(root, &entry.id))
        .ok()
        .as_deref()
        == Some(&entry.id)
    {
        return true;
    }
    // Legacy roots retain their source Session ID, whose history is machine-local.
    let Some(id) = entry
        .origin_session_id
        .as_deref()
        .filter(|id| uuid::Uuid::parse_str(id).is_ok())
    else {
        return false;
    };
    gwt_agent::Session::load(&gwt_core::paths::gwt_sessions_dir().join(format!("{id}.toml")))
        .is_ok_and(|session| session.id == id)
}

trait DiscussionTransport {
    fn list(&self, cursor: Option<&str>) -> Result<BoardDiscussionsPage, ApiError>;
    fn create(
        &self,
        category: &str,
        title: &str,
        body: &str,
    ) -> Result<CreatedDiscussion, ApiError>;
    fn read(&self, number: u64, cursor: Option<&str>) -> Result<DiscussionCommentsPage, ApiError>;
    fn post(&self, discussion_id: &str, body: &str) -> Result<(), ApiError>;
}

impl<T: HttpTransport> DiscussionTransport for HttpIssueClient<T> {
    fn list(&self, cursor: Option<&str>) -> Result<BoardDiscussionsPage, ApiError> {
        self.board_discussions(cursor)
    }
    fn create(
        &self,
        category: &str,
        title: &str,
        body: &str,
    ) -> Result<CreatedDiscussion, ApiError> {
        self.create_github_discussion(category, title, body)
    }
    fn read(&self, number: u64, cursor: Option<&str>) -> Result<DiscussionCommentsPage, ApiError> {
        self.discussion_comments(number, cursor)
    }
    fn post(&self, discussion_id: &str, body: &str) -> Result<(), ApiError> {
        self.add_discussion_comment(discussion_id, body)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Topic {
    discussion_id: String,
    number: u64,
    cursor: Option<String>,
    root: BoardEntry,
}

#[derive(Debug, Serialize, Deserialize)]
struct SyncState {
    binding: GitHubDiscussionConfig,
    discovery_cursor: Option<String>,
    topics: BTreeMap<String, Topic>,
    next_topic: usize,
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
            discovery_cursor: None,
            topics: BTreeMap::new(),
            next_topic: 0,
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
    topics: usize,
    topic_limit: usize,
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
            topics: state.map_or(0, |s| s.topics.len()),
            topic_limit: MAX_TOPICS,
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

fn topic_title(entry: &BoardEntry) -> String {
    let owners = entry
        .related_owners
        .iter()
        .filter_map(|owner| owner.trim_start_matches('#').parse::<u64>().ok())
        .map(|number| format!("Issue #{number}"))
        .collect::<Vec<_>>()
        .join(", ");
    let summary = entry
        .title
        .as_deref()
        .unwrap_or(&entry.body)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let title = if owners.is_empty() {
        summary
    } else {
        format!("{owners}: {summary}")
    };
    title.chars().take(100).collect()
}

fn encode_root(entry: &BoardEntry, binding: &GitHubDiscussionConfig) -> Result<String, String> {
    let links = entry
        .related_owners
        .iter()
        .filter_map(|owner| owner.trim_start_matches('#').parse::<u64>().ok())
        .map(|number| {
            format!(
                "[Issue #{number}](https://github.com/{}/{}/issues/{number})",
                binding.owner, binding.repo
            )
        })
        .collect::<Vec<_>>()
        .join(" · ");
    let body = format!(
        "Board entry: `{}`\n\n{}\n\n{}",
        entry.id,
        links,
        encode_entry(entry)?.replace(MARKER, ROOT_MARKER)
    );
    if body.len() > MAX_COMMENT_BYTES {
        return Err(format!(
            "Board entry {} exceeds Discussion payload limit",
            entry.id
        ));
    }
    Ok(body)
}

fn decode_root(body: &str) -> Option<BoardEntry> {
    let entry = decode_entry(&body.replace(ROOT_MARKER, MARKER))?;
    (body.contains(ROOT_MARKER) && entry.parent_id.is_none()).then_some(entry)
}

fn topic_root(
    root: &Path,
    entry: &BoardEntry,
    entries: &BTreeMap<String, BoardEntry>,
    topics: &BTreeMap<String, Topic>,
) -> Option<String> {
    let mut current = entry.clone();
    let mut seen = BTreeSet::new();
    loop {
        if !seen.insert(current.id.clone()) {
            return None;
        }
        let Some(parent) = &current.parent_id else {
            return current
                .kind
                .is_remote_shareable()
                .then(|| current.id.clone());
        };
        if topics.contains_key(parent) {
            return Some(parent.clone());
        }
        current = entries
            .get(parent)
            .cloned()
            .or_else(|| coordination::load_board_entry(root, parent).ok().flatten())?;
    }
}

#[derive(Serialize, Deserialize)]
struct SharedReply {
    root_id: String,
    entry: BoardEntry,
}

fn encode_reply(entry: &BoardEntry, root_id: &str) -> Result<String, String> {
    let mut entry = entry.clone();
    entry.body_html = None;
    let reply = SharedReply {
        root_id: root_id.into(),
        entry,
    };
    let body = format!(
        "{}\n\n{REPLY_MARKER}{}\n-->",
        reply.entry.body,
        serde_json::to_string(&reply).map_err(|e| e.to_string())?
    );
    if body.len() > MAX_COMMENT_BYTES {
        return Err("Board reply exceeds Discussion payload limit".into());
    }
    Ok(body)
}

fn received_comment(comment: &DiscussionComment, topic: &Topic) -> Option<BoardEntry> {
    if let Some((_, payload)) = comment.body.rsplit_once(REPLY_MARKER) {
        if comment.body.len() > MAX_COMMENT_BYTES {
            return None;
        }
        let reply: SharedReply = serde_json::from_str(payload.strip_suffix("\n-->")?).ok()?;
        return (reply.root_id == topic.root.id
            && reply.entry.kind.is_remote_shareable()
            && reply.entry.parent_id.is_some()
            && !reply.entry.id.trim().is_empty()
            && !reply.entry.body.trim().is_empty())
        .then_some(reply.entry);
    }
    if comment.body.contains(MARKER) {
        return decode_entry(&comment.body).filter(|entry| entry.parent_id.is_some());
    }
    if comment.body.trim().is_empty() || comment.body.len() > MAX_COMMENT_BYTES {
        return None;
    }
    let mut entry = BoardEntry::new(
        coordination::AuthorKind::User,
        &comment.author,
        coordination::BoardEntryKind::Decision,
        &comment.body,
        None,
        Some(topic.root.id.clone()),
        topic.root.related_topics.clone(),
        topic.root.related_owners.clone(),
    );
    entry.id = format!("github-discussion-comment:{}", comment.id);
    entry.created_at = comment.created_at;
    entry.updated_at = comment.created_at;
    entry.audience = topic.root.audience.clone();
    Some(entry)
}

fn import(root: &Path, state: &mut SyncState, entry: BoardEntry) -> Result<(), String> {
    let receipt = coordination::post_entry_exact(root, entry.clone()).map_err(|e| e.to_string())?;
    if receipt.disposition == coordination::BoardExactAppendDisposition::Appended {
        state.received += 1;
    }
    state.delivered.insert(entry.id);
    Ok(())
}

/// Discover before creating and pull comments before posting: a mutation may have
/// committed even when its response was lost. Checkpoint through exact append.
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
        .list(state.discovery_cursor.as_deref())
        .map_err(|e| e.to_string())?;
    for discussion in page.discussions {
        let Some(entry) = decode_root(&discussion.body) else {
            continue;
        };
        if let Some(topic) = state.topics.get(&entry.id) {
            if topic.discussion_id != discussion.id {
                state.last_error = Some(format!(
                    "Duplicate remote topic for Board entry {}",
                    entry.id
                ));
            }
            continue;
        }
        if state.topics.len() >= MAX_TOPICS {
            state.last_error = Some(format!(
                "Discussion topic limit {MAX_TOPICS} reached; additional topics stay local"
            ));
            continue;
        }
        import(root, state, entry.clone())?;
        state.topics.insert(
            entry.id.clone(),
            Topic {
                discussion_id: discussion.id,
                number: discussion.number,
                cursor: None,
                root: entry,
            },
        );
    }
    state.discovery_cursor = if page.has_next_page {
        Some(
            page.cursor
                .filter(|cursor| Some(cursor) != state.discovery_cursor.as_ref())
                .ok_or("Discussion discovery cursor missing or repeated")?,
        )
    } else {
        None
    };
    save_state(root, state)?;
    if page.has_next_page {
        return Ok(false);
    }

    let snapshot = coordination::load_snapshot(root).map_err(|e| e.to_string())?;
    let mut entries: BTreeMap<_, _> = snapshot
        .board
        .entries
        .iter()
        .map(|entry| (entry.id.clone(), entry.clone()))
        .collect();
    for topic in state.topics.values() {
        entries.insert(topic.root.id.clone(), topic.root.clone());
    }
    state.delivered.retain(|id| entries.contains_key(id));
    let keys: Vec<_> = state.topics.keys().cloned().collect();
    let mut ready = BTreeSet::new();
    for offset in 0..keys.len().min(MAX_TOPIC_READS) {
        let key = &keys[(state.next_topic + offset) % keys.len()];
        let topic = &state.topics[key];
        let page = remote
            .read(topic.number, topic.cursor.as_deref())
            .map_err(|e| e.to_string())?;
        if page.discussion_id != topic.discussion_id {
            return Err("Discussion identity changed".into());
        }
        for comment in page.comments {
            let Some(entry) = received_comment(&comment, &state.topics[key]) else {
                continue;
            };
            if !comment.body.contains(REPLY_MARKER)
                && topic_root(root, &entry, &entries, &state.topics).as_ref() != Some(key)
            {
                continue;
            }
            import(root, state, entry.clone())?;
            entries.insert(entry.id.clone(), entry);
        }
        let topic = state.topics.get_mut(key).expect("known topic");
        if page.has_next_page && (page.cursor.is_none() || page.cursor == topic.cursor) {
            return Err("Discussion comment cursor missing or repeated".into());
        }
        topic.cursor = page.cursor.or_else(|| topic.cursor.clone());
        if !page.has_next_page {
            ready.insert(key.clone());
        }
        save_state(root, state)?;
    }
    if !keys.is_empty() {
        state.next_topic = (state.next_topic + MAX_TOPIC_READS) % keys.len();
    }

    let mut mutations = 0;
    let mut caught_up = true;
    for entry in snapshot
        .board
        .entries
        .iter()
        .filter(|entry| entry.kind.is_remote_shareable() && entry.parent_id.is_none())
    {
        if state.topics.contains_key(&entry.id) {
            continue;
        }
        if !can_create_root(root, entry) {
            state.last_error = Some(format!(
                "Board root {} awaits its source machine; local creation origin is unproven",
                entry.id
            ));
            continue;
        }
        if state.topics.len() >= MAX_TOPICS {
            state.last_error = Some(format!(
                "Discussion topic limit {MAX_TOPICS} reached; additional topics stay local"
            ));
            continue;
        }
        if mutations >= MAX_MUTATIONS {
            caught_up = false;
            break;
        }
        let body = match encode_root(entry, &state.binding) {
            Ok(body) => body,
            Err(error) => {
                state.last_error = Some(error);
                continue;
            }
        };
        let discussion = remote
            .create(&state.binding.category, &topic_title(entry), &body)
            .map_err(|e| e.to_string())?;
        state.topics.insert(
            entry.id.clone(),
            Topic {
                discussion_id: discussion.id,
                number: discussion.number,
                cursor: None,
                root: entry.clone(),
            },
        );
        state.delivered.insert(entry.id.clone());
        state.sent += 1;
        mutations += 1;
        // A new topic has no earlier comments to reconcile.
        ready.insert(entry.id.clone());
        save_state(root, state)?;
    }
    for entry in snapshot
        .board
        .entries
        .iter()
        .filter(|entry| entry.kind.is_remote_shareable() && entry.parent_id.is_some())
    {
        if state.delivered.contains(&entry.id) || entry.id.starts_with("github-discussion-comment:")
        {
            continue;
        }
        let Some(key) = topic_root(root, entry, &entries, &state.topics) else {
            continue;
        };
        if !ready.contains(&key) {
            caught_up = false;
            continue;
        }
        if mutations >= MAX_MUTATIONS {
            caught_up = false;
            break;
        }
        let body = match encode_reply(entry, &key) {
            Ok(body) => body,
            Err(error) => {
                state.last_error = Some(error);
                continue;
            }
        };
        remote
            .post(&state.topics[&key].discussion_id, &body)
            .map_err(|e| e.to_string())?;
        state.delivered.insert(entry.id.clone());
        state.sent += 1;
        mutations += 1;
        save_state(root, state)?;
    }
    Ok(caught_up)
}

/// Run one due sync, exposing remote failures as diagnostic state rather than local failure.
pub fn sync_now(root: &Path) -> Result<SyncReport, String> {
    let Some(binding) = binding_for(root) else {
        return Ok(SyncReport::from_state("disabled", None));
    };
    if binding.owner.trim().is_empty()
        || binding.repo.trim().is_empty()
        || binding.category.trim().is_empty()
    {
        return Err("github_discussion requires owner, repo, and a nonempty category".into());
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
    if binding.owner.trim().is_empty()
        || binding.repo.trim().is_empty()
        || binding.category.trim().is_empty()
    {
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
    use gwt_core::coordination::{AuthorKind, BoardEntryKind};
    use gwt_github::client::http::BoardDiscussion;
    use std::cell::{Cell, RefCell};

    struct RemoteTopic {
        id: String,
        body: String,
        title: String,
        comments: Vec<DiscussionComment>,
    }
    #[derive(Default)]
    struct Discussion {
        topics: RefCell<Vec<RemoteTopic>>,
        fail_create_after_commit: Cell<bool>,
        fail_post_after_commit: Cell<bool>,
        fail_read: Cell<bool>,
        reads: RefCell<Vec<u64>>,
    }
    impl DiscussionTransport for Discussion {
        fn list(&self, cursor: Option<&str>) -> Result<BoardDiscussionsPage, ApiError> {
            assert!(gwt_core::operation_deadline::current().is_some());
            if self.fail_read.get() {
                return Err(ApiError::RateLimited {
                    retry_after: Some(300),
                });
            }
            let topics = self.topics.borrow();
            let start = cursor.unwrap_or("0").parse::<usize>().unwrap();
            let end = (start + 50).min(topics.len());
            Ok(BoardDiscussionsPage {
                discussions: topics[start..end]
                    .iter()
                    .enumerate()
                    .map(|(index, topic)| BoardDiscussion {
                        id: topic.id.clone(),
                        number: (start + index + 1) as u64,
                        body: topic.body.clone(),
                    })
                    .collect(),
                cursor: (end > start).then(|| end.to_string()),
                has_next_page: end < topics.len(),
            })
        }
        fn create(
            &self,
            category: &str,
            title: &str,
            body: &str,
        ) -> Result<CreatedDiscussion, ApiError> {
            assert_eq!(category, "General");
            let mut topics = self.topics.borrow_mut();
            let number = topics.len() as u64 + 1;
            let id = format!("D_{number}");
            topics.push(RemoteTopic {
                id: id.clone(),
                body: body.into(),
                title: title.into(),
                comments: vec![],
            });
            if self.fail_create_after_commit.replace(false) {
                return Err(ApiError::Network("response lost".into()));
            }
            Ok(CreatedDiscussion {
                id,
                number,
                url: format!("https://github.com/example/project/discussions/{number}"),
            })
        }
        fn read(
            &self,
            number: u64,
            cursor: Option<&str>,
        ) -> Result<DiscussionCommentsPage, ApiError> {
            self.reads.borrow_mut().push(number);
            let topics = self.topics.borrow();
            let topic = &topics[number as usize - 1];
            let start = cursor.unwrap_or("0").parse::<usize>().unwrap();
            Ok(DiscussionCommentsPage {
                discussion_id: topic.id.clone(),
                comments: topic.comments[start..].to_vec(),
                cursor: (start < topic.comments.len()).then(|| topic.comments.len().to_string()),
                has_next_page: false,
            })
        }
        fn post(&self, id: &str, body: &str) -> Result<(), ApiError> {
            let mut topics = self.topics.borrow_mut();
            let topic = topics.iter_mut().find(|topic| topic.id == id).unwrap();
            topic.comments.push(DiscussionComment {
                id: format!("{id}_C{}", topic.comments.len()),
                body: body.into(),
                author: "collaborator".into(),
                created_at: DateTime::UNIX_EPOCH,
            });
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
            vec!["4833".into()],
        );
        coordination::post_entry(root, entry.clone()).unwrap();
        record_local_root(root, &entry).unwrap();
        entry
    }
    fn reply(root: &Path, parent: &BoardEntry, kind: BoardEntryKind, body: &str) -> BoardEntry {
        let mut entry = BoardEntry::new(
            AuthorKind::Agent,
            "agent",
            kind,
            body,
            None,
            Some(parent.id.clone()),
            vec![],
            parent.related_owners.clone(),
        );
        entry.created_at = parent.created_at + chrono::Duration::seconds(1);
        entry.updated_at = entry.created_at;
        coordination::post_entry(root, entry.clone()).unwrap();
        entry
    }
    fn state() -> SyncState {
        SyncState::new(GitHubDiscussionConfig {
            owner: "example".into(),
            repo: "project".into(),
            number: 0,
            category: "General".into(),
        })
    }
    #[test]
    fn root_is_a_discussion_instead_of_a_comment() {
        let root = tempfile::tempdir().unwrap();
        let remote = Discussion::default();
        let mut state = state();
        let entry = post(root.path(), BoardEntryKind::Decision, "topic root");
        synchronize(root.path(), &mut state, &remote).unwrap();
        let topics = remote.topics.borrow();
        assert_eq!(topics.len(), 1);
        assert!(topics[0].comments.is_empty());
        assert_eq!(topics[0].title, "Issue #4833: topic root");
        assert!(topics[0].body.contains(&entry.id));
        assert!(topics[0]
            .body
            .contains("https://github.com/example/project/issues/4833"));
    }
    #[test]
    fn roots_are_separate_topics_and_nested_shared_replies_use_their_topic() {
        let root = tempfile::tempdir().unwrap();
        let remote = Discussion::default();
        let mut state = state();
        let a = post(root.path(), BoardEntryKind::Decision, "first");
        let b = post(root.path(), BoardEntryKind::Handoff, "second");
        let first_reply = reply(root.path(), &a, BoardEntryKind::Decision, "reply");
        reply(root.path(), &first_reply, BoardEntryKind::Handoff, "nested");
        reply(root.path(), &b, BoardEntryKind::Status, "local status");
        let local_root = post(root.path(), BoardEntryKind::Claim, "local claim");
        reply(
            root.path(),
            &local_root,
            BoardEntryKind::Decision,
            "still local",
        );
        synchronize(root.path(), &mut state, &remote).unwrap();
        let topics = remote.topics.borrow();
        assert_eq!(topics.len(), 2);
        assert_eq!(
            topics
                .iter()
                .find(|topic| topic.body.contains(&a.id))
                .unwrap()
                .comments
                .len(),
            2
        );
        assert!(topics
            .iter()
            .find(|topic| topic.body.contains(&b.id))
            .unwrap()
            .comments
            .is_empty());
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
            coordination::load_snapshot(b.path())
                .unwrap()
                .board
                .entries
                .as_slice(),
            std::slice::from_ref(&decision)
        );
        let response = reply(b.path(), &decision, BoardEntryKind::Handoff, "引き継ぎ");
        synchronize(b.path(), &mut sb, &remote).unwrap();
        synchronize(a.path(), &mut sa, &remote).unwrap();
        synchronize(a.path(), &mut sa, &remote).unwrap();
        assert_eq!(remote.topics.borrow().len(), 1);
        assert_eq!(remote.topics.borrow()[0].comments.len(), 1);
        assert!(coordination::load_snapshot(a.path())
            .unwrap()
            .board
            .entries
            .contains(&response));
    }
    #[test]
    fn local_origin_can_be_republished_after_an_incomplete_old_marker() {
        let root = tempfile::tempdir().unwrap();
        let entry = post(root.path(), BoardEntryKind::Decision, "local source");
        std::fs::write(origin_path(root.path(), &entry.id), "").unwrap();
        record_local_root(root.path(), &entry).unwrap();
        assert!(can_create_root(root.path(), &entry));
    }

    #[test]
    fn imported_legacy_root_is_not_created_by_a_second_machine() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let remote = Discussion::default();
        let root = post(a.path(), BoardEntryKind::Decision, "old shared root");
        coordination::post_entry_exact(b.path(), root).unwrap();
        // Peer B wins discovery while the source machine A has not created yet.
        synchronize(b.path(), &mut state(), &remote).unwrap();
        assert!(remote.topics.borrow().is_empty());
        synchronize(a.path(), &mut state(), &remote).unwrap();
        synchronize(b.path(), &mut state(), &remote).unwrap();
        assert_eq!(remote.topics.borrow().len(), 1);
    }

    #[test]
    fn current_reply_to_an_archived_parent_is_still_delivered() {
        let root = tempfile::tempdir().unwrap();
        let remote = Discussion::default();
        let mut state = state();
        let topic = post(root.path(), BoardEntryKind::Decision, "topic");
        let parent = reply(root.path(), &topic, BoardEntryKind::Decision, "old reply");
        synchronize(root.path(), &mut state, &remote).unwrap();
        synchronize(root.path(), &mut state, &remote).unwrap();
        for index in 0..500 {
            let mut status = BoardEntry::new(
                AuthorKind::Agent,
                "agent",
                BoardEntryKind::Status,
                "local traffic",
                None,
                None,
                vec![],
                vec![],
            );
            status.created_at = topic.created_at + chrono::Duration::seconds(index + 10);
            status.updated_at = status.created_at;
            coordination::post_entry(root.path(), status).unwrap();
        }
        let mut child = BoardEntry::new(
            AuthorKind::Agent,
            "agent",
            BoardEntryKind::Handoff,
            "new reply",
            None,
            Some(parent.id.clone()),
            vec![],
            vec![],
        );
        child.created_at = topic.created_at + chrono::Duration::seconds(1000);
        child.updated_at = child.created_at;
        coordination::post_entry(root.path(), child).unwrap();
        assert!(!coordination::load_snapshot(root.path())
            .unwrap()
            .board
            .entries
            .contains(&parent));
        synchronize(root.path(), &mut state, &remote).unwrap();
        assert_eq!(remote.topics.borrow()[0].comments.len(), 2);
    }

    #[test]
    fn shared_reply_survives_an_unshared_intermediate_parent() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let remote = Discussion::default();
        let root = post(a.path(), BoardEntryKind::Decision, "topic");
        let local = reply(a.path(), &root, BoardEntryKind::Status, "local only");
        let shared = reply(
            a.path(),
            &local,
            BoardEntryKind::Handoff,
            "share this reply",
        );
        synchronize(a.path(), &mut state(), &remote).unwrap();
        synchronize(b.path(), &mut state(), &remote).unwrap();
        let entries = coordination::load_snapshot(b.path()).unwrap().board.entries;
        assert!(entries.contains(&shared));
        assert!(!entries.contains(&local));
    }

    #[test]
    fn unknown_create_and_post_outcomes_are_reconciled_before_retry() {
        let root = tempfile::tempdir().unwrap();
        let remote = Discussion::default();
        let mut state = state();
        let entry = post(root.path(), BoardEntryKind::Decision, "topic");
        remote.fail_create_after_commit.set(true);
        assert!(synchronize(root.path(), &mut state, &remote).is_err());
        synchronize(root.path(), &mut state, &remote).unwrap();
        assert_eq!(remote.topics.borrow().len(), 1);
        reply(root.path(), &entry, BoardEntryKind::Decision, "response");
        remote.fail_post_after_commit.set(true);
        assert!(synchronize(root.path(), &mut state, &remote).is_err());
        // Recover from the durable checkpoint as a new worker would.
        let mut state = load_state(root.path(), &state.binding).unwrap();
        synchronize(root.path(), &mut state, &remote).unwrap();
        assert_eq!(remote.topics.borrow()[0].comments.len(), 1);
    }
    #[test]
    fn plain_remote_comments_are_stable_board_replies_without_echo() {
        let root = tempfile::tempdir().unwrap();
        let remote = Discussion::default();
        let mut state = state();
        let entry = post(root.path(), BoardEntryKind::Decision, "topic");
        synchronize(root.path(), &mut state, &remote).unwrap();
        remote.post("D_1", "human response").unwrap();
        synchronize(root.path(), &mut state, &remote).unwrap();
        let comments = coordination::load_snapshot(root.path())
            .unwrap()
            .board
            .entries;
        let received = comments
            .iter()
            .find(|entry| entry.body == "human response")
            .unwrap();
        assert_eq!(received.parent_id.as_deref(), Some(entry.id.as_str()));
        assert_eq!(received.author, "collaborator");
        assert_eq!(received.created_at, DateTime::UNIX_EPOCH);
        let cursor = state.topics[&entry.id].cursor.clone();
        synchronize(root.path(), &mut state, &remote).unwrap();
        assert_eq!(state.topics[&entry.id].cursor, cursor);
        assert_eq!(remote.topics.borrow()[0].comments.len(), 1);
    }
    #[test]
    fn oversized_posts_do_not_starve_later_milestones() {
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
        assert_eq!(remote.topics.borrow().len(), 1);
        assert!(state.last_error.unwrap().contains("payload limit"));
    }
    #[test]
    fn topic_and_tick_limits_keep_local_board_available() {
        let root = tempfile::tempdir().unwrap();
        let remote = Discussion::default();
        let mut state = state();
        for index in 0..MAX_MUTATIONS + 1 {
            post(
                root.path(),
                BoardEntryKind::Decision,
                &format!("topic {index}"),
            );
        }
        assert!(!synchronize(root.path(), &mut state, &remote).unwrap());
        assert_eq!(remote.topics.borrow().len(), MAX_MUTATIONS);
        synchronize(root.path(), &mut state, &remote).unwrap();
        assert_eq!(remote.reads.borrow().len(), MAX_TOPIC_READS);
        let existing_topics = remote.topics.borrow().len();
        for index in existing_topics..MAX_TOPICS + 1 {
            let entry = BoardEntry::new(
                AuthorKind::Agent,
                "remote",
                BoardEntryKind::Decision,
                format!("remote {index}"),
                None,
                None,
                vec![],
                vec![],
            );
            remote
                .create(
                    "General",
                    "topic",
                    &encode_root(&entry, &state.binding).unwrap(),
                )
                .unwrap();
        }
        assert!(!synchronize(root.path(), &mut state, &remote).unwrap()); // first discovery page
        synchronize(root.path(), &mut state, &remote).unwrap();
        assert_eq!(state.topics.len(), MAX_TOPICS);
        assert!(state.last_error.as_ref().unwrap().contains("topic limit"));
        let local = post(
            root.path(),
            BoardEntryKind::Decision,
            "overflow stays usable",
        );
        assert!(coordination::load_snapshot(root.path())
            .unwrap()
            .board
            .entries
            .contains(&local));
        assert_eq!(remote.topics.borrow().len(), MAX_TOPICS + 1);
    }
    #[test]
    fn rate_limit_does_not_prevent_local_board_operations() {
        let root = tempfile::tempdir().unwrap();
        let remote = Discussion::default();
        let mut state = state();
        remote.fail_read.set(true);
        assert!(synchronize(root.path(), &mut state, &remote).is_err());
        assert!(state.discovery_cursor.is_none());
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
        assert_eq!(remote.topics.borrow().len(), 1);
    }
}
