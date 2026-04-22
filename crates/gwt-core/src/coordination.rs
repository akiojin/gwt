//! Repo-local coordination storage for a shared board chat timeline.

use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{paths::gwt_project_dir_for_repo_path, GwtError, Result};

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
    #[serde(default)]
    pub origin_branch: Option<String>,
    #[serde(default)]
    pub origin_session_id: Option<String>,
    #[serde(default)]
    pub origin_agent_id: Option<String>,
}

impl BoardEntry {
    pub fn with_origin_branch(mut self, value: impl Into<String>) -> Self {
        self.origin_branch = Some(value.into());
        self
    }

    pub fn with_origin_session_id(mut self, value: impl Into<String>) -> Self {
        self.origin_session_id = Some(value.into());
        self
    }

    pub fn with_origin_agent_id(mut self, value: impl Into<String>) -> Self {
        self.origin_agent_id = Some(value.into());
        self
    }

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
            origin_branch: None,
            origin_session_id: None,
            origin_agent_id: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CoordinationEvent {
    #[serde(alias = "board_post")]
    MessageAppended { entry: BoardEntry },
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CoordinationSnapshot {
    pub board: BoardProjection,
}

pub fn coordination_dir(worktree_root: &Path) -> PathBuf {
    coordination_project_dir(worktree_root)
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
    if let Some(project_dir) = coordination_project_dir(worktree_root) {
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
    })
}

pub fn post_entry(worktree_root: &Path, entry: BoardEntry) -> Result<CoordinationSnapshot> {
    append_event(worktree_root, &CoordinationEvent::MessageAppended { entry })
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
        // Parse as a generic Value first so legacy event types (e.g. the
        // retired `agent_card_upsert` from the pre-shared-chat era) can be
        // skipped without failing the whole rebuild.
        let value: serde_json::Value = serde_json::from_str(trimmed).map_err(json_error)?;
        let event_type = value
            .get("type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        match event_type {
            "message_appended" | "board_post" => {
                let event: CoordinationEvent = serde_json::from_value(value).map_err(json_error)?;
                let CoordinationEvent::MessageAppended { entry } = event;
                board_entries.push(entry);
            }
            _ => continue,
        }
    }

    board_entries.sort_by_key(|entry| entry.created_at);

    let now = Utc::now();

