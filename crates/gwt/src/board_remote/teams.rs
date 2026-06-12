//! Microsoft Teams remote Board provider (SPEC-2963 FR-004/008/010/014).
//!
//! Implements the synchronous [`BoardProvider`] trait against Microsoft Graph
//! using delegated tokens (posts appear as the signed-in user — Graph forbids
//! app-only channel posting outside migration). HTTP is abstracted behind the
//! shared [`HttpClient`] so the provider is unit-testable with a mock. Reads use
//! the same time-window cache as Slack to bound API calls; failures surface as
//! errors rather than a silent local fallback (FR-010).

use std::collections::BTreeMap;
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use gwt_core::coordination::{
    AuthorKind, BoardAudienceScope, BoardEntry, BoardEntryKind, BoardHistoryPage, BoardProjection,
    BoardProvider, CoordinationSnapshot,
};
use gwt_core::{GwtError, Result};

use super::cache::TimedCache;
use super::mapping;
use super::markdown;
use super::slack::{HttpClient, HttpResponse};

const GRAPH_API: &str = "https://graph.microsoft.com/v1.0";
const MESSAGE_LIMIT: usize = 20;

/// Teams-backed Board provider.
pub struct TeamsProvider {
    token: String,
    /// `team_id/channel_id` used when a Work has no explicit mapping.
    default_channel: String,
    channel_map: BTreeMap<String, String>,
    http: Box<dyn HttpClient>,
    cache: TimedCache<Vec<BoardEntry>>,
}

impl TeamsProvider {
    pub fn new(
        token: impl Into<String>,
        default_channel: impl Into<String>,
        channel_map: BTreeMap<String, String>,
        http: Box<dyn HttpClient>,
        cache_ttl_secs: i64,
    ) -> Self {
        Self {
            token: token.into(),
            default_channel: default_channel.into(),
            channel_map,
            http,
            cache: TimedCache::new(cache_ttl_secs),
        }
    }

    fn workspace_for_channel(&self, channel: &str) -> String {
        self.channel_map
            .iter()
            .find(|(_, ch)| ch.as_str() == channel)
            .map(|(ws, _)| ws.clone())
            .unwrap_or_default()
    }

    fn fetch_history(&self, channel: &str) -> Result<Vec<BoardEntry>> {
        let (team, chan) = split_channel(channel)?;
        let top = MESSAGE_LIMIT.to_string();
        let url = format!("{GRAPH_API}/teams/{team}/channels/{chan}/messages");
        let response = self
            .http
            .get(&url, &self.token, &[("$top", &top)])
            .map_err(GwtError::Other)?;
        check_status(&response, "list messages")?;
        let parsed: GraphMessages = serde_json::from_str(&response.body)
            .map_err(|err| GwtError::Other(format!("graph messages parse: {err}")))?;
        let workspace = self.workspace_for_channel(channel);
        let mut entries: Vec<BoardEntry> = parsed
            .value
            .iter()
            // Skip system events (join/leave, etc.) and deleted messages so the
            // Board shows only real posts.
            .filter(|message| message.is_renderable())
            .map(|message| graph_message_to_entry(message, &workspace))
            .collect();
        entries.sort_by_key(|entry| entry.created_at);
        Ok(entries)
    }

    fn cached_history(&self) -> Result<Vec<BoardEntry>> {
        let now = Utc::now();
        if let Some(cached) = self.cache.get(now) {
            return Ok(cached);
        }
        let entries = self.fetch_history(&self.default_channel)?;
        self.cache.put(now, entries.clone());
        Ok(entries)
    }

    fn snapshot_from(entries: Vec<BoardEntry>) -> CoordinationSnapshot {
        let total_entries = entries.len();
        let oldest_entry_id = entries.first().map(|entry| entry.id.clone());
        let newest_entry_id = entries.last().map(|entry| entry.id.clone());
        CoordinationSnapshot {
            board: BoardProjection {
                entries,
                has_more_before: false,
                oldest_entry_id,
                newest_entry_id,
                total_entries,
                updated_at: Utc::now(),
            },
        }
    }

