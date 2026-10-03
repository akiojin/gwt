//! The runner cannot persist its own SIGKILL. A small companion waits for its
//! private pipe to close and settles only that run's unfinished record. It owns
//! no verification lease and can never produce passing evidence.

use std::{
    io::{self, BufRead, Read, Write},
    path::Path,
    process::{Child, ChildStdin, Stdio},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{integrity_ok, load, save, VerificationRunRecord};

pub const WATCHDOG_ARG: &str = "--verification-watchdog";
const READY: &str = "verification-watchdog-ready\n";
const INFRASTRUCTURE_FAILURE: &str = "execution infrastructure failure: verification was externally terminated twice consecutively on the same HEAD; automatic third execution is refused. Inspect the runner/host termination cause and correct it before verifying a corrected HEAD";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Interrupted,
}

/// Absent on completed (including legacy) records. An unfinished record is
/// never evidence of test success or test failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunLifecycle {
    pub status: RunStatus,
    pub runner_pid: u32,
    /// The private pipe carries the token; only its digest reaches a record.
    /// A record ID from a diagnostic mirror does not authorize interruption.
    pub watchdog_token_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Consecutive authenticated runner deaths on this HEAD. Omission keeps
    /// existing record hashes compatible; ordinary completion drops lifecycle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_terminations: Option<u8>,
}

impl RunLifecycle {
    pub(super) fn running(token: &str) -> Self {
        Self {
            status: RunStatus::Running,
            runner_pid: std::process::id(),
            watchdog_token_hash: token_hash(token),
            current_command: None,
            reason: None,
            external_terminations: None,
        }
    }
}

/// Call under the trusted write lease. Preserve a live predecessor, and
/// recover a dead runner even when its watchdog also died (for example reboot).
/// Diff or plan edits must not reset the budget, which is tied to commit HEAD.
pub(super) fn previous_external_terminations(
    worktree: &Path,
    head: Option<&str>,
) -> io::Result<Option<u8>> {
    let mut record = match load(worktree) {
        Ok(Some(record)) => record,
        // verify.run is the documented writer that repairs unreadable evidence.
        // Invalid JSON is not trusted retry history; filesystem errors still fail.
        Ok(None) => return Ok(None),
        Err(error) if error.kind() == io::ErrorKind::InvalidData => return Ok(None),
        Err(error) => return Err(error),
    };
    if record.content_hash.is_empty() || !integrity_ok(&record) {
        return Ok(None);
    }
    if let Some(lifecycle) = record
        .lifecycle
        .as_ref()
        .filter(|lifecycle| lifecycle.status == RunStatus::Running)
    {
        if crate::process::is_host_process_alive(lifecycle.runner_pid) {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "the previous verification runner is unsettled; wait for its terminal result before retrying (its watchdog may still be recording an external termination)",
            ));
        }
        persist_interrupted(worktree, &mut record, false,
            "external termination: runner is no longer alive and no terminal result was recorded; signal unknown; rerun required")?;
    }
    if head.is_none() || record.verified_head.as_deref() != head {
        return Ok(None);
    }
    let count = record.lifecycle.as_ref().and_then(|lifecycle| {
        (lifecycle.status == RunStatus::Interrupted)
            .then_some(lifecycle.external_terminations)
            .flatten()
    });
    if count.is_some_and(|count| count >= 2) {
        return Err(io::Error::other(INFRASTRUCTURE_FAILURE));
    }
    Ok(count)
}

pub(super) struct Watchdog {
    child: Child,
    pipe: Option<ChildStdin>,
}

impl Watchdog {
    /// Used by the real gwtd entrypoint; unit-level in-process runs have no
    /// gwtd executable and exercise the record transitions directly instead.
    pub(super) fn start(worktree: &Path, record_id: &str, token: &str) -> io::Result<Self> {
        let mut child = gwt_core::process::hidden_command(std::env::current_exe()?)
            .arg(WATCHDOG_ARG)
            .arg(worktree)
            .arg(record_id)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let pipe = child.stdin.take();
        let mut watchdog = Self { child, pipe };
        let pipe = watchdog.pipe.as_mut().expect("piped stdin");
        pipe.write_all(token.as_bytes())?;
        pipe.write_all(b"\n")?;
        let mut ready = String::new();
        io::BufReader::new(watchdog.child.stdout.take().expect("piped stdout"))
            .read_line(&mut ready)?;
        if ready != READY {
            return Err(io::Error::other(
                "verification watchdog did not become ready",
            ));
        }
        Ok(watchdog)
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        if let Some(mut pipe) = self.pipe.take() {
            // A regular return (including an error) is distinguishable from
            // process death. A killed runner cannot send this byte.
            let _ = pipe.write_all(b"R");
        }
        let _ = self.child.wait();
    }
}

