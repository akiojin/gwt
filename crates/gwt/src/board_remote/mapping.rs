//! Field mapping between [`BoardEntry`] and remote (Slack) messages
//! (SPEC-2963 FR-008). Remote-sole means the remote message is the source of
//! truth; mapping back to a `BoardEntry` only needs the fields the Board UI
//! renders (id, author, body, created_at, parent_id, audience).

use std::collections::BTreeMap;

use chrono::{DateTime, TimeZone, Utc};
use gwt_core::coordination::{AuthorKind, BoardEntry, BoardEntryKind};

/// Parse a Slack message timestamp (`"1700000000.123456"`) into UTC.
pub fn slack_ts_to_datetime(ts: &str) -> Option<DateTime<Utc>> {
    let mut parts = ts.split('.');
    let secs: i64 = parts.next()?.parse().ok()?;
    let micros: u32 = parts.next().and_then(|m| m.parse().ok()).unwrap_or(0);
    Utc.timestamp_opt(secs, micros.saturating_mul(1_000))
        .single()
}

/// The text to post to Slack for a Board entry. The body is the canonical
/// content; remote-sole keeps Slack human-readable rather than encoding gwt
/// metadata into the message text.
pub fn board_entry_to_slack_text(entry: &BoardEntry) -> String {
    entry.body.clone()
}

/// Resolve the Slack channel for an entry: first audience workspace mapped via
/// `channel_map`, otherwise `default_channel` (FR-007).
pub fn resolve_channel(
    entry: &BoardEntry,
    channel_map: &BTreeMap<String, String>,
    default_channel: Option<&str>,
) -> Option<String> {
    for workspace_id in &entry.audience {
        if let Some(channel) = channel_map.get(workspace_id) {
            return Some(channel.clone());
        }
    }
    default_channel
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// Minimal shape of a Slack message read from `conversations.history`/`replies`.
#[derive(Debug, Clone, Default)]
pub struct SlackMessage {
    pub ts: String,
    pub text: String,
    pub user: Option<String>,
    pub username: Option<String>,
    pub bot_id: Option<String>,
    pub thread_ts: Option<String>,
}

/// Convert a Slack message into a [`BoardEntry`] for display. `workspace_id` is
/// the gwt Work the channel maps to (used as the entry's lane audience).
pub fn slack_message_to_board_entry(msg: &SlackMessage, workspace_id: &str) -> BoardEntry {
    let author = msg
        .username
        .clone()
        .or_else(|| msg.user.clone())
        .unwrap_or_else(|| "Slack".to_string());
    let author_kind = if msg.bot_id.is_some() {
        AuthorKind::Agent
    } else {
        AuthorKind::User
    };
    // thread_ts equal to ts marks a thread root, not a reply.
    let parent_id = msg
        .thread_ts
        .clone()
        .filter(|thread| !thread.is_empty() && *thread != msg.ts);

    let mut entry = BoardEntry::new(
        author_kind,
        author,
        BoardEntryKind::Status,
        msg.text.clone(),
        None,
        parent_id,
        vec![],
        vec![],
    );
    entry.id = msg.ts.clone();
    if let Some(dt) = slack_ts_to_datetime(&msg.ts) {
        entry.created_at = dt;
        entry.updated_at = dt;
    }
    let workspace_id = workspace_id.trim();
    if !workspace_id.is_empty() {
        entry = entry.with_audience(vec![workspace_id.to_string()]);
    }
    entry
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_slack_ts() {
        let dt = slack_ts_to_datetime("1700000000.000100").unwrap();
        assert_eq!(dt.timestamp(), 1_700_000_000);
        assert!(slack_ts_to_datetime("not-a-ts").is_none());
        // missing micros still parses.
        assert_eq!(
            slack_ts_to_datetime("1700000000").unwrap().timestamp(),
            1_700_000_000
        );
    }

    #[test]
    fn slack_root_message_maps_to_entry_without_parent() {
        let msg = SlackMessage {
            ts: "1700000000.000100".into(),
            text: "hello board".into(),
            user: Some("U1".into()),
            username: Some("Akio".into()),
            bot_id: None,
            thread_ts: None,
        };
        let entry = slack_message_to_board_entry(&msg, "ws-a");
        assert_eq!(entry.id, "1700000000.000100");
        assert_eq!(entry.author, "Akio");
        assert_eq!(entry.author_kind, AuthorKind::User);
        assert_eq!(entry.body, "hello board");
        assert_eq!(entry.parent_id, None);
        assert_eq!(entry.audience, vec!["ws-a".to_string()]);
        assert_eq!(entry.created_at.timestamp(), 1_700_000_000);
    }

    #[test]
    fn slack_reply_maps_parent_id_from_thread_ts() {
        let msg = SlackMessage {
            ts: "1700000050.000200".into(),
            text: "reply".into(),
            user: None,
            username: None,
            bot_id: Some("B1".into()),
            thread_ts: Some("1700000000.000100".into()),
        };
        let entry = slack_message_to_board_entry(&msg, "");
        assert_eq!(entry.parent_id.as_deref(), Some("1700000000.000100"));
        assert_eq!(entry.author_kind, AuthorKind::Agent);
        assert_eq!(entry.author, "Slack"); // no user/username → fallback
        assert!(entry.audience.is_empty());
    }

    #[test]
    fn thread_ts_equal_to_ts_is_root_not_reply() {
        let msg = SlackMessage {
            ts: "1700000000.000100".into(),
            text: "root".into(),
            thread_ts: Some("1700000000.000100".into()),
            ..Default::default()
        };
        let entry = slack_message_to_board_entry(&msg, "ws");
        assert_eq!(entry.parent_id, None);
    }

    #[test]
    fn channel_resolution_prefers_mapping_then_default() {
        let mut map = BTreeMap::new();
        map.insert("ws-a".to_string(), "CH-A".to_string());
        let mut mapped = BoardEntry::new(
            AuthorKind::User,
            "You",
            BoardEntryKind::Status,
            "x",
            None,
            None,
            vec![],
            vec![],
        )
        .with_audience(vec!["ws-a".to_string()]);
        assert_eq!(
            resolve_channel(&mapped, &map, Some("CH-DEFAULT")),
            Some("CH-A".to_string())
        );
        // unmapped workspace → default.
        mapped = mapped.with_audience(vec!["ws-z".to_string()]);
        assert_eq!(
            resolve_channel(&mapped, &map, Some("CH-DEFAULT")),
            Some("CH-DEFAULT".to_string())
        );
        // no audience, no default → None.
        let broadcast = BoardEntry::new(
            AuthorKind::User,
            "You",
            BoardEntryKind::Status,
            "x",
            None,
            None,
            vec![],
            vec![],
        );
        assert_eq!(resolve_channel(&broadcast, &map, None), None);
    }
}
