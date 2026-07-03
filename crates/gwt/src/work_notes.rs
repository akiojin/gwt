//! Machine-local work-notes scratch IO (SPEC-3214 FR-007/FR-008).
//!
//! Project memory (`memory.md`) and the discussion log (`discussions.md`)
//! live branch-independently under `~/.gwt/projects/<repo-hash>/work-notes/`
//! — the same placement convention as Board / Work state — and are shared by
//! every worktree of a repository. Durable team-shared outcomes belong to
//! GitHub (Issue / `gwt-spec` Issue); these files are machine-local scratch.
//!
//! Writers always target the home file and serialize through
//! [`with_work_notes_lock`] because parallel intake sessions share one home
//! file per repository. Legacy sources are imported once, on the first write
//! while the home file does not exist yet:
//!
//! - the git-tracked repo-local file (`<repo_root>/.gwt/work/*.md`) is
//!   COPIED — deleting it would dirty the user's working tree
//! - the untracked `tasks/{memory,lessons,discussions}.md` scratch files are
//!   MOVED (EXDEV-safe copy + remove)

use std::{
    fs::{self, OpenOptions},
    io,
    path::Path,
};

use fs2::FileExt;

/// Run `operation` while holding the exclusive work-notes lock
/// (`work-notes/.lock`) for the repository. Every append or
/// read-modify-write of a home notes file must go through this.
pub fn with_work_notes_lock<T>(
    repo_root: &Path,
    operation: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    let notes_dir = gwt_core::paths::gwt_work_notes_dir(repo_root);
    fs::create_dir_all(&notes_dir)?;
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(notes_dir.join(".lock"))?;
    lock.lock_exclusive()?;
    let result = operation();
    let unlock_result = FileExt::unlock(&lock);
    match (result, unlock_result) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
    }
}

/// Import legacy memory sources into the home work-notes file once.
/// No-op when the home file already exists. Caller must hold the
/// work-notes lock. Returns `true` when an import happened.
pub fn migrate_memory_into_home(repo_root: &Path) -> io::Result<bool> {
    let home_path = gwt_core::paths::gwt_work_notes_memory_path(repo_root);
    if home_path.exists() {
        return Ok(false);
    }

    let repo_local = gwt_core::paths::gwt_repo_local_memory_path(repo_root);
    if repo_local.exists() {
        copy_into_home(&repo_local, &home_path)?;
        return Ok(true);
    }

    let tasks_dir = repo_root.join("tasks");
    for legacy in [tasks_dir.join("memory.md"), tasks_dir.join("lessons.md")] {
        if legacy.exists() {
            move_into_home(&legacy, &home_path)?;
            return Ok(true);
        }
    }
    Ok(false)
}

/// Import legacy discussion sources into the home work-notes file once.
/// Same contract as [`migrate_memory_into_home`].
pub fn migrate_discussions_into_home(repo_root: &Path) -> io::Result<bool> {
    let home_path = gwt_core::paths::gwt_work_notes_discussions_path(repo_root);
    if home_path.exists() {
        return Ok(false);
    }

    let repo_local = gwt_core::paths::gwt_repo_local_discussions_path(repo_root);
    if repo_local.exists() {
        copy_into_home(&repo_local, &home_path)?;
        return Ok(true);
    }

    let legacy = repo_root.join("tasks").join("discussions.md");
    if legacy.exists() {
        move_into_home(&legacy, &home_path)?;
        return Ok(true);
    }
    Ok(false)
}

fn copy_into_home(source: &Path, home_path: &Path) -> io::Result<()> {
    if let Some(parent) = home_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(source, home_path).map(|_| ())
}

/// Move `source` into the home file. `fs::rename` cannot cross filesystem
/// boundaries (EXDEV) — the home dir routinely lives on another volume than
/// the worktree — so fall back to copy + remove.
fn move_into_home(source: &Path, home_path: &Path) -> io::Result<()> {
    if let Some(parent) = home_path.parent() {
        fs::create_dir_all(parent)?;
    }
    match fs::rename(source, home_path) {
        Ok(()) => Ok(()),
        Err(_) => {
            fs::copy(source, home_path)?;
            fs::remove_file(source)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_core::test_support::ScopedGwtHome;

    #[test]
    fn memory_migration_copies_repo_local_and_keeps_source() {
        let dir = tempfile::tempdir().unwrap();
        // Thread-local home override: the shared per-process cargo-test home
        // is wiped by unrelated parallel tests, which makes home reads flaky.
        let _home = ScopedGwtHome::set(dir.path().join("home"));
        let repo = dir.path();
        let repo_local = gwt_core::paths::gwt_repo_local_memory_path(repo);
        fs::create_dir_all(repo_local.parent().unwrap()).unwrap();
        fs::write(&repo_local, "# Memory\n\n## old — entry\n").unwrap();

        assert!(migrate_memory_into_home(repo).unwrap());

        let home_path = gwt_core::paths::gwt_work_notes_memory_path(repo);
        assert_eq!(
            fs::read_to_string(&home_path).unwrap(),
            "# Memory\n\n## old — entry\n"
        );
        assert!(
            repo_local.exists(),
            "git-tracked repo-local memory must be copied, not moved"
        );
        // Second call is a no-op once the home file exists.
        assert!(!migrate_memory_into_home(repo).unwrap());
    }

    #[test]
    fn memory_migration_moves_untracked_tasks_files() {
        let dir = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(dir.path().join("home"));
        let repo = dir.path();
        let tasks = repo.join("tasks");
        fs::create_dir_all(&tasks).unwrap();
        fs::write(tasks.join("lessons.md"), "# Lessons\n").unwrap();

        assert!(migrate_memory_into_home(repo).unwrap());

        let home_path = gwt_core::paths::gwt_work_notes_memory_path(repo);
        assert_eq!(fs::read_to_string(&home_path).unwrap(), "# Lessons\n");
        assert!(!tasks.join("lessons.md").exists());
    }

    #[test]
    fn discussions_migration_prefers_repo_local_copy() {
        let dir = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(dir.path().join("home"));
        let repo = dir.path();
        let repo_local = gwt_core::paths::gwt_repo_local_discussions_path(repo);
        fs::create_dir_all(repo_local.parent().unwrap()).unwrap();
        fs::write(&repo_local, "# Discussions\n").unwrap();

        assert!(migrate_discussions_into_home(repo).unwrap());

        let home_path = gwt_core::paths::gwt_work_notes_discussions_path(repo);
        assert_eq!(fs::read_to_string(&home_path).unwrap(), "# Discussions\n");
        assert!(repo_local.exists());
    }

    #[test]
    fn work_notes_lock_serializes_and_releases() {
        let dir = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(dir.path().join("home"));
        let repo = dir.path();
        let value = with_work_notes_lock(repo, || Ok(42)).unwrap();
        assert_eq!(value, 42);
        // Re-acquiring after release must succeed.
        let value = with_work_notes_lock(repo, || Ok(7)).unwrap();
        assert_eq!(value, 7);
    }
}
