//! Durable index of unresolved `blocked` Board posts (Issue #3655).
//!
//! The Board is append-only history and its hot projection keeps only the
//! most recent [`HOT_PROJECTION_ENTRY_LIMIT`](crate::coordination::HOT_PROJECTION_ENTRY_LIMIT)
//! entries, so "is anyone still blocked?" cannot be answered by looking at the
//! timeline: on a busy repository a genuine unblock request scrolls out of the
//! projection within hours, which is exactly how a stuck agent stayed invisible
//! to the PM. This module folds the Board event stream into a small,
//! separately persisted index whose only question is which escalations are
//! still open, and for which owners.
//!
//! The fold is pure and deterministic so the same answer is reachable two ways:
//! incrementally (one entry at a time as posts are appended) and by replaying
//! the whole event log, which is how a lost or truncated index file is
//! repaired.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::coordination::{BoardEntry, BoardEntryKind};

/// File name of the persisted index inside the coordination directory.
pub const ESCALATIONS_FILE_NAME: &str = "escalations.json";

/// Current on-disk schema version. A store written by a newer gwt is rebuilt
/// from the event log rather than trusted, so an unknown version is never a
/// hard failure.
pub const ESCALATION_STORE_VERSION: u32 = 1;

/// One unblock request opened by a `kind:"blocked"` Board post.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BoardEscalation {
    /// Board entry that opened the escalation. Also the handle a resolver
    /// passes back through `params.resolves`.
    pub entry_id: String,
    pub author: String,
    /// Verbatim post body. Carried in the index rather than looked up later
    /// because the PM-facing surfaces (`issue.monitor.status` readers, the PM
    /// wake prompt) must be able to state *why* work stopped without reading
    /// the pane — pane reads are exactly the channel that fails under GUI
    /// event-loop saturation (#3629).
    pub body: String,
    /// Owning Issue numbers, as written in `related_owners`.
    #[serde(default)]
    pub owners: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_branch: Option<String>,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_by_entry_id: Option<String>,
}

impl BoardEscalation {
    pub fn is_open(&self) -> bool {
        self.resolved_at.is_none()
    }

    /// Whether this escalation concerns `owner` (an Issue number as text).
    pub fn concerns_owner(&self, owner: &str) -> bool {
        let owner = owner.trim();
        !owner.is_empty() && self.owners.iter().any(|candidate| candidate == owner)
    }

    /// Owner Issue numbers that parse as numbers, for callers keyed by `u64`.
    pub fn owner_issue_numbers(&self) -> Vec<u64> {
        self.owners
            .iter()
            .filter_map(|owner| owner.trim().trim_start_matches('#').parse::<u64>().ok())
            .collect()
    }

    fn from_entry(entry: &BoardEntry) -> Self {
        Self {
            entry_id: entry.id.clone(),
            author: entry.author.clone(),
            body: entry.body.clone(),
            owners: entry.related_owners.clone(),
            origin_session_id: entry.origin_session_id.clone(),
            origin_branch: entry.origin_branch.clone(),
            created_at: entry.created_at,
            resolved_at: None,
            resolved_by_entry_id: None,
        }
    }

    fn resolve_with(&mut self, entry: &BoardEntry) {
        self.resolved_at = Some(entry.created_at);
        self.resolved_by_entry_id = Some(entry.id.clone());
    }

    /// Two escalations from the same session about the same owner set are the
    /// same standing request restated, not two independent ones.
    fn superseded_by(&self, entry: &BoardEntry) -> bool {
        let Some(session_id) = self.origin_session_id.as_deref() else {
            return false;
        };
        entry.origin_session_id.as_deref() == Some(session_id)
            && self.owners == entry.related_owners
    }
}

/// The persisted fold: every escalation ever opened, each either open or
/// carrying the entry that closed it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BoardEscalationStore {
    pub version: u32,
    #[serde(default)]
    pub escalations: Vec<BoardEscalation>,
    pub updated_at: DateTime<Utc>,
}

impl Default for BoardEscalationStore {
    fn default() -> Self {
        Self {
            version: ESCALATION_STORE_VERSION,
            escalations: Vec::new(),
            updated_at: Utc::now(),
        }
    }
}

