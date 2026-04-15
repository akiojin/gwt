//! Repo-local coordination storage for a shared board chat timeline.

use std::{
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{paths::gwt_coordination_dir_for_repo_path, GwtError, Result};

pub const COORDINATION_RELATIVE_DIR: &str = ".gwt/coordination";
pub const EVENTS_FILE_NAME: &str = "events.jsonl";
pub const BOARD_PROJECTION_FILE_NAME: &str = "board.latest.json";
const MIGRATION_MARKER_FILE_NAME: &str = ".migration-complete";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthorKind {
    User,
    Agent,
    System,
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
    #[serde(alias = "board_post")]
    MessageAppended {
        entry: BoardEntry,
    },
    AgentCardUpsert {
        card: AgentCard,
    },
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
    gwt_coordination_dir_for_repo_path(worktree_root)
        .unwrap_or_else(|| legacy_coordination_dir(worktree_root))
}

pub fn coordination_events_path(worktree_root: &Path) -> PathBuf {
    coordination_dir(worktree_root).join(EVENTS_FILE_NAME)
}

pub fn coordination_board_projection_path(worktree_root: &Path) -> PathBuf {
    coordination_dir(worktree_root).join(BOARD_PROJECTION_FILE_NAME)
}

fn coordination_lock_path(worktree_root: &Path) -> PathBuf {
    coordination_dir(worktree_root).join(".lock")
}

pub fn ensure_repo_local_files(worktree_root: &Path) -> Result<()> {
    if let Some(project_dir) = gwt_coordination_dir_for_repo_path(worktree_root) {
        let legacy_dirs = discover_legacy_coordination_dirs(worktree_root);
        migrate_legacy_coordination_dirs(&project_dir, &legacy_dirs)?;
    }

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

    Ok(())
}

pub fn load_snapshot(worktree_root: &Path) -> Result<CoordinationSnapshot> {
    ensure_repo_local_files(worktree_root)?;
    Ok(CoordinationSnapshot {
        board: load_json_or_default(&coordination_board_projection_path(worktree_root))?,
        cards: AgentCardsProjection::default(),
    })
}

pub fn post_entry(worktree_root: &Path, entry: BoardEntry) -> Result<CoordinationSnapshot> {
    append_event(worktree_root, &CoordinationEvent::MessageAppended { entry })
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
    Ok(snapshot)
}

pub fn rebuild_snapshot_from_events(event_path: &Path) -> Result<CoordinationSnapshot> {
    if !event_path.exists() {
        File::create(event_path)?;
    }
    let file = OpenOptions::new().read(true).open(event_path)?;
    let reader = BufReader::new(file);

    let mut board_entries = Vec::new();
    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let event: CoordinationEvent = serde_json::from_str(trimmed).map_err(json_error)?;
        match event {
            CoordinationEvent::MessageAppended { entry } => board_entries.push(entry),
            CoordinationEvent::AgentCardUpsert { .. } => {}
        }
    }

    board_entries.sort_by_key(|entry| entry.created_at);

    let now = Utc::now();

    Ok(CoordinationSnapshot {
        board: BoardProjection {
            entries: board_entries,
            updated_at: now,
        },
        cards: AgentCardsProjection::default(),
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

fn legacy_coordination_dir(worktree_root: &Path) -> PathBuf {
    worktree_root.join(COORDINATION_RELATIVE_DIR)
}

fn coordination_migration_marker_path(project_dir: &Path) -> PathBuf {
    project_dir.join(MIGRATION_MARKER_FILE_NAME)
}

fn discover_legacy_coordination_dirs(worktree_root: &Path) -> Vec<PathBuf> {
    let output = std::process::Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(worktree_root)
        .output();
    let mut dirs = match output {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            stdout
                .lines()
                .filter_map(|line| line.strip_prefix("worktree "))
                .map(PathBuf::from)
                .map(|path| legacy_coordination_dir(&path))
                .collect::<Vec<_>>()
        }
        _ => vec![legacy_coordination_dir(worktree_root)],
    };
    dirs.sort();
    dirs.dedup();
    dirs
}

fn migrate_legacy_coordination_dirs(project_dir: &Path, legacy_dirs: &[PathBuf]) -> Result<()> {
    if coordination_migration_marker_path(project_dir).exists() {
        return Ok(());
    }

    std::fs::create_dir_all(project_dir)?;
    let event_path = project_dir.join(EVENTS_FILE_NAME);
    let mut events = if event_path.exists() {
        load_events_from_path(&event_path)?
    } else {
        Vec::new()
    };
    let mut consumed_dirs = Vec::new();

    for legacy_dir in legacy_dirs {
        if legacy_dir == project_dir || !legacy_dir.exists() {
            continue;
        }
        let legacy_event_path = legacy_dir.join(EVENTS_FILE_NAME);
        if !legacy_event_path.exists() {
            continue;
        }
        events.extend(load_events_from_path(&legacy_event_path)?);
        consumed_dirs.push(legacy_dir.clone());
    }

    if !events.is_empty() {
        events.sort_by_key(coordination_event_timestamp);
        write_events_to_path(&event_path, &events)?;
        let snapshot = rebuild_snapshot_from_events(&event_path)?;
        write_atomic_json(
            &project_dir.join(BOARD_PROJECTION_FILE_NAME),
            &snapshot.board,
        )?;
    }

    for legacy_dir in consumed_dirs {
        std::fs::remove_dir_all(legacy_dir)?;
    }
    std::fs::write(coordination_migration_marker_path(project_dir), b"complete")?;
    Ok(())
}

fn load_events_from_path(path: &Path) -> Result<Vec<CoordinationEvent>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = OpenOptions::new().read(true).open(path)?;
    let reader = BufReader::new(file);
    let mut events = Vec::new();
    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        events.push(serde_json::from_str(trimmed).map_err(json_error)?);
    }
    Ok(events)
}

