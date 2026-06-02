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
        .map(|b| b.content.clone())
        .unwrap_or_default();
    let parent_id = message
        .reply_to_id
        .clone()
        .filter(|id| !id.trim().is_empty());

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
        let url = match entry.parent_id.as_deref() {
            Some(parent) => {
                format!("{GRAPH_API}/teams/{team}/channels/{chan}/messages/{parent}/replies")
            }
            None => format!("{GRAPH_API}/teams/{team}/channels/{chan}/messages"),
        };
        // Pin contentType=text so the body is posted verbatim (no HTML
        // interpretation of `<`, `&`, @mentions, etc.).
        let payload =
            serde_json::json!({ "body": { "contentType": "text", "content": entry.body } })
                .to_string();
        let response = self
            .http
            .post_json(&url, &self.token, &payload)
            .map_err(GwtError::Other)?;
        check_status(&response, "post message")?;
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

    fn root() -> PathBuf {
        PathBuf::from(".")
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
    fn post_entry_uses_replies_endpoint_for_threaded_post() {
        let recorded = std::sync::Arc::new(MockGraph::default());
        let prov = TeamsProvider::new(
            "tok",
            "team-1/chan-1",
            BTreeMap::new(),
            Box::new(MockGraphShared(recorded.clone())),
            60,
        );
        let mut e = entry("threaded");
        e.parent_id = Some("m-parent".to_string());
        prov.post_entry(&root(), e).unwrap();
        let url = recorded.last_post_url.lock().unwrap().clone();
        assert!(url.contains("/teams/team-1/channels/chan-1/messages/m-parent/replies"));
        let body = recorded.last_post_body.lock().unwrap().clone();
        assert!(body.contains("threaded"));
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
    fn post_body_pins_text_content_type() {
        let recorded = std::sync::Arc::new(MockGraph::default());
        let prov = TeamsProvider::new(
            "tok",
            "team-1/chan-1",
            BTreeMap::new(),
            Box::new(MockGraphShared(recorded.clone())),
            60,
        );
        prov.post_entry(&root(), entry("hello world")).unwrap();
        let body = recorded.last_post_body.lock().unwrap().clone();
        assert!(
            body.contains("\"contentType\":\"text\""),
            "post body must pin contentType=text: {body}"
        );
        assert!(body.contains("hello world"));
    }
}