impl BoardEscalationStore {
    /// Fold one Board entry into the index. Returns whether anything changed,
    /// so the caller can skip a write for the overwhelmingly common case of an
    /// ordinary post.
    ///
    /// Resolutions are applied before the entry's own escalation is inserted:
    /// a post can never resolve itself, and a restated block must not close the
    /// very escalation it opens.
    pub fn apply_entry(&mut self, entry: &BoardEntry) -> bool {
        let mut changed = false;

        for target_id in &entry.resolves_entry_ids {
            let target_id = target_id.trim();
            if target_id.is_empty() || target_id == entry.id {
                continue;
            }
            if let Some(escalation) = self
                .escalations
                .iter_mut()
                .find(|escalation| escalation.entry_id == target_id && escalation.is_open())
            {
                escalation.resolve_with(entry);
                changed = true;
            }
        }

        if entry.kind == BoardEntryKind::Blocked {
            if self
                .escalations
                .iter()
                .any(|escalation| escalation.entry_id == entry.id)
            {
                return changed;
            }
            for escalation in self
                .escalations
                .iter_mut()
                .filter(|escalation| escalation.is_open() && escalation.superseded_by(entry))
            {
                escalation.resolve_with(entry);
            }
            self.escalations.push(BoardEscalation::from_entry(entry));
            changed = true;
        }

        if changed {
            self.updated_at = Utc::now();
        }
        changed
    }

    /// Replay a whole event stream. Entries must already be ordered oldest
    /// first, which is the order both the segment loader and the hot
    /// projection produce.
    pub fn from_entries<'a>(entries: impl IntoIterator<Item = &'a BoardEntry>) -> Self {
        let mut store = Self::default();
        for entry in entries {
            store.apply_entry(entry);
        }
        store
    }

    pub fn open(&self) -> impl Iterator<Item = &BoardEscalation> {
        self.escalations
            .iter()
            .filter(|escalation| escalation.is_open())
    }

    pub fn open_escalations(&self) -> Vec<BoardEscalation> {
        self.open().cloned().collect()
    }

    pub fn open_for_owner(&self, owner: &str) -> Vec<&BoardEscalation> {
        self.open()
            .filter(|escalation| escalation.concerns_owner(owner))
            .collect()
    }

    /// Owner Issue numbers with at least one open escalation, ascending and
    /// deduplicated so callers can merge them into an existing set.
    pub fn open_owner_issue_numbers(&self) -> Vec<u64> {
        let mut numbers: Vec<u64> = self
            .open()
            .flat_map(|escalation| escalation.owner_issue_numbers())
            .collect();
        numbers.sort_unstable();
        numbers.dedup();
        numbers
    }

    /// Drop the resolved tail once it is far enough in the past to be
    /// uninteresting, so the index cannot grow without bound on a
    /// long-lived repository. Open escalations are never pruned regardless of
    /// age — an old unanswered request is the most important row there is.
    pub fn prune_resolved_before(&mut self, cutoff: DateTime<Utc>) -> bool {
        let before = self.escalations.len();
        self.escalations
            .retain(|escalation| escalation.is_open() || escalation.created_at >= cutoff);
        let changed = self.escalations.len() != before;
        if changed {
            self.updated_at = Utc::now();
        }
        changed
    }
}

/// The four things a PM needs before it can act on a stopped agent, in the
/// order a reader wants them (Issue #3655 AC-1).
///
/// Each section is matched by any of its aliases so a body may be written in
/// Japanese or English; the label must open a line and be followed by a colon.
/// The check is deliberately shallow — it asserts the *shape* that makes an
/// escalation actionable, not the quality of the prose, because a strict
/// content check would be trivially satisfiable and a strict format check
/// would push agents back to unstructured posts.
const REQUIRED_ESCALATION_SECTIONS: &[(&str, &[&str])] = &[
    (
        "事象 / Symptom",
        &["事象", "symptom", "what happened", "observed"],
    ),
    ("原因 / Cause", &["原因", "cause", "why", "root cause"]),
    (
        "依頼 / Request",
        &["依頼", "request", "ask", "needed from pm"],
    ),
    (
        "再開条件 / Resume condition",
        &["再開条件", "resume", "unblock", "resume condition"],
    ),
];

/// A copy-pasteable body skeleton, shown whenever a `blocked` post is refused.
pub const ESCALATION_BODY_TEMPLATE: &str =
    "事象: <何が起きたか。拒否された operation とエラー文言をそのまま含める>\n\
原因: <なぜ進めないのか（判明していれば根本原因、未判明ならそう書く）>\n\
依頼: <PM に何をしてほしいか（fresh launch / 仕様裁定 / ツール修正 など）>\n\
再開条件: <何が満たされれば作業を再開できるか>";

