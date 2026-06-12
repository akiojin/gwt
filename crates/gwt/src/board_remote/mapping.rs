//! Field mapping between [`BoardEntry`] and remote (Slack) messages
//! (SPEC-2963 FR-008). Remote-sole means the remote message is the source of
//! truth; mapping back to a `BoardEntry` only needs the fields the Board UI
//! renders (id, author, body, created_at, parent_id, audience).

use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};

use chrono::{DateTime, TimeZone, Utc};
use gwt_core::board_remote_roots::GENERAL_THREAD_KEY;
use gwt_core::coordination::{AuthorKind, BoardEntry, BoardEntryKind, BoardMentionTargetKind};
use gwt_core::work_projection::{WorkItem, WorkspaceStatusCategory};

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

/// SPEC-2963 Workspace threading: the thread root key for an entry — its first
/// non-empty Workspace audience, or the General thread for broadcast /
/// non-Workspace posts.
pub fn thread_key_for_entry(entry: &BoardEntry) -> String {
    entry
        .audience
        .iter()
        .map(|value| value.trim())
        .find(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| GENERAL_THREAD_KEY.to_string())
}

fn status_label(status: WorkspaceStatusCategory) -> &'static str {
    match status {
        WorkspaceStatusCategory::Active => "Active",
        WorkspaceStatusCategory::Idle => "Paused",
        WorkspaceStatusCategory::Blocked => "Blocked",
        WorkspaceStatusCategory::Done => "Done",
        WorkspaceStatusCategory::Unknown => "Unknown",
    }
}

/// Build the Workspace summary card (title + markdown body) used as the thread
/// root. The General thread gets a fixed header. When the Workspace item is not
/// yet known, the `branch_fallback` (or key) titles the card and fields show
/// "—" so a placeholder root can be created and refined later (SPEC-2963).
pub fn workspace_summary_card(
    key: &str,
    item: Option<&WorkItem>,
    branch_fallback: Option<&str>,
) -> (String, String) {
    if key == GENERAL_THREAD_KEY {
        return (
            "General".to_string(),
            "Broadcast / non-Workspace coordination.".to_string(),
        );
    }
    let branch_fallback = branch_fallback
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let Some(item) = item else {
        let title = branch_fallback
            .map(str::to_string)
            .unwrap_or_else(|| key.to_string());
        return (title, "Branch: —\nSPEC: —\nPR: —".to_string());
    };
    let container = item.execution_containers.first();
    let branch = container
        .and_then(|c| c.branch.clone())
        .or_else(|| branch_fallback.map(str::to_string));
    let title = if item.title.trim().is_empty() {
        branch.clone().unwrap_or_else(|| key.to_string())
    } else {
        item.title.clone()
    };
    let mut lines = Vec::new();
    lines.push(format!("Branch: {}", branch.as_deref().unwrap_or("—")));
    lines.push(format!("SPEC: {}", item.owner.as_deref().unwrap_or("—")));
    match container.and_then(|c| c.pr_number) {
        Some(pr) => {
            let state = container
                .and_then(|c| c.pr_state.clone())
                .map(|s| format!(" ({s})"))
                .unwrap_or_default();
            lines.push(format!("PR: #{pr}{state}"));
        }
        None => lines.push("PR: —".to_string()),
    }
    lines.push(format!("Lifecycle: {}", status_label(item.status_category)));
    (title, lines.join("\n"))
}

/// SPEC-2963: a one-line "who + kind + origin" header for a remote post so a
/// Slack/Teams reader can tell who posted and the entry type (the remote shows
/// only the OAuth identity otherwise). Mirrors the Local board / CLI
/// `format_author`: `<author> (<author_kind>) · <kind>[· <branch>[ / <session>]]`.
pub fn board_entry_meta_line(entry: &BoardEntry) -> String {
    let author_kind = match entry.author_kind {
        AuthorKind::Agent => "agent",
        AuthorKind::User => "user",
        AuthorKind::System => "system",
    };
    let mut line = format!(
        "{} ({}) · {}",
        entry.author.trim(),
        author_kind,
        entry.kind.as_str()
    );
    // Audience / mention targeting (who the post is addressed to), mirroring
    // the Local board's `boardEntryAudienceLabels`. Without this a remote
    // reader cannot tell whether a post is broadcast, pinned to a Workspace,
    // or directed at a specific user / agent.
    line.push_str(&format!(
        " · {}",
        board_entry_audience_labels(entry).join(", ")
    ));
    let branch = entry
        .origin_branch
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let session = entry
        .origin_session_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.chars().take(8).collect::<String>());
    match (branch, session) {
        (Some(branch), Some(session)) => line.push_str(&format!(" · {branch} / {session}")),
        (Some(branch), None) => line.push_str(&format!(" · {branch}")),
        (None, Some(session)) => line.push_str(&format!(" · {session}")),
        (None, None) => {}
    }
    line
}

