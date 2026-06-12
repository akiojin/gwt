//! Slack remote Board provider (SPEC-2963 FR-003/008/009/010/014).
//!
//! Implements the (synchronous) [`BoardProvider`] trait against the Slack Web
//! API. HTTP is abstracted behind [`HttpClient`] so the provider is unit
//! testable with a mock; the production client (blocking reqwest) is wired in a
//! later phase. Reads are served from a short time-window cache to stay within
//! Slack's `conversations.history` rate limit (FR-009). API/network failures
//! surface as errors — never a silent fall back to local (FR-010).

use std::collections::BTreeMap;
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use gwt_core::coordination::{
    BoardAudienceScope, BoardEntry, BoardEntryKind, BoardHistoryPage, BoardProjection,
    BoardProvider, CoordinationSnapshot,
};
use gwt_core::{GwtError, Result};

use super::cache::TimedCache;
use super::mapping::{self, SlackMessage};
use super::markdown;

const SLACK_API: &str = "https://slack.com/api";
/// Slack caps `conversations.history` at 15 objects for non-Marketplace apps.
const HISTORY_LIMIT: usize = 15;

/// Minimal HTTP response surfaced to the provider.
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
    /// `Retry-After` seconds parsed from a 429 response, if present.
    pub retry_after: Option<u64>,
}

/// HTTP abstraction so the Slack provider is unit-testable without network.
pub trait HttpClient: Send + Sync {
    /// GET `url` with a bearer token and query parameters.
    fn get(
        &self,
        url: &str,
        bearer: &str,
        query: &[(&str, &str)],
    ) -> std::result::Result<HttpResponse, String>;
    /// POST `params` as `application/x-www-form-urlencoded` with a bearer token.
    fn post_form(
        &self,
        url: &str,
        bearer: &str,
        params: &[(&str, &str)],
    ) -> std::result::Result<HttpResponse, String>;
    /// POST a raw JSON `body` with a bearer token (used by the Microsoft Graph
    /// Teams provider). Defaults to unsupported so form-only clients/mocks need
    /// not implement it.
    fn post_json(
        &self,
        _url: &str,
        _bearer: &str,
        _body: &str,
    ) -> std::result::Result<HttpResponse, String> {
        Err("post_json is not supported by this HTTP client".to_string())
    }
    /// PATCH a raw JSON `body` with a bearer token (used by the Microsoft Graph
    /// Teams provider to update a Workspace root summary card; SPEC-2963).
    fn patch_json(
        &self,
        _url: &str,
        _bearer: &str,
        _body: &str,
    ) -> std::result::Result<HttpResponse, String> {
        Err("patch_json is not supported by this HTTP client".to_string())
    }
}

/// Slack-backed Board provider.
pub struct SlackProvider {
    token: String,
    default_channel: String,
    channel_map: BTreeMap<String, String>,
    http: Box<dyn HttpClient>,
    cache: TimedCache<Vec<BoardEntry>>,
}

impl SlackProvider {
    /// Build a provider. `cache_ttl_secs` bounds how long read results are
    /// reused to stay within Slack rate limits (FR-009; default 60).
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

    /// Reverse-map a Slack channel id to the gwt Work it represents.
    fn workspace_for_channel(&self, channel: &str) -> String {
        self.channel_map
            .iter()
            .find(|(_, ch)| ch.as_str() == channel)
            .map(|(ws, _)| ws.clone())
            .unwrap_or_default()
    }

    fn fetch_history(&self, channel: &str) -> Result<Vec<BoardEntry>> {
        let limit = HISTORY_LIMIT.to_string();
        let response = self
            .http
            .get(
                &format!("{SLACK_API}/conversations.history"),
                &self.token,
                &[("channel", channel), ("limit", &limit)],
            )
            .map_err(GwtError::Other)?;
        check_status(&response, "conversations.history")?;
        let parsed: SlackHistory = serde_json::from_str(&response.body)
            .map_err(|err| GwtError::Other(format!("slack history parse: {err}")))?;
        if !parsed.ok {
            return Err(GwtError::Other(format!(
                "slack conversations.history error: {}",
                parsed.error.unwrap_or_else(|| "unknown".to_string())
            )));
        }
        let workspace = self.workspace_for_channel(channel);
        let mut entries: Vec<BoardEntry> = parsed
            .messages
            .iter()
            .map(|message| mapping::slack_message_to_board_entry(&message.to_message(), &workspace))
            .collect();
        entries.sort_by_key(|entry| entry.created_at);
        Ok(entries)
    }

