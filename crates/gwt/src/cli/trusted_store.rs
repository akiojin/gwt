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
    // A trusted-store key is defined only for an actual Git worktree (or bare
    // repository). Do not use the project-index resolver here: its
    // workspace-home compatibility fallback shells out to `git rev-parse`.
    // Diagnosis is projected for every historical Work on the GUI event
    // loop, so one non-Git/stale directory would otherwise spawn Git
    // repeatedly and freeze the Workspace surface. The core resolver reads
    // `.git` / config files directly and returns `None` for those rows,
    // preserving the documented mirror-only degenerate mode.
    let repo_hash = gwt_core::repo_hash::detect_repo_hash(worktree)?;
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

/// Write one authoritative repo-scoped record and read it back under the
/// same resolved-directory lease.
///
/// Unlike [`write()`], this operation refuses mirror-only degenerate mode:
/// callers use the returned bytes to decide whether a security-sensitive
/// outcome may be reported as successful.
pub fn write_with_readback(worktree: &Path, file_name: &str, bytes: &[u8]) -> io::Result<String> {
    let trusted_dir = trusted_dir_for_worktree(worktree).ok_or_else(|| {
        io::Error::new(
            ErrorKind::NotFound,
            "trusted write readback requires a canonical repository scope",
        )
    })?;
    with_write_lease_for_resolved_dir(&trusted_dir, || {
        write_to_resolved_dir(&trusted_dir, file_name, bytes)?;
        read_from_resolved_dir(&trusted_dir, file_name)?.ok_or_else(|| {
            io::Error::new(
                ErrorKind::NotFound,
                "trusted record disappeared after write",
            )
        })
    })
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

/// Result of moving one unhealthy trusted-state file out of the canonical
/// authority path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QuarantinedTrustedFile {
    pub source_hash: String,
    pub destination: PathBuf,
}

/// Move an unhealthy trusted-state file to a collision-resistant sibling.
///
/// The hard-link-first sequence gives the destination create-new semantics:
/// an existing path is never overwritten. Callers hold the surrounding
/// trusted-store lease, so removing the source after the link succeeds
/// completes the move without another canonical writer racing the source.
pub(crate) fn quarantine_file(source: &Path) -> io::Result<QuarantinedTrustedFile> {
    let file_name = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            io::Error::new(
                ErrorKind::InvalidInput,
                "quarantine source has no UTF-8 file name",
            )
        })?;
    let bytes = fs::read(source)?;
    let source_hash = format!("{:x}", Sha256::digest(&bytes));
    for _ in 0..16 {
        let destination = source.with_file_name(format!(
            "{file_name}.corrupt-{}",
            uuid::Uuid::new_v4().simple()
        ));
        match quarantine_file_to(source, &destination) {
            Ok(()) => {
                return Ok(QuarantinedTrustedFile {
                    source_hash,
                    destination,
                });
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        ErrorKind::AlreadyExists,
        "trusted-state quarantine could not allocate a unique destination",
    ))
}

fn quarantine_file_to(source: &Path, destination: &Path) -> io::Result<()> {
    fs::hard_link(source, destination)?;
    if let Err(error) = fs::remove_file(source) {
        let _ = fs::remove_file(destination);
        return Err(error);
    }
    Ok(())
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
        stamp_worktree_marker(dir, worktree);
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

/// Marker file naming the worktree a trusted directory belongs to. The
/// directory key is a one-way hash, so GC (T-181) needs this to decide
/// whether the worktree still exists.
const WORKTREE_MARKER_FILE: &str = "worktree-path.txt";

/// How long an orphaned trusted directory survives after its worktree
/// disappears (T-181). Long enough for post-deletion inspection and the
/// T-182 relaunch import; short enough that ephemeral worktrees do not
/// accumulate forever.
const GC_RETENTION: Duration = Duration::from_secs(7 * 24 * 60 * 60);

fn stamp_worktree_marker(trusted_dir: &Path, worktree: &Path) {
    let marker = trusted_dir.join(WORKTREE_MARKER_FILE);
    if marker.exists() {
        return;
    }
    let canonical = dunce::canonicalize(worktree).unwrap_or_else(|_| worktree.to_path_buf());
    if let Err(error) =
        gwt_github::cache::write_atomic(&marker, canonical.to_string_lossy().as_bytes())
    {
        tracing::warn!(?error, "trusted store worktree marker write failed");
    }
}

/// T-181 core: best-effort GC of sibling trusted directories whose recorded
/// worktree no longer exists and whose newest file is older than the
/// retention window. Marker-less directories (pre-T-181) are left alone —
/// GC never guesses. Runs from launch materialization; failures only warn.
pub fn gc_best_effort(current_worktree: &Path) {
    gc_with_retention(current_worktree, GC_RETENTION);
}

fn gc_with_retention(current_worktree: &Path, retention: Duration) {
    let Some(own_dir) = trusted_dir_for_worktree(current_worktree) else {
        return;
    };
    let Some(root) = own_dir.parent() else {
        return;
    };
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() || dir == own_dir {
            continue;
        }
        let marker = dir.join(WORKTREE_MARKER_FILE);
        let Ok(recorded) = fs::read_to_string(&marker) else {
            continue;
        };
        if Path::new(recorded.trim()).exists() {
            continue;
        }
        if newest_modification_age(&dir).is_none_or(|age| age < retention) {
            continue;
        }
        if let Err(error) = fs::remove_dir_all(&dir) {
            tracing::warn!(?error, path = %dir.display(), "trusted store GC failed");
        } else {
            tracing::info!(path = %dir.display(), "trusted store GC removed orphaned entry");
        }
    }
}

