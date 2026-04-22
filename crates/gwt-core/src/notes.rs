use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{paths::gwt_notes_state_path_for_repo_path, GwtError, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoNote {
    pub id: String,
    pub title: String,
    pub body: String,
    pub pinned: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoNoteDraft {
    pub title: String,
    pub body: String,
    pub pinned: bool,
}

impl MemoNoteDraft {
    pub fn new(title: impl Into<String>, body: impl Into<String>, pinned: bool) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            pinned,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoSnapshot {
    #[serde(default)]
    pub notes: Vec<MemoNote>,
}

pub fn load_snapshot(repo_path: &Path) -> Result<MemoSnapshot> {
    load_snapshot_from_path(&notes_path(repo_path))
}

pub fn create_note(repo_path: &Path, draft: MemoNoteDraft) -> Result<MemoNote> {
    with_notes_store_lock(repo_path, || {
        let now = Utc::now();
        let note = MemoNote {
            id: Uuid::new_v4().to_string(),
            title: draft.title,
            body: draft.body,
            pinned: draft.pinned,
            created_at: now,
            updated_at: now,
        };
        let mut snapshot = load_snapshot(repo_path)?;
        snapshot.notes.push(note.clone());
        sort_notes(&mut snapshot.notes);
        write_atomic_json(&notes_path(repo_path), &snapshot)?;
        Ok(note)
    })
}

pub fn update_note(repo_path: &Path, note_id: &str, draft: MemoNoteDraft) -> Result<MemoNote> {
    with_notes_store_lock(repo_path, || {
        let mut snapshot = load_snapshot(repo_path)?;
        let note = snapshot
            .notes
            .iter_mut()
            .find(|note| note.id == note_id)
            .ok_or_else(|| GwtError::Other(format!("Memo note not found: {note_id}")))?;
        note.title = draft.title;
        note.body = draft.body;
        note.pinned = draft.pinned;
        note.updated_at = Utc::now();
        let updated = note.clone();
        sort_notes(&mut snapshot.notes);
        write_atomic_json(&notes_path(repo_path), &snapshot)?;
        Ok(updated)
    })
}

pub fn delete_note(repo_path: &Path, note_id: &str) -> Result<()> {
    with_notes_store_lock(repo_path, || {
        let mut snapshot = load_snapshot(repo_path)?;
        let original_len = snapshot.notes.len();
        snapshot.notes.retain(|note| note.id != note_id);
        if snapshot.notes.len() == original_len {
            return Err(GwtError::Other(format!("Memo note not found: {note_id}")));
        }
        sort_notes(&mut snapshot.notes);
        write_atomic_json(&notes_path(repo_path), &snapshot)?;
        Ok(())
    })
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
struct LegacyMemoSnapshot {
    #[serde(default)]
    notes: Vec<LegacyMemoNote>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct LegacyMemoNote {
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    pinned: bool,
    #[serde(default)]
    created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    updated_at: Option<DateTime<Utc>>,
}

fn json_error(err: serde_json::Error) -> GwtError {
    GwtError::Other(err.to_string())
}

fn notes_path(repo_path: &Path) -> PathBuf {
    gwt_notes_state_path_for_repo_path(repo_path)
}

fn notes_lock_path(repo_path: &Path) -> PathBuf {
    let path = notes_path(repo_path);
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("notes.json");
    path.with_file_name(format!("{file_name}.lock"))
}

fn open_notes_store_lock(repo_path: &Path) -> Result<File> {
    let lock_path = notes_lock_path(repo_path);
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(lock_path)
        .map_err(Into::into)
}

fn with_notes_store_lock<T>(repo_path: &Path, action: impl FnOnce() -> Result<T>) -> Result<T> {
    let lock = open_notes_store_lock(repo_path)?;
    lock.lock_exclusive()?;
    let result = action();
    let unlock_result = lock.unlock();
    match (result, unlock_result) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error.into()),
    }
}

fn load_snapshot_from_path(path: &Path) -> Result<MemoSnapshot> {
    if !path.exists() {
        return Ok(MemoSnapshot::default());
    }

    let raw = std::fs::read_to_string(path)?;
    if raw.trim().is_empty() {
        return Ok(MemoSnapshot::default());
    }

    parse_snapshot(&raw)
}

fn parse_snapshot(raw: &str) -> Result<MemoSnapshot> {
    match serde_json::from_str::<MemoSnapshot>(raw) {
        Ok(mut snapshot) => {
            sort_notes(&mut snapshot.notes);
            Ok(snapshot)
        }
        Err(current_error) => {
            if let Ok(legacy_notes) = serde_json::from_str::<Vec<LegacyMemoNote>>(raw) {
                return Ok(convert_legacy_notes(legacy_notes));
            }
            if let Ok(legacy) = serde_json::from_str::<LegacyMemoSnapshot>(raw) {
                return Ok(convert_legacy_notes(legacy.notes));
            }
            Err(json_error(current_error))
        }
    }
}

fn convert_legacy_notes(legacy_notes: Vec<LegacyMemoNote>) -> MemoSnapshot {
    let fallback_now = Utc::now();
    let mut notes = legacy_notes
        .into_iter()
        .map(|note| {
            let updated_at = note.updated_at.or(note.created_at).unwrap_or(fallback_now);
            let created_at = note.created_at.unwrap_or(updated_at);
            MemoNote {
                id: note.id,
                title: note.title,
                body: note.body,
                pinned: note.pinned,
                created_at,
                updated_at,
            }
        })
        .collect::<Vec<_>>();
    sort_notes(&mut notes);
    MemoSnapshot { notes }
}

fn sort_notes(notes: &mut [MemoNote]) {
    notes.sort_by(|left, right| {
        right
            .pinned
            .cmp(&left.pinned)
            .then_with(|| right.updated_at.cmp(&left.updated_at))
            .then_with(|| left.id.cmp(&right.id))
    });
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
            .unwrap_or("notes"),
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

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, sync::mpsc, time::Duration};

    use chrono::TimeZone;
    use fs2::FileExt;

    use super::*;
    use crate::test_support::{env_lock, ScopedEnvVar};

    #[derive(Debug, Clone, Serialize)]
    struct LegacyMemoNote {
        id: String,
        title: String,
        body: String,
        pinned: bool,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    }

    fn note_path_for(repo_root: &Path) -> PathBuf {
        notes_path(repo_root)
    }

    fn init_git_repo(path: &Path) {
        let output = std::process::Command::new("git")
            .args(["init", path.to_str().unwrap()])
            .output()
            .expect("git init");
        assert!(output.status.success(), "git init failed");

        let email = std::process::Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(path)
            .output()
            .expect("git config user.email");
        assert!(email.status.success(), "git config user.email failed");

        let name = std::process::Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(path)
            .output()
            .expect("git config user.name");
        assert!(name.status.success(), "git config user.name failed");
    }

    fn add_origin(path: &Path, url: &str) {
        let output = std::process::Command::new("git")
            .args(["remote", "add", "origin", url])
            .current_dir(path)
            .output()
            .expect("git remote add origin");
        assert!(
            output.status.success(),
            "git remote add origin failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn commit_file(path: &Path, name: &str, body: &str) {
        std::fs::write(path.join(name), body).unwrap();
        let add = std::process::Command::new("git")
            .args(["add", name])
            .current_dir(path)
            .output()
            .expect("git add");
        assert!(add.status.success(), "git add failed");

        let commit = std::process::Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(path)
            .output()
            .expect("git commit");
        assert!(
            commit.status.success(),
            "git commit failed: {}",
            String::from_utf8_lossy(&commit.stderr)
        );
    }

    #[test]
    fn load_snapshot_returns_empty_when_notes_file_is_missing() {
        let _guard = env_lock().lock().unwrap_or_else(|p| p.into_inner());
        let home = tempfile::tempdir().unwrap();
        let _home_guard = ScopedEnvVar::set("HOME", home.path());
        let _userprofile_guard = ScopedEnvVar::set("USERPROFILE", home.path());
        let repo = tempfile::tempdir().unwrap();

        let snapshot = load_snapshot(repo.path()).unwrap();

        assert!(snapshot.notes.is_empty());
        assert!(!note_path_for(repo.path()).exists());
    }

    #[test]
    fn load_snapshot_treats_zero_byte_file_as_empty() {
        let _guard = env_lock().lock().unwrap_or_else(|p| p.into_inner());
        let home = tempfile::tempdir().unwrap();
        let _home_guard = ScopedEnvVar::set("HOME", home.path());
        let _userprofile_guard = ScopedEnvVar::set("USERPROFILE", home.path());
        let repo = tempfile::tempdir().unwrap();
        let path = note_path_for(repo.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"").unwrap();

        let snapshot = load_snapshot(repo.path()).unwrap();

        assert!(snapshot.notes.is_empty());
    }

    #[test]
    fn load_snapshot_supports_legacy_top_level_array_schema() {
        let _guard = env_lock().lock().unwrap_or_else(|p| p.into_inner());
        let home = tempfile::tempdir().unwrap();
        let _home_guard = ScopedEnvVar::set("HOME", home.path());
        let _userprofile_guard = ScopedEnvVar::set("USERPROFILE", home.path());
        let repo = tempfile::tempdir().unwrap();
        let legacy_notes = vec![LegacyMemoNote {
            id: "legacy-1".to_string(),
            title: "Legacy".to_string(),
            body: "Imported".to_string(),
            pinned: true,
            created_at: Utc.with_ymd_and_hms(2026, 4, 20, 0, 0, 0).unwrap(),
            updated_at: Utc.with_ymd_and_hms(2026, 4, 21, 0, 0, 0).unwrap(),
        }];
        let path = note_path_for(repo.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, serde_json::to_vec_pretty(&legacy_notes).unwrap()).unwrap();

        let snapshot = load_snapshot(repo.path()).unwrap();

        assert_eq!(snapshot.notes.len(), 1);
        assert_eq!(snapshot.notes[0].id, "legacy-1");
        assert!(snapshot.notes[0].pinned);
    }

    #[test]
    fn load_snapshot_errors_on_invalid_json() {
        let _guard = env_lock().lock().unwrap_or_else(|p| p.into_inner());
        let home = tempfile::tempdir().unwrap();
        let _home_guard = ScopedEnvVar::set("HOME", home.path());
        let _userprofile_guard = ScopedEnvVar::set("USERPROFILE", home.path());
        let repo = tempfile::tempdir().unwrap();
        let path = note_path_for(repo.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{").unwrap();

        let error = load_snapshot(repo.path()).expect_err("invalid json should fail");

        assert!(error.to_string().contains("EOF") || error.to_string().contains("expected"));
    }

    #[test]
    fn create_update_delete_note_round_trips_through_repo_scoped_store() {
        let _guard = env_lock().lock().unwrap_or_else(|p| p.into_inner());
        let home = tempfile::tempdir().unwrap();
        let _home_guard = ScopedEnvVar::set("HOME", home.path());
        let _userprofile_guard = ScopedEnvVar::set("USERPROFILE", home.path());
        let repo = tempfile::tempdir().unwrap();

        let created = create_note(
            repo.path(),
            MemoNoteDraft::new("Draft", "Initial body", false),
        )
        .expect("create note");
        assert!(!created.id.is_empty());
        let stored = load_snapshot(repo.path()).unwrap();
        assert_eq!(stored.notes.len(), 1);
        assert_eq!(stored.notes[0].title, "Draft");

        let updated = update_note(
            repo.path(),
            &created.id,
            MemoNoteDraft::new("Pinned note", "Updated body", true),
        )
        .expect("update note");
        assert_eq!(updated.id, created.id);
        assert!(updated.pinned);
        assert_eq!(updated.title, "Pinned note");

        delete_note(repo.path(), &created.id).expect("delete note");
        let stored = load_snapshot(repo.path()).unwrap();
        assert!(stored.notes.is_empty());
        assert!(note_path_for(repo.path()).exists());
    }

    #[test]
    fn load_snapshot_orders_pinned_first_then_updated_at_desc_then_id() {
        let _guard = env_lock().lock().unwrap_or_else(|p| p.into_inner());
        let home = tempfile::tempdir().unwrap();
        let _home_guard = ScopedEnvVar::set("HOME", home.path());
        let _userprofile_guard = ScopedEnvVar::set("USERPROFILE", home.path());
        let repo = tempfile::tempdir().unwrap();
        let snapshot = MemoSnapshot {
            notes: vec![
                MemoNote {
                    id: "z-note".to_string(),
                    title: "Older pinned".to_string(),
                    body: "".to_string(),
                    pinned: true,
                    created_at: Utc.with_ymd_and_hms(2026, 4, 20, 0, 0, 0).unwrap(),
                    updated_at: Utc.with_ymd_and_hms(2026, 4, 20, 0, 0, 0).unwrap(),
                },
                MemoNote {
                    id: "a-note".to_string(),
                    title: "Newest unpinned".to_string(),
                    body: "".to_string(),
                    pinned: false,
                    created_at: Utc.with_ymd_and_hms(2026, 4, 21, 0, 0, 0).unwrap(),
                    updated_at: Utc.with_ymd_and_hms(2026, 4, 21, 0, 0, 0).unwrap(),
                },
                MemoNote {
                    id: "b-note".to_string(),
                    title: "Newest pinned".to_string(),
                    body: "".to_string(),
                    pinned: true,
                    created_at: Utc.with_ymd_and_hms(2026, 4, 22, 0, 0, 0).unwrap(),
                    updated_at: Utc.with_ymd_and_hms(2026, 4, 22, 0, 0, 0).unwrap(),
                },
                MemoNote {
                    id: "c-note".to_string(),
                    title: "Tie breaker".to_string(),
                    body: "".to_string(),
                    pinned: false,
                    created_at: Utc.with_ymd_and_hms(2026, 4, 21, 0, 0, 0).unwrap(),
                    updated_at: Utc.with_ymd_and_hms(2026, 4, 21, 0, 0, 0).unwrap(),
                },
            ],
        };
        let path = note_path_for(repo.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        write_atomic_json(&path, &snapshot).unwrap();

        let loaded = load_snapshot(repo.path()).unwrap();

        assert_eq!(
            loaded
                .notes
                .iter()
                .map(|note| note.id.as_str())
                .collect::<Vec<_>>(),
            vec!["b-note", "z-note", "a-note", "c-note"]
        );
    }

    #[test]
    fn repo_scoped_notes_path_reuses_repo_identity_for_linked_worktrees() {
        let _guard = env_lock().lock().unwrap_or_else(|p| p.into_inner());
        let home = tempfile::tempdir().unwrap();
        let _home_guard = ScopedEnvVar::set("HOME", home.path());
        let _userprofile_guard = ScopedEnvVar::set("USERPROFILE", home.path());
        let repo = home.path().join("repo");
        let worktree = home.path().join("wt-feature");
        std::fs::create_dir_all(&repo).unwrap();
        init_git_repo(&repo);
        add_origin(&repo, "https://github.com/example/project.git");
        commit_file(&repo, "README.md", "# repo\n");

        let output = std::process::Command::new("git")
            .args([
                "worktree",
                "add",
                "-b",
                "feature/shared",
                worktree.to_str().unwrap(),
            ])
            .current_dir(&repo)
            .output()
            .expect("git worktree add");
        assert!(
            output.status.success(),
            "git worktree add failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        assert_eq!(note_path_for(&repo), note_path_for(&worktree));
    }

    #[test]
    fn create_note_waits_for_repo_scoped_lock_release() {
        let _guard = env_lock().lock().unwrap_or_else(|p| p.into_inner());
        let home = tempfile::tempdir().unwrap();
        let _home_guard = ScopedEnvVar::set("HOME", home.path());
        let _userprofile_guard = ScopedEnvVar::set("USERPROFILE", home.path());
        let repo = tempfile::tempdir().unwrap();
        let lock = open_notes_store_lock(repo.path()).expect("open notes lock");
        lock.lock_exclusive().expect("lock notes store");

        let repo_path = repo.path().to_path_buf();
        let (tx, rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let result = create_note(
                &repo_path,
                MemoNoteDraft::new("Locked note", "blocked until unlock", false),
            );
            tx.send(result.map(|note| note.title))
                .expect("send worker result");
        });

        assert!(
            rx.recv_timeout(Duration::from_millis(150)).is_err(),
            "create_note should block while the repo-scoped lock is held"
        );

        lock.unlock().expect("unlock notes store");
        let result = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("worker should finish after unlock");
        assert_eq!(result.expect("note creation should succeed"), "Locked note");
        worker.join().expect("join worker");
    }
}