    /// History for the default channel, served from cache within the TTL.
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

    /// Build Block Kit blocks (optional meta context + header + body section)
    /// and a plain-text fallback. `meta` is a small "who · kind · origin" line
    /// shown only on entry replies (SPEC-2963), not on root cards. Empty blocks
    /// serialize to `"[]"`.
    fn build_blocks(
        meta: Option<&str>,
        title: Option<&str>,
        body_markdown: &str,
    ) -> (String, String) {
        let title = title.map(str::trim).filter(|t| !t.is_empty());
        let mrkdwn = markdown::markdown_to_slack_mrkdwn(body_markdown);
        let mut blocks: Vec<serde_json::Value> = Vec::new();
        if let Some(meta) = meta.map(str::trim).filter(|m| !m.is_empty()) {
            blocks.push(serde_json::json!({
                "type": "context",
                "elements": [{ "type": "mrkdwn", "text": meta }]
            }));
        }
        if let Some(title) = title {
            let header_text: String = title.chars().take(150).collect();
            blocks.push(serde_json::json!({
                "type": "header",
                "text": { "type": "plain_text", "text": header_text, "emoji": true }
            }));
        }
        if !mrkdwn.trim().is_empty() {
            let section_text: String = mrkdwn.chars().take(3000).collect();
            blocks.push(serde_json::json!({
                "type": "section",
                "text": { "type": "mrkdwn", "text": section_text }
            }));
        }
        let blocks_json = serde_json::Value::Array(blocks).to_string();
        let body_fallback = match title {
            Some(title) => format!("{title}\n{body_markdown}"),
            None => body_markdown.to_string(),
        };
        let fallback = match meta.map(str::trim).filter(|m| !m.is_empty()) {
            Some(meta) => format!("{meta}\n{body_fallback}"),
            None => body_fallback,
        };
        (blocks_json, fallback)
    }

    /// Post a message (optionally as a reply under `thread_ts`). `meta` adds a
    /// "who · kind · origin" context line (entry replies only). Returns the new
    /// message ts (the thread root id for a root post).
    fn post_message(
        &self,
        channel: &str,
        meta: Option<&str>,
        title: Option<&str>,
        body_markdown: &str,
        thread_ts: Option<&str>,
    ) -> Result<String> {
        let (blocks_json, fallback) = Self::build_blocks(meta, title, body_markdown);
        let mut params: Vec<(&str, &str)> = vec![("channel", channel), ("text", &fallback)];
        if blocks_json != "[]" {
            params.push(("blocks", &blocks_json));
        }
        if let Some(thread_ts) = thread_ts {
            params.push(("thread_ts", thread_ts));
        }
        let response = self
            .http
            .post_form(
                &format!("{SLACK_API}/chat.postMessage"),
                &self.token,
                &params,
            )
            .map_err(GwtError::Other)?;
        check_status(&response, "chat.postMessage")?;
        let parsed: SlackPostResponse = serde_json::from_str(&response.body)
            .map_err(|err| GwtError::Other(format!("slack post parse: {err}")))?;
        if !parsed.ok {
            return Err(GwtError::Other(format!(
                "slack chat.postMessage error: {}",
                parsed.error.unwrap_or_else(|| "unknown".to_string())
            )));
        }
        parsed
            .ts
            .filter(|ts| !ts.is_empty())
            .ok_or_else(|| GwtError::Other("slack chat.postMessage returned no ts".to_string()))
    }

    /// Update an existing message (the Workspace root summary card; SPEC-2963).
    /// The root card carries no per-entry meta line.
    fn update_message(
        &self,
        channel: &str,
        ts: &str,
        title: Option<&str>,
        body_markdown: &str,
    ) -> Result<()> {
        let (blocks_json, fallback) = Self::build_blocks(None, title, body_markdown);
        let mut params: Vec<(&str, &str)> =
            vec![("channel", channel), ("ts", ts), ("text", &fallback)];
        if blocks_json != "[]" {
            params.push(("blocks", &blocks_json));
        }
        let response = self
            .http
            .post_form(&format!("{SLACK_API}/chat.update"), &self.token, &params)
            .map_err(GwtError::Other)?;
        check_status(&response, "chat.update")?;
        let parsed: SlackPostResponse = serde_json::from_str(&response.body)
            .map_err(|err| GwtError::Other(format!("slack update parse: {err}")))?;
        if !parsed.ok {
            return Err(GwtError::Other(format!(
                "slack chat.update error: {}",
                parsed.error.unwrap_or_else(|| "unknown".to_string())
            )));
        }
        Ok(())
    }