fn write_events_to_path(path: &Path, events: &[CoordinationEvent]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| GwtError::Other(format!("path has no parent: {}", path.display())))?;
    std::fs::create_dir_all(parent)?;
    let tmp_path = parent.join(format!(
        ".{}.tmp-{}-{}",
        path.file_name()
            .and_then(|value| value.to_str())
            .unwrap_or(EVENTS_FILE_NAME),
        std::process::id(),
        Uuid::new_v4()
    ));
    {
        let mut file = File::create(&tmp_path)?;
        for event in events {
            serde_json::to_writer(&mut file, event).map_err(json_error)?;
            file.write_all(b"\n")?;
        }
        file.sync_all()?;
    }
    if cfg!(windows) && path.exists() {
        std::fs::remove_file(path)?;
    }
    std::fs::rename(&tmp_path, path)?;
    Ok(())
}

fn coordination_event_timestamp(event: &CoordinationEvent) -> DateTime<Utc> {
    match event {
        CoordinationEvent::MessageAppended { entry } => entry.created_at,
        CoordinationEvent::AgentCardUpsert { card } => card.updated_at,
    }
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
    use std::{sync::Arc, thread};

    use chrono::TimeZone;

    use super::*;

    #[test]
    fn load_snapshot_bootstraps_empty_files() {
        let dir = tempfile::tempdir().unwrap();

        let snapshot = load_snapshot(dir.path()).unwrap();

        assert!(snapshot.board.entries.is_empty());
        assert!(coordination_events_path(dir.path()).exists());
        assert!(coordination_board_projection_path(dir.path()).exists());
        assert!(!coordination_dir(dir.path())
            .join("cards.latest.json")
            .exists());
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
        assert!(raw.contains("\"type\":\"message_appended\""));
    }

    #[test]
    fn rebuild_snapshot_reconstructs_message_order_without_cards() {
        let dir = tempfile::tempdir().unwrap();

        append_event(
            dir.path(),
            &CoordinationEvent::MessageAppended {
                entry: BoardEntry::new(
                    AuthorKind::User,
                    "user",
                    BoardEntryKind::Request,
                    "Initial request",
                    None,
                    None,
                    vec![],
                    vec![],
                ),
            },
        )
        .unwrap();
        append_event(
            dir.path(),
            &CoordinationEvent::MessageAppended {
                entry: BoardEntry::new(
                    AuthorKind::Agent,
                    "Codex",
                    BoardEntryKind::Status,
                    "Investigating",
                    Some("running".into()),
                    None,
                    vec![],
                    vec![],
                ),
            },
        )
        .unwrap();

        let rebuilt = rebuild_snapshot_from_events(&coordination_events_path(dir.path())).unwrap();

        assert_eq!(rebuilt.board.entries.len(), 2);
        assert_eq!(rebuilt.board.entries[0].body, "Initial request");
        assert_eq!(rebuilt.board.entries[1].body, "Investigating");
        assert!(rebuilt.cards.cards.is_empty());
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
                "events.jsonl".to_string(),
            ]
        );
    }

    #[test]
    fn migrate_legacy_coordination_dirs_merges_events_and_deletes_sources() {
        let dir = tempfile::tempdir().unwrap();
        let project_dir = dir.path().join("home/.gwt/coordination/repo-hash");
        let legacy_one = dir.path().join("repo/.gwt/coordination");
        let legacy_two = dir.path().join("wt/.gwt/coordination");

        std::fs::create_dir_all(&legacy_one).unwrap();
        std::fs::create_dir_all(&legacy_two).unwrap();

        let mut first = BoardEntry::new(
            AuthorKind::User,
            "user",
            BoardEntryKind::Request,
            "first",
            None,
            None,
            vec![],
            vec![],
        );
        first.created_at = Utc.with_ymd_and_hms(2026, 4, 14, 0, 0, 0).unwrap();
        first.updated_at = first.created_at;

        let mut second = BoardEntry::new(
            AuthorKind::Agent,
            "codex",
            BoardEntryKind::Status,
            "second",
            Some("running".into()),
            None,
            vec![],
            vec![],
        );
        second.created_at = Utc.with_ymd_and_hms(2026, 4, 14, 0, 1, 0).unwrap();
        second.updated_at = second.created_at;

        write_events(
            legacy_one.join(EVENTS_FILE_NAME).as_path(),
            &[CoordinationEvent::MessageAppended {
                entry: second.clone(),
            }],
        );
        write_events(
            legacy_two.join(EVENTS_FILE_NAME).as_path(),
            &[CoordinationEvent::MessageAppended {
                entry: first.clone(),
            }],
        );

        migrate_legacy_coordination_dirs(&project_dir, &[legacy_one.clone(), legacy_two.clone()])
            .unwrap();

        let snapshot = rebuild_snapshot_from_events(&project_dir.join(EVENTS_FILE_NAME)).unwrap();
        assert_eq!(
            snapshot
                .board
                .entries
                .iter()
                .map(|entry| entry.body.as_str())
                .collect::<Vec<_>>(),
            vec!["first", "second"]
        );
        assert!(!legacy_one.exists());
        assert!(!legacy_two.exists());
        assert!(coordination_migration_marker_path(&project_dir).exists());
    }

    #[test]
    fn migrate_legacy_coordination_dirs_normalizes_legacy_board_post_events() {
        let dir = tempfile::tempdir().unwrap();
        let project_dir = dir.path().join("home/.gwt/coordination/repo-hash");
        let legacy_dir = dir.path().join("repo/.gwt/coordination");

        std::fs::create_dir_all(&legacy_dir).unwrap();

        let mut entry = BoardEntry::new(
            AuthorKind::Agent,
            "codex",
            BoardEntryKind::Status,
            "legacy board post",
            Some("waiting_input".into()),
            None,
            vec![],
            vec![],
        );
        entry.created_at = Utc.with_ymd_and_hms(2026, 4, 14, 1, 0, 0).unwrap();
        entry.updated_at = entry.created_at;

        write_legacy_board_post(&legacy_dir.join(EVENTS_FILE_NAME), &entry);

        migrate_legacy_coordination_dirs(&project_dir, std::slice::from_ref(&legacy_dir)).unwrap();

        let raw = std::fs::read_to_string(project_dir.join(EVENTS_FILE_NAME)).unwrap();
        assert!(raw.contains("\"type\":\"message_appended\""));
        assert!(!raw.contains("\"type\":\"board_post\""));

        let snapshot = rebuild_snapshot_from_events(&project_dir.join(EVENTS_FILE_NAME)).unwrap();
        assert_eq!(snapshot.board.entries.len(), 1);
        assert_eq!(snapshot.board.entries[0].body, "legacy board post");
        assert!(!legacy_dir.exists());
        assert!(coordination_migration_marker_path(&project_dir).exists());
    }

    fn write_events(path: &std::path::Path, events: &[CoordinationEvent]) {
        let parent = path.parent().unwrap();
        std::fs::create_dir_all(parent).unwrap();
        let mut file = std::fs::File::create(path).unwrap();
        for event in events {
            serde_json::to_writer(&mut file, event).unwrap();
            file.write_all(b"\n").unwrap();
        }
    }

    fn write_legacy_board_post(path: &std::path::Path, entry: &BoardEntry) {
        let parent = path.parent().unwrap();
        std::fs::create_dir_all(parent).unwrap();
        let mut file = std::fs::File::create(path).unwrap();
        serde_json::to_writer(
            &mut file,
            &serde_json::json!({
                "type": "board_post",
                "entry": entry,
            }),
        )
        .unwrap();
        file.write_all(b"\n").unwrap();
    }
}