/// Private gwtd companion entrypoint, before the public operation dispatcher.
/// The pipe is inherited only by the runner that spawned this process.
pub fn run_watchdog(worktree: &Path, record_id: &str) -> io::Result<()> {
    let mut input = io::BufReader::new(io::stdin());
    let mut token = String::new();
    // Bound an invalid private invocation before accepting any work.
    input.by_ref().take(128).read_line(&mut token)?;
    let token = token.trim_end();
    if token.len() != 32 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "invalid watchdog token",
        ));
    }
    io::stdout().write_all(READY.as_bytes())?;
    io::stdout().flush()?;
    let mut returned = [0];
    let normal_return = input.read(&mut returned)? != 0;
    settle_interrupted(worktree, record_id, token, normal_return)
}

fn token_hash(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

fn settle_interrupted(
    worktree: &Path,
    record_id: &str,
    token: &str,
    normal_return: bool,
) -> io::Result<()> {
    crate::cli::trusted_store::with_write_lease(worktree, || {
        let Some(mut record) = load(worktree)? else {
            return Ok(());
        };
        if record.record_id != record_id || !integrity_ok(&record) {
            return Ok(());
        }
        let Some(lifecycle) = record.lifecycle.as_mut() else {
            return Ok(());
        };
        if lifecycle.status != RunStatus::Running
            || lifecycle.watchdog_token_hash != token_hash(token)
        {
            return Ok(());
        }
        persist_interrupted(worktree, &mut record, normal_return,
            "external termination: runner pipe closed before a terminal result; signal unknown; rerun required")
    })
}

/// Both authenticated watchdog EOF and confirmed runner death settle through
/// this transition while holding the write lease, so they cannot double count.
fn persist_interrupted(
    worktree: &Path,
    record: &mut VerificationRunRecord,
    normal_return: bool,
    external_reason: &str,
) -> io::Result<()> {
    let lifecycle = record.lifecycle.as_mut().expect("unfinished run");
    lifecycle.status = RunStatus::Interrupted;
    lifecycle.external_terminations = if normal_return {
        None
    } else {
        Some(
            lifecycle
                .external_terminations
                .unwrap_or_default()
                .saturating_add(1),
        )
    };
    lifecycle.reason = Some(
        if normal_return {
            "verification runner returned without a terminal result; rerun required"
        } else if lifecycle
            .external_terminations
            .is_some_and(|count| count >= 2)
        {
            INFRASTRUCTURE_FAILURE
        } else {
            external_reason
        }
        .to_string(),
    );
    record.all_passed = false;
    record.plan_covered = false;
    record.created_at = chrono::Utc::now();
    save(worktree, record)
}

/// Call under the trusted write lease. A delayed writer must not overwrite
/// a replacement or settled record, even when both belong to one session.
pub(super) fn ensure_current(worktree: &Path, record_id: &str) -> io::Result<()> {
    match load(worktree)? {
        Some(record)
            if record.record_id == record_id
                && integrity_ok(&record)
                && record.lifecycle.as_ref().is_some_and(|lifecycle| {
                    lifecycle.status == RunStatus::Running
                }) => Ok(()),
        _ => Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "verification record was replaced or interrupted while this run was active; rerun verify.run",
        )),
    }
}