    /// SPEC-2963: get-or-create (and refresh on change) the Workspace/General
    /// thread root for an entry. Returns the root ts to thread the reply under.
    fn ensure_thread_root(
        &self,
        worktree_root: &Path,
        channel: &str,
        entry: &BoardEntry,
    ) -> Result<String> {
        let key = mapping::thread_key_for_entry(entry);
        let item =
            gwt_core::work_projection::load_or_synthesize_workspace_work_items(worktree_root)
                .ok()
                .and_then(|proj| proj.work_items.into_iter().find(|work| work.id == key));
        let (card_title, card_body) =
            mapping::workspace_summary_card(&key, item.as_ref(), entry.origin_branch.as_deref());
        let hash = mapping::card_hash(&card_title, &card_body);

        if let Some(existing) =
            gwt_core::board_remote_roots::find_root_mapping(worktree_root, "slack", channel, &key)
        {
            if existing.card_hash != hash {
                // Best-effort: a root-card refresh failure must not block the
                // post itself (the reply still lands in the thread).
                let _ =
                    self.update_message(channel, &existing.root_id, Some(&card_title), &card_body);
                let _ = gwt_core::board_remote_roots::append_root_mapping(
                    worktree_root,
                    &gwt_core::board_remote_roots::RootMapping {
                        key,
                        provider: "slack".to_string(),
                        channel: channel.to_string(),
                        root_id: existing.root_id.clone(),
                        card_hash: hash,
                        updated_at: Utc::now(),
                    },
                );
            }
            return Ok(existing.root_id);
        }

        let root_ts = self.post_message(channel, None, Some(&card_title), &card_body, None)?;
        let _ = gwt_core::board_remote_roots::append_root_mapping(
            worktree_root,
            &gwt_core::board_remote_roots::RootMapping {
                key,
                provider: "slack".to_string(),
                channel: channel.to_string(),
                root_id: root_ts.clone(),
                card_hash: hash,
                updated_at: Utc::now(),
            },
        );
        Ok(root_ts)
    }
}

fn check_status(response: &HttpResponse, op: &str) -> Result<()> {
    if response.status == 429 {
        // FR-010: surface the rate limit with Retry-After; do not fall back.
        return Err(GwtError::Other(format!(
            "slack {op} rate limited; retry after {}s",
            response.retry_after.unwrap_or(60)
        )));
    }
    if response.status >= 400 {
        return Err(GwtError::Other(format!(
            "slack {op} http {}",
            response.status
        )));
    }
    Ok(())
}

#[derive(Deserialize)]
struct SlackHistory {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    messages: Vec<SlackHistoryMessage>,
}

#[derive(Deserialize)]
struct SlackHistoryMessage {
    ts: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    bot_id: Option<String>,
    #[serde(default)]
    thread_ts: Option<String>,
}

impl SlackHistoryMessage {
    fn to_message(&self) -> SlackMessage {
        SlackMessage {
            ts: self.ts.clone(),
            text: self.text.clone(),
            user: self.user.clone(),
            username: self.username.clone(),
            bot_id: self.bot_id.clone(),
            thread_ts: self.thread_ts.clone(),
        }
    }
}

#[derive(Deserialize)]
struct SlackPostResponse {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    /// Posted message timestamp — the thread root id for Workspace threading
    /// (SPEC-2963). `chat.postMessage` returns it; `chat.update` echoes it.
    #[serde(default)]
    ts: Option<String>,
}