    /// Post a channel message (root, when `reply_to` is None) or a reply, and
    /// return the created message id. `subject` is only valid on roots. `meta`
    /// adds a bold "who · kind · origin" line before the body (entry replies
    /// only; SPEC-2963) so a reader can tell who posted and the entry type.
    fn post_graph_message(
        &self,
        team: &str,
        chan: &str,
        meta: Option<&str>,
        title: Option<&str>,
        body_markdown: &str,
        reply_to: Option<&str>,
    ) -> Result<String> {
        let url = match reply_to {
            Some(parent) => {
                format!("{GRAPH_API}/teams/{team}/channels/{chan}/messages/{parent}/replies")
            }
            None => format!("{GRAPH_API}/teams/{team}/channels/{chan}/messages"),
        };
        // Prepend the meta as a bold markdown line so the shared renderer escapes
        // and sanitizes it consistently with the body. A reply (Graph `/replies`)
        // cannot carry a `subject`, so the entry title is rendered into the body
        // too — otherwise it is silently dropped, while Slack shows it as a
        // header block. Root messages keep the title in `subject` (set below).
        let mut sections: Vec<String> = Vec::new();
        if let Some(meta) = meta.map(str::trim).filter(|m| !m.is_empty()) {
            sections.push(format!("**{meta}**"));
        }
        if reply_to.is_some() {
            if let Some(title) = title.map(str::trim).filter(|t| !t.is_empty()) {
                sections.push(format!("**{title}**"));
            }
        }
        sections.push(body_markdown.to_string());
        let content_markdown = sections.join("\n\n");
        let content = markdown::markdown_to_teams_html(&content_markdown);
        let mut payload =
            serde_json::json!({ "body": { "contentType": "html", "content": content } });
        if reply_to.is_none() {
            if let Some(title) = title.map(str::trim).filter(|t| !t.is_empty()) {
                payload["subject"] = serde_json::Value::String(title.to_string());
            }
        }
        let payload = payload.to_string();
        let response = self
            .http
            .post_json(&url, &self.token, &payload)
            .map_err(GwtError::Other)?;
        check_status(&response, "post message")?;
        let created: GraphMessage = serde_json::from_str(&response.body)
            .map_err(|err| GwtError::Other(format!("teams post parse: {err}")))?;
        if created.id.trim().is_empty() {
            return Err(GwtError::Other(
                "teams post returned no message id".to_string(),
            ));
        }
        Ok(created.id)
    }

    /// Best-effort update of a Workspace root summary card via Graph PATCH.
    /// Graph restricts channel-message edits, so failures are swallowed: the
    /// root card may stay stale, but a refresh never blocks posting (SPEC-2963).
    fn update_graph_message(&self, team: &str, chan: &str, id: &str, body_markdown: &str) {
        let url = format!("{GRAPH_API}/teams/{team}/channels/{chan}/messages/{id}");
        let content = markdown::markdown_to_teams_html(body_markdown);
        let payload = serde_json::json!({ "body": { "contentType": "html", "content": content } })
            .to_string();
        if let Ok(response) = self.http.patch_json(&url, &self.token, &payload) {
            let _ = check_status(&response, "update message");
        }
    }

    /// SPEC-2963: get-or-create (and best-effort refresh) the Workspace/General
    /// thread root for an entry. Returns the root message id to reply under.
    fn ensure_thread_root(
        &self,
        worktree_root: &Path,
        team: &str,
        chan: &str,
        channel: &str,
        entry: &BoardEntry,
    ) -> Result<String> {
        let key = mapping::thread_key_for_entry(entry);
        let item =
            gwt_core::workspace_projection::load_or_synthesize_workspace_work_items(worktree_root)
                .ok()
                .and_then(|proj| proj.work_items.into_iter().find(|work| work.id == key));
        let (card_title, card_body) =
            mapping::workspace_summary_card(&key, item.as_ref(), entry.origin_branch.as_deref());
        let hash = mapping::card_hash(&card_title, &card_body);

        if let Some(existing) =
            gwt_core::board_remote_roots::find_root_mapping(worktree_root, "teams", channel, &key)
        {
            if existing.card_hash != hash {
                self.update_graph_message(team, chan, &existing.root_id, &card_body);
                let _ = gwt_core::board_remote_roots::append_root_mapping(
                    worktree_root,
                    &gwt_core::board_remote_roots::RootMapping {
                        key,
                        provider: "teams".to_string(),
                        channel: channel.to_string(),
                        root_id: existing.root_id.clone(),
                        card_hash: hash,
                        updated_at: Utc::now(),
                    },
                );
            }
            return Ok(existing.root_id);
        }

        let root_id =
            self.post_graph_message(team, chan, None, Some(&card_title), &card_body, None)?;
        let _ = gwt_core::board_remote_roots::append_root_mapping(
            worktree_root,
            &gwt_core::board_remote_roots::RootMapping {
                key,
                provider: "teams".to_string(),
                channel: channel.to_string(),
                root_id: root_id.clone(),
                card_hash: hash,
                updated_at: Utc::now(),
            },
        );
        Ok(root_id)
    }
}

fn split_channel(channel: &str) -> Result<(String, String)> {
    let (team, chan) = channel.split_once('/').ok_or_else(|| {
        GwtError::Other(format!(
            "teams channel must be 'team_id/channel_id': {channel}"
        ))
    })?;
    if team.trim().is_empty() || chan.trim().is_empty() {
        return Err(GwtError::Other(format!(
            "teams channel must be 'team_id/channel_id': {channel}"
        )));
    }
    Ok((team.trim().to_string(), chan.trim().to_string()))
}

fn check_status(response: &HttpResponse, op: &str) -> Result<()> {
    if response.status == 429 {
        return Err(GwtError::Other(format!(
            "teams {op} rate limited; retry after {}s",
            response.retry_after.unwrap_or(30)
        )));
    }
    if response.status == 403 {
        // Self-diagnosing message for the most common Teams setup blocker: the
        // signed-in user is not a member of the target team/channel (the Teams
        // analogue of Slack `not_in_channel`).
        return Err(GwtError::Other(format!(
            "teams {op} forbidden (403): ensure the signed-in account is a member \
             of the target team and channel"
        )));
    }
    if response.status >= 400 {
        return Err(GwtError::Other(format!(
            "teams {op} http {}",
            response.status
        )));
    }
    Ok(())
}

