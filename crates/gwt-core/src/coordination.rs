//! Repo-local coordination storage for shared board and agent cards.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{GwtError, Result};

pub const COORDINATION_RELATIVE_DIR: &str = ".gwt/coordination";
pub const EVENTS_FILE_NAME: &str = "events.jsonl";
pub const BOARD_PROJECTION_FILE_NAME: &str = "board.latest.json";
pub const CARDS_PROJECTION_FILE_NAME: &str = "cards.latest.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthorKind {
    User,
    Agent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BoardEntryKind {
    Request,
    Status,
    Next,
    Claim,
    Impact,
    Question,
    Blocked,
    Handoff,
    Decision,
}

impl std::str::FromStr for BoardEntryKind {
    type Err = GwtError;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "request" => Ok(Self::Request),
            "status" => Ok(Self::Status),
            "next" => Ok(Self::Next),
            "claim" => Ok(Self::Claim),
            "impact" => Ok(Self::Impact),
            "question" => Ok(Self::Question),
            "blocked" => Ok(Self::Blocked),
            "handoff" => Ok(Self::Handoff),
            "decision" => Ok(Self::Decision),
            other => Err(GwtError::Other(format!(
                "unknown board entry kind: {other}"
            ))),
        }
    }
}

impl BoardEntryKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Request => "request",
            Self::Status => "status",
            Self::Next => "next",
            Self::Claim => "claim",
            Self::Impact => "impact",
            Self::Question => "question",
            Self::Blocked => "blocked",
            Self::Handoff => "handoff",
            Self::Decision => "decision",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BoardEntry {
    pub id: String,
    pub author_kind: AuthorKind,
    pub author: String,
    pub kind: BoardEntryKind,
    pub body: String,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub parent_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub related_topics: Vec<String>,
    #[serde(default)]
    pub related_owners: Vec<String>,
}

