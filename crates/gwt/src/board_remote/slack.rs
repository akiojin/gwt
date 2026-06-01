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
}

impl BoardProvider for SlackProvider {
    fn post_entry(&self, worktree_root: &Path, entry: BoardEntry) -> Result<CoordinationSnapshot> {
        let channel =
            mapping::resolve_channel(&entry, &self.channel_map, Some(&self.default_channel))
                .ok_or_else(|| {
                    GwtError::Other("slack: no channel resolved for post".to_string())
                })?;
        let text = mapping::board_entry_to_slack_text(&entry);
        let mut params: Vec<(&str, &str)> = vec![("channel", &channel), ("text", &text)];
        if let Some(parent) = entry.parent_id.as_deref() {
            params.push(("thread_ts", parent));
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
    fn post_entry_routes_to_mapped_channel_and_threads() {
        // Use a recording client to capture the exact post params.
        let mut map = BTreeMap::new();
        map.insert("ws-a".to_string(), "CH-A".to_string());
        let recorded = std::sync::Arc::new(Mutex::new(Vec::<(String, String)>::new()));
        struct RecordingHttp {
            recorded: std::sync::Arc<Mutex<Vec<(String, String)>>>,
            post_body: String,
            history_body: String,
        }
        impl HttpClient for RecordingHttp {
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
                _u: &str,
                _b: &str,
                params: &[(&str, &str)],
            ) -> std::result::Result<HttpResponse, String> {
                *self.recorded.lock().unwrap() = params
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect();
                Ok(HttpResponse {
                    status: 200,
                    body: self.post_body.clone(),
                    retry_after: None,
                })
            }
        }
        let http = RecordingHttp {
            recorded: recorded.clone(),
            post_body: r#"{"ok":true}"#.to_string(),
            history_body: r#"{"ok":true,"messages":[]}"#.to_string(),
        };
        let prov = SlackProvider::new("t", "CH-DEFAULT", map, Box::new(http), 60);
        let mut e = entry("threaded");
        e.parent_id = Some("1700000000.000100".to_string());
        e = e.with_audience(vec!["ws-a".to_string()]);
        prov.post_entry(&root(), e).unwrap();
        let params = recorded.lock().unwrap().clone();
        assert!(params.contains(&("channel".to_string(), "CH-A".to_string())));
        assert!(params.contains(&("text".to_string(), "threaded".to_string())));
        assert!(params.contains(&("thread_ts".to_string(), "1700000000.000100".to_string())));
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
}