#[derive(Deserialize)]
struct GraphMessages {
    #[serde(default)]
    value: Vec<GraphMessage>,
}

#[derive(Deserialize)]
struct GraphMessage {
    id: String,
    #[serde(rename = "replyToId", default)]
    reply_to_id: Option<String>,
    /// Channel-message subject (root posts only; replies have none).
    #[serde(default)]
    subject: Option<String>,
    #[serde(rename = "createdDateTime", default)]
    created_date_time: Option<String>,
    #[serde(default)]
    body: Option<GraphBody>,
    #[serde(default)]
    from: Option<GraphFrom>,
    /// `message` for user posts; `systemEventMessage` etc. for join/leave and
    /// other system entries we should not render as Board posts.
    #[serde(rename = "messageType", default)]
    message_type: Option<String>,
    /// Set when the message was deleted (body is then empty); skip these.
    #[serde(rename = "deletedDateTime", default)]
    deleted_date_time: Option<String>,
}

impl GraphMessage {
    /// Whether this is a normal, non-deleted user message worth rendering.
    fn is_renderable(&self) -> bool {
        self.message_type.as_deref().unwrap_or("message") == "message"
            && self.deleted_date_time.is_none()
    }
}

#[derive(Deserialize)]
struct GraphBody {
    #[serde(default)]
    content: String,
    /// `text` or `html` (SPEC-2963). When `html`, the read-back path strips
    /// tags for the plaintext board (POST-only formatting scope: no HTML→
    /// Markdown reconstruction).
    #[serde(rename = "contentType", default)]
    content_type: Option<String>,
}

#[derive(Deserialize)]
struct GraphFrom {
    #[serde(default)]
    user: Option<GraphUser>,
}

#[derive(Deserialize)]
struct GraphUser {
    #[serde(rename = "displayName", default)]
    display_name: Option<String>,
}

fn graph_message_to_entry(message: &GraphMessage, workspace: &str) -> BoardEntry {
    let author = message
        .from
        .as_ref()
        .and_then(|from| from.user.as_ref())
        .and_then(|user| user.display_name.clone())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "Teams".to_string());
    let body = message
        .body
        .as_ref()
        .map(|b| {
            let is_html = b
                .content_type
                .as_deref()
                .is_some_and(|ct| ct.eq_ignore_ascii_case("html"));
            if is_html {
                markdown::strip_html_tags(&b.content)
            } else {
                b.content.clone()
            }
        })
        .unwrap_or_default();
    let parent_id = message
        .reply_to_id
        .clone()
        .filter(|id| !id.trim().is_empty());
    let title = message
        .subject
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let mut entry = BoardEntry::new(
        AuthorKind::User,
        author,
        BoardEntryKind::Status,
        body,
        None,
        parent_id,
        vec![],
        vec![],
    );
    entry.id = message.id.clone();
    entry.title = title;
    if let Some(dt) = message
        .created_date_time
        .as_deref()
        .and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
    {
        let utc = dt.with_timezone(&Utc);
        entry.created_at = utc;
        entry.updated_at = utc;
    }
    let workspace = workspace.trim();
    if !workspace.is_empty() {
        entry = entry.with_audience(vec![workspace.to_string()]);
    }
    entry
}

impl BoardProvider for TeamsProvider {
    fn post_entry(&self, worktree_root: &Path, entry: BoardEntry) -> Result<CoordinationSnapshot> {
        let channel =
            mapping::resolve_channel(&entry, &self.channel_map, Some(&self.default_channel))
                .ok_or_else(|| {
                    GwtError::Other("teams: no channel resolved for post".to_string())
                })?;
        let (team, chan) = split_channel(&channel)?;
        // SPEC-2963 Workspace threading: every post is a reply under the
        // Workspace (or General) thread root — a summary card created once and
        // refreshed when the Workspace changes. `entry.parent_id` collapses into
        // the single Workspace thread (Teams channel replies are one level deep).
        let root_id = self.ensure_thread_root(worktree_root, &team, &chan, &channel, &entry)?;
        // The reply carries a "who · kind · origin" meta line plus the entry
        // title so a reader can tell who posted, the entry type, and its subject
        // (SPEC-2963). Replies cannot carry a Graph subject, so both live in the
        // body; the title would otherwise be dropped (Slack shows it as a header).
        let meta = mapping::board_entry_meta_line(&entry);
        self.post_graph_message(
            &team,
            &chan,
            Some(&meta),
            entry.title.as_deref(),
            &entry.body,
            Some(&root_id),
        )?;
        self.cache.invalidate();
        self.load_snapshot(worktree_root)
    }

    fn load_snapshot(&self, _worktree_root: &Path) -> Result<CoordinationSnapshot> {
        Ok(Self::snapshot_from(self.cached_history()?))
    }

