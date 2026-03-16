//! Persistent storage for agent sessions

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Path, PathBuf};

use super::session::{AgentSession, SessionStatus};
use super::task::TaskStatus;
use super::types::SessionId;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum SessionStoreError {
    Io(std::io::Error),
    Parse(String),
    NotFound,
}

impl fmt::Display for SessionStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SessionStoreError::Io(e) => write!(f, "I/O error: {e}"),
            SessionStoreError::Parse(msg) => write!(f, "Parse error: {msg}"),
            SessionStoreError::NotFound => write!(f, "Session not found"),
        }
    }
}

impl std::error::Error for SessionStoreError {}

impl From<std::io::Error> for SessionStoreError {
    fn from(e: std::io::Error) -> Self {
        SessionStoreError::Io(e)
    }
}

// ---------------------------------------------------------------------------
// SessionSummary
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub session_id: SessionId,
    pub status: SessionStatus,
    pub updated_at: Option<DateTime<Utc>>,
}

// ---------------------------------------------------------------------------
// SessionStore
// ---------------------------------------------------------------------------

pub struct SessionStore {
    sessions_dir: PathBuf,
}

impl Default for SessionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionStore {
    /// Create a new `SessionStore` that persists under `~/.gwt/sessions/`.
    ///
    /// The directory is created with mode 0700 on Unix if it does not exist.
    pub fn new() -> Self {
        let home = dirs::home_dir().expect("failed to determine home directory");
        let sessions_dir = home.join(".gwt").join("sessions");
        Self::with_dir(sessions_dir)
    }

    /// Create a `SessionStore` backed by an arbitrary directory (useful for tests).
    pub fn with_dir(sessions_dir: PathBuf) -> Self {
        if !sessions_dir.exists() {
            std::fs::create_dir_all(&sessions_dir).expect("failed to create sessions directory");
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o700);
            let _ = std::fs::set_permissions(&sessions_dir, perms);
        }