/// Age of the most recently modified file in the directory (None when the
/// directory is unreadable or clocks misbehave — GC then keeps it).
fn newest_modification_age(dir: &Path) -> Option<Duration> {
    let mut newest: Option<std::time::SystemTime> = None;
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let modified = entry.metadata().ok()?.modified().ok()?;
        newest = Some(match newest {
            Some(current) if current >= modified => current,
            _ => modified,
        });
    }
    newest?.elapsed().ok()
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

/// T-177 core: turn a trusted-state I/O or parse failure into an
/// actionable repair message for the canonical operations. The raw error
/// used to surface as a misleading "network error"; store failures are a
/// local health problem with local repair paths.
#[must_use]
pub fn store_health_error(context: &str, err: &std::io::Error) -> String {
    format!(
        "trusted state unhealthy while {context}: {err}. The execution/verification records \
         under the repo-scoped trusted store (`~/.gwt/projects/<repo-hash>/trusted/<worktree-key>/`) \
         or their worktree mirrors (`.gwt/skill-state/`) could not be read or parsed. Repair by \
         rerunning the canonical writer: `execution.repair` quarantines an unreadable execution \
         control record and materializes a fresh Active one (`execution.adopt` takes over only \
         records that still pass integrity); `verify.plan` / `verify.run` rewrite verification state; \
         `intake.outcome.record` rewrites the intake outcome. If the failure persists, inspect the \
         store directory for filesystem problems."
    )
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
    fn quarantine_preserves_source_bytes_and_never_overwrites_a_collision() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("execution-control.json");
        let collision = dir.path().join("execution-control.json.corrupt-fixed");
        fs::write(&source, b"corrupt authority").unwrap();
        fs::write(&collision, b"existing quarantine").unwrap();

        let error = quarantine_file_to(&source, &collision).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&source).unwrap(), b"corrupt authority");
        assert_eq!(fs::read(&collision).unwrap(), b"existing quarantine");

        let quarantined = quarantine_file(&source).unwrap();
        assert!(!source.exists());
        assert_eq!(
            fs::read(&quarantined.destination).unwrap(),
            b"corrupt authority"
        );
        assert_eq!(
            quarantined.source_hash,
            format!("{:x}", Sha256::digest(b"corrupt authority"))
        );
        assert_ne!(quarantined.destination, collision);
    }

    // T-181: GC removes orphaned sibling entries (marker points at a gone
    // worktree, files older than retention) and keeps live and marker-less
    // ones.
    #[test]
    fn gc_removes_only_orphaned_marked_siblings() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let dir = tempfile::tempdir().unwrap();
        init_git_repo_with_origin(dir.path());
        write(dir.path(), "own.json", b"{}").unwrap();
        let own_dir = trusted_dir_for_worktree(dir.path()).unwrap();
        let root = own_dir.parent().unwrap().to_path_buf();

        // Orphan: marker points at a deleted worktree.
        let orphan = root.join("00000000deadbeef");
        std::fs::create_dir_all(&orphan).unwrap();
        std::fs::write(orphan.join("execution-control.json"), "{}").unwrap();
        std::fs::write(
            orphan.join(WORKTREE_MARKER_FILE),
            dir.path()
                .join("no-such-worktree")
                .to_string_lossy()
                .as_bytes(),
        )
        .unwrap();
        // Live sibling: marker points at an existing path.
        let live = root.join("00000000cafebabe");
        std::fs::create_dir_all(&live).unwrap();
        std::fs::write(live.join("execution-control.json"), "{}").unwrap();
        std::fs::write(
            live.join(WORKTREE_MARKER_FILE),
            dir.path().to_string_lossy().as_bytes(),
        )
        .unwrap();
        // Marker-less legacy sibling: never touched.
        let legacy = root.join("00000000feedf00d");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("execution-control.json"), "{}").unwrap();

        gc_with_retention(dir.path(), Duration::ZERO);
        assert!(!orphan.exists(), "orphaned entry must be removed");
        assert!(live.exists(), "live entry must survive");
        assert!(legacy.exists(), "marker-less legacy entry must survive");
        assert!(own_dir.exists(), "own entry must survive");

        // Fresh orphans inside the retention window survive.
        std::fs::create_dir_all(&orphan).unwrap();
        std::fs::write(orphan.join("execution-control.json"), "{}").unwrap();
        std::fs::write(
            orphan.join(WORKTREE_MARKER_FILE),
            dir.path()
                .join("no-such-worktree")
                .to_string_lossy()
                .as_bytes(),
        )
        .unwrap();
        gc_with_retention(dir.path(), Duration::from_secs(3600));
        assert!(
            orphan.exists(),
            "entries younger than retention must survive"
        );
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