    fn load_snapshot_for_scope(
        &self,
        worktree_root: &Path,
        _scope: &BoardAudienceScope,
    ) -> Result<CoordinationSnapshot> {
        self.load_snapshot(worktree_root)
    }

    fn load_entries_since(
        &self,
        _worktree_root: &Path,
        since: DateTime<Utc>,
    ) -> Result<Vec<BoardEntry>> {
        Ok(self
            .cached_history()?
            .into_iter()
            .filter(|entry| entry.updated_at > since)
            .collect())
    }

    fn load_entries_since_for_scope(
        &self,
        worktree_root: &Path,
        since: DateTime<Utc>,
        _scope: &BoardAudienceScope,
    ) -> Result<Vec<BoardEntry>> {
        self.load_entries_since(worktree_root, since)
    }

    fn has_recent_post_by(
        &self,
        _worktree_root: &Path,
        author: &str,
        kind: &BoardEntryKind,
        within: chrono::Duration,
    ) -> Result<bool> {
        let threshold = Utc::now() - within;
        Ok(self.cached_history()?.iter().any(|entry| {
            entry.author == author && entry.kind == *kind && entry.updated_at > threshold
        }))
    }

    fn board_entry_exists(&self, _worktree_root: &Path, entry_id: &str) -> Result<bool> {
        Ok(self
            .cached_history()?
            .iter()
            .any(|entry| entry.id == entry_id))
    }

    fn load_entries_before(
        &self,
        _worktree_root: &Path,
        before_entry_id: Option<&str>,
        limit: usize,
    ) -> Result<BoardHistoryPage> {
        if limit == 0 {
            return Ok(BoardHistoryPage::default());
        }
        let entries = self.cached_history()?;
        let cutoff = before_entry_id
            .and_then(|id| entries.iter().position(|entry| entry.id == id))
            .unwrap_or(entries.len());
        let older = &entries[..cutoff];
        let has_more_before = older.len() > limit;
        let start = older.len().saturating_sub(limit);
        Ok(BoardHistoryPage {
            entries: older[start..].to_vec(),
            has_more_before,
        })
    }