impl BoardProvider for SlackProvider {
    fn post_entry(&self, worktree_root: &Path, entry: BoardEntry) -> Result<CoordinationSnapshot> {
        let channel =
            mapping::resolve_channel(&entry, &self.channel_map, Some(&self.default_channel))
                .ok_or_else(|| {
                    GwtError::Other("slack: no channel resolved for post".to_string())
                })?;
        // SPEC-2963 Workspace threading: every post is a reply under the
        // Workspace (or General) thread root — a summary card created once and
        // refreshed when the Workspace changes. `entry.parent_id` (a reply to a
        // specific Board entry) collapses into the single Workspace thread since
        // Slack threads are one level deep.
        let root_ts = self.ensure_thread_root(worktree_root, &channel, &entry)?;
        // The reply carries a "who · kind · origin" meta line so a Slack reader
        // can tell who posted and the entry type (SPEC-2963).
        let meta = mapping::board_entry_meta_line(&entry);
        self.post_message(
            &channel,
            Some(&meta),
            entry.title.as_deref(),
            &entry.body,
            Some(&root_ts),
        )?;
        // The post invalidates the read cache so the next load reflects it.
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
        // Remote-sole: the channel is the scope.
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
    use gwt_core::coordination::AuthorKind;
    use std::path::PathBuf;
    use std::sync::Mutex;

    /// Mock HTTP client returning canned responses by endpoint.
    #[derive(Default)]
    struct MockHttp {
        history_body: String,
        history_status: u16,
        post_body: String,
        post_status: u16,
    }

    impl HttpClient for MockHttp {
        fn get(
            &self,
            url: &str,
            _bearer: &str,
            _query: &[(&str, &str)],
        ) -> std::result::Result<HttpResponse, String> {
            assert!(url.contains("conversations.history"));
            Ok(HttpResponse {
                status: if self.history_status == 0 {
                    200
                } else {
                    self.history_status
                },
                body: self.history_body.clone(),
                retry_after: if self.history_status == 429 {
                    Some(30)
                } else {
                    None
                },
            })
        }

        fn post_form(
            &self,
            url: &str,
            _bearer: &str,
            _params: &[(&str, &str)],
        ) -> std::result::Result<HttpResponse, String> {
            assert!(url.contains("chat.postMessage"));
            Ok(HttpResponse {
                status: if self.post_status == 0 {
                    200
                } else {
                    self.post_status
                },
                body: self.post_body.clone(),
                retry_after: None,
            })
        }
    }

    fn provider(mock: MockHttp) -> SlackProvider {
        let mut map = BTreeMap::new();
        map.insert("ws-a".to_string(), "CH-A".to_string());
        SlackProvider::new("xoxb-token", "CH-DEFAULT", map, Box::new(mock), 60)
    }

    /// A unique throwaway repo root per call so the SPEC-2963 root-mapping
    /// JSONL (`.gwt/work/board-remote-roots.jsonl`) and `.gitattributes` are
    /// written under an isolated temp dir, never the real working tree. Multi-
    /// post tests bind one root and reuse it.
    fn root() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let mut path = std::env::temp_dir();
        path.push(format!(
            "gwt-board-roots-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::create_dir_all(&path);
        path
    }

    /// Captured post calls: `(url, params)` per post_form invocation.
    type CallLog = std::sync::Arc<Mutex<Vec<(String, Vec<(String, String)>)>>>;

    /// Recording mock that captures every post (url + params) and returns an
    /// incrementing `ts` so root-thread get-or-create works in tests.
    struct RecordingPosts {
        calls: CallLog,
        history_body: String,
        ts: std::sync::Arc<std::sync::atomic::AtomicU64>,
    }

    impl RecordingPosts {
        fn new() -> Self {
            Self {
                calls: std::sync::Arc::new(Mutex::new(Vec::new())),
                history_body: r#"{"ok":true,"messages":[]}"#.to_string(),
                ts: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            }
        }
        fn handle(&self) -> CallLog {
            self.calls.clone()
        }
    }

    impl HttpClient for RecordingPosts {
        fn get(
            &self,
            _u: &str,
            _b: &str,
            _q: &[(&str, &str)],
        ) -> std::result::Result<HttpResponse, String> {
            Ok(HttpResponse {
                status: 200,
                body: self.history_body.clone(),
                retry_after: None,
            })
        }
        fn post_form(
            &self,
            url: &str,
            _b: &str,
            params: &[(&str, &str)],
        ) -> std::result::Result<HttpResponse, String> {
            self.calls.lock().unwrap().push((
                url.to_string(),
                params
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            ));
            let n = self.ts.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            Ok(HttpResponse {
                status: 200,
                body: format!(r#"{{"ok":true,"ts":"ts-{n}"}}"#),
                retry_after: None,
            })
        }
    }

    fn post_calls(calls: &CallLog, endpoint: &str) -> Vec<Vec<(String, String)>> {
        calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(url, _)| url.contains(endpoint))
            .map(|(_, params)| params.clone())
            .collect()
    }

    fn has_param(params: &[(String, String)], key: &str, value: &str) -> bool {
        params.iter().any(|(k, v)| k == key && v == value)
    }

    fn param_value(params: &[(String, String)], key: &str) -> Option<String> {
        params
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
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
    fn load_snapshot_maps_slack_history() {
        let mock = MockHttp {
            history_body: r#"{"ok":true,"messages":[
                {"ts":"1700000000.000100","text":"first","username":"Akio"},
                {"ts":"1700000050.000200","text":"reply","bot_id":"B1","thread_ts":"1700000000.000100"}
            ]}"#
            .to_string(),
            ..Default::default()
        };
        let snapshot = provider(mock).load_snapshot(&root()).unwrap();
        assert_eq!(snapshot.board.entries.len(), 2);
        assert_eq!(snapshot.board.entries[0].id, "1700000000.000100");
        assert_eq!(snapshot.board.entries[0].body, "first");
        assert_eq!(
            snapshot.board.entries[1].parent_id.as_deref(),
            Some("1700000000.000100")
        );
        assert_eq!(snapshot.board.total_entries, 2);
    }

    #[test]
    fn post_entry_creates_workspace_root_then_threads_reply() {
        // SPEC-2963: first post to a Workspace creates the summary-card root in
        // the mapped channel; the entry itself is a reply threaded under it.
        let mut map = BTreeMap::new();
        map.insert("ws-a".to_string(), "CH-A".to_string());
        let mock = RecordingPosts::new();
        let calls = mock.handle();
        let prov = SlackProvider::new("t", "CH-DEFAULT", map, Box::new(mock), 60);
        let root = root();
        let e = entry("threaded").with_audience(vec!["ws-a".to_string()]);
        prov.post_entry(&root, e).unwrap();

        let posts = post_calls(&calls, "chat.postMessage");
        assert_eq!(
            posts.len(),
            2,
            "first post creates the Workspace root, second is the reply"
        );
        // Root card: mapped channel, posted at top level (no thread_ts).
        assert!(has_param(&posts[0], "channel", "CH-A"));
        assert!(!posts[0].iter().any(|(k, _)| k == "thread_ts"));
        // Reply: same channel, threads under the root ts (ts-1), carries body.
        assert!(has_param(&posts[1], "channel", "CH-A"));
        assert!(has_param(&posts[1], "thread_ts", "ts-1"));
        let reply_text = param_value(&posts[1], "text").unwrap_or_default();
        assert!(
            reply_text.contains("threaded"),
            "reply carries the body: {reply_text}"
        );
        // SPEC-2963: the reply carries a who·kind meta context block; the root
        // card (posts[0]) does not.
        let reply_blocks = param_value(&posts[1], "blocks").unwrap_or_default();
        assert!(
            reply_blocks.contains("\"context\"") && reply_blocks.contains("status"),
            "reply has a meta context block naming the kind: {reply_blocks}"
        );
        let root_blocks = param_value(&posts[0], "blocks").unwrap_or_default();
        assert!(
            !root_blocks.contains("\"context\""),
            "root card has no meta context block: {root_blocks}"
        );
    }

    #[test]
    fn post_entry_builds_block_kit_for_title_and_markdown() {
        let mock = RecordingPosts::new();
        let calls = mock.handle();
        let prov = SlackProvider::new("t", "CH-DEFAULT", BTreeMap::new(), Box::new(mock), 60);
        prov.post_entry(&root(), entry("**bold** text").with_title("Release notes"))
            .unwrap();
        // No audience -> General thread: posts[0] is the General root card, and
        // posts[1] is the entry reply carrying its own Block Kit blocks.
        let posts = post_calls(&calls, "chat.postMessage");
        let reply = &posts[1];
        let blocks = reply
            .iter()
            .find(|(k, _)| k == "blocks")
            .map(|(_, v)| v.clone())
            .expect("blocks param must be present");
        assert!(
            blocks.contains("\"type\":\"header\""),
            "title must become a header block: {blocks}"
        );
        assert!(
            blocks.contains("Release notes"),
            "header must carry the title: {blocks}"
        );
        assert!(
            blocks.contains("\"type\":\"mrkdwn\"") && blocks.contains("*bold*"),
            "body must become an mrkdwn section: {blocks}"
        );
        // Accessibility/notification fallback still carries title + body.
        assert!(reply
            .iter()
            .any(|(k, v)| k == "text" && v.contains("Release notes")));
    }

    /// Restore an env var on drop (HOME redirect for home-store isolation).
    struct ScopedEnv {
        key: &'static str,
        previous: Option<std::ffi::OsString>,
    }

    impl ScopedEnv {
        fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
        }
    }