    Ok(CoordinationSnapshot {
        board: BoardProjection {
            entries: board_entries,
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

fn legacy_coordination_dir(worktree_root: &Path) -> PathBuf {
    worktree_root.join(COORDINATION_RELATIVE_DIR)
}

fn coordination_project_dir(worktree_root: &Path) -> Option<PathBuf> {
    let repo_root = coordination_repo_root(worktree_root)?;
    Some(gwt_project_dir_for_repo_path(&repo_root).join("coordination"))
}

fn coordination_repo_root(worktree_root: &Path) -> Option<PathBuf> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(worktree_root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let repo_root = stdout
        .lines()
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    Some(dunce::canonicalize(repo_root).unwrap_or_else(|_| PathBuf::from(repo_root)))
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
    replace_path_with_temp(path, &tmp_path)
}

fn coordination_event_timestamp(event: &CoordinationEvent) -> DateTime<Utc> {
    match event {
        CoordinationEvent::MessageAppended { entry } => entry.created_at,
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
    replace_path_with_temp(path, &tmp_path)
}

fn replace_path_with_temp(path: &Path, tmp_path: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        const MAX_RETRIES: usize = 20;
        const SLEEP_MS: u64 = 25;

        for attempt in 0..MAX_RETRIES {
            match try_replace_path_with_temp(path, tmp_path) {
                Ok(()) => return Ok(()),
                Err(err)
                    if err.kind() == std::io::ErrorKind::PermissionDenied
                        && attempt + 1 < MAX_RETRIES =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(SLEEP_MS));
                }
                Err(err) => return Err(err.into()),
            }
        }

        unreachable!("Windows retry loop should always return or error");
    }

    #[cfg(not(windows))]
    {
        try_replace_path_with_temp(path, tmp_path)?;
        Ok(())
    }
}

fn try_replace_path_with_temp(path: &Path, tmp_path: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    if path.exists() {
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
    }

    std::fs::rename(tmp_path, path)
}

fn json_error(err: serde_json::Error) -> GwtError {
    GwtError::Other(err.to_string())
}

// --- Phase 8.1: diff-injection / reminders sidecar APIs ---

/// Per-agent-session reminder state persisted at
/// `~/.gwt/projects/<hash>/coordination/reminders/<agent-session-id>.json`.
///
/// Owned by the `board-reminder` hook; not part of the shared Board projection.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemindersState {
    #[serde(default)]
    pub last_injected_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub last_reminded_kind: HashMap<String, DateTime<Utc>>,
}

/// Directory that stores per-agent-session reminder sidecar files.
pub fn reminders_dir(worktree_root: &Path) -> PathBuf {
    coordination_dir(worktree_root).join("reminders")
}

fn reminders_path(worktree_root: &Path, agent_session_id: &str) -> PathBuf {
    reminders_dir(worktree_root).join(format!("{agent_session_id}.json"))
}

/// Load reminder state for the given agent session. Returns a default state
/// when no sidecar file exists yet.
pub fn load_reminders_state(
    worktree_root: &Path,
    agent_session_id: &str,
) -> Result<RemindersState> {
    load_json_or_default(&reminders_path(worktree_root, agent_session_id))
}

/// Atomically persist reminder state for the given agent session.
pub fn write_reminders_state(
    worktree_root: &Path,
    agent_session_id: &str,
    state: &RemindersState,
) -> Result<()> {
    let path = reminders_path(worktree_root, agent_session_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    write_atomic_json(&path, state)
}

/// Return Board entries whose `updated_at` is strictly later than `since`,
/// sorted chronologically (same ordering as the projection).
pub fn load_entries_since(worktree_root: &Path, since: DateTime<Utc>) -> Result<Vec<BoardEntry>> {
    let snapshot = load_snapshot(worktree_root)?;
    Ok(snapshot
        .board
        .entries
        .into_iter()
        .filter(|entry| entry.updated_at > since)
        .collect())
}

/// Check whether `author` has posted a message of the given `kind` within the
/// trailing `within` duration. Used by `board-reminder` for redundancy
/// suppression.
pub fn has_recent_post_by(
    worktree_root: &Path,
    author: &str,
    kind: &BoardEntryKind,
    within: chrono::Duration,
) -> Result<bool> {
    let snapshot = load_snapshot(worktree_root)?;
    let threshold = Utc::now() - within;
    Ok(snapshot
        .board
        .entries
        .iter()
        .any(|entry| entry.author == author && entry.kind == *kind && entry.updated_at > threshold))
}

#[cfg(test)]
mod tests {
    use std::{str::FromStr, sync::Arc, thread};

    use chrono::TimeZone;

    use super::*;
    use crate::paths::gwt_project_dir_for_repo_path;
    use crate::test_support::{env_lock, ScopedEnvVar};

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
    fn rebuild_skips_legacy_agent_card_upsert_events() {
        use std::io::Write;

        let dir = tempfile::tempdir().unwrap();
        ensure_repo_local_files(dir.path()).unwrap();

        // Simulate an events.jsonl written by the pre-shared-chat code path.
        // The legacy `agent_card_upsert` line must be tolerated and simply
        // skipped — not treated as a parse error.
        let events_path = coordination_events_path(dir.path());
        {
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .create(true)
                .open(&events_path)
                .unwrap();
            let entry = BoardEntry::new(
                AuthorKind::User,
                "user",
                BoardEntryKind::Request,
                "after legacy line",
                None,
                None,
                vec![],
                vec![],
            );
            let legacy = serde_json::json!({
                "type": "agent_card_upsert",
                "card": {
                    "agent_id": "codex",
                    "branch": "feature/legacy",
                    "updated_at": "2026-04-14T00:00:00Z"
                }
            });
            writeln!(file, "{}", legacy).unwrap();
            writeln!(
                file,
                "{}",
                serde_json::json!({
                    "type": "message_appended",
                    "entry": entry,
                })
            )
            .unwrap();
        }

        let rebuilt = rebuild_snapshot_from_events(&events_path).unwrap();
        assert_eq!(rebuilt.board.entries.len(), 1);
        assert_eq!(rebuilt.board.entries[0].body, "after legacy line");
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
    fn git_repo_without_origin_uses_project_scoped_coordination_dir() {
        let _guard = env_lock().lock().unwrap_or_else(|p| p.into_inner());
        let home = tempfile::tempdir().unwrap();
        let _home_guard = ScopedEnvVar::set("HOME", home.path());
        let _userprofile_guard = ScopedEnvVar::set("USERPROFILE", home.path());

        let repo = home.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let output = std::process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&repo)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git init failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let snapshot = load_snapshot(&repo).unwrap();
        assert!(snapshot.board.entries.is_empty());

        let project_dir = gwt_project_dir_for_repo_path(&repo).join("coordination");
        assert_eq!(coordination_dir(&repo), project_dir);
        assert!(project_dir.join(EVENTS_FILE_NAME).exists());
        assert!(project_dir.join(BOARD_PROJECTION_FILE_NAME).exists());
        assert!(!legacy_coordination_dir(&repo).exists());
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
        let project_dir = dir.path().join("home/.gwt/projects/repo-hash/coordination");
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
        let project_dir = dir.path().join("home/.gwt/projects/repo-hash/coordination");
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

    #[test]
    fn load_entries_since_returns_only_newer_entries() {
        let dir = tempfile::tempdir().unwrap();

        let mut old_entry = BoardEntry::new(
            AuthorKind::Agent,
            "Codex",
            BoardEntryKind::Status,
            "old post",
            None,
            None,
            vec![],
            vec![],
        );
        old_entry.created_at = chrono::Utc.with_ymd_and_hms(2026, 4, 14, 0, 0, 0).unwrap();
        old_entry.updated_at = old_entry.created_at;
        post_entry(dir.path(), old_entry).unwrap();

        let mut new_entry = BoardEntry::new(
            AuthorKind::Agent,
            "Claude",
            BoardEntryKind::Status,
            "new post",
            None,
            None,
            vec![],
            vec![],
        );
        new_entry.created_at = chrono::Utc.with_ymd_and_hms(2026, 4, 20, 0, 0, 0).unwrap();
        new_entry.updated_at = new_entry.created_at;
        post_entry(dir.path(), new_entry).unwrap();

        let since = chrono::Utc.with_ymd_and_hms(2026, 4, 15, 0, 0, 0).unwrap();
        let entries = load_entries_since(dir.path(), since).unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].body, "new post");
    }

    #[test]
    fn has_recent_post_by_respects_within_window() {
        let dir = tempfile::tempdir().unwrap();

        let now = chrono::Utc::now();
        let recent = now - chrono::Duration::minutes(5);
        let mut entry = BoardEntry::new(
            AuthorKind::Agent,
            "Codex",
            BoardEntryKind::Status,
            "recent status",
            None,
            None,
            vec![],
            vec![],
        );
        entry.created_at = recent;
        entry.updated_at = recent;
        post_entry(dir.path(), entry).unwrap();

        assert!(has_recent_post_by(
            dir.path(),
            "Codex",
            &BoardEntryKind::Status,
            chrono::Duration::minutes(10)
        )
        .unwrap());

        assert!(!has_recent_post_by(
            dir.path(),
            "Codex",
            &BoardEntryKind::Status,
            chrono::Duration::minutes(1)
        )
        .unwrap());

        assert!(!has_recent_post_by(
            dir.path(),
            "Claude",
            &BoardEntryKind::Status,
            chrono::Duration::minutes(10)
        )
        .unwrap());

        assert!(!has_recent_post_by(
            dir.path(),
            "Codex",
            &BoardEntryKind::Decision,
            chrono::Duration::minutes(10)
        )
        .unwrap());
    }

    #[test]
    fn reminders_sidecar_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let agent_session_id = "sess-test-123";

        let empty = load_reminders_state(dir.path(), agent_session_id).unwrap();
        assert!(empty.last_injected_at.is_none());
        assert!(empty.last_reminded_kind.is_empty());

        let now = chrono::Utc::now();
        let state = RemindersState {
            last_injected_at: Some(now),
            last_reminded_kind: HashMap::from([("status".to_string(), now)]),
        };
        write_reminders_state(dir.path(), agent_session_id, &state).unwrap();

        let loaded = load_reminders_state(dir.path(), agent_session_id).unwrap();
        assert_eq!(loaded.last_injected_at, Some(now));
        assert_eq!(loaded.last_reminded_kind.get("status").copied(), Some(now));

        let sidecar_path = reminders_dir(dir.path()).join("sess-test-123.json");
        assert!(sidecar_path.exists());
    }