    fn load_entries_before_for_scope(
        &self,
        worktree_root: &Path,
        before_entry_id: Option<&str>,
        limit: usize,
        _scope: &BoardAudienceScope,
    ) -> Result<BoardHistoryPage> {
        self.load_entries_before(worktree_root, before_entry_id, limit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Mutex;

    struct MockGraph {
        messages_body: String,
        messages_status: u16,
        post_status: u16,
        last_post_url: Mutex<String>,
        last_post_body: Mutex<String>,
    }

    impl Default for MockGraph {
        fn default() -> Self {
            Self {
                messages_body: r#"{"value":[]}"#.to_string(),
                messages_status: 200,
                post_status: 201,
                last_post_url: Mutex::new(String::new()),
                last_post_body: Mutex::new(String::new()),
            }
        }
    }

    impl HttpClient for MockGraph {
        fn get(
            &self,
            url: &str,
            _bearer: &str,
            _query: &[(&str, &str)],
        ) -> std::result::Result<HttpResponse, String> {
            assert!(url.contains("/messages"));
            Ok(HttpResponse {
                status: self.messages_status,
                body: self.messages_body.clone(),
                retry_after: if self.messages_status == 429 {
                    Some(30)
                } else {
                    None
                },
            })
        }
        fn post_form(
            &self,
            _url: &str,
            _bearer: &str,
            _params: &[(&str, &str)],
        ) -> std::result::Result<HttpResponse, String> {
            Err("teams uses post_json".to_string())
        }
        fn post_json(
            &self,
            url: &str,
            _bearer: &str,
            body: &str,
        ) -> std::result::Result<HttpResponse, String> {
            *self.last_post_url.lock().unwrap() = url.to_string();
            *self.last_post_body.lock().unwrap() = body.to_string();
            Ok(HttpResponse {
                status: self.post_status,
                body: r#"{"id":"m-new"}"#.to_string(),
                retry_after: None,
            })
        }
    }

    /// Unique throwaway repo root per call so the SPEC-2963 root mapping is
    /// isolated from the real working tree (mirrors the Slack tests).
    fn root() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let mut path = std::env::temp_dir();
        path.push(format!(
            "gwt-board-roots-teams-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::create_dir_all(&path);
        path
    }

    /// Records every post_json / patch_json (url + body) and returns an
    /// incrementing message id so root get-or-create works in tests.
    struct RecordingGraph {
        posts: std::sync::Arc<Mutex<Vec<(String, String)>>>,
        patches: std::sync::Arc<Mutex<Vec<(String, String)>>>,
        id: std::sync::Arc<std::sync::atomic::AtomicU64>,
    }

    impl RecordingGraph {
        fn new() -> Self {
            Self {
                posts: std::sync::Arc::new(Mutex::new(Vec::new())),
                patches: std::sync::Arc::new(Mutex::new(Vec::new())),
                id: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            }
        }
        fn posts(&self) -> std::sync::Arc<Mutex<Vec<(String, String)>>> {
            self.posts.clone()
        }
        fn patches(&self) -> std::sync::Arc<Mutex<Vec<(String, String)>>> {
            self.patches.clone()
        }
    }

    impl HttpClient for RecordingGraph {
        fn get(
            &self,
            _u: &str,
            _b: &str,
            _q: &[(&str, &str)],
        ) -> std::result::Result<HttpResponse, String> {
            Ok(HttpResponse {
                status: 200,
                body: r#"{"value":[]}"#.to_string(),
                retry_after: None,
            })
        }
        fn post_form(
            &self,
            _u: &str,
            _b: &str,
            _p: &[(&str, &str)],
        ) -> std::result::Result<HttpResponse, String> {
            Err("teams uses post_json".to_string())
        }
        fn post_json(
            &self,
            url: &str,
            _b: &str,
            body: &str,
        ) -> std::result::Result<HttpResponse, String> {
            self.posts
                .lock()
                .unwrap()
                .push((url.to_string(), body.to_string()));
            let n = self.id.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            Ok(HttpResponse {
                status: 201,
                body: format!(r#"{{"id":"m-{n}"}}"#),
                retry_after: None,
            })
        }
        fn patch_json(
            &self,
            url: &str,
            _b: &str,
            body: &str,
        ) -> std::result::Result<HttpResponse, String> {
            self.patches
                .lock()
                .unwrap()
                .push((url.to_string(), body.to_string()));
            Ok(HttpResponse {
                status: 200,
                body: r#"{"id":"patched"}"#.to_string(),
                retry_after: None,
            })
        }
    }

    fn entry(body: &str) -> BoardEntry {
        BoardEntry::new(
            AuthorKind::User,
            "You",
            BoardEntryKind::Status,
            body,
            None,
            None,
            vec![],
            vec![],
        )
    }

    #[test]
    fn load_snapshot_maps_graph_messages() {
        let mock = MockGraph {
            messages_body: r#"{"value":[
                {"id":"m1","createdDateTime":"2026-01-01T10:00:00Z","body":{"content":"hi"},"from":{"user":{"displayName":"Akio"}}},
                {"id":"m2","createdDateTime":"2026-01-01T10:05:00Z","replyToId":"m1","body":{"content":"re"}}
            ]}"#
            .to_string(),
            ..Default::default()
        };
        let prov = TeamsProvider::new("tok", "team-1/chan-1", BTreeMap::new(), Box::new(mock), 60);
        let snapshot = prov.load_snapshot(&root()).unwrap();
        assert_eq!(snapshot.board.entries.len(), 2);
        assert_eq!(snapshot.board.entries[0].id, "m1");
        assert_eq!(snapshot.board.entries[0].author, "Akio");
        assert_eq!(snapshot.board.entries[1].parent_id.as_deref(), Some("m1"));
    }

    #[test]
    fn post_entry_creates_root_then_replies_under_it() {
        // SPEC-2963: the entry threads under the Workspace root, not under a raw
        // parent_id. First a top-level channel message (root), then a reply to
        // that root's message id.
        let mut map = BTreeMap::new();
        map.insert("ws-a".to_string(), "team-1/chan-1".to_string());
        let mock = RecordingGraph::new();
        let posts = mock.posts();
        let prov = TeamsProvider::new("tok", "team-1/chan-1", map, Box::new(mock), 60);
        let root = root();
        prov.post_entry(
            &root,
            entry("threaded").with_audience(vec!["ws-a".to_string()]),
        )
        .unwrap();
        let calls = posts.lock().unwrap().clone();
        assert_eq!(calls.len(), 2, "root create + reply");
        assert!(calls[0]
            .0
            .ends_with("/teams/team-1/channels/chan-1/messages"));
        assert!(calls[1]
            .0
            .contains("/teams/team-1/channels/chan-1/messages/m-1/replies"));
        assert!(calls[1].1.contains("threaded"));
    }

    // Share one MockGraph between the provider and the test assertions.
    struct MockGraphShared(std::sync::Arc<MockGraph>);
    impl HttpClient for MockGraphShared {
        fn get(
            &self,
            url: &str,
            b: &str,
            q: &[(&str, &str)],
        ) -> std::result::Result<HttpResponse, String> {
            self.0.get(url, b, q)
        }
        fn post_form(
            &self,
            url: &str,
            b: &str,
            p: &[(&str, &str)],
        ) -> std::result::Result<HttpResponse, String> {
            self.0.post_form(url, b, p)
        }
        fn post_json(
            &self,
            url: &str,
            b: &str,
            body: &str,
        ) -> std::result::Result<HttpResponse, String> {
            self.0.post_json(url, b, body)
        }
    }

    #[test]
    fn invalid_channel_format_errors() {
        let prov = TeamsProvider::new(
            "tok",
            "no-slash",
            BTreeMap::new(),
            Box::new(MockGraph::default()),
            60,
        );
        assert!(prov.load_snapshot(&root()).is_err());
    }

    #[test]
    fn rate_limit_surfaces_error() {
        let mock = MockGraph {
            messages_status: 429,
            ..Default::default()
        };
        let prov = TeamsProvider::new("tok", "team-1/chan-1", BTreeMap::new(), Box::new(mock), 60);
        let err = prov.load_snapshot(&root()).unwrap_err();
        assert!(err.to_string().contains("rate limited"));
    }

    #[test]
    fn skips_system_and_deleted_messages() {
        let mock = MockGraph {
            messages_body: r#"{"value":[
                {"id":"m1","createdDateTime":"2026-01-01T10:00:00Z","body":{"content":"hi"},"from":{"user":{"displayName":"Akio"}}},
                {"id":"sys","messageType":"systemEventMessage","createdDateTime":"2026-01-01T10:01:00Z","body":{"content":"joined the channel"}},
                {"id":"del","createdDateTime":"2026-01-01T10:02:00Z","deletedDateTime":"2026-01-01T10:03:00Z","body":{"content":""}}
            ]}"#
            .to_string(),
            ..Default::default()
        };
        let prov = TeamsProvider::new("tok", "team-1/chan-1", BTreeMap::new(), Box::new(mock), 60);
        let snapshot = prov.load_snapshot(&root()).unwrap();
        assert_eq!(
            snapshot.board.entries.len(),
            1,
            "system + deleted must be skipped"
        );
        assert_eq!(snapshot.board.entries[0].id, "m1");
    }