    impl Drop for ScopedEnv {
        fn drop(&mut self) {
            match self.previous.as_ref() {
                Some(previous) => std::env::set_var(self.key, previous),
                None => std::env::remove_var(self.key),
            }
        }
    }

    fn init_repo_with_origin(path: &std::path::Path, url: &str) {
        std::fs::create_dir_all(path).unwrap();
        for args in [vec!["init"], vec!["remote", "add", "origin", url]] {
            let output = std::process::Command::new("git")
                .args(&args)
                .current_dir(path)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    #[test]
    fn broadcast_from_second_worktree_reuses_general_root_via_home_store() {
        // SPEC-2963 FR-022..024 (SC-019): worktree A mints the General root;
        // a fresh worktree B of the same repo (no local mapping yet — git
        // propagation is still in flight) must thread its broadcast under
        // A's root instead of creating a second "General" card. This is the
        // duplicate-General regression observed live (#3023 downstream).
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let _home = ScopedEnv::set("HOME", &home);
        let wt_a = dir.path().join("wt-a");
        let wt_b = dir.path().join("wt-b");
        let origin = "git@github.com:example/general-root.git";
        init_repo_with_origin(&wt_a, origin);
        init_repo_with_origin(&wt_b, origin);

        let mock_a = RecordingPosts::new();
        let calls_a = mock_a.handle();
        let prov_a = SlackProvider::new("t", "CH-DEFAULT", BTreeMap::new(), Box::new(mock_a), 60);
        prov_a.post_entry(&wt_a, entry("from worktree A")).unwrap();
        let posts_a = post_calls(&calls_a, "chat.postMessage");
        assert_eq!(posts_a.len(), 2, "A creates the General root + its reply");

        let mock_b = RecordingPosts::new();
        let calls_b = mock_b.handle();
        let prov_b = SlackProvider::new("t", "CH-DEFAULT", BTreeMap::new(), Box::new(mock_b), 60);
        prov_b.post_entry(&wt_b, entry("from worktree B")).unwrap();
        let posts_b = post_calls(&calls_b, "chat.postMessage");
        assert_eq!(
            posts_b.len(),
            1,
            "B must not mint a second General root: {posts_b:?}"
        );
        assert!(
            has_param(&posts_b[0], "thread_ts", "ts-1"),
            "B's post threads under A's root: {posts_b:?}"
        );
    }

    #[test]
    fn rate_limited_history_surfaces_error_no_fallback() {
        let mock = MockHttp {
            history_status: 429,
            history_body: String::new(),
            ..Default::default()
        };
        let err = provider(mock).load_snapshot(&root()).unwrap_err();
        assert!(err.to_string().contains("rate limited"));
    }

    #[test]
    fn slack_api_error_is_surfaced() {
        let mock = MockHttp {
            history_body: r#"{"ok":false,"error":"channel_not_found"}"#.to_string(),
            ..Default::default()
        };
        let err = provider(mock).load_snapshot(&root()).unwrap_err();
        assert!(err.to_string().contains("channel_not_found"));
    }

    #[test]
    fn board_entry_exists_scans_history() {
        let mock = MockHttp {
            history_body: r#"{"ok":true,"messages":[{"ts":"1700000000.000100","text":"x"}]}"#
                .to_string(),
            ..Default::default()
        };
        let prov = provider(mock);
        assert!(prov
            .board_entry_exists(&root(), "1700000000.000100")
            .unwrap());
        assert!(!prov.board_entry_exists(&root(), "missing").unwrap());
    }

    fn three_message_mock() -> MockHttp {
        MockHttp {
            history_body: r#"{"ok":true,"messages":[
                {"ts":"100.0001","text":"a","username":"U"},
                {"ts":"200.0002","text":"b","username":"U"},
                {"ts":"300.0003","text":"c","username":"U"}
            ]}"#
            .to_string(),
            post_body: r#"{"ok":true,"ts":"root-ts"}"#.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn post_through_mock_form_path() {
        // Drives MockHttp::post_form + the post_entry happy path on the default
        // channel (no audience -> CH-DEFAULT).
        let prov = provider(three_message_mock());
        let snapshot = prov.post_entry(&root(), entry("hello")).unwrap();
        assert_eq!(snapshot.board.entries.len(), 3);
    }

    #[test]
    fn trait_read_methods_cover_since_recent_and_pagination() {
        let prov = provider(three_message_mock());
        let scope = BoardAudienceScope::All;

        // load_snapshot_for_scope delegates to load_snapshot.
        assert_eq!(
            prov.load_snapshot_for_scope(&root(), &scope)
                .unwrap()
                .board
                .entries
                .len(),
            3
        );

        // load_entries_since: an epoch-0 `since` returns every cached entry.
        let epoch = DateTime::<Utc>::from_timestamp(0, 0).unwrap();
        assert_eq!(prov.load_entries_since(&root(), epoch).unwrap().len(), 3);
        assert_eq!(
            prov.load_entries_since_for_scope(&root(), epoch, &scope)
                .unwrap()
                .len(),
            3
        );

        // has_recent_post_by executes the predicate over every entry.
        let wide = chrono::Duration::days(1_000_000);
        assert!(prov
            .has_recent_post_by(&root(), "nobody", &BoardEntryKind::Status, wide)
            .is_ok());

        // load_entries_before: empty for limit 0.
        let zero = prov.load_entries_before(&root(), None, 0).unwrap();
        assert!(zero.entries.is_empty() && !zero.has_more_before);

        // Newest-2 with more available behind them.
        let page = prov.load_entries_before(&root(), None, 2).unwrap();
        assert_eq!(page.entries.len(), 2);
        assert!(page.has_more_before);

        // Everything strictly before a known id, no more behind.
        let before = prov
            .load_entries_before(&root(), Some("200.0002"), 5)
            .unwrap();
        assert_eq!(before.entries.len(), 1);
        assert!(!before.has_more_before);

        // Scope variant delegates.
        assert_eq!(
            prov.load_entries_before_for_scope(&root(), None, 2, &scope)
                .unwrap()
                .entries
                .len(),
            2
        );
    }

    #[test]
    fn http_5xx_surfaces_error() {
        let mock = MockHttp {
            history_status: 500,
            ..Default::default()
        };
        let err = provider(mock).load_snapshot(&root()).unwrap_err();
        assert!(err.to_string().contains("http 500"));
    }

    #[test]
    fn post_api_error_is_surfaced() {
        let mock = MockHttp {
            history_body: r#"{"ok":true,"messages":[]}"#.to_string(),
            post_body: r#"{"ok":false,"error":"not_in_channel"}"#.to_string(),
            ..Default::default()
        };
        let err = provider(mock).post_entry(&root(), entry("x")).unwrap_err();
        assert!(err.to_string().contains("not_in_channel"));
    }

    #[test]
    fn post_without_resolvable_channel_errors() {
        // Empty default + no audience => no channel resolves.
        let prov = SlackProvider::new("t", "", BTreeMap::new(), Box::new(MockHttp::default()), 60);
        assert!(prov.post_entry(&root(), entry("x")).is_err());
    }

    #[test]
    fn post_json_default_is_unsupported() {
        // The Slack mock does not override post_json, so it hits the trait
        // default which reports the operation as unsupported.
        assert!(MockHttp::default().post_json("u", "b", "{}").is_err());
    }

    #[test]
    fn patch_json_default_is_unsupported() {
        assert!(MockHttp::default().patch_json("u", "b", "{}").is_err());
    }

    #[test]
    fn second_post_to_same_workspace_reuses_root() {
        // SPEC-2963 get-or-create: the Workspace root is created once; the
        // second post for the same Workspace reuses it (no duplicate root).
        let mut map = BTreeMap::new();
        map.insert("ws-a".to_string(), "CH-A".to_string());
        let mock = RecordingPosts::new();
        let calls = mock.handle();
        let prov = SlackProvider::new("t", "CH-DEFAULT", map, Box::new(mock), 60);
        let root = root();
        prov.post_entry(&root, entry("one").with_audience(vec!["ws-a".to_string()]))
            .unwrap();
        prov.post_entry(&root, entry("two").with_audience(vec!["ws-a".to_string()]))
            .unwrap();

        // 1 root create + 2 replies = 3 postMessage calls (no second root).
        let posts = post_calls(&calls, "chat.postMessage");
        assert_eq!(posts.len(), 3, "root created once, then two replies");
        // Exactly one persisted root for ws-a.
        let mappings = gwt_core::board_remote_roots::load_root_mappings(&root);
        assert_eq!(
            mappings.keys().filter(|(_, _, key)| key == "ws-a").count(),
            1
        );
        // Both replies thread under the same root ts (ts-1).
        assert!(has_param(&posts[1], "thread_ts", "ts-1"));
        assert!(has_param(&posts[2], "thread_ts", "ts-1"));
    }

    #[test]
    fn two_workspaces_get_distinct_roots() {
        let mut map = BTreeMap::new();
        map.insert("ws-a".to_string(), "CH-A".to_string());
        map.insert("ws-b".to_string(), "CH-B".to_string());
        let mock = RecordingPosts::new();
        let prov = SlackProvider::new("t", "CH-DEFAULT", map, Box::new(mock), 60);
        let root = root();
        prov.post_entry(&root, entry("a").with_audience(vec!["ws-a".to_string()]))
            .unwrap();
        prov.post_entry(&root, entry("b").with_audience(vec!["ws-b".to_string()]))
            .unwrap();

        let mappings = gwt_core::board_remote_roots::load_root_mappings(&root);
        assert!(mappings
            .keys()
            .any(|(prov, ch, key)| prov == "slack" && ch == "CH-A" && key == "ws-a"));
        assert!(mappings
            .keys()
            .any(|(prov, ch, key)| prov == "slack" && ch == "CH-B" && key == "ws-b"));
    }

    #[test]
    fn broadcast_post_uses_general_thread() {
        // Empty audience -> the General thread root (key "general").
        let mock = RecordingPosts::new();
        let calls = mock.handle();
        let prov = SlackProvider::new("t", "CH-DEFAULT", BTreeMap::new(), Box::new(mock), 60);
        let root = root();
        prov.post_entry(&root, entry("hello")).unwrap();

        let posts = post_calls(&calls, "chat.postMessage");
        // Root card carries the "General" header.
        let root_blocks = posts[0]
            .iter()
            .find(|(k, _)| k == "blocks")
            .map(|(_, v)| v.clone())
            .unwrap_or_default();
        assert!(root_blocks.contains("General"), "root is the General card");
        let mappings = gwt_core::board_remote_roots::load_root_mappings(&root);
        assert!(mappings.keys().any(|(_, _, key)| key == "general"));
    }

    #[test]
    fn changed_workspace_card_triggers_chat_update() {
        // SPEC-2963 root refresh: a pre-existing root whose stored card hash is
        // stale gets a chat.update before the reply is threaded under it.
        let mut map = BTreeMap::new();
        map.insert("ws-a".to_string(), "CH-A".to_string());
        let mock = RecordingPosts::new();
        let calls = mock.handle();
        let prov = SlackProvider::new("t", "CH-DEFAULT", map, Box::new(mock), 60);
        let root = root();
        // Seed an existing root with a deliberately stale hash so the next post
        // recomputes a different card and updates the root.
        gwt_core::board_remote_roots::append_root_mapping(
            &root,
            &gwt_core::board_remote_roots::RootMapping {
                key: "ws-a".to_string(),
                provider: "slack".to_string(),
                channel: "CH-A".to_string(),
                root_id: "OLD-ROOT".to_string(),
                card_hash: "stale".to_string(),
                updated_at: DateTime::<Utc>::from_timestamp(0, 0).unwrap(),
            },
        )
        .unwrap();

        prov.post_entry(&root, entry("x").with_audience(vec!["ws-a".to_string()]))
            .unwrap();

        // chat.update was called against the existing root.
        let updates = post_calls(&calls, "chat.update");
        assert_eq!(updates.len(), 1, "stale root card is refreshed once");
        assert!(has_param(&updates[0], "ts", "OLD-ROOT"));
        // The reply threads under the (reused) existing root, not a new one.
        let posts = post_calls(&calls, "chat.postMessage");
        assert_eq!(posts.len(), 1, "no new root created");
        assert!(has_param(&posts[0], "thread_ts", "OLD-ROOT"));
    }
}