/// Why a `blocked` body was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EscalationBodyError {
    #[error(
        "a blocked Board post must state {}; missing: {}.\n\nUse this shape:\n{ESCALATION_BODY_TEMPLATE}",
        "事象 / 原因 / 依頼 / 再開条件",
        .missing.join(", ")
    )]
    MissingSections { missing: Vec<String> },
}

/// Check that a `blocked` post body carries all four required sections.
///
/// Enforced at the posting surface rather than left to guidance: the whole
/// point of this Issue is that an escalation which depends on an agent
/// remembering to write it well does not reliably happen.
pub fn validate_escalation_body(body: &str) -> Result<(), EscalationBodyError> {
    let missing: Vec<String> = REQUIRED_ESCALATION_SECTIONS
        .iter()
        .filter(|(_, aliases)| !body_has_section(body, aliases))
        .map(|(label, _)| (*label).to_string())
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    Err(EscalationBodyError::MissingSections { missing })
}

fn body_has_section(body: &str, aliases: &[&str]) -> bool {
    body.lines().any(|line| {
        let line = line
            .trim()
            .trim_start_matches(['-', '*', '#', '>'])
            .trim_start()
            .trim_start_matches(['*', '_'])
            .trim_start();
        let lowered = line.to_lowercase();
        aliases.iter().any(|alias| {
            let Some(rest) = lowered.strip_prefix(alias) else {
                return false;
            };
            rest.trim_start()
                .trim_start_matches(['*', '_'])
                .starts_with([':', '：'])
        })
    })
}

/// Why an operation was refused in a way the agent cannot work around
/// (Issue #3655 AC-2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationRefusalKind {
    /// The target is terminal and can never accept this operation again.
    Immutability,
    /// The caller is not the principal this operation binds to.
    Authority,
    /// The operation is not permitted here, or reaches a surface that refuses
    /// to act at all.
    Permission,
}

impl OperationRefusalKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Immutability => "immutability",
            Self::Authority => "authority",
            Self::Permission => "permission",
        }
    }

    fn request_hint(&self) -> &'static str {
        match self {
            Self::Immutability => {
                "この record は終端で、この window からは再開できません。残作業を続けるなら fresh launch を手配してください。"
            }
            Self::Authority => {
                "この session には権限がありません。正しい authority を持つ session を割り当てるか、authority の不整合を解消してください。"
            }
            Self::Permission => {
                "この操作はこの surface では拒否されます。PM 側での代行、または設定・ツール側の修正が必要です。"
            }
        }
    }
}

/// Operation families whose refusals are governance decisions rather than
/// caller mistakes. A typo in `issue.view` is the agent's problem; a terminal
/// Execution Control Record is not.
///
/// The list is deliberately the three surfaces Issue #3655 observed in
/// production, and nothing adjacent. Two kinds of neighbour were considered and
/// left out:
///
/// - `build.complete`, `verify.*`, `pr.create` / `pr.ready` refuse for missing
///   or stale verification. That is the agent's own next task, not a decision
///   only the owner can make, so escalating it would teach the PM to skim.
/// - `workspace.update` refuses as part of the terminal-settlement protocol,
///   and a refused authority is contractually forbidden from writing anything
///   locally — an escalation there would itself be the mutation the refusal
///   promises not to make.
fn refusal_eligible_operation(operation: &str) -> bool {
    const FAMILIES: &[&str] = &[
        "execution.",
        "workspace.ensure",
        "pane.close",
        "pane.stop",
        "pane.send",
    ];
    FAMILIES
        .iter()
        .any(|family| operation.starts_with(family) || operation == family.trim_end_matches('.'))
}