    #[test]
    fn forbidden_surfaces_membership_hint() {
        let mock = MockGraph {
            messages_status: 403,
            ..Default::default()
        };
        let prov = TeamsProvider::new("tok", "team-1/chan-1", BTreeMap::new(), Box::new(mock), 60);
        let err = prov.load_snapshot(&root()).unwrap_err().to_string();
        assert!(
            err.contains("forbidden") && err.contains("member"),
            "403 must surface an actionable membership hint: {err}"
        );
    }

    #[test]
    fn root_card_carries_subject_and_reply_renders_html_without_subject() {
        // SPEC-2963: the Workspace root card carries the subject + html content;
        // the entry reply carries html body and no subject (Graph rejects it).
        let mut map = BTreeMap::new();
        map.insert("ws-a".to_string(), "team-1/chan-1".to_string());
        let mock = RecordingGraph::new();
        let posts = mock.posts();
        let prov = TeamsProvider::new("tok", "team-1/chan-1", map, Box::new(mock), 60);
        let root = root();
        let mut e = entry("**bold** and _italic_").with_audience(vec!["ws-a".to_string()]);
        e.origin_branch = Some("feature/x".to_string());
        prov.post_entry(&root, e).unwrap();
        let calls = posts.lock().unwrap().clone();
        // Root card.
        assert!(calls[0].1.contains("\"contentType\":\"html\""));
        assert!(
            calls[0].1.contains("\"subject\""),
            "root card carries a subject: {}",
            calls[0].1
        );
        // Reply.
        assert!(
            calls[1].1.contains("<strong>bold</strong>"),
            "markdown bold must render to <strong>: {}",
            calls[1].1
        );
        assert!(
            !calls[1].1.contains("\"subject\""),
            "replies must not carry a subject: {}",
            calls[1].1
        );
    }

    #[test]
    fn reply_payload_renders_multiline_body_as_teams_br_tags() {
        // SPEC-2963 Phase 9 regression: Teams renders raw newline text nodes in
        // Graph HTML bodies as visible "n" boxes. The provider payload must
        // carry explicit <br> tags instead.
        let mut map = BTreeMap::new();
        map.insert("ws-a".to_string(), "team-1/chan-1".to_string());
        let mock = RecordingGraph::new();
        let posts = mock.posts();
        let prov = TeamsProvider::new("tok", "team-1/chan-1", map, Box::new(mock), 60);
        let root = root();
        prov.post_entry(
            &root,
            entry("Current state: A\n\nReason: B\n\nNext: C")
                .with_audience(vec!["ws-a".to_string()]),
        )
        .unwrap();
        let calls = posts.lock().unwrap().clone();
        let payload: serde_json::Value = serde_json::from_str(&calls[1].1).unwrap();
        let content = payload["body"]["content"].as_str().unwrap();
        assert!(
            content.contains("Current state: A<br><br>Reason: B<br><br>Next: C"),
            "multiline reply content must use Teams-safe breaks: {content}"
        );
        assert!(
            !content.contains('\n'),
            "Teams HTML content must not carry raw newlines: {content:?}"
        );
        assert!(
            !content.contains("\\n"),
            "Teams HTML content must not carry escaped newlines: {content:?}"
        );
    }

