use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

use fs2::FileExt;

#[derive(Debug)]
pub struct GuiInstanceLock {
    path: PathBuf,
    file: File,
}

#[derive(Debug, thiserror::Error)]
pub enum GuiInstanceLockError {
    #[error("gwt GUI is already running for worktree {project_root} (lock: {lock_path})")]
    AlreadyRunning {
        project_root: PathBuf,
        lock_path: PathBuf,
    },
    #[error("failed to prepare gwt GUI single-instance lock for {project_root}: {source}")]
    Io {
        project_root: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to resolve gwt GUI single-instance scope for {project_root}: {reason}")]
    Scope {
        project_root: PathBuf,
        reason: String,
    },
}

impl Drop for GuiInstanceLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
        let mut registry = process_lock_registry()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        registry.remove(&self.path);
    }
}

pub fn gui_instance_lock_path(
    gwt_home: &Path,
    project_root: &Path,
) -> Result<PathBuf, GuiInstanceLockError> {
    let repo_hash = gwt_core::paths::project_scope_hash(project_root);
    let worktree_hash =
        gwt_core::worktree_hash::compute_worktree_hash(project_root).map_err(|error| {
            GuiInstanceLockError::Scope {
                project_root: project_root.to_path_buf(),
                reason: error.to_string(),
            }
        })?;
    Ok(gwt_home
        .join("projects")
        .join(repo_hash.as_str())
        .join("runtime")
        .join("gui")
        .join(format!("{}.lock", worktree_hash.as_str())))
}

pub fn acquire_gui_instance_lock(
    gwt_home: &Path,
    project_root: &Path,
) -> Result<GuiInstanceLock, GuiInstanceLockError> {
    let lock_path = gui_instance_lock_path(gwt_home, project_root)?;
    let parent = lock_path
        .parent()
        .ok_or_else(|| GuiInstanceLockError::Scope {
            project_root: project_root.to_path_buf(),
            reason: format!("lock path has no parent: {}", lock_path.display()),
        })?;
    fs::create_dir_all(parent).map_err(|source| GuiInstanceLockError::Io {
        project_root: project_root.to_path_buf(),
        source,
    })?;

    {
        let mut registry = process_lock_registry()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if registry.contains(&lock_path) {
            return Err(GuiInstanceLockError::AlreadyRunning {
                project_root: project_root.to_path_buf(),
                lock_path,
            });
        }
        registry.insert(lock_path.clone());
    }

    let file = match OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
    {
        Ok(file) => file,
        Err(source) => {
            unregister_process_lock(&lock_path);
            return Err(GuiInstanceLockError::Io {
                project_root: project_root.to_path_buf(),
                source,
            });
        }
    };

    match file.try_lock_exclusive() {
        Ok(()) => Ok(GuiInstanceLock {
            path: lock_path,
            file,
        }),
        Err(_) => {
            unregister_process_lock(&lock_path);
            Err(GuiInstanceLockError::AlreadyRunning {
                project_root: project_root.to_path_buf(),
                lock_path,
            })
        }
    }
}

fn unregister_process_lock(path: &Path) {
    let mut registry = process_lock_registry()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    registry.remove(path);
}

fn process_lock_registry() -> &'static Mutex<HashSet<PathBuf>> {
    static REGISTRY: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashSet::new()))
}