pub(super) fn checkpoint(worktree: &Path, record: &VerificationRunRecord) -> io::Result<()> {
    crate::cli::trusted_store::with_write_lease(worktree, || {
        ensure_current(worktree, &record.record_id)?;
        save(worktree, record)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The trusted store resolves under the gwt home, which sibling tests move
    /// by rewriting HOME / USERPROFILE under the env lock. A run whose save and
    /// settlement straddle such a move leaves an unsettled Running record
    /// behind in the other home, and the next run refuses admission. Pin this
    /// thread's home instead of reading the process environment.
    struct IsolatedWorktree {
        dir: tempfile::TempDir,
        _home_guard: gwt_core::test_support::ScopedGwtHome,
        _home: tempfile::TempDir,
    }

    fn isolated_git_worktree() -> IsolatedWorktree {
        let home = tempfile::tempdir().unwrap();
        let home_guard = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        IsolatedWorktree {
            dir,
            _home_guard: home_guard,
            _home: home,
        }
    }

    // Exercise the authenticated transition directly: no live process is killed.
    fn simulate_external_termination(worktree: &Path) {
        let result = super::super::run_verification_inner(
            worktree,
            "sess-retry",
            &["git --version".to_string()],
            None,
            &[],
            super::super::RunOptions::default(),
            || {
                let token = "0123456789abcdef0123456789abcdef";
                let mut record = load(worktree).unwrap().unwrap();
                record.lifecycle.as_mut().unwrap().watchdog_token_hash = token_hash(token);
                save(worktree, &record).unwrap();
                settle_interrupted(worktree, &record.record_id, token, false).unwrap();
            },
        );
        let msg = result.unwrap_err();
        assert!(
            msg.contains("replaced or interrupted"),
            "unexpected error: {msg}"
        );
    }

    #[test]
    fn dead_runner_without_watchdog_recovers_without_resetting_retry_budget() {
        let fixture = isolated_git_worktree();
        let dir = &fixture.dir;
        let token = "0123456789abcdef0123456789abcdef";
        let mut record = super::super::tests::passing_record("sess-retry", "unused");
        record.verified_head = super::super::current_head_sha(dir.path()).ok();
        record.all_passed = false;
        for previous_count in [None, Some(1)] {
            record.lifecycle = Some(RunLifecycle::running(token));
            let lifecycle = record.lifecycle.as_mut().unwrap();
            lifecycle.runner_pid = 0; // The existing host probe defines PID 0 as absent.
            lifecycle.external_terminations = previous_count;
            save(dir.path(), &record).unwrap();
            let admission = crate::cli::trusted_store::with_write_lease(dir.path(), || {
                previous_external_terminations(dir.path(), record.verified_head.as_deref())
            });
            if previous_count.is_none() {
                assert_eq!(admission.unwrap(), Some(1));
            } else {
                assert!(admission
                    .unwrap_err()
                    .to_string()
                    .contains("execution infrastructure failure"));
            }
            let interrupted = load(dir.path()).unwrap().unwrap();
            assert_eq!(
                interrupted.lifecycle.as_ref().unwrap().status,
                RunStatus::Interrupted
            );
            assert!(!interrupted.all_passed && !interrupted.plan_covered);
            settle_interrupted(dir.path(), &record.record_id, token, false).unwrap();
            assert_eq!(
                load(dir.path()).unwrap().unwrap(),
                interrupted,
                "late watchdog must not double count"
            );
        }
        let status = gwt_core::process::hidden_command("git")
            .args(["commit", "--allow-empty", "-qm", "corrected HEAD"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        assert!(status.success());
        let (completed, _) = super::super::run_verification(
            dir.path(),
            "sess-retry",
            &["git --version".to_string()],
        )
        .unwrap();
        assert!(completed.all_passed);
    }

    #[test]
    fn unsettled_runner_cannot_be_replaced_before_watchdog_records_second_death() {
        let fixture = isolated_git_worktree();
        let dir = &fixture.dir;
        let token = "0123456789abcdef0123456789abcdef";
        let mut running = super::super::tests::passing_record("sess-retry", "unused");
        running.verified_head = super::super::current_head_sha(dir.path()).ok();
        running.lifecycle = Some(RunLifecycle::running(token));
        running.lifecycle.as_mut().unwrap().external_terminations = Some(1);
        save(dir.path(), &running).unwrap();
        let before = load(dir.path()).unwrap().unwrap();

        let result = super::super::run_verification(
            dir.path(),
            "sess-retry",
            &["git config --local gwt.retry-dispatched true".to_string()],
        );
        assert!(result.unwrap_err().contains("wait for its terminal result"));
        assert_eq!(load(dir.path()).unwrap().unwrap(), before);
        let marker = gwt_core::process::hidden_command("git")
            .args(["config", "--local", "--get", "gwt.retry-dispatched"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert_eq!(marker.status.code(), Some(1));
        // HEAD changes must not replace a still-unsettled predecessor either.
        assert!(crate::cli::trusted_store::with_write_lease(dir.path(), || {
            previous_external_terminations(dir.path(), Some("different-head"))
        })
        .unwrap_err()
        .to_string()
        .contains("wait for its terminal result"));

        settle_interrupted(dir.path(), &running.record_id, token, false).unwrap();
        let interrupted = load(dir.path()).unwrap().unwrap();
        assert_eq!(
            interrupted.lifecycle.unwrap().external_terminations,
            Some(2)
        );
        assert!(crate::cli::trusted_store::with_write_lease(dir.path(), || {
            previous_external_terminations(dir.path(), running.verified_head.as_deref())
        })
        .unwrap_err()
        .to_string()
        .contains("execution infrastructure failure"));
    }

    #[test]
    fn two_external_terminations_refuse_third_run_on_same_head() {
        let fixture = isolated_git_worktree();
        let dir = &fixture.dir;
        simulate_external_termination(dir.path());
        // Changing uncommitted inputs cannot buy another attempt at this HEAD.
        std::fs::write(dir.path().join("changed.txt"), "changed").unwrap();
        simulate_external_termination(dir.path());
        let before = load(dir.path()).unwrap().unwrap();
        assert!(before
            .lifecycle
            .as_ref()
            .unwrap()
            .reason
            .as_ref()
            .unwrap()
            .contains("execution infrastructure failure"));
        let mut dispatched = false;
        let result = super::super::run_verification_inner(
            dir.path(),
            "sess-retry",
            &["git config --local gwt.retry-dispatched true".to_string()],
            None,
            &[],
            super::super::RunOptions::default(),
            || dispatched = true,
        );
        assert!(!dispatched, "third run must not dispatch commands");
        let marker = gwt_core::process::hidden_command("git")
            .args(["config", "--local", "--get", "gwt.retry-dispatched"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert_eq!(marker.status.code(), Some(1), "third command ran");
        assert!(result
            .unwrap_err()
            .contains("execution infrastructure failure"));
        assert_eq!(load(dir.path()).unwrap().unwrap(), before);

        let status = gwt_core::process::hidden_command("git")
            .args(["commit", "--allow-empty", "-qm", "new HEAD"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        assert!(status.success());
        simulate_external_termination(dir.path());
        let (completed, _) = super::super::run_verification(
            dir.path(),
            "sess-retry",
            &["git --version".to_string()],
        )
        .unwrap();
        assert!(completed.all_passed);
        // A normally completed run also clears the consecutive count.
        simulate_external_termination(dir.path());
        simulate_external_termination(dir.path());
    }

    #[test]
    fn watchdog_only_settles_its_authenticated_unfinished_record() {
        let dir = tempfile::tempdir().unwrap();
        let token = "0123456789abcdef0123456789abcdef";
        let mut record = super::super::tests::passing_record("sess-1", "no-git");
        record.lifecycle = Some(RunLifecycle::running(token));
        record.all_passed = false;
        record.plan_covered = false;
        save(dir.path(), &record).unwrap();
        let initial = load(dir.path()).unwrap().unwrap();
        settle_interrupted(dir.path(), &record.record_id, "wrong token", false).unwrap();
        settle_interrupted(dir.path(), "older-run", token, false).unwrap();
        assert_eq!(load(dir.path()).unwrap().unwrap(), initial);

        settle_interrupted(dir.path(), &record.record_id, token, false).unwrap();
        let interrupted = load(dir.path()).unwrap().unwrap();
        assert_eq!(
            interrupted.lifecycle.as_ref().unwrap().status,
            RunStatus::Interrupted
        );
        assert!(!interrupted.all_passed && !interrupted.plan_covered);
        assert!(integrity_ok(&interrupted));

        record.lifecycle = None;
        record.all_passed = true;
        save(dir.path(), &record).unwrap();
        let completed = load(dir.path()).unwrap().unwrap();
        settle_interrupted(dir.path(), &record.record_id, token, false).unwrap();
        assert_eq!(load(dir.path()).unwrap().unwrap(), completed);
    }
}