    #[test]
    fn board_entry_has_origin_metadata() {
        let entry = BoardEntry::new(
            AuthorKind::Agent,
            "Codex",
            BoardEntryKind::Status,
            "started work",
            None,
            None,
            vec![],
            vec![],
        )
        .with_origin_branch("feature/update-board")
        .with_origin_session_id("sess-a3f2")
        .with_origin_agent_id("agent-codex-001");

        assert_eq!(entry.origin_branch.as_deref(), Some("feature/update-board"));
        assert_eq!(entry.origin_session_id.as_deref(), Some("sess-a3f2"));
        assert_eq!(entry.origin_agent_id.as_deref(), Some("agent-codex-001"));
    }

    #[test]
    fn board_entry_deserializes_legacy_without_origin_fields() {
        let legacy_json = r#"{
            "id": "00000000-0000-0000-0000-000000000001",
            "author_kind": "agent",
            "author": "Codex",
            "kind": "status",
            "body": "legacy entry",
            "created_at": "2026-04-14T00:00:00Z",
            "updated_at": "2026-04-14T00:00:00Z"
        }"#;

        let entry: BoardEntry = serde_json::from_str(legacy_json).unwrap();
        assert_eq!(entry.origin_branch, None);
        assert_eq!(entry.origin_session_id, None);
        assert_eq!(entry.origin_agent_id, None);
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

    #[test]
    fn board_entry_kind_round_trip() {
        for (kind, value) in [
            (BoardEntryKind::Request, "request"),
            (BoardEntryKind::Status, "status"),
            (BoardEntryKind::Next, "next"),
            (BoardEntryKind::Claim, "claim"),
            (BoardEntryKind::Impact, "impact"),
            (BoardEntryKind::Question, "question"),
            (BoardEntryKind::Blocked, "blocked"),
            (BoardEntryKind::Handoff, "handoff"),
            (BoardEntryKind::Decision, "decision"),
        ] {
            assert_eq!(BoardEntryKind::from_str(value).unwrap(), kind);
            assert_eq!(kind.as_str(), value);
        }
        assert!(BoardEntryKind::from_str("mystery").is_err());
    }
}