    #[test]
    fn second_post_reuses_root_and_general_for_broadcast() {
        // get-or-create: same Workspace reuses its root; broadcast uses General.
        let mut map = BTreeMap::new();
        map.insert("ws-a".to_string(), "team-1/chan-1".to_string());
        let mock = RecordingGraph::new();
        let posts = mock.posts();
        let prov = TeamsProvider::new("tok", "team-1/chan-1", map, Box::new(mock), 60);
        let root = root();
        prov.post_entry(&root, entry("one").with_audience(vec!["ws-a".to_string()]))
            .unwrap();
        prov.post_entry(&root, entry("two").with_audience(vec!["ws-a".to_string()]))
            .unwrap();
        // 1 root + 2 replies = 3 posts (root reused).
        assert_eq!(posts.lock().unwrap().len(), 3);
        let mappings = gwt_core::board_remote_roots::load_root_mappings(&root);
        assert_eq!(
            mappings.keys().filter(|(_, _, key)| key == "ws-a").count(),
            1
        );

        // A broadcast post (no audience) opens a distinct General root.
        prov.post_entry(&root, entry("broadcast")).unwrap();
        let mappings = gwt_core::board_remote_roots::load_root_mappings(&root);
        assert!(mappings.keys().any(|(_, _, key)| key == "general"));
    }

    #[test]
    fn changed_card_triggers_graph_patch() {
        // SPEC-2963 root refresh: a stale stored hash triggers a PATCH against
        // the existing root before the reply is threaded under it.
        let mut map = BTreeMap::new();
        map.insert("ws-a".to_string(), "team-1/chan-1".to_string());
        let mock = RecordingGraph::new();
        let posts = mock.posts();
        let patches = mock.patches();
        let prov = TeamsProvider::new("tok", "team-1/chan-1", map, Box::new(mock), 60);
        let root = root();
        gwt_core::board_remote_roots::append_root_mapping(
            &root,
            &gwt_core::board_remote_roots::RootMapping {
                key: "ws-a".to_string(),
                provider: "teams".to_string(),
                channel: "team-1/chan-1".to_string(),
                root_id: "OLD".to_string(),
                card_hash: "stale".to_string(),
                updated_at: DateTime::<Utc>::from_timestamp(0, 0).unwrap(),
            },
        )
        .unwrap();
        prov.post_entry(&root, entry("x").with_audience(vec!["ws-a".to_string()]))
            .unwrap();

        // One PATCH against the existing root, and only the reply is posted.
        let patches = patches.lock().unwrap().clone();
        assert_eq!(patches.len(), 1, "stale root patched once");
        assert!(patches[0]
            .0
            .contains("/teams/team-1/channels/chan-1/messages/OLD"));
        let posts = posts.lock().unwrap().clone();
        assert_eq!(posts.len(), 1, "no new root created");
        assert!(posts[0]
            .0
            .contains("/teams/team-1/channels/chan-1/messages/OLD/replies"));
    }

    #[test]
    fn reply_carries_author_meta_root_does_not() {
        // SPEC-2963: the reply body leads with a bold "who · kind" meta so a
        // reader can tell who posted; the root summary card carries no meta.
        let mut map = BTreeMap::new();
        map.insert("ws-a".to_string(), "team-1/chan-1".to_string());
        let mock = RecordingGraph::new();
        let posts = mock.posts();
        let prov = TeamsProvider::new("tok", "team-1/chan-1", map, Box::new(mock), 60);
        let root = root();
        prov.post_entry(
            &root,
            entry("hello").with_audience(vec!["ws-a".to_string()]),
        )
        .unwrap();
        let calls = posts.lock().unwrap().clone();
        // Root card (calls[0]) is the Workspace summary, no author/kind meta.
        assert!(
            !calls[0].1.contains("(user)") && !calls[0].1.contains("· status"),
            "root card has no meta: {}",
            calls[0].1
        );
        // Reply (calls[1]) leads with the who·kind meta.
        assert!(
            calls[1].1.contains("You (user)") && calls[1].1.contains("status"),
            "reply carries the meta: {}",
            calls[1].1
        );
    }

    #[test]
    fn reply_renders_title_in_body_without_subject() {
        // SPEC-2963: Graph rejects a `subject` on /replies, so the entry title
        // is rendered into the reply body (otherwise it is dropped — Slack shows
        // it as a header block). The payload must still carry no `subject` key.
        let recorded = std::sync::Arc::new(MockGraph::default());
        let prov = TeamsProvider::new(
            "tok",
            "team-1/chan-1",
            BTreeMap::new(),
            Box::new(MockGraphShared(recorded.clone())),
            60,
        );
        let mut reply = entry("body text").with_title("Release v2");
        reply.parent_id = Some("m1".to_string());
        prov.post_entry(&root(), reply).unwrap();
        let body = recorded.last_post_body.lock().unwrap().clone();
        assert!(
            !body.contains("\"subject\""),
            "replies must not carry a subject (Graph rejects it): {body}"
        );
        assert!(
            body.contains("Release v2"),
            "reply body carries the entry title: {body}"
        );
    }