/// Decide whether a failed operation is a governance refusal worth escalating.
///
/// Both halves have to agree — the operation must belong to a family that can
/// refuse on principle, and the message must read like a refusal rather than a
/// malformed call. Either half alone over-fires: every mistyped parameter in a
/// lifecycle operation would raise an alarm, and every unrelated command that
/// happens to say "unavailable" would too.
pub fn classify_operation_refusal(operation: &str, error: &str) -> Option<OperationRefusalKind> {
    if !refusal_eligible_operation(operation) {
        return None;
    }
    let lowered = error.to_lowercase();
    // The needle lists are the refusal vocabulary these surfaces actually
    // emit, not an attempt to anticipate English. When a surface starts
    // refusing with new wording, extend the list — over-broad matching would
    // escalate ordinary failures and train the PM to ignore the signal.
    for needle in [
        "immutable",
        "is terminal",
        "are terminal",
        "already completed",
        "use a fresh launch",
    ] {
        if lowered.contains(needle) {
            return Some(OperationRefusalKind::Immutability);
        }
    }
    for needle in [
        "authority",
        "not the registered",
        "foreign principal",
        "another session",
        "owned by another",
    ] {
        if lowered.contains(needle) {
            return Some(OperationRefusalKind::Authority);
        }
    }
    for needle in [
        "refused",
        "refuses",
        "not authorized",
        "unauthorized",
        "not permitted",
        "forbidden",
        "is unavailable",
        "no-op",
        "did not close",
        "requires a correlated acceptance",
        "left the authenticated project scope",
    ] {
        if lowered.contains(needle) {
            return Some(OperationRefusalKind::Permission);
        }
    }
    None
}

/// Render the four-section body for an automatically filed escalation.
///
/// Built from the refusal itself so the PM reads the exact operation and the
/// exact error text — the two facts that decide whether the answer is a fresh
/// launch, a spec ruling, or a tool fix.
pub fn render_operation_refusal_body(
    operation: &str,
    error: &str,
    kind: OperationRefusalKind,
) -> String {
    format!(
        "事象: JSON operation `{operation}` が拒否されました。\n\
         ```\n{error}\n```\n\
         原因: {kind} 由来の拒否です。agent 側の入力の作り直しでは解消しません。\n\
         依頼: {request}\n\
         再開条件: 上記が解消され、`{operation}` 相当の操作が通る状態になること。\n\
         \n\
         この投稿は gwt が自動起票しました（Issue #3655 AC-2）。担当 agent は必要に応じて \
         事象・原因を補足してください。",
        kind = kind.as_str(),
        request = kind.request_hint(),
    )
}

/// Render the four-section body for an agent that declared itself blocked
/// through `execution.blocked` (Issue #3655 AC-1).
///
/// `execution.blocked` *is* the moment an agent concludes it cannot proceed,
/// so the escalation is raised from that operation rather than from a separate
/// Board post the agent also has to remember. In the incident that motivated
/// this Issue the agent had already worked out what it needed — a fresh launch
/// — and the only thing that reached the Board was the routine ready notice.
pub fn render_declared_block_body(reason: &str, missing_verification: Option<&str>) -> String {
    let verification = missing_verification
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| format!("\n未実施の検証: {value}"))
        .unwrap_or_default();
    format!(
        "事象: agent が `execution.blocked` で実行不能を宣言しました。\n\
         原因: {reason}{verification}\n\
         依頼: 上記の blocker を解消してください（fresh launch / 仕様裁定 / ツール修正 の\
         いずれかを判断してください）。\n\
         再開条件: blocker が解消され、この Issue の残作業を実行できる状態になること。\n\
         \n\
         この投稿は gwt が自動起票しました（Issue #3655 AC-1）。担当 agent は判明している\
         調査結果を補足してください。",
        reason = reason.trim(),
    )
}

/// Render the Issue comment that mirrors an escalation (AC-6).
///
/// The Board and the pane are both volatile — the Board scrolls and a closed
/// pane takes its transcript with it — so the investigation has to land
/// somewhere a freshly launched agent will look, which is the Issue itself.
pub fn render_escalation_issue_comment(escalation: &BoardEscalation) -> String {
    format!(
        "## Blocked — PM への unblock 要請\n\n\
         Board entry: `{entry_id}`\n\
         Agent: {author}{branch}\n\
         Posted: {created_at}\n\n\
         {body}\n\n\
         ---\n\
         この Issue が unblock されたら、解消した側は Board へ \
         `params.resolves:[\"{entry_id}\"]` を付けて投稿してください。\n",
        entry_id = escalation.entry_id,
        author = escalation.author,
        branch = escalation
            .origin_branch
            .as_deref()
            .map(|branch| format!(" ({branch})"))
            .unwrap_or_default(),
        created_at = escalation
            .created_at
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        body = escalation.body.trim(),
    )
}