impl BoardEntry {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        author_kind: AuthorKind,
        author: impl Into<String>,
        kind: BoardEntryKind,
        body: impl Into<String>,
        state: Option<String>,
        parent_id: Option<String>,
        related_topics: Vec<String>,
        related_owners: Vec<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            author_kind,
            author: author.into(),
            kind,
            body: body.into(),
            state,
            parent_id,
            created_at: now,
            updated_at: now,
            related_topics,
            related_owners,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentCard {
    pub agent_id: String,
    #[serde(default)]
    pub session_id: Option<String>,
    pub branch: String,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub responsibility: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub current_focus: Option<String>,
    #[serde(default)]
    pub next_action: Option<String>,
    #[serde(default)]
    pub blocked_reason: Option<String>,
    #[serde(default)]
    pub related_topics: Vec<String>,
    #[serde(default)]
    pub related_owners: Vec<String>,
    #[serde(default)]
    pub working_scope: Option<String>,
    #[serde(default)]
    pub handoff_target: Option<String>,
    pub updated_at: DateTime<Utc>,
}

impl AgentCard {
    pub fn key(&self) -> String {
        if let Some(session_id) = self.session_id.as_deref() {
            if !session_id.trim().is_empty() {
                return format!("session:{session_id}");
            }
        }
        format!("agent:{}:{}", self.agent_id, self.branch)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentCardPatch {
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub responsibility: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub current_focus: Option<String>,
    #[serde(default)]
    pub next_action: Option<String>,
    #[serde(default)]
    pub blocked_reason: Option<String>,
    #[serde(default)]
    pub related_topics: Option<Vec<String>>,
    #[serde(default)]
    pub related_owners: Option<Vec<String>>,
    #[serde(default)]
    pub working_scope: Option<String>,
    #[serde(default)]
    pub handoff_target: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentCardContext {
    pub agent_id: String,
    pub session_id: Option<String>,
    pub branch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CoordinationEvent {
    BoardPost { entry: BoardEntry },
    AgentCardUpsert { card: AgentCard },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BoardProjection {
    #[serde(default)]
    pub entries: Vec<BoardEntry>,
    pub updated_at: DateTime<Utc>,
}

impl Default for BoardProjection {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            updated_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentCardsProjection {
    #[serde(default)]
    pub cards: Vec<AgentCard>,
    pub updated_at: DateTime<Utc>,
}

impl Default for AgentCardsProjection {
    fn default() -> Self {
        Self {
            cards: Vec::new(),
            updated_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CoordinationSnapshot {
    pub board: BoardProjection,
    pub cards: AgentCardsProjection,
}

pub fn coordination_dir(worktree_root: &Path) -> PathBuf {
    worktree_root.join(COORDINATION_RELATIVE_DIR)
}

pub fn coordination_events_path(worktree_root: &Path) -> PathBuf {
    coordination_dir(worktree_root).join(EVENTS_FILE_NAME)
}

pub fn coordination_board_projection_path(worktree_root: &Path) -> PathBuf {
    coordination_dir(worktree_root).join(BOARD_PROJECTION_FILE_NAME)
}

pub fn coordination_cards_projection_path(worktree_root: &Path) -> PathBuf {
    coordination_dir(worktree_root).join(CARDS_PROJECTION_FILE_NAME)
}

fn coordination_lock_path(worktree_root: &Path) -> PathBuf {
    coordination_dir(worktree_root).join(".lock")
}

pub fn ensure_repo_local_files(worktree_root: &Path) -> Result<()> {
    let dir = coordination_dir(worktree_root);
    std::fs::create_dir_all(&dir)?;

    if !coordination_events_path(worktree_root).exists() {
        File::create(coordination_events_path(worktree_root))?;
    }
    if !coordination_board_projection_path(worktree_root).exists() {
        write_atomic_json(
            &coordination_board_projection_path(worktree_root),
            &BoardProjection::default(),
        )?;
    }
    if !coordination_cards_projection_path(worktree_root).exists() {
        write_atomic_json(
            &coordination_cards_projection_path(worktree_root),
            &AgentCardsProjection::default(),
        )?;
    }

    Ok(())
}

pub fn load_snapshot(worktree_root: &Path) -> Result<CoordinationSnapshot> {
    ensure_repo_local_files(worktree_root)?;
    Ok(CoordinationSnapshot {
        board: load_json_or_default(&coordination_board_projection_path(worktree_root))?,
        cards: load_json_or_default(&coordination_cards_projection_path(worktree_root))?,
    })
}

pub fn post_entry(worktree_root: &Path, entry: BoardEntry) -> Result<CoordinationSnapshot> {
    append_event(worktree_root, &CoordinationEvent::BoardPost { entry })
}

pub fn apply_agent_card_patch(
    worktree_root: &Path,
    context: AgentCardContext,
    patch: AgentCardPatch,
) -> Result<CoordinationSnapshot> {
    ensure_repo_local_files(worktree_root)?;
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(coordination_lock_path(worktree_root))?;
    lock.lock_exclusive()?;

    let result = apply_agent_card_patch_locked(worktree_root, context, patch);
    let unlock_result = lock.unlock();
    match (result, unlock_result) {
        (Ok(snapshot), Ok(())) => Ok(snapshot),
        (Err(err), _) => Err(err),
        (Ok(_), Err(err)) => Err(err.into()),
    }
}

fn apply_agent_card_patch_locked(
    worktree_root: &Path,
    context: AgentCardContext,
    patch: AgentCardPatch,
) -> Result<CoordinationSnapshot> {
    let snapshot = load_snapshot(worktree_root)?;
    let key = if let Some(session_id) = context.session_id.as_deref() {
        if !session_id.trim().is_empty() {
            format!("session:{session_id}")
        } else {
            format!("agent:{}:{}", context.agent_id, context.branch)
        }
    } else {
        format!("agent:{}:{}", context.agent_id, context.branch)
    };
    let now = Utc::now();
    let mut card = snapshot
        .cards
        .cards
        .iter()
        .find(|candidate| candidate.key() == key)
        .cloned()
        .unwrap_or(AgentCard {
            agent_id: context.agent_id,
            session_id: context.session_id,
            branch: context.branch,
            role: None,
            responsibility: None,
            status: None,
            current_focus: None,
            next_action: None,
            blocked_reason: None,
            related_topics: Vec::new(),
            related_owners: Vec::new(),
            working_scope: None,
            handoff_target: None,
            updated_at: now,
        });

    if let Some(value) = patch.role {
        card.role = Some(value);
    }
    if let Some(value) = patch.responsibility {
        card.responsibility = Some(value);
    }
    if let Some(value) = patch.status {
        card.status = Some(value);
    }
    if let Some(value) = patch.current_focus {
        card.current_focus = Some(value);
    }
    if let Some(value) = patch.next_action {
        card.next_action = Some(value);
    }
    if let Some(value) = patch.blocked_reason {
        card.blocked_reason = Some(value);
    }
    if let Some(value) = patch.related_topics {
        card.related_topics = value;
    }
    if let Some(value) = patch.related_owners {
        card.related_owners = value;
    }
    if let Some(value) = patch.working_scope {
        card.working_scope = Some(value);
    }
    if let Some(value) = patch.handoff_target {
        card.handoff_target = Some(value);
    }
    card.updated_at = now;

    append_event_locked(worktree_root, &CoordinationEvent::AgentCardUpsert { card })
}

pub fn append_event(
    worktree_root: &Path,
    event: &CoordinationEvent,
) -> Result<CoordinationSnapshot> {
    ensure_repo_local_files(worktree_root)?;
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(coordination_lock_path(worktree_root))?;
    lock.lock_exclusive()?;

    let result = append_event_locked(worktree_root, event);
    let unlock_result = lock.unlock();
    match (result, unlock_result) {
        (Ok(snapshot), Ok(())) => Ok(snapshot),
        (Err(err), _) => Err(err),
        (Ok(_), Err(err)) => Err(err.into()),
    }
}

fn append_event_locked(
    worktree_root: &Path,
    event: &CoordinationEvent,
) -> Result<CoordinationSnapshot> {
    let event_path = coordination_events_path(worktree_root);
    let mut file = OpenOptions::new()
        .append(true)
        .create(true)
        .open(&event_path)?;
    serde_json::to_writer(&mut file, event).map_err(json_error)?;
    file.write_all(b"\n")?;
    file.sync_all()?;

    let snapshot = rebuild_snapshot_from_events(&event_path)?;
    write_atomic_json(
        &coordination_board_projection_path(worktree_root),
        &snapshot.board,
    )?;
    write_atomic_json(
        &coordination_cards_projection_path(worktree_root),
        &snapshot.cards,
    )?;
    Ok(snapshot)
}

pub fn rebuild_snapshot_from_events(event_path: &Path) -> Result<CoordinationSnapshot> {
    if !event_path.exists() {
        File::create(event_path)?;
    }
    let file = OpenOptions::new().read(true).open(event_path)?;
    let reader = BufReader::new(file);

    let mut board_entries = Vec::new();
    let mut cards = BTreeMap::<String, AgentCard>::new();

    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let event: CoordinationEvent = serde_json::from_str(trimmed).map_err(json_error)?;
        match event {
            CoordinationEvent::BoardPost { entry } => board_entries.push(entry),
            CoordinationEvent::AgentCardUpsert { card } => {
                cards.insert(card.key(), card);
            }
        }
    }

    board_entries.sort_by_key(|entry| entry.created_at);

    let now = Utc::now();
    let mut cards_vec: Vec<AgentCard> = cards.into_values().collect();
    cards_vec.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

    Ok(CoordinationSnapshot {
        board: BoardProjection {
            entries: board_entries,
            updated_at: now,
        },
        cards: AgentCardsProjection {
            cards: cards_vec,
            updated_at: now,
        },
    })
}

fn load_json_or_default<T>(path: &Path) -> Result<T>
where
    T: serde::de::DeserializeOwned + Default,
{
    if !path.exists() {
        return Ok(T::default());
    }
    let raw = std::fs::read_to_string(path)?;
    if raw.trim().is_empty() {
        return Ok(T::default());
    }
    serde_json::from_str(&raw).map_err(json_error)
}

fn write_atomic_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value).map_err(json_error)?;
    write_atomic(path, &bytes)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| GwtError::Other(format!("path has no parent: {}", path.display())))?;
    std::fs::create_dir_all(parent)?;
    let tmp_path = parent.join(format!(
        ".{}.tmp-{}-{}",
        path.file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("coordination"),
        std::process::id(),
        Uuid::new_v4()
    ));
    {
        let mut file = File::create(&tmp_path)?;
        file.write_all(bytes)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
    }
    if cfg!(windows) && path.exists() {
        std::fs::remove_file(path)?;
    }
    std::fs::rename(&tmp_path, path)?;
    Ok(())
}

fn json_error(err: serde_json::Error) -> GwtError {
    GwtError::Other(err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn load_snapshot_bootstraps_empty_files() {
        let dir = tempfile::tempdir().unwrap();

        let snapshot = load_snapshot(dir.path()).unwrap();

        assert!(snapshot.board.entries.is_empty());
        assert!(snapshot.cards.cards.is_empty());
        assert!(coordination_events_path(dir.path()).exists());
        assert!(coordination_board_projection_path(dir.path()).exists());
        assert!(coordination_cards_projection_path(dir.path()).exists());
    }

    #[test]
    fn post_entry_updates_event_log_and_board_projection() {
        let dir = tempfile::tempdir().unwrap();

        let snapshot = post_entry(
            dir.path(),
            BoardEntry::new(
                AuthorKind::User,
                "user",
                BoardEntryKind::Request,
                "Need a coordination surface",
                None,
                None,
                vec!["coordination".into()],
                vec![],
            ),
        )
        .unwrap();

        assert_eq!(snapshot.board.entries.len(), 1);
        assert_eq!(
            snapshot.board.entries[0].body,
            "Need a coordination surface"
        );
        let raw = std::fs::read_to_string(coordination_events_path(dir.path())).unwrap();
        assert!(raw.contains("\"type\":\"board_post\""));
    }

    #[test]
    fn apply_agent_card_patch_keeps_latest_card_state() {
        let dir = tempfile::tempdir().unwrap();

        apply_agent_card_patch(
            dir.path(),
            AgentCardContext {
                agent_id: "Codex".into(),
                session_id: Some("sess-1".into()),
                branch: "feature/coordination".into(),
            },
            AgentCardPatch {
                status: Some("running".into()),
                current_focus: Some("Wire storage".into()),
                ..AgentCardPatch::default()
            },
        )
        .unwrap();

        let snapshot = apply_agent_card_patch(
            dir.path(),
            AgentCardContext {
                agent_id: "Codex".into(),
                session_id: Some("sess-1".into()),
                branch: "feature/coordination".into(),
            },
            AgentCardPatch {
                next_action: Some("Add CLI".into()),
                ..AgentCardPatch::default()
            },
        )
        .unwrap();

        assert_eq!(snapshot.cards.cards.len(), 1);
        let card = &snapshot.cards.cards[0];
        assert_eq!(card.status.as_deref(), Some("running"));
        assert_eq!(card.current_focus.as_deref(), Some("Wire storage"));
        assert_eq!(card.next_action.as_deref(), Some("Add CLI"));
    }

    #[test]
    fn rebuild_snapshot_reconstructs_latest_cards_and_threads() {
        let dir = tempfile::tempdir().unwrap();

        let first = BoardEntry::new(
            AuthorKind::User,
            "user",
            BoardEntryKind::Request,
            "Initial request",
            None,
            None,
            vec![],
            vec![],
        );
        let parent_id = first.id.clone();
        append_event(dir.path(), &CoordinationEvent::BoardPost { entry: first }).unwrap();
        append_event(
            dir.path(),
            &CoordinationEvent::BoardPost {
                entry: BoardEntry::new(
                    AuthorKind::Agent,
                    "Codex",
                    BoardEntryKind::Claim,
                    "I will take this",
                    None,
                    Some(parent_id),
                    vec![],
                    vec![],
                ),
            },
        )
        .unwrap();
        apply_agent_card_patch(
            dir.path(),
            AgentCardContext {
                agent_id: "Codex".into(),
                session_id: Some("sess-1".into()),
                branch: "feature/coordination".into(),
            },
            AgentCardPatch {
                status: Some("waiting_input".into()),
                ..AgentCardPatch::default()
            },
        )
        .unwrap();

        let rebuilt = rebuild_snapshot_from_events(&coordination_events_path(dir.path())).unwrap();

        assert_eq!(rebuilt.board.entries.len(), 2);
        assert_eq!(
            rebuilt.board.entries[1].parent_id.as_deref(),
            Some(rebuilt.board.entries[0].id.as_str())
        );
        assert_eq!(rebuilt.cards.cards.len(), 1);
        assert_eq!(
            rebuilt.cards.cards[0].status.as_deref(),
            Some("waiting_input")
        );
    }

    #[test]
    fn concurrent_post_entry_preserves_all_records() {
        let dir = tempfile::tempdir().unwrap();
        let root = Arc::new(dir.path().to_path_buf());

        let mut handles = Vec::new();
        for idx in 0..8 {
            let root = Arc::clone(&root);
            handles.push(thread::spawn(move || {
                post_entry(
                    &root,
                    BoardEntry::new(
                        AuthorKind::User,
                        "user",
                        BoardEntryKind::Request,
                        format!("request-{idx}"),
                        None,
                        None,
                        vec![],
                        vec![],
                    ),
                )
                .unwrap();
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }

        let snapshot = load_snapshot(root.as_path()).unwrap();
        assert_eq!(snapshot.board.entries.len(), 8);
    }

    #[test]
    fn repeated_projection_writes_leave_no_tmp_files() {
        let dir = tempfile::tempdir().unwrap();

        for idx in 0..3 {
            post_entry(
                dir.path(),
                BoardEntry::new(
                    AuthorKind::User,
                    "user",
                    BoardEntryKind::Status,
                    format!("update-{idx}"),
                    None,
                    None,
                    Vec::new(),
                    Vec::new(),
                ),
            )
            .unwrap();
        }

        let mut names: Vec<String> = std::fs::read_dir(coordination_dir(dir.path()))
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();

        assert_eq!(
            names,
            vec![
                ".lock".to_string(),
                "board.latest.json".to_string(),
                "cards.latest.json".to_string(),
                "events.jsonl".to_string(),
            ]
        );
    }

    #[test]
    fn concurrent_agent_card_patches_merge_fields_without_losing_updates() {
        let dir = tempfile::tempdir().unwrap();
        let root = Arc::new(dir.path().to_path_buf());
        ensure_repo_local_files(root.as_path()).unwrap();

        let lock_file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(coordination_lock_path(root.as_path()))
            .unwrap();
        lock_file.lock_exclusive().unwrap();

        let barrier = Arc::new(std::sync::Barrier::new(3));
        let first_root = Arc::clone(&root);
        let first_barrier = Arc::clone(&barrier);
        let first = thread::spawn(move || {
            first_barrier.wait();
            apply_agent_card_patch(
                first_root.as_path(),
                AgentCardContext {
                    agent_id: "Codex".into(),
                    session_id: Some("sess-1".into()),
                    branch: "feature/coordination".into(),
                },
                AgentCardPatch {
                    status: Some("running".into()),
                    current_focus: Some("Wire storage".into()),
                    ..AgentCardPatch::default()
                },
            )
            .unwrap();
        });

        let second_root = Arc::clone(&root);
        let second_barrier = Arc::clone(&barrier);
        let second = thread::spawn(move || {
            second_barrier.wait();
            apply_agent_card_patch(
                second_root.as_path(),
                AgentCardContext {
                    agent_id: "Codex".into(),
                    session_id: Some("sess-1".into()),
                    branch: "feature/coordination".into(),
                },
                AgentCardPatch {
                    next_action: Some("Add CLI".into()),
                    ..AgentCardPatch::default()
                },
            )
            .unwrap();
        });

        barrier.wait();
        thread::sleep(std::time::Duration::from_millis(50));
        lock_file.unlock().unwrap();

        first.join().unwrap();
        second.join().unwrap();

        let snapshot = load_snapshot(root.as_path()).unwrap();
        assert_eq!(snapshot.cards.cards.len(), 1);
        let card = &snapshot.cards.cards[0];
        assert_eq!(card.status.as_deref(), Some("running"));
        assert_eq!(card.current_focus.as_deref(), Some("Wire storage"));
        assert_eq!(card.next_action.as_deref(), Some("Add CLI"));
    }
}