    #[test]
    fn read_back_strips_html_and_maps_subject_to_title() {
        let mock = MockGraph {
            messages_body: r#"{"value":[
                {"id":"m1","createdDateTime":"2026-01-01T10:00:00Z","subject":"Release notes","body":{"contentType":"html","content":"<strong>bold</strong> text"},"from":{"user":{"displayName":"Akio"}}}
            ]}"#
            .to_string(),
            ..Default::default()
        };
        let prov = TeamsProvider::new("tok", "team-1/chan-1", BTreeMap::new(), Box::new(mock), 60);
        let snapshot = prov.load_snapshot(&root()).unwrap();
        assert_eq!(snapshot.board.entries.len(), 1);
        let e = &snapshot.board.entries[0];
        assert_eq!(e.title.as_deref(), Some("Release notes"));
        assert_eq!(
            e.body, "bold text",
            "html tags stripped for plaintext board"
        );
    }

    fn three_message_mock() -> MockGraph {
        MockGraph {
            messages_body: r#"{"value":[
                {"id":"m1","createdDateTime":"2026-01-01T10:00:00Z","body":{"content":"a"},"from":{"user":{"displayName":"U"}}},
                {"id":"m2","createdDateTime":"2026-01-01T10:05:00Z","body":{"content":"b"},"from":{"user":{"displayName":"U"}}},
                {"id":"m3","createdDateTime":"2026-01-01T10:10:00Z","body":{"content":"c"},"from":{"user":{"displayName":"U"}}}
            ]}"#
            .to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn second_read_is_served_from_cache() {
        let prov = TeamsProvider::new(
            "tok",
            "team-1/chan-1",
            BTreeMap::new(),
            Box::new(three_message_mock()),
            60,
        );
        assert_eq!(prov.load_snapshot(&root()).unwrap().board.entries.len(), 3);
        // Within the TTL the second read hits the cache rather than the mock.
        assert_eq!(prov.load_snapshot(&root()).unwrap().board.entries.len(), 3);
    }

    #[test]
    fn channel_with_empty_segment_errors() {
        let prov = TeamsProvider::new(
            "tok",
            "team-1/",
            BTreeMap::new(),
            Box::new(MockGraph::default()),
            60,
        );
        assert!(prov.load_snapshot(&root()).is_err());
    }

    #[test]
    fn http_5xx_surfaces_error() {
        let mock = MockGraph {
            messages_status: 500,
            ..Default::default()
        };
        let prov = TeamsProvider::new("tok", "team-1/chan-1", BTreeMap::new(), Box::new(mock), 60);
        let err = prov.load_snapshot(&root()).unwrap_err();
        assert!(err.to_string().contains("http 500"));
    }

    #[test]
    fn mapped_channel_tags_entries_with_workspace_audience() {
        let mut map = BTreeMap::new();
        map.insert("ws-x".to_string(), "team-1/chan-1".to_string());
        let prov = TeamsProvider::new(
            "tok",
            "team-1/chan-1",
            map,
            Box::new(three_message_mock()),
            60,
        );
        let snapshot = prov.load_snapshot(&root()).unwrap();
        assert!(snapshot
            .board
            .entries
            .iter()
            .all(|entry| entry.audience.contains(&"ws-x".to_string())));
    }

    #[test]
    fn post_without_resolvable_channel_errors() {
        let prov = TeamsProvider::new(
            "tok",
            "",
            BTreeMap::new(),
            Box::new(MockGraph::default()),
            60,
        );
        assert!(prov.post_entry(&root(), entry("x")).is_err());
    }

    #[test]
    fn trait_read_methods_cover_since_recent_exists_and_pagination() {
        let prov = TeamsProvider::new(
            "tok",
            "team-1/chan-1",
            BTreeMap::new(),
            Box::new(three_message_mock()),
            60,
        );
        let scope = BoardAudienceScope::All;

        assert_eq!(
            prov.load_snapshot_for_scope(&root(), &scope)
                .unwrap()
                .board
                .entries
                .len(),
            3
        );

        let epoch = DateTime::<Utc>::from_timestamp(0, 0).unwrap();
        assert_eq!(prov.load_entries_since(&root(), epoch).unwrap().len(), 3);
        assert_eq!(
            prov.load_entries_since_for_scope(&root(), epoch, &scope)
                .unwrap()
                .len(),
            3
        );

        let wide = chrono::Duration::days(1_000_000);
        assert!(prov
            .has_recent_post_by(&root(), "nobody", &BoardEntryKind::Status, wide)
            .is_ok());

        assert!(prov.board_entry_exists(&root(), "m2").unwrap());
        assert!(!prov.board_entry_exists(&root(), "missing").unwrap());

        let zero = prov.load_entries_before(&root(), None, 0).unwrap();
        assert!(zero.entries.is_empty() && !zero.has_more_before);

        let page = prov.load_entries_before(&root(), None, 2).unwrap();
        assert_eq!(page.entries.len(), 2);
        assert!(page.has_more_before);

        let before = prov.load_entries_before(&root(), Some("m2"), 5).unwrap();
        assert_eq!(before.entries.len(), 1);
        assert!(!before.has_more_before);

        assert_eq!(
            prov.load_entries_before_for_scope(&root(), None, 2, &scope)
                .unwrap()
                .entries
                .len(),
            2
        );
    }

    #[test]
    fn mock_post_form_paths_are_unsupported() {
        // Teams posts via post_json; the form path is never used and reports so.
        assert!(MockGraph::default().post_form("u", "b", &[]).is_err());
        let shared = MockGraphShared(std::sync::Arc::new(MockGraph::default()));
        assert!(shared.post_form("u", "b", &[]).is_err());
    }
}