/// Audience / mention targeting labels, mirroring the Local board's
/// `boardEntryAudienceLabels` (`board-surface.js`). Unlike the Local helper —
/// which short-circuits on Workspace audience and so hides an explicit
/// `@user` / `@agent` mention behind the Workspace label — this concatenates
/// Workspace audience *and* explicit mentions so a directed ping is never
/// dropped. Falls back to `"Broadcast"` when a post targets no one.
fn board_entry_audience_labels(entry: &BoardEntry) -> Vec<String> {
    let mut labels: Vec<String> = Vec::new();
    let push = |label: String, labels: &mut Vec<String>| {
        if !label.is_empty() && !labels.contains(&label) {
            labels.push(label);
        }
    };
    for workspace in &entry.audience {
        let id = workspace.trim();
        if !id.is_empty() {
            push(format!("Workspace: {id}"), &mut labels);
        }
    }
    for mention in &entry.mentions {
        let label_text = mention
            .label
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| mention.target.trim());
        if label_text.is_empty() {
            continue;
        }
        let formatted = match mention.target_kind {
            BoardMentionTargetKind::Agent | BoardMentionTargetKind::User => {
                format!("To: {label_text}")
            }
            BoardMentionTargetKind::Session => format!("Session: {label_text}"),
            BoardMentionTargetKind::Branch => format!("Branch: {label_text}"),
            BoardMentionTargetKind::Workspace => format!("Workspace: {label_text}"),
        };
        push(formatted, &mut labels);
    }
    if labels.is_empty() {
        labels.push("Broadcast".to_string());
    }
    labels
}

/// Stable hash of a rendered root card (title + body), used to detect when the
/// Workspace summary changed so the root message is updated. Deterministic
/// across runs (fixed-seed `DefaultHasher`).
pub fn card_hash(title: &str, body: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    title.hash(&mut hasher);
    "\u{0}".hash(&mut hasher);
    body.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
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
    use gwt_core::coordination::BoardMention;

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

    #[test]
    fn meta_line_names_author_kind_and_origin() {
        // SPEC-2963: a remote reader must be able to tell who posted + the kind.
        let mut agent = BoardEntry::new(
            AuthorKind::Agent,
            "Codex",
            BoardEntryKind::Status,
            "body",
            None,
            None,
            vec![],
            vec![],
        );
        agent.origin_branch = Some("feature/x".to_string());
        agent.origin_session_id = Some("95862acd-a761-4fd0".to_string());
        let line = board_entry_meta_line(&agent);
        assert!(line.contains("Codex (agent)"), "author + kind tag: {line}");
        assert!(line.contains("status"), "entry kind: {line}");
        assert!(line.contains("feature/x"), "origin branch: {line}");
        assert!(line.contains("95862acd"), "short session id: {line}");
        assert!(
            !line.contains("95862acd-a761"),
            "session truncated to 8: {line}"
        );
        // No audience / mentions -> Broadcast, placed before the origin suffix.
        assert!(line.contains("status · Broadcast · feature/x"), "{line}");

        // User / system author kinds, and no origin -> Broadcast audience only.
        let user = BoardEntry::new(
            AuthorKind::User,
            "akiojin",
            BoardEntryKind::Decision,
            "b",
            None,
            None,
            vec![],
            vec![],
        );
        assert_eq!(
            board_entry_meta_line(&user),
            "akiojin (user) · decision · Broadcast"
        );
        let system = BoardEntry::new(
            AuthorKind::System,
            "gwt",
            BoardEntryKind::Blocked,
            "b",
            None,
            None,
            vec![],
            vec![],
        );
        assert_eq!(
            board_entry_meta_line(&system),
            "gwt (system) · blocked · Broadcast"
        );
    }

    #[test]
    fn meta_line_names_audience_and_mentions() {
        // SPEC-2963: a remote reader must see who a post is addressed to, the
        // same way the Local board shows Workspace / To / Session badges.
        let mut entry = BoardEntry::new(
            AuthorKind::Agent,
            "Codex",
            BoardEntryKind::Handoff,
            "body",
            None,
            None,
            vec![],
            vec![],
        );
        entry.audience = vec!["ws-alpha".to_string()];
        entry.mentions = vec![
            BoardMention::new(BoardMentionTargetKind::User, "akiojin"),
            BoardMention::new(BoardMentionTargetKind::Agent, "codex"),
        ];
        let line = board_entry_meta_line(&entry);
        assert!(
            line.contains("Workspace: ws-alpha"),
            "workspace audience: {line}"
        );
        assert!(
            line.contains("To: akiojin"),
            "user mention not hidden: {line}"
        );
        assert!(line.contains("To: codex"), "agent mention: {line}");

        // Session / branch mention kinds render with their own labels.
        let mut targeted = BoardEntry::new(
            AuthorKind::Agent,
            "Codex",
            BoardEntryKind::Question,
            "body",
            None,
            None,
            vec![],
            vec![],
        );
        targeted.mentions = vec![
            BoardMention::new(BoardMentionTargetKind::Session, "0d293994"),
            BoardMention::new(BoardMentionTargetKind::Branch, "feature/y"),
        ];
        let line = board_entry_meta_line(&targeted);
        assert!(
            line.contains("Session: 0d293994"),
            "session mention: {line}"
        );
        assert!(line.contains("Branch: feature/y"), "branch mention: {line}");
        assert!(
            !line.contains("Broadcast"),
            "targeted is not broadcast: {line}"
        );
    }
}
