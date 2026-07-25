//! Repo-scoped trusted store (SPEC-3248 P9b, T-172/T-173-lite).
//!
//! The authoritative copies of the Execution Control Record, Verification
//! Run Record, and Verification Plan Record live under the machine-local
//! repo-scoped store — `~/.gwt/projects/<repo-hash>/trusted/<worktree-key>/`
//! — instead of the worktree. The worktree's `.gwt/skill-state/*.json`
//! files remain as human-inspectable **mirrors**: every canonical writer
//! writes both, and every gate reads the repo-scoped copy first, so editing
//! the mirror changes nothing the gates trust (T-174 core) and the records
//! survive ephemeral worktree deletion (T-175 core).
//!
//! A worktree without a trusted copy falls back to the mirror as a legacy
//! (pre-P9b) record — same one-release-cycle sunset policy as the P9a
//! integrity hashes. Worktrees where the repo hash cannot be resolved
//! (non-git test dirs) run in mirror-only degenerate mode.
//!
//! Follow-ups (dependent): store health gates (T-177), GC/retention
//! (T-181), legacy import (T-182), cross-worktree conflict surfacing.

use std::{
    fs,
    io::{self, ErrorKind},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use fs2::FileExt;
use sha2::{Digest, Sha256};

/// Resolve the repo-scoped trusted directory for a worktree. `None` when the
/// repo hash cannot be determined (non-git dirs — degenerate mirror-only
/// mode).
#[must_use]
pub fn trusted_dir_for_worktree(worktree: &Path) -> Option<PathBuf> {
    let repo_hash = crate::index_worker::detect_repo_hash(worktree)?;
    Some(
        gwt_core::paths::gwt_projects_dir()
            .join(repo_hash.as_str())
            .join("trusted")
            .join(worktree_key(worktree)),
    )
}

/// Stable key for one worktree: sha256 of the canonicalized, normalized
/// absolute path (backslashes unified, lowercased — Windows paths reach the
/// writers and readers in different spellings, and the key must not fork).
/// `dunce` keeps the canonical form free of the Windows `\\?\` prefix so the
/// key stays identical when canonicalization later fails (e.g. reading the
/// record of an already-deleted ephemeral worktree, T-175).
fn worktree_key(worktree: &Path) -> String {
    let canonical = dunce::canonicalize(worktree).unwrap_or_else(|_| worktree.to_path_buf());
    let normalized = canonical
        .to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_string();
    // Case-fold only where the filesystem does: on case-sensitive systems
    // two paths differing in case are genuinely different worktrees and
    // must not share a trusted directory.
    #[cfg(windows)]
    let normalized = normalized.to_lowercase();
    let digest = Sha256::digest(normalized.as_bytes());
    format!("{digest:x}")[..16].to_string()
}

/// Read the trusted copy of `file_name` for the worktree. `Ok(None)` when the
/// store or the file is absent (legacy / degenerate mode — callers fall back
/// to the worktree mirror).
pub fn read(worktree: &Path, file_name: &str) -> io::Result<Option<String>> {
    let Some(dir) = trusted_dir_for_worktree(worktree) else {
        return Ok(None);
    };
    read_from_resolved_dir(&dir, file_name)
}

/// Read from a trusted directory that the caller already resolved. Use this
/// inside a write lease so a mutable repository identity cannot redirect the
/// read-modify-write cycle to a different store.
pub(crate) fn read_from_resolved_dir(
    trusted_dir: &Path,
    file_name: &str,
) -> io::Result<Option<String>> {
    match fs::read_to_string(trusted_dir.join(file_name)) {
        Ok(contents) => Ok(Some(contents)),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

/// Write the trusted copy atomically. A worktree without a resolvable repo
/// hash is a silent no-op (degenerate mode; the mirror still carries the
/// record).
pub fn write(worktree: &Path, file_name: &str, bytes: &[u8]) -> io::Result<()> {
    let Some(dir) = trusted_dir_for_worktree(worktree) else {
        return Ok(());
    };
    write_to_resolved_dir(&dir, file_name, bytes)
}

/// Write to a trusted directory that the caller already resolved and leased.
/// This prevents a second resolver call from moving the authoritative write
/// beneath a directory whose lease is not held.
pub(crate) fn write_to_resolved_dir(
    trusted_dir: &Path,
    file_name: &str,
    bytes: &[u8],
) -> io::Result<()> {
    gwt_github::cache::write_atomic(&trusted_dir.join(file_name), bytes)
}

/// Write the authoritative trusted copy, then the worktree mirror. Once the
/// trusted copy is written the mirror is informational only — its failure is
/// logged, not surfaced, so an operation can never report failure while the
/// gates already honor the new record. In degenerate mode the mirror is the
/// only copy and its failure propagates.
pub fn write_with_mirror(
    worktree: &Path,
    file_name: &str,
    mirror_path: &Path,
    bytes: &[u8],
) -> io::Result<()> {
    let trusted_dir = trusted_dir_for_worktree(worktree);
    if let Some(dir) = &trusted_dir {
        gwt_github::cache::write_atomic(&dir.join(file_name), bytes)?;
    }
    match gwt_github::cache::write_atomic(mirror_path, bytes) {
        Err(err) if trusted_dir.is_some() => {
            tracing::warn!(
                ?err,
                path = %mirror_path.display(),
                "worktree mirror write failed after trusted store write"
            );
            Ok(())
        }
        result => result,
    }
}

/// Bounded wait before a second concurrent writer is refused (T-149). Long
/// enough to ride out another writer's normal read-modify-write cycle,
/// short enough that a stuck holder surfaces as an explicit retry error
/// instead of a hang.
const WRITE_LEASE_WAIT: Duration = Duration::from_secs(2);
const WRITE_LEASE_POLL: Duration = Duration::from_millis(25);

#[cfg(test)]
std::thread_local! {
    static WRITE_LEASE_ACQUIRED_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn set_write_lease_acquired_hook(hook: impl FnOnce() + 'static) {
    WRITE_LEASE_ACQUIRED_HOOK.with(|slot| {
        let previous = slot.replace(Some(Box::new(hook)));
        assert!(
            previous.is_none(),
            "write-lease acquired hook must not be installed recursively"
        );
    });
}

#[cfg(test)]
fn run_write_lease_acquired_hook() {
    WRITE_LEASE_ACQUIRED_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

/// SPEC-3248 T-149 owner write lease: serialize gwt-originated
/// read-modify-write cycles on this worktree's execution/verification/intake
/// state records across processes. The lease is an fs2 advisory lock on
/// `.write-lease` in the repo-scoped trusted directory (or, in degenerate
/// mirror-only mode, in the worktree's `.gwt/skill-state/`), so every
/// canonical writer for the same worktree contends on the same file. A
/// second concurrent writer waits briefly, then gets an explicit-retry
/// refusal — never a silent last-writer-wins interleave.
///
/// Callers wrap one whole RMW cycle and must not nest leases (fs2 locks on a
/// second handle to the same file block within one process too).
pub fn with_write_lease<T>(
    worktree: &Path,
    operation: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    with_write_lease_wait(worktree, WRITE_LEASE_WAIT, operation)
}

/// [`with_write_lease`] with an explicit wait bound (tests use a short one
/// to assert the refusal path quickly).
pub fn with_write_lease_wait<T>(
    worktree: &Path,
    wait: Duration,
    operation: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    let dir = trusted_dir_for_worktree(worktree)
        .unwrap_or_else(|| worktree.join(".gwt").join("skill-state"));
    with_write_lease_for_resolved_dir_wait(&dir, wait, operation)
}

/// Hold the write lease beneath one directory that the caller already
/// resolved. The same directory can then be passed to resolved read/write
/// helpers for one stable read-modify-write transaction.
pub(crate) fn with_write_lease_for_resolved_dir<T>(
    trusted_dir: &Path,
    operation: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    with_write_lease_for_resolved_dir_wait(trusted_dir, WRITE_LEASE_WAIT, operation)
}

fn with_write_lease_for_resolved_dir_wait<T>(
    dir: &Path,
    wait: Duration,
    operation: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    fs::create_dir_all(dir)?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(dir.join(".write-lease"))?;
    let deadline = Instant::now() + wait;
    // Contention is WouldBlock on Unix but a raw OS error (33) wrapped as
    // Uncategorized on Windows — compare against fs2's canonical error.
    let is_contended = |err: &io::Error| {
        err.kind() == ErrorKind::WouldBlock
            || err.raw_os_error() == fs2::lock_contended_error().raw_os_error()
    };
    loop {
        match lock.try_lock_exclusive() {
            Ok(()) => break,
            Err(err) if is_contended(&err) => {
                let now = Instant::now();
                if now >= deadline {
                    return Err(io::Error::new(
                        ErrorKind::WouldBlock,
                        "owner write lease is held by another gwt writer for this worktree — \
                         retry the operation after the concurrent write settles (T-149; \
                         last-writer-wins interleaving is refused)",
                    ));
                }
                std::thread::sleep(WRITE_LEASE_POLL.min(deadline.saturating_duration_since(now)));
            }
            Err(err) => return Err(err),
        }
    }
    #[cfg(test)]
    run_write_lease_acquired_hook();
    let result = operation();
    let _ = FileExt::unlock(&lock);
    result
}

/// True when the worktree is under trusted-store management: launch
/// materialization wrote the Execution Control Record's trusted copy, so
/// every later canonical `verify.plan` / `verify.run` write produced a
/// trusted copy too. Readers use this to refuse mirror-only verification
/// state in managed worktrees — there, a mirror without a trusted copy can
/// only be a forgery or a pre-P9b binary write, never canonical evidence.
#[must_use]
pub fn under_trusted_management(worktree: &Path) -> bool {
    trusted_dir_for_worktree(worktree)
        .is_some_and(|dir| dir.join("execution-control.json").exists())
}

/// Initialize a git repo with an `origin` remote so `detect_repo_hash`
/// resolves (it derives the repo hash from the origin URL). Shared by the
/// authority-precedence tests in `execution_state` / `verification_record`.
#[cfg(test)]
pub(crate) fn init_git_repo_with_origin(dir: &Path) {
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "user.name", "t"],
        vec![
            "remote",
            "add",
            "origin",
            "https://example.com/t/trusted-store.git",
        ],
        vec!["commit", "--allow-empty", "-qm", "init"],
    ] {
        let status = gwt_core::process::hidden_command("git")
            .args(&args)
            .current_dir(dir)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_core::test_support::ScopedEnvVar;

    // T-149: two concurrent writers serialize — the second waits for the
    // first and both complete, in order.
    #[test]
    fn write_lease_serializes_concurrent_writers() {
        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path().to_path_buf();
        let order = std::sync::Arc::new(std::sync::Mutex::new(Vec::<&str>::new()));

        let order_a = order.clone();
        let worktree_a = worktree.clone();
        let (acquired_tx, acquired_rx) = std::sync::mpsc::channel();
        let holder = std::thread::spawn(move || {
            with_write_lease(&worktree_a, || {
                order_a.lock().unwrap().push("a-start");
                acquired_tx.send(()).unwrap();
                std::thread::sleep(Duration::from_millis(300));
                order_a.lock().unwrap().push("a-end");
                Ok(())
            })
            .unwrap();
        });
        // Handshake: contend only after the holder actually holds the lease.
        acquired_rx.recv().unwrap();
        with_write_lease(&worktree, || {
            order.lock().unwrap().push("b");
            Ok(())
        })
        .unwrap();
        holder.join().unwrap();
        assert_eq!(*order.lock().unwrap(), vec!["a-start", "a-end", "b"]);
    }

    // T-149: a second writer that exceeds the bounded wait is refused with
    // an explicit-retry error and its operation NEVER runs — no
    // last-writer-wins interleave.
    #[test]
    fn write_lease_refuses_second_writer_with_explicit_retry() {
        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path().to_path_buf();

        let worktree_a = worktree.clone();
        let (acquired_tx, acquired_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let holder = std::thread::spawn(move || {
            with_write_lease(&worktree_a, || {
                acquired_tx.send(()).unwrap();
                let _ = release_rx.recv_timeout(Duration::from_secs(10));
                Ok(())
            })
            .unwrap();
        });
        acquired_rx.recv().unwrap();
        let mut ran = false;
        let err = with_write_lease_wait(&worktree, Duration::from_millis(50), || {
            ran = true;
            Ok(())
        })
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::WouldBlock);
        assert!(err.to_string().contains("retry"), "{err}");
        assert!(err.to_string().contains("T-149"), "{err}");
        assert!(!ran, "refused writer must not run its operation");
        release_tx.send(()).unwrap();
        holder.join().unwrap();
    }

    #[test]
    fn non_git_dir_is_degenerate_mirror_only() {
        let dir = tempfile::tempdir().unwrap();
        assert!(trusted_dir_for_worktree(dir.path()).is_none());
        assert_eq!(read(dir.path(), "x.json").unwrap(), None);
        write(dir.path(), "x.json", b"{}").unwrap();
        assert_eq!(read(dir.path(), "x.json").unwrap(), None);
    }

    #[test]
    fn git_worktree_roundtrips_through_repo_scoped_store() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let dir = tempfile::tempdir().unwrap();
        init_git_repo_with_origin(dir.path());

        assert_eq!(read(dir.path(), "r.json").unwrap(), None);
        write(dir.path(), "r.json", b"{\"a\":1}").unwrap();
        assert_eq!(
            read(dir.path(), "r.json").unwrap().as_deref(),
            Some("{\"a\":1}")
        );
        // Store lives under the scoped HOME, outside the worktree.
        let trusted = trusted_dir_for_worktree(dir.path()).unwrap();
        assert!(trusted.starts_with(home.path()));
        assert!(!trusted.starts_with(dir.path()));

        // Key is stable across path spellings of the same worktree.
        let respelled = dir.path().to_string_lossy().to_uppercase();
        let respelled_key_dir = trusted_dir_for_worktree(Path::new(&respelled));
        if let Some(respelled_dir) = respelled_key_dir {
            assert_eq!(respelled_dir, trusted);
        }

        // T-175 core: the stored bytes live under HOME, so deleting the
        // (ephemeral) worktree does not take the record with it.
        let stored_file = trusted.join("r.json");
        drop(dir);
        assert!(stored_file.exists());
    }
}