        Self { sessions_dir }
    }

    pub fn sessions_dir(&self) -> &Path {
        &self.sessions_dir
    }

    // -- save ---------------------------------------------------------------

    /// Atomically save a session as `{session_id}.json`.
    pub fn save(&self, session: &AgentSession) -> Result<(), std::io::Error> {
        let json = serde_json::to_string_pretty(session)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        let final_path = self.session_path(&session.id);
        let tmp_path = self.sessions_dir.join(format!("{}.tmp", session.id.0));

        std::fs::write(&tmp_path, json.as_bytes())?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            let _ = std::fs::set_permissions(&tmp_path, perms);
        }

        std::fs::rename(&tmp_path, &final_path)?;
        Ok(())
    }

    // -- load ---------------------------------------------------------------

    /// Load a session by its ID. On parse failure the file is renamed to `.broken`.
    pub fn load(&self, session_id: &SessionId) -> Result<AgentSession, SessionStoreError> {
        let path = self.session_path(session_id);
        if !path.exists() {
            return Err(SessionStoreError::NotFound);
        }

        let data = std::fs::read_to_string(&path)?;
        match serde_json::from_str::<AgentSession>(&data) {
            Ok(session) => Ok(session),
            Err(e) => {
                let broken = self
                    .sessions_dir
                    .join(format!("{}.json.broken", session_id.0));
                let _ = std::fs::rename(&path, &broken);
                Err(SessionStoreError::Parse(e.to_string()))
            }
        }
    }

    // -- list_sessions ------------------------------------------------------

    /// List all sessions found in the sessions directory.
    pub fn list_sessions(&self) -> Result<Vec<SessionSummary>, std::io::Error> {
        let mut summaries = Vec::new();

        for entry in std::fs::read_dir(&self.sessions_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }

            let data = match std::fs::read_to_string(&path) {
                Ok(d) => d,
                Err(_) => continue,
            };

            if let Ok(session) = serde_json::from_str::<AgentSession>(&data) {
                summaries.push(SessionSummary {
                    session_id: session.id,
                    status: session.status,
                    updated_at: Some(session.updated_at),
                });
            }
        }

        Ok(summaries)
    }

    // -- validate_session ---------------------------------------------------

    /// Validate a session's worktree paths. Missing worktrees cause their
    /// associated tasks to be marked `Failed`. Returns a list of warnings.
    pub fn validate_session(&self, session: &mut AgentSession) -> Vec<String> {
        let mut warnings = Vec::new();

        for wt in &session.worktrees {
            if !wt.path.exists() {
                warnings.push(format!(
                    "Worktree path missing: {} (branch: {})",
                    wt.path.display(),
                    wt.branch_name
                ));

                for task_id in &wt.task_ids {
                    if let Some(task) = session.tasks.iter_mut().find(|t| t.id == *task_id) {
                        match task.status {
                            TaskStatus::Pending | TaskStatus::Ready | TaskStatus::Running => {
                                task.status = TaskStatus::Failed;
                                warnings.push(format!(
                                    "Task {} marked Failed (worktree missing)",
                                    task_id.0
                                ));
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        warnings
    }

    // -- helpers ------------------------------------------------------------

    fn session_path(&self, session_id: &SessionId) -> PathBuf {
        self.sessions_dir.join(format!("{}.json", session_id.0))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::task::Task;
    use crate::agent::types::TaskId;
    use crate::agent::worktree::WorktreeRef;

    fn make_store(dir: &Path) -> SessionStore {
        SessionStore::with_dir(dir.to_path_buf())
    }

    fn make_session() -> AgentSession {
        AgentSession::new(
            SessionId("test-session-1".to_string()),
            PathBuf::from("/repo"),
        )
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = make_store(dir.path());

        let session = make_session();
        store.save(&session).unwrap();

        let loaded = store.load(&session.id).unwrap();
        assert_eq!(loaded.id, session.id);
        assert_eq!(loaded.status, SessionStatus::Active);
    }

    #[test]
    fn test_load_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let store = make_store(dir.path());

        let result = store.load(&SessionId("nonexistent".to_string()));
        assert!(matches!(result, Err(SessionStoreError::NotFound)));
    }

    #[test]
    fn test_load_broken_json_renames_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = make_store(dir.path());

        let id = SessionId("bad-session".to_string());
        let path = dir.path().join("bad-session.json");
        std::fs::write(&path, "{ invalid json").unwrap();

        let result = store.load(&id);
        assert!(matches!(result, Err(SessionStoreError::Parse(_))));
        assert!(!path.exists());
        assert!(dir.path().join("bad-session.json.broken").exists());
    }

    #[test]
    fn test_list_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let store = make_store(dir.path());

        let s1 = AgentSession::new(SessionId("s1".to_string()), PathBuf::from("/r1"));
        let s2 = AgentSession::new(SessionId("s2".to_string()), PathBuf::from("/r2"));
        store.save(&s1).unwrap();
        store.save(&s2).unwrap();

        // Also place a non-json file to verify it is skipped
        std::fs::write(dir.path().join("notes.txt"), "ignored").unwrap();

        let list = store.list_sessions().unwrap();
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn test_validate_session_missing_worktree() {
        let dir = tempfile::tempdir().unwrap();
        let store = make_store(dir.path());

        let mut session = make_session();

        let task_id = TaskId("t1".to_string());
        let task = Task::new(task_id.clone(), "task", "desc");
        session.tasks.push(task);

        let wt = WorktreeRef::new(
            "agent/branch",
            PathBuf::from("/nonexistent/path"),
            vec![task_id],
        );
        session.worktrees.push(wt);

        let warnings = store.validate_session(&mut session);
        assert!(!warnings.is_empty());
        assert_eq!(session.tasks[0].status, TaskStatus::Failed);
    }

    #[test]
    fn test_validate_session_existing_worktree() {
        let dir = tempfile::tempdir().unwrap();
        let wt_dir = tempfile::tempdir().unwrap();
        let store = make_store(dir.path());

        let mut session = make_session();

        let task_id = TaskId("t2".to_string());
        let task = Task::new(task_id.clone(), "task", "desc");
        session.tasks.push(task);

        let wt = WorktreeRef::new("agent/ok", wt_dir.path().to_path_buf(), vec![task_id]);
        session.worktrees.push(wt);

        let warnings = store.validate_session(&mut session);
        assert!(warnings.is_empty());
        assert_eq!(session.tasks[0].status, TaskStatus::Pending);
    }

    #[test]
    fn test_save_overwrites_existing() {
        let dir = tempfile::tempdir().unwrap();
        let store = make_store(dir.path());

        let mut session = make_session();
        store.save(&session).unwrap();

        session.status = SessionStatus::Completed;
        store.save(&session).unwrap();

        let loaded = store.load(&session.id).unwrap();
        assert_eq!(loaded.status, SessionStatus::Completed);
    }

    #[cfg(unix)]
    #[test]
    fn test_sessions_dir_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let sessions_path = dir.path().join("secure");
        let _store = SessionStore::with_dir(sessions_path.clone());

        use std::os::unix::fs::PermissionsExt;
        let meta = std::fs::metadata(&sessions_path).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o700);
    }

    #[cfg(unix)]
    #[test]
    fn test_saved_file_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let store = make_store(dir.path());

        let session = make_session();
        store.save(&session).unwrap();

        use std::os::unix::fs::PermissionsExt;
        let path = dir.path().join("test-session-1.json");
        let meta = std::fs::metadata(&path).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    }
}