/// Render open escalations as PM-facing context lines, one per escalation.
///
/// Returns lines rather than a block so each caller controls the separator:
/// a pane injection has to stay on one physical line (an embedded newline
/// submits the prompt early), while a document surface wants real line breaks.
/// The wording itself stays in one place so two surfaces cannot describe the
/// same blocker differently.
pub fn render_open_escalation_lines(
    escalations: &[BoardEscalation],
    body_char_limit: usize,
) -> Vec<String> {
    escalations
        .iter()
        .map(|escalation| {
            let owners = if escalation.owners.is_empty() {
                "(no owner)".to_string()
            } else {
                escalation
                    .owners
                    .iter()
                    .map(|owner| format!("#{}", owner.trim_start_matches('#')))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            format!(
                "{owners} blocked by {author} at {created_at} [resolve with params.resolves:[\"{entry_id}\"]]: {body}",
                author = escalation.author,
                created_at = escalation
                    .created_at
                    .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                entry_id = escalation.entry_id,
                body = truncate_body(&escalation.body, body_char_limit),
            )
        })
        .collect()
}

fn truncate_body(body: &str, limit: usize) -> String {
    let collapsed = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= limit {
        return collapsed;
    }
    let mut truncated: String = collapsed.chars().take(limit).collect();
    truncated.push('…');
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordination::AuthorKind;
    use chrono::TimeZone;

    fn entry(id: &str, kind: BoardEntryKind, owners: &[&str], body: &str) -> BoardEntry {
        let mut entry = BoardEntry::new(
            AuthorKind::Agent,
            "Claude Code",
            kind,
            body,
            None,
            None,
            Vec::new(),
            owners.iter().map(|owner| owner.to_string()).collect(),
        );
        entry.id = id.to_string();
        entry.created_at = Utc.with_ymd_and_hms(2026, 8, 18, 6, 0, 0).unwrap();
        entry.updated_at = entry.created_at;
        entry
    }

    fn blocked(id: &str, owners: &[&str]) -> BoardEntry {
        entry(id, BoardEntryKind::Blocked, owners, "事象: 実行不能")
    }

    #[test]
    fn a_blocked_post_opens_an_escalation_for_its_owner() {
        let mut store = BoardEscalationStore::default();
        assert!(store.apply_entry(&blocked("b1", &["2338"])));

        assert_eq!(store.open_owner_issue_numbers(), vec![2338]);
        assert_eq!(store.open_for_owner("2338").len(), 1);
        assert_eq!(store.open_for_owner("3645").len(), 0);
    }

    #[test]
    fn an_ordinary_post_neither_opens_nor_changes_anything() {
        let mut store = BoardEscalationStore::default();
        store.apply_entry(&blocked("b1", &["2338"]));

        let status = entry(
            "s1",
            BoardEntryKind::Status,
            &["2338"],
            "Claude Code is ready for the next instruction on Issue #2338",
        );
        assert!(
            !store.apply_entry(&status),
            "a routine status post must not touch the escalation index"
        );
        assert_eq!(
            store.open_owner_issue_numbers(),
            vec![2338],
            "the Stop-gate status post must never close an unblock request"
        );
    }

    #[test]
    fn an_explicit_resolution_closes_exactly_the_named_escalation() {
        let mut store = BoardEscalationStore::default();
        store.apply_entry(&blocked("b1", &["2338"]));
        store.apply_entry(&blocked("b2", &["3645"]));

        let mut resolution = entry(
            "r1",
            BoardEntryKind::Decision,
            &["2338"],
            "fresh launch を手配しました",
        );
        resolution.resolves_entry_ids = vec!["b1".to_string()];
        assert!(store.apply_entry(&resolution));

        assert_eq!(store.open_owner_issue_numbers(), vec![3645]);
        let closed = store
            .escalations
            .iter()
            .find(|escalation| escalation.entry_id == "b1")
            .unwrap();
        assert_eq!(closed.resolved_by_entry_id.as_deref(), Some("r1"));
        assert!(closed.resolved_at.is_some());
    }

    #[test]
    fn resolving_an_unknown_or_already_closed_escalation_is_a_no_op() {
        let mut store = BoardEscalationStore::default();
        store.apply_entry(&blocked("b1", &["2338"]));
        let mut resolution = entry("r1", BoardEntryKind::Status, &["2338"], "解消しました");
        resolution.resolves_entry_ids = vec!["b1".to_string()];
        store.apply_entry(&resolution);

        let mut again = entry("r2", BoardEntryKind::Status, &["2338"], "もう一度");
        again.resolves_entry_ids = vec!["b1".to_string(), "does-not-exist".to_string()];
        assert!(!store.apply_entry(&again));
        assert_eq!(
            store
                .escalations
                .iter()
                .find(|escalation| escalation.entry_id == "b1")
                .unwrap()
                .resolved_by_entry_id
                .as_deref(),
            Some("r1"),
            "the first resolver keeps the audit trail"
        );
    }

    #[test]
    fn a_post_cannot_resolve_itself() {
        let mut store = BoardEscalationStore::default();
        let mut selfish = blocked("b1", &["2338"]);
        selfish.resolves_entry_ids = vec!["b1".to_string()];
        store.apply_entry(&selfish);

        assert_eq!(
            store.open_owner_issue_numbers(),
            vec![2338],
            "a blocked post that names its own id must still be open"
        );
    }

    #[test]
    fn the_same_session_restating_a_block_supersedes_its_own_earlier_one() {
        let mut first = blocked("b1", &["2338"]);
        first.origin_session_id = Some("session-a".to_string());
        let mut second = blocked("b2", &["2338"]);
        second.origin_session_id = Some("session-a".to_string());

        let mut store = BoardEscalationStore::default();
        store.apply_entry(&first);
        store.apply_entry(&second);

        assert_eq!(store.open().count(), 1);
        assert_eq!(store.open().next().unwrap().entry_id, "b2");
        assert_eq!(store.open_owner_issue_numbers(), vec![2338]);
    }

    #[test]
    fn two_sessions_blocked_on_one_owner_stay_two_escalations() {
        let mut first = blocked("b1", &["2338"]);
        first.origin_session_id = Some("session-a".to_string());
        let mut second = blocked("b2", &["2338"]);
        second.origin_session_id = Some("session-b".to_string());

        let mut store = BoardEscalationStore::default();
        store.apply_entry(&first);
        store.apply_entry(&second);

        assert_eq!(
            store.open().count(),
            2,
            "another session's block is an independent request"
        );
        assert_eq!(store.open_owner_issue_numbers(), vec![2338]);
    }

    #[test]
    fn replaying_the_same_entry_twice_is_idempotent() {
        let posted = blocked("b1", &["2338"]);
        let mut store = BoardEscalationStore::default();
        assert!(store.apply_entry(&posted));
        assert!(!store.apply_entry(&posted));
        assert_eq!(store.escalations.len(), 1);
    }

    #[test]
    fn replaying_the_event_stream_reproduces_the_incremental_fold() {
        let mut resolution = entry("r1", BoardEntryKind::Status, &["2338"], "解消");
        resolution.resolves_entry_ids = vec!["b1".to_string()];
        let stream = vec![
            blocked("b1", &["2338"]),
            blocked("b2", &["3645"]),
            resolution,
        ];

        let mut incremental = BoardEscalationStore::default();
        for entry in &stream {
            incremental.apply_entry(entry);
        }
        let replayed = BoardEscalationStore::from_entries(&stream);

        assert_eq!(
            replayed.open_owner_issue_numbers(),
            incremental.open_owner_issue_numbers()
        );
        assert_eq!(replayed.escalations.len(), incremental.escalations.len());
    }

    #[test]
    fn an_escalation_without_an_owner_is_still_open_but_names_no_issue() {
        let mut store = BoardEscalationStore::default();
        store.apply_entry(&blocked("b1", &[]));

        assert_eq!(store.open().count(), 1);
        assert!(store.open_owner_issue_numbers().is_empty());
    }

    #[test]
    fn owner_numbers_tolerate_a_hash_prefix_and_ignore_non_numeric_owners() {
        let mut store = BoardEscalationStore::default();
        store.apply_entry(&blocked("b1", &["#2338", "SPEC-3645", "3607"]));

        assert_eq!(store.open_owner_issue_numbers(), vec![2338, 3607]);
    }

    #[test]
    fn pruning_drops_old_resolved_rows_and_keeps_every_open_one() {
        let mut store = BoardEscalationStore::default();
        let mut old_open = blocked("b1", &["2338"]);
        old_open.created_at = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        store.apply_entry(&old_open);

        let mut old_closed = blocked("b2", &["3645"]);
        old_closed.created_at = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        store.apply_entry(&old_closed);
        let mut resolution = entry("r1", BoardEntryKind::Status, &["3645"], "解消");
        resolution.resolves_entry_ids = vec!["b2".to_string()];
        store.apply_entry(&resolution);

        assert!(store.prune_resolved_before(Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap()));
        assert_eq!(store.escalations.len(), 1);
        assert_eq!(
            store.open_owner_issue_numbers(),
            vec![2338],
            "an unanswered request must survive pruning at any age"
        );
    }

    #[test]
    fn rendering_names_the_owner_author_and_body() {
        let mut store = BoardEscalationStore::default();
        let mut posted = blocked("b1", &["2338"]);
        posted.body =
            "事象: execution.reopen が immutable で拒否\n原因: Completed ECR\n依頼: fresh launch\n再開条件: 新 pane"
                .to_string();
        store.apply_entry(&posted);

        let lines = render_open_escalation_lines(&store.open_escalations(), 200);
        assert_eq!(lines.len(), 1);
        let rendered = &lines[0];
        assert!(rendered.contains("#2338"), "{rendered}");
        assert!(rendered.contains("Claude Code"), "{rendered}");
        assert!(rendered.contains("execution.reopen"), "{rendered}");
        assert!(rendered.contains("fresh launch"), "{rendered}");
        assert!(
            rendered.contains("params.resolves:[\"b1\"]"),
            "the reader must be told how to close it: {rendered}"
        );
        assert!(
            !rendered.contains('\n'),
            "each line must stay injectable into a pane: {rendered}"
        );
    }

    #[test]
    fn a_four_section_body_passes_validation_in_japanese() {
        let body = "事象: execution.reopen が immutable で拒否された\n\
                    原因: Completed ECR は不変で、この window では reopen できない\n\
                    依頼: fresh launch を手配してほしい\n\
                    再開条件: 新しい pane が #2338 に紐づいて起動されること";
        assert!(validate_escalation_body(body).is_ok());
    }

    #[test]
    fn a_four_section_body_passes_validation_in_english() {
        let body = "Symptom: execution.reopen was refused as immutable\n\
                    Cause: the ECR is Completed and cannot be reopened here\n\
                    Request: schedule a fresh launch\n\
                    Resume: a new pane is launched against #2338";
        assert!(validate_escalation_body(body).is_ok());
    }

    #[test]
    fn markdown_decoration_around_the_label_still_matches() {
        let body = "- **事象**: 拒否された\n\
                    * 原因: immutable\n\
                    # 依頼: fresh launch\n\
                    > 再開条件: 新 pane";
        assert!(validate_escalation_body(body).is_ok());
    }

    #[test]
    fn a_body_missing_sections_is_refused_with_a_usable_template() {
        let error = validate_escalation_body("進められません").unwrap_err();
        let message = error.to_string();
        for expected in ["事象", "原因", "依頼", "再開条件"] {
            assert!(message.contains(expected), "{message}");
        }
        assert!(
            message.contains("拒否された operation とエラー文言"),
            "the refusal must hand back a copy-pasteable shape: {message}"
        );
    }

    #[test]
    fn a_body_missing_only_the_resume_condition_names_only_that_section() {
        let body = "事象: 拒否された\n原因: immutable\n依頼: fresh launch";
        let error = validate_escalation_body(body).unwrap_err();
        let EscalationBodyError::MissingSections { missing } = error;
        assert_eq!(missing, vec!["再開条件 / Resume condition".to_string()]);
    }

    #[test]
    fn a_label_that_merely_appears_mid_sentence_does_not_count() {
        let body = "作業の原因: を調べたが事象: も依頼: も再開条件: も 1 行に詰め込んだ";
        // A label has to open its own line. Everything crammed onto one line
        // reads as prose, and prose is what the PM could not act on.
        let error = validate_escalation_body(body).unwrap_err();
        let EscalationBodyError::MissingSections { missing } = error;
        assert_eq!(missing.len(), 4, "{missing:?}");
    }

    #[test]
    fn execution_refusals_require_typed_disposition_while_legacy_families_remain_classified() {
        assert_eq!(
            classify_operation_refusal(
                "execution.reopen",
                "Completed issue #2338 is immutable; use a fresh launch for new work"
            ),
            None,
            "execution display wording must never decide escalation"
        );
        assert_eq!(
            classify_operation_refusal(
                "workspace.ensure",
                "typed workspace.ensure compatibility continuation is available only for an exact Host Session authority"
            ),
            Some(OperationRefusalKind::Authority)
        );
        assert_eq!(
            classify_operation_refusal(
                "pane.close",
                "pane close: backend did not close window-3; the target may be this authenticated Session and requires a correlated acceptance"
            ),
            Some(OperationRefusalKind::Permission)
        );
    }

    #[test]
    fn a_pane_that_simply_does_not_exist_is_not_a_governance_refusal() {
        assert_eq!(
            classify_operation_refusal("pane.close", "pane close: unknown pane nope"),
            None,
            "a stale pane id is a caller mistake, not something the PM can unblock"
        );
    }

    #[test]
    fn an_ordinary_caller_mistake_is_not_escalated() {
        assert_eq!(
            classify_operation_refusal("execution.blocked", "missing required flag: reason"),
            None,
            "a malformed call is the agent's own problem"
        );
        assert_eq!(
            classify_operation_refusal("issue.view", "issue #99 is unavailable"),
            None,
            "a read operation outside the governance families must not raise an alarm"
        );
        assert_eq!(
            classify_operation_refusal("search", "index refused to build"),
            None
        );
    }

    /// Neighbouring surfaces that refuse for reasons the agent must handle
    /// itself. Pinned as negatives: adding them would be an easy "improvement"
    /// that quietly turns the escalation channel into noise, and would put a
    /// local write inside a refusal contractually promising none.
    #[test]
    fn agent_fixable_and_no_write_refusals_stay_out_of_the_escalation_channel() {
        for (operation, error) in [
            (
                "build.complete",
                "build complete refused: no fresh verification record",
            ),
            ("verify.run", "verify.run refused: the plan is stale"),
            (
                "pr.ready",
                "pr.ready refused: verification evidence is unavailable",
            ),
            (
                "workspace.update",
                "workspace.update refused: workspace_ensure_required",
            ),
        ] {
            assert_eq!(
                classify_operation_refusal(operation, error),
                None,
                "{operation} must not auto-escalate"
            );
        }
    }

    #[test]
    fn an_auto_filed_body_satisfies_the_same_four_section_contract() {
        let body = render_operation_refusal_body(
            "execution.reopen",
            "Completed issue #2338 is immutable; use a fresh launch for new work",
            OperationRefusalKind::Immutability,
        );
        validate_escalation_body(&body).expect("the auto-filed body must pass the AC-1 contract");
        assert!(body.contains("execution.reopen"), "{body}");
        assert!(
            body.contains("Completed issue #2338 is immutable"),
            "the verbatim error is what tells the PM which lever to pull: {body}"
        );
        assert!(body.contains("fresh launch"), "{body}");
    }

    #[test]
    fn a_declared_block_body_satisfies_the_same_four_section_contract() {
        let body = render_declared_block_body(
            "Completed ECR のため、この window では残 AC を実装できない",
            Some("cargo test -p gwt --bin gwt"),
        );
        validate_escalation_body(&body).expect("a declared block must satisfy AC-1");
        assert!(body.contains("execution.blocked"), "{body}");
        assert!(body.contains("Completed ECR"), "{body}");
        assert!(
            body.contains("cargo test -p gwt --bin gwt"),
            "the verification the agent could not run is part of the ask: {body}"
        );
    }

    #[test]
    fn a_declared_block_without_missing_verification_omits_that_line() {
        let body = render_declared_block_body("provider quota が枯渇した", None);
        validate_escalation_body(&body).expect("a declared block must satisfy AC-1");
        assert!(!body.contains("未実施の検証"), "{body}");
    }

    #[test]
    fn the_issue_comment_mirror_carries_the_body_and_the_resolve_handle() {
        let mut store = BoardEscalationStore::default();
        let mut posted = blocked("b1", &["2338"]);
        posted.body =
            "事象: 拒否\n原因: immutable\n依頼: fresh launch\n再開条件: 新 pane".to_string();
        posted.origin_branch = Some("work/issue-2338".to_string());
        store.apply_entry(&posted);

        let comment = render_escalation_issue_comment(&store.open_escalations()[0]);
        assert!(comment.contains("fresh launch"), "{comment}");
        assert!(comment.contains("work/issue-2338"), "{comment}");
        assert!(
            comment.contains("params.resolves:[\"b1\"]"),
            "the mirror must tell the resolver how to close it: {comment}"
        );
    }

    #[test]
    fn rendering_truncates_a_long_body_without_splitting_a_character() {
        let mut store = BoardEscalationStore::default();
        let mut posted = blocked("b1", &["2338"]);
        posted.body = "あ".repeat(400);
        store.apply_entry(&posted);

        let rendered = render_open_escalation_lines(&store.open_escalations(), 20).remove(0);
        assert!(rendered.contains('…'), "{rendered}");
        assert!(rendered.chars().count() < 200, "{rendered}");
    }
}
